//! Pure decision engine shared by reconciliation and read-only plans.

use std::collections::BTreeSet;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::journal::{
    AttachmentRecord, Journal, JournalGuard, Ownership, Phase, PolicyView, observer_boot_id,
};
use crate::policy::{Config, ManagedNetwork, Target};

pub const REPORT_SCHEMA_VERSION: u32 = 1;
pub const PLAN_TTL_SECS: u64 = 300;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub operation: String,
    pub network: String,
    pub network_id: Option<String>,
    pub endpoint_id: Option<String>,
    pub reason_code: String,
}

impl Action {
    pub(crate) fn new(operation: &str, network: &ManagedNetwork, reason: &str) -> Self {
        Self {
            operation: operation.to_owned(),
            network: network.name.clone(),
            network_id: Some(network.id.clone()),
            endpoint_id: None,
            reason_code: reason.to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub network: String,
    pub network_id: String,
    pub phase: Phase,
    pub ownership: Ownership,
    pub observed_attached: bool,
    pub desired_attached: bool,
    pub endpoint_id: Option<String>,
    pub reference_ids: BTreeSet<String>,
    pub action: String,
    pub reason_code: String,
    pub unused_since: Option<u64>,
    pub empty_observations: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkObservation {
    pub network: ManagedNetwork,
    pub endpoint_container_ids: BTreeSet<String>,
    pub target_endpoint_id: Option<String>,
    pub reference_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanInputs {
    pub api_version: String,
    pub target: Target,
    pub networks: Vec<NetworkObservation>,
    pub journal: JournalGuard,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub complete: bool,
    pub all_containers: bool,
    pub containers_seen: usize,
    pub managed_networks_seen: usize,
    pub started_at: u64,
    pub finished_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema_version: u32,
    pub controller_version: String,
    pub target_id: Option<String>,
    pub dry_run: bool,
    pub managed_networks: usize,
    pub observed_at: u64,
    pub observation_complete: bool,
    pub observation: Observation,
    pub plan_expires_at: u64,
    pub policy: PolicyView,
    pub journal: JournalGuard,
    pub inputs: Option<PlanInputs>,
    pub decisions: Vec<Decision>,
    pub applied: Vec<Action>,
    pub pending: Vec<Action>,
    pub post_verified: bool,
}

impl Report {
    pub(crate) fn empty(cfg: &Config, now: u64, dry_run: bool) -> Self {
        Self {
            schema_version: REPORT_SCHEMA_VERSION,
            controller_version: env!("CARGO_PKG_VERSION").to_owned(),
            target_id: None,
            dry_run,
            managed_networks: 0,
            observed_at: now,
            observation_complete: false,
            observation: Observation {
                complete: false,
                all_containers: false,
                containers_seen: 0,
                managed_networks_seen: 0,
                started_at: now,
                finished_at: now,
            },
            plan_expires_at: now.saturating_add(PLAN_TTL_SECS),
            policy: PolicyView::from(cfg),
            journal: JournalGuard::from_journal(None),
            inputs: None,
            decisions: Vec::new(),
            applied: Vec::new(),
            pending: Vec::new(),
            post_verified: false,
        }
    }
}

pub(crate) struct Evaluation {
    pub report: Report,
    pub journal: Journal,
}

pub(crate) struct CompleteObservation<'a> {
    pub api: &'a str,
    pub target: &'a Target,
    pub networks: &'a [NetworkObservation],
    pub containers_seen: usize,
    pub started_at: u64,
    pub finished_at: u64,
}

/// A successful full snapshot is one observation, irrespective of the number
/// of event hints. Multiple calls in the same second cannot advance the count.
pub(crate) fn evaluate(
    cfg: &Config,
    observed: CompleteObservation<'_>,
    previous: Option<&Journal>,
    dry_run: bool,
    prepare_app: Option<&str>,
) -> Result<Evaluation> {
    let now = observed.finished_at;
    let policy = PolicyView::from(cfg);
    let mut journal = match previous {
        Some(journal) => journal.clone(),
        None => Journal::new(cfg)?,
    };
    if journal.observed_policy.as_ref() != Some(&policy) {
        journal.reset_grace();
    }
    let boot_id = observer_boot_id()?;
    if journal.observer_boot_id != boot_id {
        journal.reset_grace();
        journal.observer_boot_id = boot_id;
    }
    journal.observed_policy = Some(policy.clone());
    // A deleted network's full identity cannot reappear. Removing obsolete
    // records prevents unbounded history without transferring ownership by name.
    journal
        .attachments
        .retain(|id, _| observed.networks.iter().any(|n| &n.network.id == id));

    let mut report = Report::empty(cfg, now, dry_run);
    report.target_id = Some(observed.target.id.clone());
    report.managed_networks = observed.networks.len();
    report.observation_complete = true;
    report.observation = Observation {
        complete: true,
        all_containers: true,
        containers_seen: observed.containers_seen,
        managed_networks_seen: observed.networks.len(),
        started_at: observed.started_at,
        finished_at: now,
    };
    report.journal = JournalGuard::from_journal(previous);
    report.inputs = Some(PlanInputs {
        api_version: observed.api.to_owned(),
        target: observed.target.clone(),
        networks: observed.networks.to_vec(),
        journal: report.journal.clone(),
    });

    for edge in observed.networks {
        let network = &edge.network;
        let current_endpoint = observed.target.attached.get(&network.name);
        let is_attached = current_endpoint.is_some();
        let confirmed_endpoint = current_endpoint
            .and_then(|endpoint| endpoint.endpoint_id.as_ref())
            .filter(|id| edge.target_endpoint_id.as_ref() == Some(id));
        let record = journal
            .attachments
            .entry(network.id.clone())
            .or_insert_with(|| AttachmentRecord::new(network, &observed.target.id));
        if record.network != *network {
            *record = AttachmentRecord::new(network, &observed.target.id);
        }
        if record.target_id != observed.target.id
            || (record.ownership != Ownership::Unowned
                && record.endpoint_id.as_ref() != confirmed_endpoint)
        {
            record.relinquish();
            record.target_id = observed.target.id.clone();
        }
        if prepare_app == Some(network.owner.as_str()) {
            record.phase = Phase::Prepared;
            record.ever_used = false;
            record.clear_grace();
        }

        let protected = cfg.is_protected(&network.name);
        let owned = record.ownership != Ownership::Unowned
            && record.endpoint_id.is_some()
            && record.endpoint_id.as_ref() == confirmed_endpoint;
        let has_references = !edge.reference_ids.is_empty();
        let mut desired_attached = true;
        let reason;
        let mut operation = "keep";

        if protected {
            record.clear_grace();
            desired_attached = is_attached;
            reason = "PROTECTED_NETWORK";
        } else if has_references {
            record.phase = Phase::InUse;
            record.ever_used = true;
            record.clear_grace();
            reason = if is_attached {
                "REFERENCED"
            } else {
                "ATTACHMENT_MISSING"
            };
            if !is_attached {
                operation = "connect";
            }
        } else if record.phase == Phase::Detached {
            record.clear_grace();
            desired_attached = false;
            if is_attached {
                operation = "blocked";
                reason = "OWNERSHIP_UNKNOWN";
            } else {
                reason = "ALREADY_DETACHED";
            }
        } else if !record.ever_used {
            record.phase = Phase::Prepared;
            record.clear_grace();
            reason = if is_attached {
                "PREPARED_NEVER_USED"
            } else {
                "ATTACHMENT_MISSING"
            };
            if !is_attached {
                operation = "connect";
            }
        } else if !is_attached && cfg.auto_detach {
            // Also recovers a successful disconnect followed by a crash before
            // journal commit. No ownership is inferred from an absent endpoint.
            record.phase = Phase::Detached;
            record.relinquish();
            desired_attached = false;
            reason = "ALREADY_DETACHED";
        } else {
            record.phase = Phase::UnusedPending;
            if !cfg.auto_detach {
                record.clear_grace();
                reason = "AUTO_DETACH_DISABLED";
                if !is_attached {
                    operation = "connect";
                }
            } else if !owned {
                record.clear_grace();
                operation = "blocked";
                reason = if confirmed_endpoint.is_none() {
                    "ENDPOINT_ID_UNAVAILABLE"
                } else {
                    "OWNERSHIP_UNKNOWN"
                };
            } else {
                let maximum_gap = cfg
                    .interval_secs
                    .saturating_mul(3)
                    .max(cfg.api_timeout_secs.saturating_mul(3));
                let continuous = record.last_observed_at.is_some_and(|last| {
                    now.checked_sub(last).is_some_and(|gap| gap <= maximum_gap)
                });
                if !continuous || record.unused_since.is_none() {
                    record.unused_since = Some(now);
                    record.empty_observations = 1;
                } else if record.last_observed_at != Some(now) {
                    record.empty_observations = record.empty_observations.saturating_add(1);
                }
                record.last_observed_at = Some(now);
                let elapsed = record
                    .unused_since
                    .and_then(|since| now.checked_sub(since))
                    .unwrap_or(0);
                if elapsed >= cfg.detach_grace_secs && record.empty_observations >= 2 {
                    desired_attached = false;
                    operation = "disconnect";
                    reason = "UNUSED_GRACE_ELAPSED";
                } else {
                    reason = "GRACE_PENDING";
                }
            }
        }

        let decision = Decision {
            network: network.name.clone(),
            network_id: network.id.clone(),
            phase: record.phase,
            ownership: record.ownership,
            observed_attached: is_attached,
            desired_attached,
            endpoint_id: confirmed_endpoint.cloned(),
            reference_ids: edge.reference_ids.clone(),
            action: operation.to_owned(),
            reason_code: reason.to_owned(),
            unused_since: record.unused_since,
            empty_observations: record.empty_observations,
        };
        if matches!(operation, "connect" | "disconnect") {
            let mut action = Action::new(operation, network, reason);
            if operation == "disconnect" {
                action.endpoint_id = confirmed_endpoint.cloned();
            }
            report.pending.push(action);
        }
        report.decisions.push(decision);
    }
    Ok(Evaluation { report, journal })
}

/// Timestamp-dependent informational fields may age between plan and apply;
/// identities, policy, journal revision and the exact permitted actions may not.
pub(crate) fn same_plan(saved: &Report, current: &Report, now: u64) -> bool {
    saved.schema_version == REPORT_SCHEMA_VERSION
        && saved.controller_version == env!("CARGO_PKG_VERSION")
        && saved.dry_run
        && saved.applied.is_empty()
        && !saved.post_verified
        && saved.observation_complete
        && saved.observation.complete
        && saved.observation.all_containers
        && saved.observed_at <= now
        && saved.plan_expires_at == saved.observed_at.saturating_add(PLAN_TTL_SECS)
        && now <= saved.plan_expires_at
        && saved.target_id == current.target_id
        && saved.policy == current.policy
        && saved.inputs == current.inputs
        && saved.journal == current.journal
        && saved.pending == current.pending
        && same_decisions(&saved.decisions, &current.decisions)
        && saved.managed_networks == current.managed_networks
        && saved
            .pending
            .iter()
            .all(|action| matches!(action.operation.as_str(), "connect" | "disconnect"))
}

fn same_decisions(left: &[Decision], right: &[Decision]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter().zip(right).all(|(left, right)| {
        let mut left = left.clone();
        let mut right = right.clone();
        // The preview's projected observation time/count is informative only.
        // Current policy re-evaluates and persists its own grace evidence.
        left.unused_since = None;
        right.unused_since = None;
        left.empty_observations = 0;
        right.empty_observations = 0;
        left == right
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{config, id, network, target};

    fn owned(cfg: &Config) -> Journal {
        let mut journal = Journal::new(cfg).unwrap();
        journal.observed_policy = Some(PolicyView::from(cfg));
        let mut record = AttachmentRecord::new(&network(), &id('b'));
        record.ownership = Ownership::Connected;
        record.endpoint_id = Some(id('1'));
        record.phase = Phase::InUse;
        record.ever_used = true;
        journal.attachments.insert(network().id, record);
        journal
    }

    fn observation(
        cfg: &Config,
        previous: Option<&Journal>,
        target: &Target,
        references: &[String],
        now: u64,
        prepare: Option<&str>,
    ) -> Evaluation {
        let endpoint_id = target
            .attached
            .get(&network().name)
            .and_then(|endpoint| endpoint.endpoint_id.clone());
        let mut endpoints: BTreeSet<String> = references.iter().cloned().collect();
        if target.attached.contains_key(&network().name) {
            endpoints.insert(target.id.clone());
        }
        let networks = vec![NetworkObservation {
            network: network(),
            endpoint_container_ids: endpoints,
            target_endpoint_id: endpoint_id,
            reference_ids: references.iter().cloned().collect(),
        }];
        evaluate(
            cfg,
            CompleteObservation {
                api: "1.56",
                target,
                networks: &networks,
                containers_seen: 1 + references.len(),
                started_at: now,
                finished_at: now,
            },
            previous,
            true,
            prepare,
        )
        .unwrap()
    }

    fn decision(evaluation: &Evaluation) -> &Decision {
        &evaluation.report.decisions[0]
    }

    #[test]
    fn never_used_preparation_is_retained_indefinitely() {
        let cfg = config();
        let mut journal = owned(&cfg);
        let record = journal.attachments.get_mut(&network().id).unwrap();
        record.phase = Phase::Prepared;
        record.ever_used = false;
        let first = observation(&cfg, Some(&journal), &target(true), &[], 100, None);
        let later = observation(
            &cfg,
            Some(&first.journal),
            &target(true),
            &[],
            100_000,
            None,
        );
        assert!(later.report.pending.is_empty());
        assert_eq!(decision(&later).phase, Phase::Prepared);
        assert_eq!(decision(&later).reason_code, "PREPARED_NEVER_USED");
        assert_eq!(decision(&later).empty_observations, 0);
    }

    #[test]
    fn missing_prepared_edge_connects_without_inventing_ownership() {
        let cfg = config();
        let first = observation(&cfg, None, &target(false), &[], 100, None);
        assert_eq!(first.report.pending.len(), 1);
        assert_eq!(first.report.pending[0].operation, "connect");
        assert_eq!(decision(&first).ownership, Ownership::Unowned);
        assert_eq!(decision(&first).phase, Phase::Prepared);
    }

    #[test]
    fn preexisting_and_ambiguous_attachments_remain_unowned_after_use() {
        let cfg = config();
        let first = observation(&cfg, None, &target(true), &[id('9')], 100, None);
        assert_eq!(decision(&first).ownership, Ownership::Unowned);
        assert_eq!(decision(&first).phase, Phase::InUse);
        let unused = observation(&cfg, Some(&first.journal), &target(true), &[], 160, None);
        let much_later = observation(
            &cfg,
            Some(&unused.journal),
            &target(true),
            &[],
            10_000,
            None,
        );
        assert_eq!(decision(&much_later).action, "blocked");
        assert_eq!(decision(&much_later).reason_code, "OWNERSHIP_UNKNOWN");
        assert!(much_later.report.pending.is_empty());
    }

    #[test]
    fn automatic_detach_requires_grace_and_two_complete_observations() {
        let cfg = config();
        let journal = owned(&cfg);
        let first = observation(&cfg, Some(&journal), &target(true), &[], 100, None);
        assert_eq!(decision(&first).reason_code, "GRACE_PENDING");
        assert_eq!(decision(&first).empty_observations, 1);
        let early = observation(&cfg, Some(&first.journal), &target(true), &[], 159, None);
        assert!(early.report.pending.is_empty());
        let ready = observation(&cfg, Some(&early.journal), &target(true), &[], 160, None);
        assert_eq!(ready.report.pending[0].operation, "disconnect");
        assert_eq!(ready.report.pending[0].endpoint_id, Some(id('1')));
        assert_eq!(decision(&ready).reason_code, "UNUSED_GRACE_ELAPSED");
    }

    #[test]
    fn event_bursts_in_same_second_do_not_increment_observation_count() {
        let cfg = config();
        let journal = owned(&cfg);
        let first = observation(&cfg, Some(&journal), &target(true), &[], 100, None);
        let burst = observation(&cfg, Some(&first.journal), &target(true), &[], 100, None);
        assert_eq!(decision(&burst).empty_observations, 1);
        assert!(burst.report.pending.is_empty());
    }

    #[test]
    fn returned_reference_cancels_grace_including_stopped_references() {
        let cfg = config();
        let first = observation(&cfg, Some(&owned(&cfg)), &target(true), &[], 100, None);
        // Reference lifetime matters, not process running/healthy status.
        let returned = observation(
            &cfg,
            Some(&first.journal),
            &target(true),
            &[id('9')],
            160,
            None,
        );
        assert_eq!(decision(&returned).reason_code, "REFERENCED");
        assert_eq!(decision(&returned).unused_since, None);
        let unused_again =
            observation(&cfg, Some(&returned.journal), &target(true), &[], 161, None);
        assert_eq!(decision(&unused_again).unused_since, Some(161));
        assert_eq!(decision(&unused_again).empty_observations, 1);
        assert!(unused_again.report.pending.is_empty());
    }

    #[test]
    fn observation_gaps_and_backward_clock_reset_elapsed_time() {
        let cfg = config();
        let first = observation(&cfg, Some(&owned(&cfg)), &target(true), &[], 100, None);
        for now in [99, 191, 1_000_000] {
            let result = observation(&cfg, Some(&first.journal), &target(true), &[], now, None);
            assert_eq!(decision(&result).unused_since, Some(now));
            assert_eq!(decision(&result).empty_observations, 1);
            assert!(result.report.pending.is_empty());
        }
    }

    #[test]
    fn boot_replacement_resets_one_shot_observation_continuity() {
        let cfg = config();
        let mut first = observation(&cfg, Some(&owned(&cfg)), &target(true), &[], 100, None);
        first.journal.observer_boot_id = "00000000-0000-0000-0000-000000000000".into();
        let next_boot = observation(&cfg, Some(&first.journal), &target(true), &[], 160, None);
        assert_eq!(decision(&next_boot).unused_since, Some(160));
        assert_eq!(decision(&next_boot).empty_observations, 1);
        assert!(next_boot.report.pending.is_empty());
    }

    #[test]
    fn connect_only_policy_and_enabling_auto_detach_restart_grace() {
        let mut cfg = config();
        cfg.auto_detach = false;
        let first = observation(&cfg, Some(&owned(&cfg)), &target(true), &[], 100, None);
        let later = observation(&cfg, Some(&first.journal), &target(true), &[], 160, None);
        assert_eq!(decision(&later).reason_code, "AUTO_DETACH_DISABLED");
        assert_eq!(decision(&later).empty_observations, 0);
        cfg.auto_detach = true;
        let enabled = observation(&cfg, Some(&later.journal), &target(true), &[], 161, None);
        assert_eq!(decision(&enabled).empty_observations, 1);
        assert_eq!(decision(&enabled).unused_since, Some(161));
        assert!(enabled.report.pending.is_empty());
    }

    #[test]
    fn target_replacement_does_not_inherit_old_endpoint_ownership() {
        let cfg = config();
        let first = observation(&cfg, Some(&owned(&cfg)), &target(true), &[], 100, None);
        let mut replaced = target(true);
        replaced.id = id('2');
        let result = observation(&cfg, Some(&first.journal), &replaced, &[], 160, None);
        assert_eq!(decision(&result).ownership, Ownership::Unowned);
        assert_eq!(decision(&result).reason_code, "OWNERSHIP_UNKNOWN");
        assert!(result.report.pending.is_empty());
    }

    #[test]
    fn endpoint_replacement_never_inherits_ownership_from_network_name() {
        let cfg = config();
        let first = observation(&cfg, Some(&owned(&cfg)), &target(true), &[], 100, None);
        let mut replaced = target(true);
        replaced
            .attached
            .get_mut(&network().name)
            .unwrap()
            .endpoint_id = Some(id('3'));
        let result = observation(&cfg, Some(&first.journal), &replaced, &[], 160, None);
        assert_eq!(decision(&result).ownership, Ownership::Unowned);
        assert_eq!(decision(&result).empty_observations, 0);
        assert!(result.report.pending.is_empty());
    }

    #[test]
    fn missing_endpoint_identity_revokes_detach_authority() {
        let cfg = config();
        let mut missing = target(true);
        missing
            .attached
            .get_mut(&network().name)
            .unwrap()
            .endpoint_id = None;
        let result = observation(&cfg, Some(&owned(&cfg)), &missing, &[], 100, None);
        assert_eq!(decision(&result).ownership, Ownership::Unowned);
        assert_eq!(decision(&result).reason_code, "ENDPOINT_ID_UNAVAILABLE");
        assert!(result.report.pending.is_empty());
    }

    #[test]
    fn missing_journal_does_not_auto_adopt_current_attachment() {
        let cfg = config();
        let result = observation(&cfg, None, &target(true), &[], 100, None);
        assert_eq!(decision(&result).ownership, Ownership::Unowned);
        assert_eq!(decision(&result).phase, Phase::Prepared);
        assert!(!result.report.journal.present);
        assert!(result.report.pending.is_empty());
    }

    #[test]
    fn detached_empty_network_stays_detached_and_returning_application_reconnects() {
        let cfg = config();
        let first = observation(&cfg, Some(&owned(&cfg)), &target(true), &[], 100, None);
        let detached = observation(&cfg, Some(&first.journal), &target(false), &[], 160, None);
        assert_eq!(decision(&detached).phase, Phase::Detached);
        let next = observation(
            &cfg,
            Some(&detached.journal),
            &target(false),
            &[],
            190,
            None,
        );
        assert_eq!(decision(&next).reason_code, "ALREADY_DETACHED");
        assert!(next.report.pending.is_empty());
        let returned = observation(
            &cfg,
            Some(&next.journal),
            &target(false),
            &[id('9')],
            200,
            None,
        );
        assert_eq!(decision(&returned).phase, Phase::InUse);
        assert_eq!(returned.report.pending[0].operation, "connect");
    }

    #[test]
    fn explicit_prepare_reserves_a_detached_or_previously_used_empty_edge() {
        let cfg = config();
        let detached = observation(&cfg, Some(&owned(&cfg)), &target(false), &[], 100, None);
        let prepared = observation(
            &cfg,
            Some(&detached.journal),
            &target(false),
            &[],
            160,
            Some("echo"),
        );
        assert_eq!(decision(&prepared).phase, Phase::Prepared);
        assert!(!prepared.journal.attachments[&network().id].ever_used);
        assert_eq!(prepared.report.pending[0].operation, "connect");
        let busy = observation(
            &cfg,
            Some(&owned(&cfg)),
            &target(true),
            &[id('9')],
            100,
            Some("echo"),
        );
        assert_eq!(decision(&busy).phase, Phase::InUse);
    }

    #[test]
    fn manual_reconnection_after_detach_is_preserved_without_ownership() {
        let cfg = config();
        let detached = observation(&cfg, Some(&owned(&cfg)), &target(false), &[], 100, None);
        let manual = observation(&cfg, Some(&detached.journal), &target(true), &[], 160, None);
        assert_eq!(decision(&manual).ownership, Ownership::Unowned);
        assert_eq!(decision(&manual).action, "blocked");
        assert!(manual.report.pending.is_empty());
    }

    #[test]
    fn configured_protection_prevents_both_connect_and_disconnect() {
        let mut cfg = config();
        cfg.protected_networks.insert(network().name);
        for present in [false, true] {
            let result = observation(&cfg, Some(&owned(&cfg)), &target(present), &[], 100, None);
            assert!(result.report.pending.is_empty());
            assert_eq!(decision(&result).reason_code, "PROTECTED_NETWORK");
            assert_eq!(decision(&result).desired_attached, present);
        }
    }

    #[test]
    fn read_only_evaluation_never_advances_its_input_journal() {
        let cfg = config();
        let journal = owned(&cfg);
        let unchanged = journal.clone();
        let first = observation(&cfg, Some(&journal), &target(true), &[], 100, None);
        let later_read = observation(&cfg, Some(&journal), &target(true), &[], 160, None);
        assert_eq!(journal, unchanged);
        assert_eq!(decision(&first).empty_observations, 1);
        assert_eq!(decision(&later_read).empty_observations, 1);
        assert!(later_read.report.pending.is_empty());
    }

    #[test]
    fn saved_plan_accepts_only_current_exact_policy_and_material_observation() {
        let cfg = config();
        let first = observation(&cfg, None, &target(false), &[], 100, None);
        let current = observation(&cfg, None, &target(false), &[], 101, None);
        assert!(same_plan(&first.report, &current.report, 101));
        let mut changed = current.report.clone();
        changed.policy.auto_detach = !changed.policy.auto_detach;
        assert!(!same_plan(&first.report, &changed, 101));
        changed = current.report.clone();
        changed.inputs.as_mut().unwrap().target.id = id('4');
        assert!(!same_plan(&first.report, &changed, 101));
        changed = current.report.clone();
        changed.inputs.as_mut().unwrap().networks[0]
            .reference_ids
            .insert(id('9'));
        assert!(!same_plan(&first.report, &changed, 101));
        changed = current.report.clone();
        changed.inputs.as_mut().unwrap().journal.revision += 1;
        assert!(!same_plan(&first.report, &changed, 101));
    }

    #[test]
    fn expired_future_or_forged_saved_plan_never_becomes_authority() {
        let cfg = config();
        let current = observation(&cfg, None, &target(false), &[], 100, None).report;
        assert!(!same_plan(&current, &current, 401));
        assert!(!same_plan(&current, &current, 99));
        for operation in ["remove", "delete", "adopt", "exec", "disconnect"] {
            let mut forged = current.clone();
            forged.pending[0].operation = operation.into();
            assert!(!same_plan(&forged, &current, 100));
        }
        let mut forged = current.clone();
        forged.decisions[0].action = "keep".into();
        assert!(!same_plan(&forged, &current, 100));
        forged = current.clone();
        forged.plan_expires_at += 1;
        assert!(!same_plan(&forged, &current, 100));
        forged = current.clone();
        forged.observation.complete = false;
        assert!(!same_plan(&forged, &current, 100));
    }

    #[test]
    fn saved_plan_becomes_stale_when_grace_makes_an_extra_action_eligible() {
        let cfg = config();
        let first = observation(&cfg, Some(&owned(&cfg)), &target(true), &[], 100, None);
        let plan = observation(&cfg, Some(&first.journal), &target(true), &[], 159, None);
        let now = observation(&cfg, Some(&first.journal), &target(true), &[], 160, None);
        assert!(plan.report.pending.is_empty());
        assert_eq!(now.report.pending[0].operation, "disconnect");
        assert!(!same_plan(&plan.report, &now.report, 160));
    }

    #[test]
    fn a_replaced_network_full_id_discards_old_name_based_ownership() {
        let cfg = config();
        let old = owned(&cfg);
        let mut new_network = network();
        new_network.id = id('5');
        let mut current = target(true);
        current
            .attached
            .get_mut(&new_network.name)
            .unwrap()
            .network_id = new_network.id.clone();
        let edges = [NetworkObservation {
            network: new_network.clone(),
            endpoint_container_ids: BTreeSet::from([current.id.clone()]),
            target_endpoint_id: Some(id('1')),
            reference_ids: BTreeSet::new(),
        }];
        let result = evaluate(
            &cfg,
            CompleteObservation {
                api: "1.56",
                target: &current,
                networks: &edges,
                containers_seen: 1,
                started_at: 100,
                finished_at: 100,
            },
            Some(&old),
            true,
            None,
        )
        .unwrap();
        assert_eq!(decision(&result).ownership, Ownership::Unowned);
        assert!(!result.journal.attachments.contains_key(&network().id));
        assert_eq!(
            result.journal.attachments[&new_network.id].phase,
            Phase::Prepared
        );
    }

    #[test]
    fn structured_plan_round_trip_preserves_contract_and_unknown_fields_refuse() {
        let cfg = config();
        let report = observation(&cfg, None, &target(false), &[], 100, None).report;
        let mut data = serde_json::to_value(&report).unwrap();
        let restored: Report = serde_json::from_value(data.clone()).unwrap();
        assert!(same_plan(&restored, &report, 100));
        data["force"] = serde_json::json!(true);
        assert!(serde_json::from_value::<Report>(data).is_err());
    }
}
