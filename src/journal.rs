//! Durable attachment evidence. A network label never substitutes for a record
//! tying this controller to one exact target/network/endpoint generation.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::policy::{Config, ManagedNetwork, is_full_id, network_name};

pub const JOURNAL_FILE: &str = "attachments.json";
const MAX_JOURNAL_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyView {
    pub socket_path: PathBuf,
    pub state_dir: PathBuf,
    pub target_name: String,
    pub target_project: String,
    pub target_service: String,
    pub egress_network: String,
    pub auto_detach: bool,
    pub detach_grace_secs: u64,
    pub interval_secs: u64,
    pub api_timeout_secs: u64,
    pub dry_run: bool,
    pub protected_networks: BTreeSet<String>,
}

impl From<&Config> for PolicyView {
    fn from(cfg: &Config) -> Self {
        let mut protected_networks = cfg.protected_networks.clone();
        protected_networks.insert(cfg.egress_network.clone());
        Self {
            socket_path: cfg.socket_path.clone(),
            state_dir: cfg.state_dir.clone(),
            target_name: cfg.target_name.clone(),
            target_project: cfg.target_project.clone(),
            target_service: cfg.target_service.clone(),
            egress_network: cfg.egress_network.clone(),
            auto_detach: cfg.auto_detach,
            detach_grace_secs: cfg.detach_grace_secs,
            interval_secs: cfg.interval_secs,
            api_timeout_secs: cfg.api_timeout_secs,
            dry_run: cfg.dry_run,
            protected_networks,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scope {
    socket_path: PathBuf,
    target_name: String,
    target_project: String,
    target_service: String,
    egress_network: String,
}

impl From<&Config> for Scope {
    fn from(cfg: &Config) -> Self {
        Self {
            socket_path: cfg.socket_path.clone(),
            target_name: cfg.target_name.clone(),
            target_project: cfg.target_project.clone(),
            target_service: cfg.target_service.clone(),
            egress_network: cfg.egress_network.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Prepared,
    InUse,
    UnusedPending,
    Detached,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ownership {
    Unowned,
    Connected,
    Adopted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachmentRecord {
    pub network: ManagedNetwork,
    pub target_id: String,
    pub endpoint_id: Option<String>,
    pub ownership: Ownership,
    pub phase: Phase,
    pub ever_used: bool,
    pub unused_since: Option<u64>,
    pub empty_observations: u32,
    pub last_observed_at: Option<u64>,
}

impl AttachmentRecord {
    pub fn new(network: &ManagedNetwork, target_id: &str) -> Self {
        Self {
            network: network.clone(),
            target_id: target_id.to_owned(),
            endpoint_id: None,
            ownership: Ownership::Unowned,
            phase: Phase::Prepared,
            ever_used: false,
            unused_since: None,
            empty_observations: 0,
            last_observed_at: None,
        }
    }

    pub fn clear_grace(&mut self) {
        self.unused_since = None;
        self.empty_observations = 0;
        self.last_observed_at = None;
    }

    pub fn relinquish(&mut self) {
        self.ownership = Ownership::Unowned;
        self.endpoint_id = None;
        self.clear_grace();
    }

    pub(crate) fn adopt(&mut self, target_id: &str, endpoint_id: &str) -> Result<()> {
        ensure!(
            is_full_id(target_id) && is_full_id(endpoint_id),
            "adoption identities must be full Docker IDs"
        );
        self.target_id = target_id.to_owned();
        self.endpoint_id = Some(endpoint_id.to_owned());
        self.ownership = Ownership::Adopted;
        if self.phase == Phase::Detached {
            self.phase = if self.ever_used {
                Phase::UnusedPending
            } else {
                Phase::Prepared
            };
        }
        self.clear_grace();
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    pub schema_version: u32,
    pub controller_id: String,
    pub revision: u64,
    pub observer_boot_id: String,
    scope: Scope,
    pub observed_policy: Option<PolicyView>,
    pub attachments: BTreeMap<String, AttachmentRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalGuard {
    pub present: bool,
    pub controller_id: Option<String>,
    pub revision: u64,
}

impl JournalGuard {
    pub fn from_journal(journal: Option<&Journal>) -> Self {
        Self {
            present: journal.is_some(),
            controller_id: journal.map(|j| j.controller_id.clone()),
            revision: journal.map_or(0, |j| j.revision),
        }
    }
}

fn random_id() -> Result<String> {
    let mut bytes = [0_u8; 32];
    File::open("/dev/urandom")
        .context("cannot open operating system random source")?
        .read_exact(&mut bytes)
        .context("cannot obtain controller identity randomness")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(crate) fn observer_boot_id() -> Result<String> {
    let value = fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .context("cannot read kernel boot identity")?;
    let value = value.trim();
    ensure!(valid_boot_id(value), "kernel boot identity is malformed");
    Ok(value.to_owned())
}

fn valid_boot_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
            }
        })
}

impl Journal {
    pub fn new(cfg: &Config) -> Result<Self> {
        Ok(Self {
            schema_version: 1,
            controller_id: random_id()?,
            revision: 0,
            observer_boot_id: observer_boot_id()?,
            scope: Scope::from(cfg),
            observed_policy: None,
            attachments: BTreeMap::new(),
        })
    }

    fn validate(&self, cfg: &Config) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "unsupported attachment journal schema"
        );
        ensure!(
            is_full_id(&self.controller_id),
            "invalid controller identity in journal"
        );
        ensure!(
            valid_boot_id(&self.observer_boot_id),
            "invalid boot identity in journal"
        );
        ensure!(
            self.scope == Scope::from(cfg),
            "attachment journal scope mismatch"
        );
        ensure!(
            self.attachments.len() <= 1024,
            "attachment journal exceeds record limit"
        );
        for (key, record) in &self.attachments {
            ensure!(
                key == &record.network.id && is_full_id(key),
                "invalid journal network identity"
            );
            ensure!(
                network_name(&record.network.owner)? == record.network.name,
                "journal network name/owner mismatch"
            );
            ensure!(
                is_full_id(&record.target_id),
                "invalid journal target identity"
            );
            if let Some(endpoint_id) = &record.endpoint_id {
                ensure!(is_full_id(endpoint_id), "invalid journal endpoint identity");
            }
            ensure!(
                record.ownership == Ownership::Unowned || record.endpoint_id.is_some(),
                "owned journal attachment has no endpoint identity"
            );
            ensure!(
                record.ownership != Ownership::Unowned || record.endpoint_id.is_none(),
                "unowned journal attachment contains ownership identity"
            );
            ensure!(
                record.empty_observations == 0 || record.unused_since.is_some(),
                "invalid journal grace evidence"
            );
            ensure!(
                record.unused_since.is_none() || record.phase == Phase::UnusedPending,
                "grace evidence outside unused state"
            );
            ensure!(
                record.phase != Phase::Prepared || !record.ever_used,
                "prepared journal edge was already used"
            );
            ensure!(
                record.phase == Phase::Prepared || record.ever_used,
                "used lifecycle has no usage evidence"
            );
        }
        Ok(())
    }

    /// Absence is not corruption and does not confer ownership on any existing
    /// endpoint. Callers must never reconstruct ownership from Docker labels.
    pub fn load(cfg: &Config) -> Result<Option<Self>> {
        match fs::symlink_metadata(&cfg.state_dir) {
            Ok(meta) => ensure!(meta.is_dir(), "state path must be a real directory"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("cannot inspect state directory"),
        }
        let path = cfg.state_dir.join(JOURNAL_FILE);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("cannot inspect attachment journal"),
        };
        ensure!(
            metadata.is_file() && metadata.len() <= MAX_JOURNAL_BYTES,
            "attachment journal must be a bounded regular file"
        );
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "attachment journal permissions must exclude group and other access"
        );
        let mut bytes = Vec::new();
        File::open(&path)?
            .take(MAX_JOURNAL_BYTES + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_JOURNAL_BYTES,
            "attachment journal grew beyond limit"
        );
        let journal: Self =
            serde_json::from_slice(&bytes).context("attachment journal is malformed")?;
        journal.validate(cfg)?;
        Ok(Some(journal))
    }

    /// The operation lock must be held by the caller. Revisions are also a
    /// saved-plan guard; a new complete observation invalidates older plans.
    pub fn save(&mut self, cfg: &Config) -> Result<()> {
        crate::controller::ensure_state_dir(&cfg.state_dir)?;
        self.validate(cfg)?;
        let path = cfg.state_dir.join(JOURNAL_FILE);
        match fs::symlink_metadata(&path) {
            Ok(meta) => ensure!(
                meta.is_file(),
                "attachment journal destination is not a regular file"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("cannot inspect journal destination"),
        }
        let next_revision = self
            .revision
            .checked_add(1)
            .context("attachment journal revision exhausted")?;
        let mut next = self.clone();
        next.revision = next_revision;
        let bytes = serde_json::to_vec_pretty(&next)?;
        ensure!(
            bytes.len() as u64 <= MAX_JOURNAL_BYTES,
            "attachment journal exceeds byte limit"
        );
        let temporary = cfg.state_dir.join(format!(
            ".attachments.{}.{}.tmp",
            std::process::id(),
            random_id()?
        ));
        let result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &path).context("cannot commit attachment journal")?;
            File::open(&cfg.state_dir)?
                .sync_all()
                .context("cannot sync attachment journal directory")?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        self.revision = next_revision;
        Ok(())
    }

    pub fn reset_grace(&mut self) -> bool {
        let before = self.attachments.clone();
        for record in self.attachments.values_mut() {
            record.clear_grace();
        }
        before != self.attachments
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{TestDirectory, id, network};

    fn owned(cfg: &Config) -> Journal {
        let mut journal = Journal::new(cfg).unwrap();
        let mut record = AttachmentRecord::new(&network(), &id('b'));
        record.ownership = Ownership::Connected;
        record.endpoint_id = Some(id('1'));
        journal.attachments.insert(network().id, record);
        journal
    }

    #[test]
    fn durable_identity_and_revision_survive_round_trip() {
        let dir = TestDirectory::new();
        let cfg = dir.config();
        assert_eq!(Journal::load(&cfg).unwrap(), None);
        let mut journal = owned(&cfg);
        let identity = journal.controller_id.clone();
        journal.save(&cfg).unwrap();
        assert_eq!(journal.revision, 1);
        let mut restored = Journal::load(&cfg).unwrap().unwrap();
        assert_eq!(restored, journal);
        restored.save(&cfg).unwrap();
        assert_eq!(restored.controller_id, identity);
        assert_eq!(restored.revision, 2);
        assert_eq!(
            fs::metadata(dir.0.join(JOURNAL_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&dir.0).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn missing_journal_does_not_recover_an_old_identity() {
        let dir = TestDirectory::new();
        let cfg = dir.config();
        let mut journal = owned(&cfg);
        journal.save(&cfg).unwrap();
        fs::remove_file(dir.0.join(JOURNAL_FILE)).unwrap();
        assert!(Journal::load(&cfg).unwrap().is_none());
        let replacement = Journal::new(&cfg).unwrap();
        assert_ne!(replacement.controller_id, journal.controller_id);
        assert!(replacement.attachments.is_empty());
    }

    #[test]
    fn malformed_unsafe_and_symlinked_state_is_not_reset() {
        let dir = TestDirectory::new();
        let cfg = dir.config();
        let mut journal = owned(&cfg);
        journal.save(&cfg).unwrap();
        let path = dir.0.join(JOURNAL_FILE);
        fs::write(&path, b"{truncated").unwrap();
        assert!(Journal::load(&cfg).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{truncated");
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("missing", &path).unwrap();
        assert!(Journal::load(&cfg).is_err());
        assert!(journal.save(&cfg).is_err());
        fs::remove_file(&path).unwrap();
        journal.save(&cfg).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Journal::load(&cfg).is_err());
    }

    #[test]
    fn corrupt_ownership_or_scope_cannot_be_saved_or_loaded() {
        let dir = TestDirectory::new();
        let cfg = dir.config();
        let mut journal = owned(&cfg);
        journal
            .attachments
            .get_mut(&network().id)
            .unwrap()
            .endpoint_id = None;
        assert!(journal.save(&cfg).is_err());
        journal = owned(&cfg);
        journal.save(&cfg).unwrap();
        let mut different = cfg.clone();
        different.target_project = "other".into();
        assert!(Journal::load(&different).is_err());
        let mut data: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.0.join(JOURNAL_FILE)).unwrap()).unwrap();
        data["schema_version"] = serde_json::json!(99);
        fs::write(dir.0.join(JOURNAL_FILE), serde_json::to_vec(&data).unwrap()).unwrap();
        assert!(Journal::load(&cfg).is_err());
    }

    #[test]
    fn restart_reset_preserves_ownership_but_discards_elapsed_grace() {
        let dir = TestDirectory::new();
        let cfg = dir.config();
        let mut journal = owned(&cfg);
        let record = journal.attachments.get_mut(&network().id).unwrap();
        record.phase = Phase::UnusedPending;
        record.ever_used = true;
        record.unused_since = Some(100);
        record.empty_observations = 10;
        record.last_observed_at = Some(160);
        journal.save(&cfg).unwrap();
        crate::controller::begin_session(&cfg).unwrap();
        let restored = Journal::load(&cfg).unwrap().unwrap();
        assert_eq!(restored.controller_id, journal.controller_id);
        let record = &restored.attachments[&network().id];
        assert_eq!(record.ownership, Ownership::Connected);
        assert_eq!(record.endpoint_id, Some(id('1')));
        assert_eq!(record.phase, Phase::UnusedPending);
        assert_eq!(record.unused_since, None);
        assert_eq!(record.empty_observations, 0);
    }

    #[test]
    fn dry_run_and_absent_state_do_not_initialize_or_reset_journal() {
        let dir = TestDirectory::new();
        let mut cfg = dir.config();
        crate::controller::begin_session(&cfg).unwrap();
        assert!(Journal::load(&cfg).unwrap().is_none());
        let mut journal = owned(&cfg);
        journal.save(&cfg).unwrap();
        let before = fs::read(dir.0.join(JOURNAL_FILE)).unwrap();
        cfg.dry_run = true;
        crate::controller::begin_session(&cfg).unwrap();
        crate::controller::reset_grace(&cfg).unwrap();
        assert_eq!(fs::read(dir.0.join(JOURNAL_FILE)).unwrap(), before);
    }

    #[test]
    fn adoption_of_manually_reattached_detached_edge_resets_to_pending() {
        let mut record = AttachmentRecord::new(&network(), &id('b'));
        record.phase = Phase::Detached;
        record.ever_used = true;
        record.adopt(&id('b'), &id('2')).unwrap();
        assert_eq!(record.phase, Phase::UnusedPending);
        assert_eq!(record.ownership, Ownership::Adopted);
        assert_eq!(record.endpoint_id, Some(id('2')));
        assert_eq!(record.unused_since, None);
        assert_eq!(record.empty_observations, 0);
        assert!(record.adopt("short", &id('2')).is_err());
    }

    #[test]
    fn adopting_never_used_edge_does_not_invent_usage_history() {
        let mut record = AttachmentRecord::new(&network(), &id('b'));
        record.adopt(&id('b'), &id('1')).unwrap();
        assert_eq!(record.phase, Phase::Prepared);
        assert!(!record.ever_used);
    }
}
