//! Trust-boundary validation for the network controller.
//!
//! This module does not call Docker or mutate the environment. In particular,
//! network labels select candidates; they do not bypass inspection of the
//! network's identity and properties before a write.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const LABEL_AUTOJOIN: &str = "com.nofmem.traefik.autojoin";
pub const LABEL_AUTOJOIN_VALUE: &str = "true";
pub const LABEL_ROLE: &str = "com.nofmem.traefik.role";
pub const LABEL_ROLE_VALUE: &str = "application-edge";
pub const LABEL_OWNER: &str = "com.nofmem.traefik.owner";
pub const NETWORK_PREFIX: &str = "traefik_app_";
pub const GATEWAY_PRIORITY: i64 = -1;
pub const EGRESS_PRIORITY: i64 = 100;

const MAX_APP_LENGTH: usize = 48;
const MAX_INTERVAL_SECS: u64 = 86_400;
const MAX_API_TIMEOUT_SECS: u64 = 300;
const MAX_HEALTH_AGE_SECS: u64 = 604_800;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub socket_path: PathBuf,
    pub state_dir: PathBuf,
    pub target_name: String,
    pub target_project: String,
    pub target_service: String,
    pub egress_network: String,
    pub interval_secs: u64,
    pub api_timeout_secs: u64,
    pub health_max_age_secs: u64,
    pub dry_run: bool,
    pub auto_detach: bool,
    pub detach_grace_secs: u64,
    pub protected_networks: BTreeSet<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|key| match env::var(key) {
            Ok(value) => Ok(Some(value)),
            Err(env::VarError::NotPresent) => Ok(None),
            Err(env::VarError::NotUnicode(_)) => {
                bail!("{key} must contain valid UTF-8")
            }
        })
    }

    // Injected reads let tests cover configuration without changing process-
    // global environment variables or racing other tests.
    fn from_lookup<F>(mut lookup: F) -> Result<Self>
    where
        F: FnMut(&str) -> Result<Option<String>>,
    {
        let socket_path = absolute_path(
            setting(&mut lookup, "TNC_SOCKET_PATH", "/var/run/docker.sock")?,
            "TNC_SOCKET_PATH",
        )?;
        let state_dir = absolute_path(
            setting(&mut lookup, "TNC_STATE_DIR", "/run/tnc")?,
            "TNC_STATE_DIR",
        )?;
        let target_name = setting(&mut lookup, "TNC_TARGET_NAME", "traefik")?;
        validate_object_name(&target_name, "TNC_TARGET_NAME")?;

        let target_project = lookup("TNC_TARGET_PROJECT")?
            .context("TNC_TARGET_PROJECT is required; specify the actual Docker Compose project")?;
        validate_project_name(&target_project)?;

        let target_service = setting(&mut lookup, "TNC_TARGET_SERVICE", "traefik")?;
        validate_service_name(&target_service)?;

        let egress_network = setting(&mut lookup, "TNC_EGRESS_NETWORK", "traefik_egress")?;
        validate_object_name(&egress_network, "TNC_EGRESS_NETWORK")?;

        let interval_secs = positive_seconds(
            &setting(&mut lookup, "TNC_INTERVAL_SECS", "30")?,
            "TNC_INTERVAL_SECS",
            MAX_INTERVAL_SECS,
        )?;
        let api_timeout_secs = positive_seconds(
            &setting(&mut lookup, "TNC_API_TIMEOUT_SECS", "10")?,
            "TNC_API_TIMEOUT_SECS",
            MAX_API_TIMEOUT_SECS,
        )?;
        let health_max_age_secs = positive_seconds(
            &setting(&mut lookup, "TNC_HEALTH_MAX_AGE_SECS", "90")?,
            "TNC_HEALTH_MAX_AGE_SECS",
            MAX_HEALTH_AGE_SECS,
        )?;
        let dry_run = match setting(&mut lookup, "TNC_DRY_RUN", "false")?.as_str() {
            "true" => true,
            "false" => false,
            _ => bail!("TNC_DRY_RUN must be exactly true or false"),
        };
        let auto_detach = match setting(&mut lookup, "TNC_AUTO_DETACH", "false")?.as_str() {
            "true" => true,
            "false" => false,
            _ => bail!("TNC_AUTO_DETACH must be exactly true or false"),
        };
        let detach_grace_secs = positive_seconds(
            &setting(&mut lookup, "TNC_DETACH_GRACE_SECS", "300")?,
            "TNC_DETACH_GRACE_SECS",
            MAX_HEALTH_AGE_SECS,
        )?;
        let protected_csv = setting(&mut lookup, "TNC_PROTECTED_NETWORKS", "")?;
        let mut protected_networks = BTreeSet::new();
        if !protected_csv.is_empty() {
            for name in protected_csv.split(',') {
                let name = name.trim();
                validate_object_name(name, "TNC_PROTECTED_NETWORKS entry")?;
                protected_networks.insert(name.to_owned());
            }
        }
        protected_networks.insert(egress_network.clone());

        Ok(Self {
            socket_path,
            state_dir,
            target_name,
            target_project,
            target_service,
            egress_network,
            interval_secs,
            api_timeout_secs,
            health_max_age_secs,
            dry_run,
            auto_detach,
            detach_grace_secs,
            protected_networks,
        })
    }

    pub fn is_protected(&self, name: &str) -> bool {
        name == self.egress_network || self.protected_networks.contains(name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedNetwork {
    pub id: String,
    pub name: String,
    pub owner: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub network_id: String,
    pub gw_priority: i64,
    /// Missing/empty identity can be observed, but can never grant ownership.
    pub endpoint_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub id: String,
    pub attached: BTreeMap<String, Endpoint>,
}

/// Accept an application slug, not an arbitrary Docker object name or path.
pub fn validate_app(app: &str) -> Result<()> {
    let bytes = app.as_bytes();
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_APP_LENGTH,
        "application must contain between 1 and {MAX_APP_LENGTH} ASCII characters"
    );
    ensure!(
        bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit(),
        "application must start with a lowercase ASCII letter or digit"
    );
    ensure!(
        bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(*byte, b'_' | b'-')
        }),
        "application may contain only lowercase ASCII letters, digits, underscores and hyphens"
    );
    Ok(())
}

pub fn network_name(app: &str) -> Result<String> {
    validate_app(app)?;
    Ok(format!("{NETWORK_PREFIX}{app}"))
}

/// Validate a full Docker network-inspect response before it is managed.
/// Endpoint membership (`Containers`) is checked separately by retirement.
pub fn validate_network(value: &Value) -> Result<ManagedNetwork> {
    ensure!(
        value.is_object(),
        "network inspection must be a JSON object"
    );
    let id = string_field(value, "Id", "network")?;
    validate_id(id, "network Id")?;
    let name = string_field(value, "Name", "network")?;
    let labels = object_field(value, "Labels", "network")?;
    validate_label_types(labels, "network")?;

    ensure!(
        label(labels, LABEL_AUTOJOIN, "network")? == LABEL_AUTOJOIN_VALUE,
        "network must have {LABEL_AUTOJOIN}={LABEL_AUTOJOIN_VALUE}"
    );
    ensure!(
        label(labels, LABEL_ROLE, "network")? == LABEL_ROLE_VALUE,
        "network must have {LABEL_ROLE}={LABEL_ROLE_VALUE}"
    );
    let owner = label(labels, LABEL_OWNER, "network")?;
    validate_app(owner).context("network owner label is not a valid application")?;
    let expected_name = network_name(owner)?;
    ensure!(
        name == expected_name,
        "network Name must exactly match its application owner and the managed prefix"
    );

    ensure!(
        string_field(value, "Driver", "network")? == "bridge",
        "managed network Driver must be bridge"
    );
    ensure!(
        string_field(value, "Scope", "network")? == "local",
        "managed network Scope must be local"
    );
    for field in ["Internal", "Ingress", "ConfigOnly"] {
        ensure!(
            !bool_field(value, field, "network")?,
            "managed network {field} must be false"
        );
    }

    // Default Docker bridges can serialize a nil options map as null. Any
    // explicit option needs a separate policy review, even an apparently
    // innocuous bridge-name option; do not maintain an implicit allowlist.
    match value.get("Options") {
        Some(Value::Null) => {}
        Some(Value::Object(options)) if options.is_empty() => {}
        Some(_) => bail!("managed network Options must be an empty object or null"),
        None => bail!("network inspection is missing Options"),
    }

    Ok(ManagedNetwork {
        id: id.to_owned(),
        name: name.to_owned(),
        owner: owner.to_owned(),
    })
}

/// Verify the intended running Traefik container and all reported endpoints.
pub fn validate_target(value: &Value, cfg: &Config) -> Result<Target> {
    ensure!(value.is_object(), "target inspection must be a JSON object");
    let id = string_field(value, "Id", "target")?;
    validate_id(id, "target Id")?;
    let raw_name = string_field(value, "Name", "target")?;
    let name = raw_name.strip_prefix('/').unwrap_or(raw_name);
    ensure!(
        name == cfg.target_name,
        "target Name does not match TNC_TARGET_NAME"
    );

    let config = value
        .get("Config")
        .filter(|value| value.is_object())
        .context("target Config must be a JSON object")?;
    let labels = object_field(config, "Labels", "target Config")?;
    validate_label_types(labels, "target")?;
    ensure!(
        label(labels, "com.docker.compose.project", "target")? == cfg.target_project,
        "target Compose project does not match TNC_TARGET_PROJECT"
    );
    ensure!(
        label(labels, "com.docker.compose.service", "target")? == cfg.target_service,
        "target Compose service does not match TNC_TARGET_SERVICE"
    );

    let state = value
        .get("State")
        .filter(|value| value.is_object())
        .context("target State must be a JSON object")?;
    ensure!(
        bool_field(state, "Running", "target State")?,
        "target must be running"
    );

    let settings = value
        .get("NetworkSettings")
        .filter(|value| value.is_object())
        .context("target NetworkSettings must be a JSON object")?;
    let networks = object_field(settings, "Networks", "target NetworkSettings")?;
    let mut attached = BTreeMap::new();
    for (name, endpoint) in networks {
        validate_object_name(name, "target attachment network name")?;
        ensure!(
            endpoint.is_object(),
            "target endpoint must be a JSON object"
        );
        let network_id = string_field(endpoint, "NetworkID", "target endpoint")?;
        validate_id(network_id, "target endpoint NetworkID")?;
        let gw_priority = endpoint.get("GwPriority").and_then(Value::as_i64).context(
            "target endpoint GwPriority must be an integer; Engine API 1.48 or newer is required",
        )?;
        let endpoint_id = match endpoint.get("EndpointID") {
            None | Some(Value::Null) => None,
            Some(Value::String(id)) if id.is_empty() => None,
            Some(Value::String(id)) => {
                validate_id(id, "target endpoint EndpointID")?;
                Some(id.clone())
            }
            Some(_) => bail!("target endpoint EndpointID must be a string"),
        };
        attached.insert(
            name.to_owned(),
            Endpoint {
                network_id: network_id.to_owned(),
                gw_priority,
                endpoint_id,
            },
        );
    }

    let egress = attached
        .get(&cfg.egress_network)
        .context("target is not attached to TNC_EGRESS_NETWORK")?;
    ensure!(
        egress.gw_priority == EGRESS_PRIORITY,
        "target egress GwPriority must be {EGRESS_PRIORITY}"
    );
    ensure!(
        attached
            .iter()
            .all(|(name, endpoint)| name == &cfg.egress_network
                || endpoint.gw_priority < EGRESS_PRIORITY),
        "another target attachment can override or tie the fixed egress gateway priority"
    );

    Ok(Target {
        id: id.to_owned(),
        attached,
    })
}

/// A summary from `GET /containers/json?all=1` references the network if its
/// network name or full network ID matches. Container running state is ignored.
///
/// Malformed summaries conservatively count as references: retirement must
/// prove absence. A valid differently named summary endpoint may omit its ID
/// (or report an empty ID), so its name remains usable for stopped containers.
pub fn has_container_reference(container_summary: &Value, network: &ManagedNetwork) -> bool {
    let Some(id) = container_summary.get("Id").and_then(Value::as_str) else {
        return true;
    };
    if !is_full_id(id) {
        return true;
    }
    let Some(networks) = container_summary
        .get("NetworkSettings")
        .and_then(|value| value.get("Networks"))
        .and_then(Value::as_object)
    else {
        return true;
    };

    for (name, endpoint) in networks {
        if name == &network.name {
            return true;
        }
        if validate_object_name(name, "container summary network name").is_err()
            || !endpoint.is_object()
        {
            return true;
        }
        match endpoint.get("NetworkID") {
            Some(Value::String(id)) if id == &network.id => return true,
            Some(Value::String(id)) if id.is_empty() || is_full_id(id) => {}
            None => {}
            // A null/non-string ID or a truncated ID is not a reliable proof
            // that this container belongs only to a different network.
            _ => return true,
        }
    }
    false
}

/// Validate the fields needed to prove an all-container inventory complete.
/// Stopped containers may retain a network name without a populated ID.
pub fn validate_container_summary(value: &Value) -> Result<String> {
    let id = string_field(value, "Id", "container summary")?;
    validate_id(id, "container summary Id")?;
    let settings = value
        .get("NetworkSettings")
        .context("container summary has no NetworkSettings")?;
    let networks = object_field(settings, "Networks", "container summary")?;
    for (name, endpoint) in networks {
        validate_object_name(name, "container summary network name")?;
        ensure!(
            endpoint.is_object(),
            "container summary endpoint is malformed"
        );
        match endpoint.get("NetworkID") {
            None => {}
            Some(Value::String(id)) if id.is_empty() || is_full_id(id) => {}
            _ => bail!("container summary NetworkID is malformed"),
        }
    }
    Ok(id.to_owned())
}

fn setting<F>(lookup: &mut F, key: &str, default: &str) -> Result<String>
where
    F: FnMut(&str) -> Result<Option<String>>,
{
    Ok(lookup(key)?.unwrap_or_else(|| default.to_owned()))
}

fn absolute_path(value: String, field: &str) -> Result<PathBuf> {
    ensure!(!value.contains('\0'), "{field} must not contain a NUL byte");
    let path = PathBuf::from(value);
    ensure!(path.is_absolute(), "{field} must be an absolute path");
    Ok(path)
}

fn positive_seconds(value: &str, field: &str, maximum: u64) -> Result<u64> {
    ensure!(
        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()),
        "{field} must be a positive unsigned decimal integer"
    );
    let number = value
        .parse::<u64>()
        .with_context(|| format!("{field} is outside the range of an unsigned 64-bit integer"))?;
    ensure!(
        (1..=maximum).contains(&number),
        "{field} must be between 1 and {maximum} seconds"
    );
    Ok(number)
}

fn validate_object_name(name: &str, field: &str) -> Result<()> {
    let mut bytes = name.bytes();
    ensure!(
        bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
            && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-')),
        "{field} must be a nonempty Docker name starting with an ASCII letter or digit and containing only letters, digits, underscores, dots or hyphens"
    );
    Ok(())
}

fn validate_project_name(name: &str) -> Result<()> {
    let mut bytes = name.bytes();
    ensure!(
        bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            && bytes.all(|byte| byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'-')),
        "TNC_TARGET_PROJECT must be the exact nonempty Compose project name: lowercase ASCII letters, digits, underscores and hyphens, starting with a letter or digit"
    );
    Ok(())
}

fn validate_service_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-')),
        "TNC_TARGET_SERVICE must be a nonempty Compose service name containing only ASCII letters, digits, underscores, dots or hyphens"
    );
    Ok(())
}

pub fn is_full_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn validate_id(id: &str, field: &str) -> Result<()> {
    ensure!(
        is_full_id(id),
        "{field} must be a full 64-character lowercase hexadecimal Docker ID"
    );
    Ok(())
}

fn string_field<'a>(value: &'a Value, key: &str, context: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .with_context(|| format!("{context} {key} must be a nonempty string"))
}

fn bool_field(value: &Value, key: &str, context: &str) -> Result<bool> {
    value
        .get(key)
        .and_then(Value::as_bool)
        .with_context(|| format!("{context} {key} must be a boolean"))
}

fn object_field<'a>(value: &'a Value, key: &str, context: &str) -> Result<&'a Map<String, Value>> {
    value
        .get(key)
        .and_then(Value::as_object)
        .with_context(|| format!("{context} {key} must be a JSON object"))
}

fn validate_label_types(labels: &Map<String, Value>, context: &str) -> Result<()> {
    ensure!(
        labels.values().all(Value::is_string),
        "{context} label values must be strings"
    );
    Ok(())
}

fn label<'a>(labels: &'a Map<String, Value>, key: &str, context: &str) -> Result<&'a str> {
    labels
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .with_context(|| format!("{context} must have a nonempty {key} label"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id(character: char) -> String {
        character.to_string().repeat(64)
    }

    fn config(extra: &[(&str, &str)]) -> Result<Config> {
        let mut vars: BTreeMap<String, String> =
            BTreeMap::from([("TNC_TARGET_PROJECT".to_owned(), "ingress-prod".to_owned())]);
        for (key, value) in extra {
            vars.insert((*key).to_owned(), (*value).to_owned());
        }
        Config::from_lookup(|key| Ok(vars.get(key).cloned()))
    }

    fn network() -> Value {
        json!({
            "Id": id('a'),
            "Name": "traefik_app_echo",
            "Driver": "bridge",
            "Scope": "local",
            "Internal": false,
            "Ingress": false,
            "ConfigOnly": false,
            "EnableIPv6": true,
            "Options": {},
            "Labels": {
                (LABEL_AUTOJOIN): LABEL_AUTOJOIN_VALUE,
                (LABEL_ROLE): LABEL_ROLE_VALUE,
                (LABEL_OWNER): "echo"
            }
        })
    }

    fn target() -> Value {
        json!({
            "Id": id('b'),
            "Name": "/traefik",
            "Config": {
                "Labels": {
                    "com.docker.compose.project": "ingress-prod",
                    "com.docker.compose.service": "traefik"
                }
            },
            "State": {"Running": true},
            "NetworkSettings": {
                "Networks": {
                    "traefik_egress": {"NetworkID": id('c'), "GwPriority": 100},
                    "ingress-prod_traefik_control": {"NetworkID": id('d'), "GwPriority": 0},
                    "traefik_app_echo": {"NetworkID": id('a'), "GwPriority": -1}
                }
            }
        })
    }

    fn summary(networks: Value) -> Value {
        json!({
            "Id": id('e'),
            "State": "exited",
            "NetworkSettings": {"Networks": networks}
        })
    }

    #[test]
    fn environment_defaults_and_exact_overrides() {
        let cfg = config(&[]).unwrap();
        assert_eq!(cfg.socket_path, PathBuf::from("/var/run/docker.sock"));
        assert_eq!(cfg.state_dir, PathBuf::from("/run/tnc"));
        assert_eq!(cfg.target_name, "traefik");
        assert_eq!(cfg.target_project, "ingress-prod");
        assert_eq!(cfg.target_service, "traefik");
        assert_eq!(cfg.egress_network, "traefik_egress");
        assert_eq!(cfg.interval_secs, 30);
        assert_eq!(cfg.api_timeout_secs, 10);
        assert_eq!(cfg.health_max_age_secs, 90);
        assert!(!cfg.dry_run);
        assert!(!cfg.auto_detach);
        assert_eq!(cfg.detach_grace_secs, 300);
        assert!(cfg.is_protected("traefik_egress"));

        let cfg = config(&[
            ("TNC_DRY_RUN", "true"),
            ("TNC_SOCKET_PATH", "/run/custom/docker.sock"),
            ("TNC_STATE_DIR", "/state/tnc"),
            ("TNC_TARGET_NAME", "edge-traefik"),
            ("TNC_TARGET_SERVICE", "reverse.proxy"),
            ("TNC_EGRESS_NETWORK", "edge_egress"),
            ("TNC_INTERVAL_SECS", "15"),
            ("TNC_AUTO_DETACH", "true"),
            ("TNC_DETACH_GRACE_SECS", "60"),
            ("TNC_PROTECTED_NETWORKS", "control, waf,control"),
        ])
        .unwrap();
        assert!(cfg.dry_run);
        assert_eq!(cfg.target_name, "edge-traefik");
        assert_eq!(cfg.target_service, "reverse.proxy");
        assert_eq!(cfg.egress_network, "edge_egress");
        assert_eq!(cfg.interval_secs, 15);
        assert!(cfg.auto_detach);
        assert_eq!(cfg.detach_grace_secs, 60);
        assert_eq!(
            cfg.protected_networks,
            BTreeSet::from(["control".into(), "waf".into(), "edge_egress".into()])
        );
    }

    #[test]
    fn project_is_required_and_never_inferred_or_trimmed() {
        assert!(Config::from_lookup(|_| Ok(None)).is_err());
        for value in [
            "",
            " ",
            " ingress-prod",
            "ingress-prod\n",
            "Prod",
            "a/b",
            "a.b",
        ] {
            assert!(
                config(&[("TNC_TARGET_PROJECT", value)]).is_err(),
                "{value:?}"
            );
        }
    }

    #[test]
    fn environment_paths_names_numbers_and_booleans_are_strict() {
        for field in ["TNC_SOCKET_PATH", "TNC_STATE_DIR"] {
            for value in ["", "relative/path", "~/docker.sock", "/run/invalid\0path"] {
                assert!(config(&[(field, value)]).is_err(), "{field}={value:?}");
            }
        }
        for field in [
            "TNC_TARGET_NAME",
            "TNC_TARGET_SERVICE",
            "TNC_EGRESS_NETWORK",
        ] {
            for value in ["", "with space", "a/b", "x\n", "é"] {
                assert!(config(&[(field, value)]).is_err(), "{field}={value:?}");
            }
        }
        assert!(config(&[("TNC_TARGET_NAME", "/traefik")]).is_err());
        for field in [
            "TNC_INTERVAL_SECS",
            "TNC_API_TIMEOUT_SECS",
            "TNC_HEALTH_MAX_AGE_SECS",
            "TNC_DETACH_GRACE_SECS",
        ] {
            for value in [
                "",
                "0",
                "-1",
                "+1",
                "1.0",
                " 30",
                "30 ",
                "18446744073709551616",
            ] {
                assert!(config(&[(field, value)]).is_err(), "{field}={value:?}");
            }
        }
        for value in ["", "TRUE", "False", "1", "0", " true", "false\n"] {
            assert!(config(&[("TNC_DRY_RUN", value)]).is_err(), "{value:?}");
            assert!(config(&[("TNC_AUTO_DETACH", value)]).is_err(), "{value:?}");
        }
        for value in [
            ",",
            "control,",
            ",control",
            "with space",
            "../control",
            "a/b",
        ] {
            assert!(
                config(&[("TNC_PROTECTED_NETWORKS", value)]).is_err(),
                "{value:?}"
            );
        }
    }

    #[test]
    fn app_slug_cannot_escape_the_owned_network_namespace() {
        for app in ["a", "0", "echo", "echo-prod_2"] {
            assert_eq!(network_name(app).unwrap(), format!("traefik_app_{app}"));
        }
        assert!(validate_app(&"a".repeat(48)).is_ok());
        assert!(validate_app(&"a".repeat(49)).is_err());
        for app in [
            "",
            "Echo",
            "../control",
            "a.b",
            "a/b",
            "_a",
            "-a",
            "a b",
            "a\n",
            "é",
        ] {
            assert!(network_name(app).is_err(), "{app:?}");
        }
    }

    #[test]
    fn duration_limits_are_finite_and_inclusive() {
        for (field, maximum) in [
            ("TNC_INTERVAL_SECS", MAX_INTERVAL_SECS),
            ("TNC_API_TIMEOUT_SECS", MAX_API_TIMEOUT_SECS),
            ("TNC_HEALTH_MAX_AGE_SECS", MAX_HEALTH_AGE_SECS),
            ("TNC_DETACH_GRACE_SECS", MAX_HEALTH_AGE_SECS),
        ] {
            assert!(config(&[(field, "1")]).is_ok());
            assert!(config(&[(field, &maximum.to_string())]).is_ok());
            assert!(config(&[(field, &(maximum + 1).to_string())]).is_err());
            assert!(config(&[(field, &u64::MAX.to_string())]).is_err());
        }
    }

    #[test]
    fn default_managed_bridge_and_ipv6_are_accepted() {
        let expected = ManagedNetwork {
            id: id('a'),
            name: "traefik_app_echo".to_owned(),
            owner: "echo".to_owned(),
        };
        assert_eq!(validate_network(&network()).unwrap(), expected);
        let mut value = network();
        value["Options"] = Value::Null;
        assert_eq!(validate_network(&value).unwrap(), expected);
    }

    #[test]
    fn network_identity_and_label_contract_are_not_just_a_name_prefix() {
        for replacement in [json!("echo"), json!("traefik_app_other"), json!("")] {
            let mut value = network();
            value["Name"] = replacement;
            assert!(validate_network(&value).is_err());
        }
        for (key, replacement) in [
            (LABEL_AUTOJOIN, json!("false")),
            (LABEL_AUTOJOIN, json!(true)),
            (LABEL_ROLE, json!("database")),
            (LABEL_OWNER, json!("other")),
            (LABEL_OWNER, json!("../control")),
        ] {
            let mut value = network();
            value["Labels"][key] = replacement;
            assert!(validate_network(&value).is_err());
        }
        for labels in [json!({}), Value::Null, json!([]), json!("labels")] {
            let mut value = network();
            value["Labels"] = labels;
            assert!(validate_network(&value).is_err());
        }
    }

    #[test]
    fn unsafe_or_ambiguous_network_properties_fail_closed() {
        for (key, replacement) in [
            ("Driver", json!("overlay")),
            ("Scope", json!("swarm")),
            ("Internal", json!(true)),
            ("Ingress", json!(true)),
            ("ConfigOnly", json!(true)),
            ("Internal", json!("false")),
            (
                "Options",
                json!({"com.docker.network.bridge.gateway_mode_ipv4": "nat-unprotected"}),
            ),
            (
                "Options",
                json!({"com.docker.network.bridge.name": "br-edge"}),
            ),
            ("Options", json!([])),
        ] {
            let mut value = network();
            value[key] = replacement;
            assert!(validate_network(&value).is_err(), "{key}");
        }
        for key in [
            "Driver",
            "Scope",
            "Internal",
            "Ingress",
            "ConfigOnly",
            "Options",
            "Labels",
        ] {
            let mut value = network();
            value.as_object_mut().unwrap().remove(key);
            assert!(validate_network(&value).is_err(), "missing {key}");
        }
    }

    #[test]
    fn docker_inspect_identity_is_id_not_id_uppercase_or_a_short_prefix() {
        let cfg = config(&[]).unwrap();
        for invalid_id in [
            json!("a".repeat(12)),
            json!("A".repeat(64)),
            json!("g".repeat(64)),
            json!("a".repeat(65)),
            json!(64),
            Value::Null,
        ] {
            let mut value = network();
            value["Id"] = invalid_id.clone();
            assert!(validate_network(&value).is_err());
            let mut value = target();
            value["Id"] = invalid_id;
            assert!(validate_target(&value, &cfg).is_err());
        }
        let mut value = network();
        let id = value.as_object_mut().unwrap().remove("Id").unwrap();
        value["ID"] = id;
        assert!(validate_network(&value).is_err());
        let mut value = target();
        let id = value.as_object_mut().unwrap().remove("Id").unwrap();
        value["ID"] = id;
        assert!(validate_target(&value, &cfg).is_err());
    }

    #[test]
    fn exact_running_target_with_fixed_egress_is_accepted() {
        let cfg = config(&[]).unwrap();
        let parsed = validate_target(&target(), &cfg).unwrap();
        assert_eq!(parsed.id, id('b'));
        assert_eq!(parsed.attached.len(), 3);
        assert_eq!(parsed.attached["traefik_egress"].gw_priority, 100);
        assert_eq!(parsed.attached["traefik_app_echo"].network_id, id('a'));
        assert_eq!(parsed.attached["traefik_app_echo"].gw_priority, -1);
        let mut value = target();
        value["Name"] = json!("traefik");
        assert!(validate_target(&value, &cfg).is_ok());
    }

    #[test]
    fn target_name_compose_identity_and_running_state_all_matter() {
        let cfg = config(&[]).unwrap();
        for name in ["/other", "//traefik", "/traefik "] {
            let mut value = target();
            value["Name"] = json!(name);
            assert!(validate_target(&value, &cfg).is_err());
        }
        for label_name in ["com.docker.compose.project", "com.docker.compose.service"] {
            let mut value = target();
            value["Config"]["Labels"][label_name] = json!("other");
            assert!(validate_target(&value, &cfg).is_err());
        }
        for labels in [json!({}), Value::Null] {
            let mut value = target();
            value["Config"]["Labels"] = labels;
            assert!(validate_target(&value, &cfg).is_err());
        }
        for state in [
            json!({}),
            json!({"Running": false}),
            json!({"Running": "true"}),
            Value::Null,
        ] {
            let mut value = target();
            value["State"] = state;
            assert!(validate_target(&value, &cfg).is_err());
        }
    }

    #[test]
    fn egress_requires_exact_priority_and_no_tie_or_override() {
        let cfg = config(&[]).unwrap();
        for priority in [
            json!(99),
            json!(101),
            json!("100"),
            json!(100.0),
            Value::Null,
        ] {
            let mut value = target();
            value["NetworkSettings"]["Networks"]["traefik_egress"]["GwPriority"] = priority;
            assert!(validate_target(&value, &cfg).is_err());
        }
        for priority in [100, 101] {
            let mut value = target();
            value["NetworkSettings"]["Networks"]["traefik_app_echo"]["GwPriority"] =
                json!(priority);
            assert!(validate_target(&value, &cfg).is_err());
        }
        let mut value = target();
        value["NetworkSettings"]["Networks"]
            .as_object_mut()
            .unwrap()
            .remove("traefik_egress");
        assert!(validate_target(&value, &cfg).is_err());
    }

    #[test]
    fn endpoint_identity_and_schema_cannot_be_silently_defaulted() {
        let cfg = config(&[]).unwrap();
        for endpoint in [
            Value::Null,
            json!({}),
            json!({"NetworkID": "short", "GwPriority": -1}),
            json!({"NetworkID": id('a')}),
            json!({"NetworkId": id('a'), "GwPriority": -1}),
        ] {
            let mut value = target();
            value["NetworkSettings"]["Networks"]["traefik_app_echo"] = endpoint;
            assert!(validate_target(&value, &cfg).is_err());
        }
        for identity in [json!("short"), json!(true), json!(id('A'))] {
            let mut value = target();
            value["NetworkSettings"]["Networks"]["traefik_app_echo"]["EndpointID"] = identity;
            assert!(validate_target(&value, &cfg).is_err());
        }
        for identity in [Value::Null, json!("")] {
            let mut value = target();
            value["NetworkSettings"]["Networks"]["traefik_app_echo"]["EndpointID"] = identity;
            assert_eq!(
                validate_target(&value, &cfg).unwrap().attached["traefik_app_echo"].endpoint_id,
                None
            );
        }
    }

    #[test]
    fn stopped_container_references_match_by_name_or_id() {
        let network = validate_network(&network()).unwrap();
        let by_name = summary(json!({"traefik_app_echo": {}}));
        assert!(has_container_reference(&by_name, &network));
        let by_id = summary(json!({"old-network-name": {"NetworkID": network.id}}));
        assert!(has_container_reference(&by_id, &network));
        let old_id_same_name = summary(json!({"traefik_app_echo": {"NetworkID": id('f')}}));
        assert!(has_container_reference(&old_id_same_name, &network));
        let other = summary(json!({"other": {"NetworkID": id('f')}}));
        assert!(!has_container_reference(&other, &network));
        assert!(!has_container_reference(&summary(json!({})), &network));
        assert!(!has_container_reference(
            &summary(json!({"other": {}})),
            &network
        ));
        assert!(!has_container_reference(
            &summary(json!({"other": {"NetworkID": ""}})),
            &network
        ));
    }

    #[test]
    fn malformed_reference_inventory_cannot_authorize_retirement() {
        let network = validate_network(&network()).unwrap();
        for value in [
            Value::Null,
            json!({}),
            json!({"Id": id('e')}),
            json!({"Id": "short", "NetworkSettings": {"Networks": {}}}),
            summary(Value::Null),
            summary(json!([])),
            summary(json!({"other": null})),
            summary(json!({"other": {"NetworkID": false}})),
            summary(json!({"other": {"NetworkID": "short"}})),
        ] {
            assert!(has_container_reference(&value, &network), "{value}");
        }
    }
}
