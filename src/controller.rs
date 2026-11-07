//! Validated Docker observations, durable ownership, and narrowly scoped writes.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::docker::Docker;
use crate::journal::{Journal, JournalGuard, Ownership, Phase};
use crate::lifecycle::{self, CompleteObservation, Evaluation, NetworkObservation};
use crate::policy::{
    Config, GATEWAY_PRIORITY, LABEL_AUTOJOIN, LABEL_AUTOJOIN_VALUE, LABEL_OWNER, LABEL_ROLE,
    LABEL_ROLE_VALUE, ManagedNetwork, Target, has_container_reference, is_full_id, network_name,
    validate_container_summary, validate_network, validate_target,
};

pub use crate::lifecycle::{Action, Report};

#[derive(Debug)]
pub struct ControllerError {
    pub code: &'static str,
    pub message: String,
    pub applied: Vec<Action>,
}

impl fmt::Display for ControllerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for ControllerError {}

fn failure(code: &'static str, message: impl Into<String>) -> anyhow::Error {
    ControllerError {
        code,
        message: message.into(),
        applied: Vec::new(),
    }
    .into()
}

fn coded(code: &'static str, error: anyhow::Error) -> anyhow::Error {
    if error.downcast_ref::<ControllerError>().is_some() {
        error
    } else {
        failure(code, format!("{error:#}"))
    }
}

fn with_applied(error: anyhow::Error, applied: &[Action]) -> anyhow::Error {
    let code = error_code(&error);
    let mut completed = applied.to_vec();
    if let Some(typed) = error.downcast_ref::<ControllerError>() {
        for action in &typed.applied {
            if !completed.contains(action) {
                completed.push(action.clone());
            }
        }
    }
    ControllerError {
        code,
        message: format!("{error:#}"),
        applied: completed,
    }
    .into()
}

pub fn error_code(error: &anyhow::Error) -> &'static str {
    error
        .downcast_ref::<ControllerError>()
        .map_or("OPERATION_FAILED", |typed| typed.code)
}

pub fn error_details(error: &anyhow::Error) -> Value {
    let applied: &[Action] = error
        .downcast_ref::<ControllerError>()
        .map_or(&[], |typed| typed.applied.as_slice());
    json!({
        "applied": applied,
        "post_verified": false
    })
}

struct Snapshot {
    api: String,
    target: Target,
    networks: Vec<NetworkObservation>,
    containers_seen: usize,
    started_at: u64,
    finished_at: u64,
}

impl Snapshot {
    fn observation(&self) -> CompleteObservation<'_> {
        CompleteObservation {
            api: &self.api,
            target: &self.target,
            networks: &self.networks,
            containers_seen: self.containers_seen,
            started_at: self.started_at,
            finished_at: self.finished_at,
        }
    }
}

pub fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn log_event(level: &str, event: &str, details: Value) {
    eprintln!(
        "{}",
        json!({"timestamp": epoch_seconds(), "level": level, "event": event, "details": details})
    );
}

pub fn ensure_state_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).context("cannot create state directory")?;
    let metadata = fs::symlink_metadata(path).context("cannot inspect state directory")?;
    ensure!(metadata.is_dir(), "state path must be a real directory");
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .context("cannot restrict state directory permissions")
}

fn open_lock(cfg: &Config, name: &str) -> Result<File> {
    ensure_state_dir(&cfg.state_dir)?;
    let path = cfg.state_dir.join(name);
    if let Ok(meta) = fs::symlink_metadata(&path) {
        ensure!(meta.is_file(), "lock path must be a regular file");
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .context("cannot open controller lock")
}

pub fn daemon_lock(cfg: &Config) -> Result<File> {
    let file = open_lock(cfg, "daemon.lock")?;
    file.try_lock()
        .context("another daemon holds this state directory")?;
    Ok(file)
}

pub async fn operation_lock(cfg: &Config) -> Result<File> {
    let file = open_lock(cfg, "operations.lock")?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) => {
                ensure!(
                    tokio::time::Instant::now() < deadline,
                    "controller operation lock timed out after 30 seconds"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => return Err(error).context("cannot acquire controller operation lock"),
        }
    }
}

fn load_journal(cfg: &Config) -> Result<Option<Journal>> {
    Journal::load(cfg).map_err(|error| coded("JOURNAL_INVALID", error))
}

fn save_journal(cfg: &Config, journal: &mut Journal) -> Result<()> {
    journal
        .save(cfg)
        .map_err(|error| coded("JOURNAL_WRITE_FAILED", error))
}

/// Hold the operation lock when calling this at daemon startup. A restart
/// preserves ownership, but cannot turn downtime into completed grace time.
pub fn begin_session(cfg: &Config) -> Result<()> {
    reset_grace(cfg)
}

/// An unsuccessful observation breaks the chain of positive absence evidence.
/// Read-only commands must not call this helper.
pub fn reset_grace(cfg: &Config) -> Result<()> {
    if cfg.dry_run {
        return Ok(());
    }
    if let Some(mut journal) = load_journal(cfg)?
        && journal.reset_grace()
    {
        save_journal(cfg, &mut journal)?;
    }
    Ok(())
}

fn full_id(value: &Value) -> Result<String> {
    let id = value
        .get("Id")
        .and_then(Value::as_str)
        .context("Docker object has no Id")?;
    ensure!(
        is_full_id(id),
        "Docker object Id must be full lowercase hexadecimal"
    );
    Ok(id.to_owned())
}

async fn target(docker: &Docker, api: &str, cfg: &Config) -> Result<Target> {
    let raw = docker
        .get(api, &format!("/containers/{}/json", cfg.target_name))
        .await?;
    validate_target(&raw, cfg)
}

async fn same_target(
    docker: &Docker,
    api: &str,
    cfg: &Config,
    expected: &Target,
) -> Result<Target> {
    let current = target(docker, api, cfg).await?;
    if current.id != expected.id {
        return Err(failure(
            "TARGET_CHANGED",
            "Traefik generation changed; obtain a new snapshot",
        ));
    }
    Ok(current)
}

async fn inspect_owned(
    docker: &Docker,
    api: &str,
    expected: &ManagedNetwork,
) -> Result<(ManagedNetwork, Value)> {
    let raw = docker
        .get(api, &format!("/networks/{}", expected.id))
        .await?;
    let actual = validate_network(&raw)?;
    if &actual != expected {
        return Err(failure(
            "NETWORK_CHANGED",
            "network identity changed; no mutation authorized",
        ));
    }
    Ok((actual, raw))
}

fn attached(target: &Target, network: &ManagedNetwork) -> Result<bool> {
    match target.attached.get(&network.name) {
        Some(endpoint) => {
            ensure!(
                endpoint.network_id == network.id,
                "attached network name refers to an unexpected network Id"
            );
            ensure!(
                endpoint.gw_priority == GATEWAY_PRIORITY,
                "managed attachment has wrong gateway priority; no automatic disconnect/reconnect"
            );
            Ok(true)
        }
        None => {
            ensure!(
                !target
                    .attached
                    .values()
                    .any(|endpoint| endpoint.network_id == network.id),
                "managed network is attached under an unexpected name"
            );
            Ok(false)
        }
    }
}

fn endpoint_ids(raw: &Value) -> Result<BTreeSet<String>> {
    let value = raw
        .get("Containers")
        .context("network endpoint inventory is missing")?;
    if value.is_null() {
        return Ok(BTreeSet::new());
    }
    let map = value
        .as_object()
        .context("network endpoint inventory is malformed")?;
    let mut ids = BTreeSet::new();
    for (key, endpoint) in map {
        ensure!(
            is_full_id(key) && endpoint.is_object(),
            "network endpoint has an invalid container Id or shape"
        );
        ids.insert(key.clone());
    }
    Ok(ids)
}

fn endpoint_identity(raw: &Value, container_id: &str) -> Result<Option<String>> {
    let endpoint = raw
        .get("Containers")
        .and_then(|value| value.get(container_id));
    match endpoint.and_then(|value| value.get("EndpointID")) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id)) if id.is_empty() => Ok(None),
        Some(Value::String(id)) if is_full_id(id) => Ok(Some(id.clone())),
        _ => bail!("network inspection EndpointID is malformed"),
    }
}

fn confirmed_endpoint(
    target: &Target,
    network: &ManagedNetwork,
    raw: &Value,
) -> Result<Option<String>> {
    let in_target = attached(target, network)?;
    let endpoints = endpoint_ids(raw)?;
    ensure!(
        in_target == endpoints.contains(&target.id),
        "target and network endpoint inventories disagree"
    );
    let target_id = target
        .attached
        .get(&network.name)
        .and_then(|endpoint| endpoint.endpoint_id.clone());
    let network_id = endpoint_identity(raw, &target.id)?;
    if let (Some(left), Some(right)) = (&target_id, &network_id) {
        ensure!(
            left == right,
            "target and network EndpointID values disagree"
        );
        Ok(target_id)
    } else {
        Ok(None)
    }
}

async fn snapshot(docker: &Docker, cfg: &Config) -> Result<Snapshot> {
    snapshot_inner(docker, cfg)
        .await
        .map_err(|error| coded("OBSERVATION_INCOMPLETE", error))
}

async fn snapshot_inner(docker: &Docker, cfg: &Config) -> Result<Snapshot> {
    let started_at = epoch_seconds();
    let api = docker.negotiate().await?;
    let target = target(docker, &api, cfg).await?;
    let egress_endpoint = target
        .attached
        .get(&cfg.egress_network)
        .context("fixed egress attachment is missing")?;
    let egress = docker
        .get(&api, &format!("/networks/{}", egress_endpoint.network_id))
        .await?;
    ensure!(
        full_id(&egress)? == egress_endpoint.network_id
            && egress.get("Name").and_then(Value::as_str) == Some(&cfg.egress_network)
            && egress.get("Driver").and_then(Value::as_str) == Some("bridge")
            && egress.get("Scope").and_then(Value::as_str) == Some("local")
            && egress.get("Internal").and_then(Value::as_bool) == Some(false),
        "fixed egress network identity/type is invalid"
    );
    ensure!(
        egress
            .get("Labels")
            .and_then(|labels| labels.get(LABEL_AUTOJOIN))
            .and_then(Value::as_str)
            != Some(LABEL_AUTOJOIN_VALUE),
        "fixed egress cannot be an autojoin network"
    );

    let filters = json!({"label": [format!("{LABEL_AUTOJOIN}={LABEL_AUTOJOIN_VALUE}")]});
    let mut url = reqwest::Url::parse("http://docker/networks")?;
    url.query_pairs_mut()
        .append_pair("filters", &filters.to_string());
    let query_path = format!("{}?{}", url.path(), url.query().unwrap_or_default());
    let listed = docker.get(&api, &query_path).await?;
    let entries = listed
        .as_array()
        .context("network inventory is not an array")?;
    ensure!(
        entries.len() <= 1024,
        "managed network inventory exceeds limit"
    );
    let mut inspected = Vec::new();
    let mut names = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for entry in entries {
        let id = full_id(entry)?;
        let raw = docker.get(&api, &format!("/networks/{id}")).await?;
        let network = validate_network(&raw)?;
        ensure!(network.id == id, "network inspect returned a different Id");
        ensure!(
            names.insert(network.name.clone()) && ids.insert(network.id.clone()),
            "duplicate managed network identity"
        );
        if !cfg.is_protected(&network.name) {
            attached(&target, &network)?;
        }
        let endpoints = endpoint_ids(&raw)?;
        let target_endpoint_id = if cfg.is_protected(&network.name) {
            None
        } else {
            confirmed_endpoint(&target, &network, &raw)?
        };
        inspected.push(NetworkObservation {
            network,
            endpoint_container_ids: endpoints,
            target_endpoint_id,
            reference_ids: BTreeSet::new(),
        });
    }

    let all = docker.get(&api, "/containers/json?all=true").await?;
    let containers = all
        .as_array()
        .context("all-container inventory is not an array")?;
    let networks = build_network_inventory(cfg, &target, inspected, containers)?;
    // A final identity read detects replacement while the inventory was built.
    let after = same_target(docker, &api, cfg, &target).await?;
    ensure!(
        after == target,
        "target attachments changed during observation; retry a complete snapshot"
    );
    Ok(Snapshot {
        api,
        target,
        networks,
        containers_seen: containers.len(),
        started_at,
        finished_at: epoch_seconds(),
    })
}

fn build_network_inventory(
    cfg: &Config,
    target: &Target,
    inspected: Vec<NetworkObservation>,
    containers: &[Value],
) -> Result<Vec<NetworkObservation>> {
    ensure!(
        containers.len() <= 16384,
        "all-container inventory exceeds limit"
    );
    let mut by_id = BTreeMap::new();
    for container in containers {
        let id = validate_container_summary(container)?;
        ensure!(
            by_id.insert(id, container).is_none(),
            "duplicate container in all-container inventory"
        );
    }
    ensure!(
        by_id.contains_key(&target.id),
        "all-container inventory omitted the running target"
    );
    let mut networks = Vec::new();
    for mut edge in inspected {
        let network = &edge.network;
        let reference_ids: BTreeSet<String> = by_id
            .iter()
            .filter(|(id, summary)| *id != &target.id && has_container_reference(summary, network))
            .map(|(id, _)| id.clone())
            .collect();
        for endpoint in &edge.endpoint_container_ids {
            ensure!(
                by_id.contains_key(endpoint),
                "all-container inventory omitted a network endpoint"
            );
            ensure!(
                endpoint == &target.id || reference_ids.contains(endpoint),
                "network and container reference inventories disagree"
            );
        }
        if !cfg.is_protected(&network.name) {
            let summary = by_id
                .get(&target.id)
                .context("target summary disappeared")?;
            ensure!(
                has_container_reference(summary, network) == attached(target, network)?,
                "target inspection and summary inventories disagree"
            );
        }
        edge.reference_ids = reference_ids;
        networks.push(edge);
    }
    networks.sort_by(|a, b| a.network.name.cmp(&b.network.name));
    Ok(networks)
}

#[derive(Debug, PartialEq, Eq)]
struct ConnectOutcome {
    written: bool,
    endpoint_id: Option<String>,
}

fn connection_evidence(
    was_attached: bool,
    acknowledged: bool,
    now_attached: bool,
    endpoint_id: Option<String>,
) -> Result<ConnectOutcome> {
    if was_attached {
        return Ok(ConnectOutcome {
            written: false,
            endpoint_id: None,
        });
    }
    if !acknowledged && now_attached {
        return Err(failure(
            "CONNECT_AMBIGUOUS",
            "connect failed but an attachment is now present; it remains unowned",
        ));
    }
    if !acknowledged {
        return Err(failure(
            "CONNECT_FAILED",
            "Docker did not acknowledge connect and no attachment is present",
        ));
    }
    if !now_attached {
        return Err(failure(
            "POST_VERIFY_FAILED",
            "Docker acknowledged connect but the expected endpoint is absent",
        ));
    }
    Ok(ConnectOutcome {
        written: true,
        endpoint_id,
    })
}

async fn connect(
    docker: &Docker,
    cfg: &Config,
    api: &str,
    expected: &Target,
    network: &ManagedNetwork,
) -> Result<ConnectOutcome> {
    inspect_owned(docker, api, network).await?;
    let current = same_target(docker, api, cfg, expected).await?;
    if attached(&current, network)? {
        // An independent writer won the race. Presence is not ownership proof.
        return connection_evidence(true, false, true, None);
    }
    let result = docker
        .post(
            api,
            &format!("/networks/{}/connect", network.id),
            &json!({
                "Container": current.id, "EndpointConfig": {"GwPriority": GATEWAY_PRIORITY}
            }),
        )
        .await;
    if result.is_ok() {
        log_event(
            "info",
            "connect_acknowledged",
            json!({"target_id": current.id, "network_id": network.id, "post_verified": false}),
        );
    }
    let after = same_target(docker, api, cfg, expected).await?;
    let (_, raw) = inspect_owned(docker, api, network).await?;
    let identity = confirmed_endpoint(&after, network, &raw)?;
    let observed_attached = attached(&after, network)?;
    if result.is_err() && observed_attached {
        log_event(
            "warn",
            "connect_ownership_unknown",
            json!({"target_id": after.id, "network_id": network.id}),
        );
    }
    connection_evidence(false, result.is_ok(), observed_attached, identity)
}

fn require_unprotected(cfg: &Config, name: &str) -> Result<()> {
    if cfg.is_protected(name) {
        Err(failure(
            "PROTECTED_NETWORK",
            "operation refused for a protected network",
        ))
    } else {
        Ok(())
    }
}

async fn auto_disconnect(
    docker: &Docker,
    cfg: &Config,
    expected: &Target,
    action: &Action,
    journal: &mut Journal,
) -> Result<()> {
    require_unprotected(cfg, &action.network)?;
    if !cfg.auto_detach {
        return Err(failure(
            "AUTO_DETACH_DISABLED",
            "automatic detach is disabled",
        ));
    }
    let fresh = snapshot(docker, cfg).await?;
    if fresh.target.id != expected.id {
        return Err(failure("TARGET_CHANGED", "target changed before detach"));
    }
    let persisted = load_journal(cfg)?.context("attachment journal disappeared before detach")?;
    if JournalGuard::from_journal(Some(&persisted)) != JournalGuard::from_journal(Some(journal)) {
        return Err(failure(
            "STALE_PLAN",
            "attachment journal changed before detach",
        ));
    }
    let reevaluated = lifecycle::evaluate(cfg, fresh.observation(), Some(&persisted), false, None)?;
    if !reevaluated.report.pending.contains(action) {
        return Err(failure(
            "DETACH_REFUSED",
            "fresh policy/ownership/reference checks no longer permit this detach",
        ));
    }
    let edge = fresh
        .networks
        .iter()
        .find(|edge| Some(&edge.network.id) == action.network_id.as_ref())
        .context("detach network disappeared")?;
    ensure_retirable(docker, &fresh.api, &edge.network, &fresh.target.id, true).await?;
    let (_, raw) = inspect_owned(docker, &fresh.api, &edge.network).await?;
    if endpoint_ids(&raw)?.iter().any(|id| id != &fresh.target.id) {
        return Err(failure(
            "REFERENCES_PRESENT",
            "a new application endpoint appeared immediately before detach",
        ));
    }
    let current = same_target(docker, &fresh.api, cfg, expected).await?;
    if confirmed_endpoint(&current, &edge.network, &raw)? != action.endpoint_id
        || action.endpoint_id.is_none()
    {
        return Err(failure(
            "OWNERSHIP_CHANGED",
            "endpoint identity changed before detach",
        ));
    }
    *journal = reevaluated.journal;
    save_journal(cfg, journal)?;
    // Docker has no expected-EndpointID CAS. The operation lock coordinates
    // controller writers only; independent Docker mutations must be coordinated.
    let response = docker
        .post(
            &fresh.api,
            &format!("/networks/{}/disconnect", edge.network.id),
            &json!({"Container": current.id, "Force": false}),
        )
        .await;
    if response.is_ok() {
        log_event(
            "info",
            "disconnect_acknowledged",
            json!({"target_id": current.id, "network_id": edge.network.id, "post_verified": false}),
        );
    }
    let after = same_target(docker, &fresh.api, cfg, expected).await?;
    let (_, after_raw) = inspect_owned(docker, &fresh.api, &edge.network).await?;
    confirmed_endpoint(&after, &edge.network, &after_raw)?;
    if attached(&after, &edge.network)? {
        response.context("network disconnect failed")?;
        return Err(failure(
            "POST_VERIFY_FAILED",
            "Traefik endpoint remains after acknowledged disconnect",
        ));
    }
    let record = journal
        .attachments
        .get_mut(&edge.network.id)
        .context("detach record disappeared")?;
    record.phase = Phase::Detached;
    record.ever_used = true;
    record.relinquish();
    save_journal(cfg, journal).map_err(|error| {
        if response.is_ok() {
            with_applied(error, std::slice::from_ref(action))
        } else {
            error
        }
    })?;
    if response.is_err() {
        return Err(failure(
            "DISCONNECT_AMBIGUOUS",
            "disconnect failed but the endpoint is now absent; detached state was preserved",
        ));
    }
    log_event(
        "info",
        "network_disconnected",
        json!({"network": edge.network.name, "network_id": edge.network.id, "target_id": after.id, "reason_code": action.reason_code}),
    );
    Ok(())
}

async fn execute(
    docker: &Docker,
    cfg: &Config,
    state: &Snapshot,
    mut evaluated: Evaluation,
    scope: Option<&str>,
    initial_applied: Vec<Action>,
) -> Result<Report> {
    if let Some(app) = scope {
        let name = network_name(app)?;
        evaluated
            .report
            .pending
            .retain(|action| action.network == name);
        evaluated
            .report
            .decisions
            .retain(|decision| decision.network == name);
    }
    if evaluated.report.dry_run || cfg.dry_run {
        return Ok(evaluated.report);
    }
    let mut report = evaluated.report;
    report.applied = initial_applied;
    let mut journal = evaluated.journal;
    save_journal(cfg, &mut journal).map_err(|error| with_applied(error, &report.applied))?;
    let actions = report.pending.clone();
    for action in &actions {
        let result = async {
            let edge = state.networks.iter().find(|edge| Some(&edge.network.id) == action.network_id.as_ref()).context("action network missing from snapshot")?;
            require_unprotected(cfg, &edge.network.name)?;
            match action.operation.as_str() {
                "connect" => {
                    // Commit an unowned intent before the Docker write. A crash
                    // or failed POST can never retroactively establish ownership.
                    let record = journal.attachments.get_mut(&edge.network.id).context("connect record missing")?;
                    record.relinquish();
                    record.target_id = state.target.id.clone();
                    save_journal(cfg, &mut journal)?;
                    let outcome = connect(docker, cfg, &state.api, &state.target, &edge.network).await?;
                    if outcome.written {
                        report.applied.push(action.clone());
                        if let Some(endpoint_id) = outcome.endpoint_id {
                            let record = journal.attachments.get_mut(&edge.network.id).context("connect record missing")?;
                            record.ownership = Ownership::Connected;
                            record.endpoint_id = Some(endpoint_id);
                            save_journal(cfg, &mut journal)?;
                            log_event("info", "network_connected", json!({"target_id": state.target.id, "network": edge.network.name, "network_id": edge.network.id}));
                        } else {
                            return Err(failure("ENDPOINT_ID_UNAVAILABLE", "connection exists, but complete endpoint identity is unavailable; attachment remains unowned"));
                        }
                    }
                }
                "disconnect" if scope.is_none() => {
                    auto_disconnect(docker, cfg, &state.target, action, &mut journal).await?;
                    report.applied.push(action.clone());
                }
                _ => return Err(failure("ACTION_REFUSED", "action is outside this operation's permitted scope")),
            }
            Ok(())
        }.await;
        if let Err(error) = result {
            return Err(with_applied(error, &report.applied));
        }
    }
    if actions.is_empty() {
        report.post_verified = true;
        report.journal = JournalGuard::from_journal(Some(&journal));
        return Ok(report);
    }
    let after = snapshot(docker, cfg)
        .await
        .map_err(|error| with_applied(error, &report.applied))?;
    if after.target.id != state.target.id {
        return Err(with_applied(
            failure("TARGET_CHANGED", "target changed during operation"),
            &report.applied,
        ));
    }
    let mut final_report =
        lifecycle::evaluate(cfg, after.observation(), Some(&journal), false, None)?.report;
    if let Some(app) = scope {
        let name = network_name(app)?;
        final_report.pending.retain(|action| action.network == name);
        final_report
            .decisions
            .retain(|decision| decision.network == name);
    }
    final_report.applied = report.applied;
    final_report.post_verified = final_report.pending.is_empty();
    Ok(final_report)
}

pub async fn reconcile(docker: &Docker, cfg: &Config, dry_run: bool) -> Result<Report> {
    let result = async {
        let journal = load_journal(cfg)?;
        let state = snapshot(docker, cfg).await?;
        let evaluated = lifecycle::evaluate(
            cfg,
            state.observation(),
            journal.as_ref(),
            dry_run || cfg.dry_run,
            None,
        )?;
        execute(docker, cfg, &state, evaluated, None, Vec::new()).await
    }
    .await;
    if result.is_err()
        && !dry_run
        && !cfg.dry_run
        && let Err(error) = reset_grace(cfg)
    {
        log_event(
            "error",
            "grace_reset_failed",
            json!({"code": error_code(&error)}),
        );
    }
    result
}

pub async fn status(docker: &Docker, cfg: &Config) -> Result<Report> {
    reconcile(docker, cfg, true).await
}

async fn apply_plan_inner(docker: &Docker, cfg: &Config, plan: &Report) -> Result<Report> {
    let journal = load_journal(cfg)?;
    let state = snapshot(docker, cfg).await?;
    let mut evaluated =
        lifecycle::evaluate(cfg, state.observation(), journal.as_ref(), true, None)?;
    if !lifecycle::same_plan(plan, &evaluated.report, epoch_seconds()) {
        return Err(failure(
            "STALE_PLAN",
            "saved plan is expired, changed, or inconsistent with current policy/identities/state; obtain a new plan",
        ));
    }
    evaluated.report.dry_run = cfg.dry_run;
    let result = execute(docker, cfg, &state, evaluated, None, Vec::new()).await;
    if result.is_err() && !cfg.dry_run {
        let _ = reset_grace(cfg);
    }
    result
}

async fn prepare_inner(docker: &Docker, cfg: &Config, app: &str) -> Result<Report> {
    let name = network_name(app)?;
    require_unprotected(cfg, &name)?;
    let journal = load_journal(cfg)?;
    let state = snapshot(docker, cfg).await?;
    let found = docker
        .get_optional(&state.api, &format!("/networks/{name}"))
        .await?;
    if let Some(raw) = found {
        let network = validate_network(&raw)?;
        ensure!(
            network.name == name && network.owner == app,
            "existing network ownership does not match requested application"
        );
        ensure!(
            state.networks.iter().any(|edge| edge.network == network),
            "preparation network changed during observation"
        );
        let mut evaluated = lifecycle::evaluate(
            cfg,
            state.observation(),
            journal.as_ref(),
            cfg.dry_run,
            Some(app),
        )?;
        // Preparing an application is a reservation, never a request to collect
        // an unrelated application's attachment.
        evaluated
            .report
            .pending
            .retain(|action| action.operation == "connect");
        return execute(docker, cfg, &state, evaluated, Some(app), Vec::new()).await;
    }
    let mut evaluated = lifecycle::evaluate(
        cfg,
        state.observation(),
        journal.as_ref(),
        cfg.dry_run,
        None,
    )?;
    if cfg.dry_run {
        evaluated.report.pending = vec![
            Action {
                operation: "create".into(),
                network: name.clone(),
                network_id: None,
                endpoint_id: None,
                reason_code: "PREPARE_REQUESTED".into(),
            },
            Action {
                operation: "connect".into(),
                network: name,
                network_id: None,
                endpoint_id: None,
                reason_code: "PREPARE_REQUESTED".into(),
            },
        ];
        evaluated.report.decisions.clear();
        return Ok(evaluated.report);
    }
    save_journal(cfg, &mut evaluated.journal)?;
    same_target(docker, &state.api, cfg, &state.target).await?;
    let body = json!({"Name": name, "Driver": "bridge", "Scope": "local", "Internal": false,
        "Labels": {(LABEL_AUTOJOIN): LABEL_AUTOJOIN_VALUE, (LABEL_ROLE): LABEL_ROLE_VALUE, (LABEL_OWNER): app}});
    let response = docker.post(&state.api, "/networks/create", &body).await?;
    let id = full_id(&response)?;
    let raw = docker.get(&state.api, &format!("/networks/{id}")).await?;
    let network = validate_network(&raw)?;
    ensure!(
        network.id == id && network.name == name && network.owner == app,
        "new network identity is unexpected; inspect it manually"
    );
    let created = Action::new("create", &network, "PREPARE_REQUESTED");
    log_event(
        "info",
        "network_created",
        json!({"network": name, "network_id": id}),
    );
    let after = snapshot(docker, cfg)
        .await
        .map_err(|error| with_applied(error, std::slice::from_ref(&created)))?;
    if after.target.id != state.target.id {
        return Err(with_applied(
            failure("TARGET_CHANGED", "target changed after network creation"),
            &[created],
        ));
    }
    let evaluated = lifecycle::evaluate(
        cfg,
        after.observation(),
        Some(&evaluated.journal),
        false,
        Some(app),
    )?;
    execute(docker, cfg, &after, evaluated, Some(app), vec![created]).await
}

async fn ensure_retirable(
    docker: &Docker,
    api: &str,
    network: &ManagedNetwork,
    target_id: &str,
    allow_target_endpoint: bool,
) -> Result<()> {
    let (_, raw) = inspect_owned(docker, api, network).await?;
    for endpoint in endpoint_ids(&raw)? {
        ensure!(
            allow_target_endpoint && endpoint == target_id,
            "network still has another container endpoint"
        );
    }
    let all = docker.get(api, "/containers/json?all=true").await?;
    let containers = all
        .as_array()
        .context("container inventory is not an array")?;
    let mut ids = BTreeSet::new();
    for container in containers {
        let id = validate_container_summary(container)?;
        ensure!(
            ids.insert(id.clone()),
            "duplicate container reference summary"
        );
        if id == target_id && allow_target_endpoint {
            continue;
        }
        if has_container_reference(container, network) {
            return Err(failure(
                "REFERENCES_PRESENT",
                "network is still referenced by a container, including stopped containers",
            ));
        }
    }
    ensure!(
        ids.contains(target_id),
        "all-container inventory omitted the target"
    );
    Ok(())
}

async fn retire_inner(docker: &Docker, cfg: &Config, app: &str) -> Result<Report> {
    let name = network_name(app)?;
    require_unprotected(cfg, &name)?;
    let journal = load_journal(cfg)?;
    let state = snapshot(docker, cfg).await?;
    let mut evaluated = lifecycle::evaluate(
        cfg,
        state.observation(),
        journal.as_ref(),
        cfg.dry_run,
        None,
    )?;
    evaluated.report.pending.clear();
    evaluated
        .report
        .decisions
        .retain(|decision| decision.network == name);
    let Some(edge) = state.networks.iter().find(|edge| edge.network.name == name) else {
        // A same-name unmanaged network must not be mistaken for absence.
        if docker
            .get_optional(&state.api, &format!("/networks/{name}"))
            .await?
            .is_some()
        {
            return Err(failure(
                "NETWORK_NOT_MANAGED",
                "retirement name exists outside validated managed inventory",
            ));
        }
        evaluated.report.post_verified = !cfg.dry_run;
        return Ok(evaluated.report);
    };
    let network = &edge.network;
    ensure!(
        network.owner == app,
        "network owner differs from retirement request"
    );
    let was_attached = attached(&state.target, network)?;
    ensure_retirable(docker, &state.api, network, &state.target.id, true).await?;
    if was_attached {
        evaluated
            .report
            .pending
            .push(Action::new("disconnect", network, "RETIRE_REQUESTED"));
    }
    evaluated
        .report
        .pending
        .push(Action::new("remove", network, "RETIRE_REQUESTED"));
    if cfg.dry_run {
        return Ok(evaluated.report);
    }
    let mut journal = evaluated.journal;
    // Persist the explicit retirement intention before Docker writes. Even a
    // disconnect/delete interruption must not make an empty edge reconnect on
    // the next connect-only pass.
    let retiring = journal
        .attachments
        .get_mut(&network.id)
        .context("retirement journal record missing")?;
    retiring.phase = Phase::Detached;
    retiring.ever_used = true;
    retiring.relinquish();
    save_journal(cfg, &mut journal)?;
    let mut report = evaluated.report;
    let outcome = async {
        let current = same_target(docker, &state.api, cfg, &state.target).await?;
        if attached(&current, network)? {
            ensure_retirable(docker, &state.api, network, &state.target.id, true).await?;
            // Explicit retire authorizes this exact unused managed network's
            // removal; unlike automatic detach, it need not adopt an old edge.
            docker
                .post(
                    &state.api,
                    &format!("/networks/{}/disconnect", network.id),
                    &json!({"Container": current.id, "Force": false}),
                )
                .await?;
            let after = same_target(docker, &state.api, cfg, &state.target).await?;
            ensure!(
                !attached(&after, network)?,
                "Traefik endpoint remains after disconnect"
            );
            report
                .applied
                .push(Action::new("disconnect", network, "RETIRE_REQUESTED"));
            let record = journal
                .attachments
                .get_mut(&network.id)
                .context("retirement journal record missing")?;
            record.phase = Phase::Detached;
            record.ever_used = true;
            record.relinquish();
            save_journal(cfg, &mut journal)?;
        }
        same_target(docker, &state.api, cfg, &state.target).await?;
        ensure_retirable(docker, &state.api, network, &state.target.id, false).await?;
        docker
            .delete(&state.api, &format!("/networks/{}", network.id))
            .await?;
        ensure!(
            docker
                .get_optional(&state.api, &format!("/networks/{}", network.id))
                .await?
                .is_none(),
            "network remains after removal"
        );
        report
            .applied
            .push(Action::new("remove", network, "RETIRE_REQUESTED"));
        journal.attachments.remove(&network.id);
        save_journal(cfg, &mut journal)?;
        log_event(
            "info",
            "network_removed",
            json!({"network": network.name, "network_id": network.id}),
        );
        Ok(())
    }
    .await;
    if let Err(error) = outcome {
        return Err(with_applied(error, &report.applied));
    }
    let after = snapshot(docker, cfg)
        .await
        .map_err(|error| with_applied(error, &report.applied))?;
    if after.target.id != state.target.id
        || after.networks.iter().any(|edge| edge.network.name == name)
    {
        return Err(with_applied(
            failure(
                "NETWORK_CHANGED",
                "target or retirement network changed during post-verification",
            ),
            &report.applied,
        ));
    }
    let mut final_report =
        lifecycle::evaluate(cfg, after.observation(), Some(&journal), false, None)?.report;
    final_report.decisions.clear();
    final_report.pending.clear();
    final_report.applied = report.applied;
    final_report.post_verified = true;
    Ok(final_report)
}

async fn adopt_inner(
    docker: &Docker,
    cfg: &Config,
    app: &str,
    expected_target_id: &str,
    expected_network_id: &str,
    expected_endpoint_id: &str,
) -> Result<Report> {
    let name = network_name(app)?;
    require_unprotected(cfg, &name)?;
    if ![
        expected_target_id,
        expected_network_id,
        expected_endpoint_id,
    ]
    .iter()
    .all(|id| is_full_id(id))
    {
        return Err(failure(
            "ADOPT_REFUSED",
            "adoption requires three full lowercase 64-character IDs",
        ));
    }
    let previous = load_journal(cfg)?;
    let state = snapshot(docker, cfg).await?;
    let edge = state
        .networks
        .iter()
        .find(|edge| edge.network.name == name)
        .ok_or_else(|| {
            failure(
                "ADOPT_REFUSED",
                "adoption network is not in managed inventory",
            )
        })?;
    if state.target.id != expected_target_id
        || edge.network.id != expected_network_id
        || edge.target_endpoint_id.as_deref() != Some(expected_endpoint_id)
    {
        return Err(failure(
            "ADOPT_REFUSED",
            "expected target, network or endpoint identity does not match current state",
        ));
    }
    let mut evaluated = lifecycle::evaluate(
        cfg,
        state.observation(),
        previous.as_ref(),
        cfg.dry_run,
        None,
    )?;
    evaluated.report.pending = vec![Action {
        operation: "adopt".into(),
        network: name,
        network_id: Some(expected_network_id.to_owned()),
        endpoint_id: Some(expected_endpoint_id.to_owned()),
        reason_code: "ADOPT_REQUESTED".into(),
    }];
    evaluated
        .report
        .decisions
        .retain(|decision| decision.network_id == expected_network_id);
    if cfg.dry_run {
        return Ok(evaluated.report);
    }
    let (_, raw) = inspect_owned(docker, &state.api, &edge.network).await?;
    let current = same_target(docker, &state.api, cfg, &state.target).await?;
    if confirmed_endpoint(&current, &edge.network, &raw)?.as_deref() != Some(expected_endpoint_id) {
        return Err(failure(
            "ADOPT_REFUSED",
            "endpoint changed before ownership adoption",
        ));
    }
    let record = evaluated
        .journal
        .attachments
        .get_mut(expected_network_id)
        .context("adoption record missing")?;
    record.adopt(expected_target_id, expected_endpoint_id)?;
    save_journal(cfg, &mut evaluated.journal)?;
    let action = evaluated.report.pending.remove(0);
    evaluated.report.applied.push(action);
    evaluated.report.post_verified = true;
    evaluated.report.journal = JournalGuard::from_journal(Some(&evaluated.journal));
    for decision in &mut evaluated.report.decisions {
        decision.phase = evaluated.journal.attachments[expected_network_id].phase;
        decision.ownership = Ownership::Adopted;
        decision.action = "keep".into();
        decision.reason_code = "ADOPT_REQUESTED".into();
        decision.desired_attached = true;
        decision.unused_since = None;
        decision.empty_observations = 0;
    }
    log_event(
        "info",
        "attachment_adopted",
        json!({"target_id": expected_target_id, "network_id": expected_network_id, "endpoint_id": expected_endpoint_id}),
    );
    Ok(evaluated.report)
}

fn mutation_result(cfg: &Config, result: Result<Report>) -> Result<Report> {
    // A stale plan is a rejected proposal, not a failed observation. All other
    // mutating-operation failures conservatively break absence continuity,
    // including errors before the first Docker write.
    if result
        .as_ref()
        .is_err_and(|error| error_code(error) != "STALE_PLAN")
        && !cfg.dry_run
        && let Err(error) = reset_grace(cfg)
    {
        log_event(
            "error",
            "grace_reset_failed",
            json!({"code": error_code(&error)}),
        );
    }
    result
}

pub async fn apply_plan(docker: &Docker, cfg: &Config, plan: &Report) -> Result<Report> {
    mutation_result(cfg, apply_plan_inner(docker, cfg, plan).await)
}

pub async fn prepare(docker: &Docker, cfg: &Config, app: &str) -> Result<Report> {
    mutation_result(cfg, prepare_inner(docker, cfg, app).await)
}

pub async fn retire(docker: &Docker, cfg: &Config, app: &str) -> Result<Report> {
    mutation_result(cfg, retire_inner(docker, cfg, app).await)
}

pub async fn adopt(
    docker: &Docker,
    cfg: &Config,
    app: &str,
    expected_target_id: &str,
    expected_network_id: &str,
    expected_endpoint_id: &str,
) -> Result<Report> {
    mutation_result(
        cfg,
        adopt_inner(
            docker,
            cfg,
            app,
            expected_target_id,
            expected_network_id,
            expected_endpoint_id,
        )
        .await,
    )
}

pub fn capabilities() -> Value {
    json!({
        "schema_version": 1,
        "controller_version": env!("CARGO_PKG_VERSION"),
        "commands": ["run", "once", "plan", "status", "doctor", "capabilities", "healthcheck", "prepare", "retire", "apply", "adopt"],
        "report_schema_version": lifecycle::REPORT_SCHEMA_VERSION,
        "plan_ttl_secs": lifecycle::PLAN_TTL_SECS,
        "read_only_commands": ["plan", "status", "doctor", "capabilities", "healthcheck"],
        "automatic_operations": ["connect", "disconnect"],
        "explicit_operations": ["create", "remove", "adopt"],
        "auto_detach_default": false,
        "detach_grace_default_secs": 300,
        "journal_file": crate::journal::JOURNAL_FILE,
        "ownership_proof": ["controller_id", "target_id", "network_id", "endpoint_id", "acknowledged_connect_or_explicit_adopt"],
        "lifecycle_states": ["prepared", "in_use", "unused_pending", "detached"],
        "reason_codes": ["PROTECTED_NETWORK", "REFERENCED", "ATTACHMENT_MISSING", "PREPARED_NEVER_USED", "ALREADY_DETACHED", "AUTO_DETACH_DISABLED", "OWNERSHIP_UNKNOWN", "ENDPOINT_ID_UNAVAILABLE", "GRACE_PENDING", "UNUSED_GRACE_ELAPSED", "PREPARE_REQUESTED", "RETIRE_REQUESTED", "ADOPT_REQUESTED"],
        "safety": {"all_container_references": true, "stopped_containers_block_detach": true, "force_disconnect": false, "automatic_network_delete": false, "plan_is_authority": false, "docker_endpoint_cas_available": false, "prepare_scope": "requested_application_only", "doctor_probes_application_http": false},
        "configuration": {"TNC_AUTO_DETACH": "false", "TNC_DETACH_GRACE_SECS": "300", "TNC_PROTECTED_NETWORKS": "comma-separated exact names; egress always protected", "TNC_STATE_DIR": "/run/tnc; persist this directory for ownership continuity"},
        "docker_api": {"minimum": "1.48", "maximum": "1.56", "transport": "unix_socket_only"}
    })
}
#[derive(Serialize, Deserialize)]
pub struct Health {
    pub schema: u32,
    pub version: String,
    pub updated_at: u64,
    pub ready: bool,
    pub target_project: String,
    pub target_name: String,
    pub target_service: String,
    pub egress_network: String,
    pub dry_run: bool,
    pub auto_detach: bool,
    pub detach_grace_secs: u64,
    pub protected_networks: BTreeSet<String>,
    pub detail: String,
}

pub fn write_health(cfg: &Config, ready: bool, detail: &str) -> Result<()> {
    let health = Health {
        schema: 1,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        updated_at: epoch_seconds(),
        ready,
        target_project: cfg.target_project.clone(),
        target_name: cfg.target_name.clone(),
        target_service: cfg.target_service.clone(),
        egress_network: cfg.egress_network.clone(),
        dry_run: cfg.dry_run,
        auto_detach: cfg.auto_detach,
        detach_grace_secs: cfg.detach_grace_secs,
        protected_networks: cfg.protected_networks.clone(),
        detail: detail.chars().take(512).collect(),
    };
    let path = cfg
        .state_dir
        .join(format!(".health.{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .context("cannot write health state")?;
    file.write_all(&serde_json::to_vec(&health)?)?;
    file.sync_all()?;
    fs::rename(path, cfg.state_dir.join("health.json")).context("cannot commit health state")
}

pub fn check_health(cfg: &Config) -> Result<()> {
    let path = cfg.state_dir.join("health.json");
    let metadata = fs::symlink_metadata(&path).context("health state is unavailable")?;
    ensure!(
        metadata.is_file() && metadata.len() <= 8192,
        "health state file is invalid"
    );
    let health: Health = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        health.schema == 1
            && health.version == env!("CARGO_PKG_VERSION")
            && health.target_project == cfg.target_project
            && health.target_name == cfg.target_name
            && health.target_service == cfg.target_service
            && health.egress_network == cfg.egress_network
            && health.dry_run == cfg.dry_run
            && health.auto_detach == cfg.auto_detach
            && health.detach_grace_secs == cfg.detach_grace_secs
            && health.protected_networks == cfg.protected_networks,
        "health state identity mismatch"
    );
    ensure!(health.ready, "controller reconciliation is not ready");
    let age = epoch_seconds()
        .checked_sub(health.updated_at)
        .context("health timestamp is in the future")?;
    ensure!(age <= cfg.health_max_age_secs, "health state is stale");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "tnc-state-test-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).expect("create isolated test directory");
            Self(path)
        }

        fn config(&self) -> Config {
            Config {
                socket_path: self.0.join("docker.sock"),
                state_dir: self.0.clone(),
                target_name: "traefik".into(),
                target_project: "test".into(),
                target_service: "traefik".into(),
                egress_network: "traefik_egress".into(),
                interval_secs: 30,
                api_timeout_secs: 10,
                health_max_age_secs: 90,
                dry_run: false,
                auto_detach: false,
                detach_grace_secs: 300,
                protected_networks: BTreeSet::from(["traefik_egress".into()]),
            }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn health_requires_ready_recent_state_and_matching_identity() {
        let directory = TestDirectory::new();
        let cfg = directory.config();
        assert!(check_health(&cfg).is_err());
        write_health(&cfg, false, "initializing").unwrap();
        assert!(check_health(&cfg).is_err());
        write_health(&cfg, true, "complete").unwrap();
        check_health(&cfg).unwrap();
        let mut different = cfg.clone();
        different.target_service = "another-service".into();
        assert!(check_health(&different).is_err());
        different = cfg.clone();
        different.egress_network = "another-egress".into();
        assert!(check_health(&different).is_err());
        different = cfg.clone();
        different.dry_run = true;
        assert!(check_health(&different).is_err());
        different = cfg.clone();
        different.auto_detach = true;
        assert!(check_health(&different).is_err());
        different = cfg.clone();
        different.detach_grace_secs += 1;
        assert!(check_health(&different).is_err());
        different = cfg.clone();
        different.protected_networks.insert("extra_control".into());
        assert!(check_health(&different).is_err());
        let path = cfg.state_dir.join("health.json");
        let mut health: Health = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        health.updated_at = epoch_seconds() - cfg.health_max_age_secs - 5;
        fs::write(&path, serde_json::to_vec(&health).unwrap()).unwrap();
        assert!(check_health(&cfg).is_err());
        health.updated_at = epoch_seconds() + 3600;
        fs::write(&path, serde_json::to_vec(&health).unwrap()).unwrap();
        assert!(check_health(&cfg).is_err());
    }

    #[test]
    fn malformed_or_symlinked_health_and_state_are_rejected() {
        let directory = TestDirectory::new();
        let cfg = directory.config();
        let path = cfg.state_dir.join("health.json");
        fs::write(&path, b"not JSON").unwrap();
        assert!(check_health(&cfg).is_err());
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("elsewhere", &path).unwrap();
        assert!(check_health(&cfg).is_err());
        let link = directory.0.join("linked-state");
        std::os::unix::fs::symlink(&directory.0, &link).unwrap();
        assert!(ensure_state_dir(&link).is_err());
    }

    #[tokio::test]
    async fn daemon_lease_is_exclusive_and_separate_from_operation_lock() {
        let directory = TestDirectory::new();
        let cfg = directory.config();
        let daemon = daemon_lock(&cfg).unwrap();
        assert!(daemon_lock(&cfg).is_err());
        let operation = operation_lock(&cfg).await.unwrap();
        let contender = open_lock(&cfg, "operations.lock").unwrap();
        assert!(matches!(
            contender.try_lock(),
            Err(TryLockError::WouldBlock)
        ));
        drop(operation);
        contender.try_lock().unwrap();
        drop(daemon);
        daemon_lock(&cfg).unwrap();
        assert_eq!(
            fs::metadata(&directory.0).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn retirement_endpoint_inventory_must_be_complete_and_canonical() {
        assert!(endpoint_ids(&json!({})).is_err());
        assert!(endpoint_ids(&json!({"Containers": []})).is_err());
        assert!(endpoint_ids(&json!({"Containers": {"short": {}}})).is_err());
        assert!(
            endpoint_ids(&json!({"Containers": null}))
                .unwrap()
                .is_empty()
        );
        let id = "a".repeat(64);
        let ids = endpoint_ids(&json!({"Containers": {(&id): {}}})).unwrap();
        assert_eq!(ids, BTreeSet::from([id]));
    }

    #[test]
    fn attachment_identity_requires_agreement_of_both_inspection_sources() {
        use crate::test_support::{id, network, target};
        let current = target(true);
        let raw = json!({"Containers": {id('b'): {"EndpointID": id('1')}}});
        assert_eq!(
            confirmed_endpoint(&current, &network(), &raw).unwrap(),
            Some(id('1'))
        );
        assert_eq!(
            confirmed_endpoint(&current, &network(), &json!({"Containers": {id('b'): {}}}))
                .unwrap(),
            None
        );
        assert!(
            confirmed_endpoint(
                &current,
                &network(),
                &json!({"Containers": {id('b'): {"EndpointID": id('2')}}})
            )
            .is_err()
        );
        assert!(confirmed_endpoint(&current, &network(), &json!({"Containers": {}})).is_err());
        assert!(confirmed_endpoint(&target(false), &network(), &raw).is_err());
        assert!(
            confirmed_endpoint(
                &current,
                &network(),
                &json!({"Containers": {id('b'): {"EndpointID": "short"}}})
            )
            .is_err()
        );
    }

    fn summary(container_id: &str, networks: Value) -> Value {
        json!({"Id": container_id, "State": "exited", "NetworkSettings": {"Networks": networks}})
    }

    fn inspected_edge(with_consumer: bool) -> NetworkObservation {
        use crate::test_support::{id, network};
        let mut endpoints = BTreeSet::from([id('b')]);
        if with_consumer {
            endpoints.insert(id('9'));
        }
        NetworkObservation {
            network: network(),
            endpoint_container_ids: endpoints,
            target_endpoint_id: Some(id('1')),
            reference_ids: BTreeSet::new(),
        }
    }

    #[test]
    fn complete_inventory_counts_stopped_name_only_references() {
        use crate::test_support::{config, id, network, target};
        let containers = vec![
            summary(
                &id('b'),
                json!({network().name: {"NetworkID": network().id}}),
            ),
            summary(&id('9'), json!({network().name: {"NetworkID": ""}})),
        ];
        let edges = build_network_inventory(
            &config(),
            &target(true),
            vec![inspected_edge(false)],
            &containers,
        )
        .unwrap();
        assert_eq!(edges[0].reference_ids, BTreeSet::from([id('9')]));
    }

    #[test]
    fn malformed_or_partial_inventory_is_never_an_empty_reference_set() {
        use crate::test_support::{config, id, network, target};
        let valid_target = summary(
            &id('b'),
            json!({network().name: {"NetworkID": network().id}}),
        );
        let cases = vec![
            Vec::new(),
            vec![valid_target.clone(), valid_target.clone()],
            vec![valid_target.clone(), json!({"Id": id('9')})],
            vec![valid_target.clone(), summary("short", json!({}))],
            vec![
                valid_target.clone(),
                summary(&id('9'), json!({"unrelated": {"NetworkID": "short"}})),
            ],
        ];
        for containers in cases {
            assert!(
                build_network_inventory(
                    &config(),
                    &target(true),
                    vec![inspected_edge(false)],
                    &containers
                )
                .is_err()
            );
        }
        assert!(
            build_network_inventory(
                &config(),
                &target(true),
                vec![inspected_edge(true)],
                std::slice::from_ref(&valid_target)
            )
            .is_err()
        );
        let inconsistent = vec![valid_target, summary(&id('9'), json!({}))];
        assert!(
            build_network_inventory(
                &config(),
                &target(true),
                vec![inspected_edge(true)],
                &inconsistent
            )
            .is_err()
        );
    }

    #[test]
    fn ambiguous_and_preexisting_connections_do_not_grant_ownership() {
        use crate::test_support::id;
        let preexisting = connection_evidence(true, false, true, Some(id('1'))).unwrap();
        assert!(!preexisting.written);
        assert_eq!(preexisting.endpoint_id, None);
        let ambiguous = connection_evidence(false, false, true, Some(id('1'))).unwrap_err();
        assert_eq!(error_code(&ambiguous), "CONNECT_AMBIGUOUS");
        let failed = connection_evidence(false, false, false, None).unwrap_err();
        assert_eq!(error_code(&failed), "CONNECT_FAILED");
        let unverified = connection_evidence(false, true, false, None).unwrap_err();
        assert_eq!(error_code(&unverified), "POST_VERIFY_FAILED");
        let valid = connection_evidence(false, true, true, Some(id('1'))).unwrap();
        assert!(valid.written);
        assert_eq!(valid.endpoint_id, Some(id('1')));
        let missing_identity = connection_evidence(false, true, true, None).unwrap();
        assert!(missing_identity.written);
        assert_eq!(missing_identity.endpoint_id, None);
    }

    #[test]
    fn partial_operation_errors_preserve_completed_actions_and_stable_code() {
        use crate::test_support::network;
        let action = Action::new("connect", &network(), "ATTACHMENT_MISSING");
        let error = with_applied(
            failure("POST_VERIFY_FAILED", "later verification failed"),
            std::slice::from_ref(&action),
        );
        let contextual = error.context("reconcile");
        assert_eq!(error_code(&contextual), "POST_VERIFY_FAILED");
        assert_eq!(
            error_details(&contextual)["applied"][0]["operation"],
            "connect"
        );
        let error = with_applied(contextual, &[action]);
        assert_eq!(
            error_details(&error)["applied"].as_array().unwrap().len(),
            1
        );
        assert_eq!(error_details(&error)["post_verified"], false);
    }

    #[test]
    fn mutation_observation_failure_resets_grace_but_stale_proposal_does_not() {
        use crate::journal::AttachmentRecord;
        use crate::test_support::{id, network};
        let directory = TestDirectory::new();
        let cfg = directory.config();
        let mut journal = Journal::new(&cfg).unwrap();
        let mut record = AttachmentRecord::new(&network(), &id('b'));
        record.ownership = Ownership::Connected;
        record.endpoint_id = Some(id('1'));
        record.phase = Phase::UnusedPending;
        record.ever_used = true;
        record.unused_since = Some(100);
        record.last_observed_at = Some(160);
        record.empty_observations = 3;
        journal.attachments.insert(network().id, record);
        journal.save(&cfg).unwrap();
        assert!(mutation_result(&cfg, Err(failure("STALE_PLAN", "old proposal"))).is_err());
        assert_eq!(Journal::load(&cfg).unwrap().unwrap(), journal);
        assert!(
            mutation_result(
                &cfg,
                Err(failure("OBSERVATION_INCOMPLETE", "container query failed"))
            )
            .is_err()
        );
        let after = Journal::load(&cfg).unwrap().unwrap();
        assert_eq!(after.attachments[&network().id].unused_since, None);
        assert_eq!(after.attachments[&network().id].empty_observations, 0);
        assert_eq!(
            after.attachments[&network().id].ownership,
            Ownership::Connected
        );
    }
}
