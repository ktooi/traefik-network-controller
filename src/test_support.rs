use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::policy::{Config, Endpoint, ManagedNetwork, Target};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct TestDirectory(pub PathBuf);

impl TestDirectory {
    pub fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "tnc-core-test-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create isolated test directory");
        Self(path)
    }
    pub fn config(&self) -> Config {
        let mut cfg = config();
        cfg.state_dir = self.0.clone();
        cfg.socket_path = self.0.join("docker.sock");
        cfg
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn id(character: char) -> String {
    character.to_string().repeat(64)
}

pub fn config() -> Config {
    Config {
        socket_path: PathBuf::from("/nonexistent/tnc-test.sock"),
        state_dir: PathBuf::from("/nonexistent/tnc-test-state"),
        target_name: "traefik".into(),
        target_project: "test".into(),
        target_service: "traefik".into(),
        egress_network: "traefik_egress".into(),
        interval_secs: 30,
        api_timeout_secs: 10,
        health_max_age_secs: 90,
        dry_run: false,
        auto_detach: true,
        detach_grace_secs: 60,
        protected_networks: BTreeSet::from(["traefik_egress".into()]),
    }
}

pub fn network() -> ManagedNetwork {
    ManagedNetwork {
        id: id('a'),
        name: "traefik_app_echo".into(),
        owner: "echo".into(),
    }
}

pub fn target(attached: bool) -> Target {
    let mut endpoints = BTreeMap::from([
        (
            "traefik_egress".into(),
            Endpoint {
                network_id: id('c'),
                gw_priority: 100,
                endpoint_id: Some(id('d')),
            },
        ),
        (
            "traefik_control".into(),
            Endpoint {
                network_id: id('e'),
                gw_priority: 0,
                endpoint_id: Some(id('f')),
            },
        ),
    ]);
    if attached {
        endpoints.insert(
            network().name,
            Endpoint {
                network_id: id('a'),
                gw_priority: -1,
                endpoint_id: Some(id('1')),
            },
        );
    }
    Target {
        id: id('b'),
        attached: endpoints,
    }
}
