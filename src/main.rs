use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{IsTerminal, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, watch};

use traefik_network_controller::controller::{
    self, Report, check_health, daemon_lock, log_event, operation_lock, write_health,
};
use traefik_network_controller::docker::Docker;
use traefik_network_controller::policy::Config;

const MAX_PLAN_BYTES: u64 = 2 * 1024 * 1024;
const INPUT_TIMEOUT_SECS: u64 = 10;

const HELP: &str = "\
traefik-network-controller

USAGE:
  traefik-network-controller [run|once|plan|status|doctor|capabilities|healthcheck]
  traefik-network-controller prepare APP
  traefik-network-controller retire APP
  traefik-network-controller apply PLAN_FILE_OR_DASH
  traefik-network-controller adopt APP --target-id ID --network-id ID --endpoint-id ID

run          Reconcile at startup, on Docker events and periodically.
once         Reconcile once under the configured attachment/detachment policy.
plan         Read-only Docker snapshot; print a guarded, expiring JSON plan.
status       Read-only current attachments, ownership and lifecycle decisions.
doctor       Read-only policy/snapshot checks and local daemon health; no HTTP probe.
capabilities Print the versioned machine-readable command contract; no Docker calls.
prepare APP  Create/validate the managed edge network and attach the target.
retire APP   Explicitly disconnect/delete an unused managed edge network.
apply FILE   Revalidate and apply a saved plan; use - for bounded JSON on stdin.
adopt APP    Explicitly claim an existing attachment with three exact identity pins.
healthcheck  Check recent daemon reconciliation state; no Docker calls.

JSON is the default output; --json and --force are not supported.
Required: TNC_TARGET_PROJECT (exact Compose project label).
Auto-detach: TNC_AUTO_DETACH=false, TNC_DETACH_GRACE_SECS=300.
Protected network names: TNC_PROTECTED_NETWORKS (comma separated).
Exit codes: 0=completed, 1=operation failed, 2=invalid command arguments.
Configuration and limitations: SPEC.md and docs/operations.md.
";

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Run,
    Once,
    Plan,
    Status,
    Doctor,
    Capabilities,
    Healthcheck,
    Prepare(String),
    Retire(String),
    Apply(String),
    Adopt {
        app: String,
        target: String,
        network: String,
        endpoint: String,
    },
}

fn parse_command(args: &[String]) -> Result<Command> {
    let name = args.first().map(String::as_str).unwrap_or("run");
    match (name, args.len()) {
        ("run", 0 | 1) => Ok(Command::Run),
        ("once", 1) => Ok(Command::Once),
        ("plan", 1) => Ok(Command::Plan),
        ("status", 1) => Ok(Command::Status),
        ("doctor", 1) => Ok(Command::Doctor),
        ("capabilities", 1) => Ok(Command::Capabilities),
        ("healthcheck", 1) => Ok(Command::Healthcheck),
        ("prepare", 2) => Ok(Command::Prepare(args[1].clone())),
        ("retire", 2) => Ok(Command::Retire(args[1].clone())),
        ("apply", 2) => Ok(Command::Apply(args[1].clone())),
        ("adopt", 8) => {
            let mut pins = BTreeMap::new();
            for pair in args[2..].as_chunks::<2>().0 {
                if !matches!(
                    pair[0].as_str(),
                    "--target-id" | "--network-id" | "--endpoint-id"
                ) || pair[1].is_empty()
                    || pins.insert(pair[0].as_str(), pair[1].clone()).is_some()
                {
                    bail!(
                        "adopt requires each of --target-id, --network-id and --endpoint-id exactly once"
                    );
                }
            }
            Ok(Command::Adopt {
                app: args[1].clone(),
                target: pins
                    .remove("--target-id")
                    .ok_or_else(|| anyhow::anyhow!("missing --target-id"))?,
                network: pins
                    .remove("--network-id")
                    .ok_or_else(|| anyhow::anyhow!("missing --network-id"))?,
                endpoint: pins
                    .remove("--endpoint-id")
                    .ok_or_else(|| anyhow::anyhow!("missing --endpoint-id"))?,
            })
        }
        _ => bail!("invalid command arguments; use --help"),
    }
}

#[derive(Debug)]
struct InputFailure {
    code: &'static str,
    message: &'static str,
}
impl std::fmt::Display for InputFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}
impl std::error::Error for InputFailure {}
fn invalid_plan(message: &'static str) -> anyhow::Error {
    InputFailure {
        code: "INVALID_PLAN",
        message,
    }
    .into()
}

async fn read_plan(path: &str) -> Result<Report> {
    let mut bytes = Vec::new();
    if path == "-" {
        if std::io::stdin().is_terminal() {
            return Err(invalid_plan("apply - requires piped JSON input"));
        }
        let read = async {
            tokio::io::stdin()
                .take(MAX_PLAN_BYTES + 1)
                .read_to_end(&mut bytes)
                .await
        };
        tokio::time::timeout(Duration::from_secs(INPUT_TIMEOUT_SECS), read)
            .await
            .map_err(|_| InputFailure {
                code: "INPUT_TIMEOUT",
                message: "plan input timed out",
            })?
            .map_err(|_| invalid_plan("could not read plan input"))?;
    } else {
        // Linux O_NOFOLLOW | O_NONBLOCK: reject symlinks/FIFOs without hanging.
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(0x20000 | 0x800)
            .open(path)
            .map_err(|_| invalid_plan("cannot open a regular, non-symlink plan file"))?;
        let meta = file
            .metadata()
            .map_err(|_| invalid_plan("cannot inspect plan file"))?;
        if !meta.is_file() || meta.len() > MAX_PLAN_BYTES {
            return Err(invalid_plan(
                "plan must be a regular file no larger than 2 MiB",
            ));
        }
        std::io::Read::take(file, MAX_PLAN_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid_plan("could not read plan file"))?;
    }
    if bytes.len() as u64 > MAX_PLAN_BYTES {
        return Err(invalid_plan("plan exceeds 2 MiB"));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| invalid_plan("plan does not match the report schema"))
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        println!("{HELP}\nVersion: {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args == ["--version"] {
        println!("traefik-network-controller {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let command = match parse_command(&args) {
        Ok(command) => command,
        Err(error) => {
            eprintln!(
                "{}",
                json!({"schema_version":1, "controller_version":env!("CARGO_PKG_VERSION"),
                "error":{"code":"INVALID_ARGUMENT", "message":error.to_string()}})
            );
            std::process::exit(2);
        }
    };
    let name = args.first().map(String::as_str).unwrap_or("run");
    if let Err(error) = execute(command).await {
        let message: String = format!("{error:#}").chars().take(1024).collect();
        let code = error
            .downcast_ref::<InputFailure>()
            .map(|e| e.code)
            .unwrap_or_else(|| controller::error_code(&error));
        eprintln!(
            "{}",
            json!({"schema_version":1, "controller_version":env!("CARGO_PKG_VERSION"),
            "command":name,"error":{"code":code,"message":message,
                "details": controller::error_details(&error)}})
        );
        std::process::exit(1);
    }
}

async fn execute(command: Command) -> Result<()> {
    if command == Command::Capabilities {
        println!(
            "{}",
            serde_json::to_string_pretty(&controller::capabilities())?
        );
        return Ok(());
    }
    let cfg = Config::from_env()?;
    if command == Command::Healthcheck {
        return check_health(&cfg);
    }
    // Read input before locking the operation; a slow pipe cannot block reconciliation.
    let saved = if let Command::Apply(path) = &command {
        Some(read_plan(path).await?)
    } else {
        None
    };
    let docker = Docker::new(&cfg.socket_path, Duration::from_secs(cfg.api_timeout_secs))?;
    if command == Command::Run {
        return run(docker, cfg).await;
    }
    let _lock = operation_lock(&cfg).await?;
    if command == Command::Doctor {
        let report = controller::status(&docker, &cfg).await?;
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "schema_version":1, "controller_version":env!("CARGO_PKG_VERSION"), "command":"doctor",
                "checks": {"docker_snapshot":"PASSED", "target_policy":"PASSED",
                    "daemon_health":if check_health(&cfg).is_ok() {"READY"} else {"NOT_READY_OR_UNAVAILABLE"},
                    "routed_request":"NOT_CHECKED"}, "report":report
            }))?
        );
        return Ok(());
    }
    let report = match command {
        Command::Once => controller::reconcile(&docker, &cfg, cfg.dry_run).await?,
        Command::Plan => controller::reconcile(&docker, &cfg, true).await?,
        Command::Status => controller::status(&docker, &cfg).await?,
        Command::Prepare(app) => controller::prepare(&docker, &cfg, &app).await?,
        Command::Retire(app) => controller::retire(&docker, &cfg, &app).await?,
        Command::Apply(_) => {
            controller::apply_plan(&docker, &cfg, saved.as_ref().expect("loaded plan")).await?
        }
        Command::Adopt {
            app,
            target,
            network,
            endpoint,
        } => controller::adopt(&docker, &cfg, &app, &target, &network, &endpoint).await?,
        _ => bail!("unsupported command"),
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

async fn one_pass(docker: &Docker, cfg: &Config) -> Result<Report> {
    let _lock = operation_lock(cfg).await?;
    controller::reconcile(docker, cfg, cfg.dry_run).await
}

async fn run(docker: Docker, cfg: Config) -> Result<()> {
    let _daemon_lock = daemon_lock(&cfg)?;
    write_health(&cfg, false, "initializing")?;
    {
        let _lock = operation_lock(&cfg).await?;
        controller::begin_session(&cfg)?;
    }
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let signal_task = tokio::spawn(async move {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
        let _ = shutdown_tx.send(true);
        Ok::<(), std::io::Error>(())
    });
    let (events_tx, mut events_rx) = mpsc::channel(1);
    let events_docker = docker.clone();
    let events_shutdown = shutdown_rx.clone();
    let events_task = tokio::spawn(async move {
        events_docker.watch_events(events_tx, events_shutdown).await;
    });
    let mut interval = tokio::time::interval(Duration::from_secs(cfg.interval_secs));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    log_event(
        "info",
        "controller_started",
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "target": cfg.target_name,
            "project": cfg.target_project,
            "dry_run": cfg.dry_run,
            "auto_detach": cfg.auto_detach,
            "interval_seconds": cfg.interval_secs
        }),
    );
    let outcome: Result<()> = async {
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => break,
                _ = interval.tick() => {},
                event = events_rx.recv() => {
                    if event.is_none() {
                        // Keep periodic reconciliation even if an event worker exits.
                        tokio::select! {
                            _ = shutdown_rx.changed() => break,
                            _ = tokio::time::sleep(Duration::from_secs(cfg.interval_secs)) => {}
                        }
                    }
                }
            }
            let result = tokio::select! {
                _ = shutdown_rx.changed() => break,
                result = one_pass(&docker, &cfg) => result
            };
            match result {
                Ok(report) => {
                    let ready = report.pending.is_empty();
                    write_health(
                        &cfg,
                        ready,
                        if ready {
                            "reconciliation complete"
                        } else {
                            "reconciliation has pending changes"
                        },
                    )?;
                    log_event("info", "reconciled", serde_json::to_value(report)?);
                }
                Err(error) => {
                    {
                        let _lock = operation_lock(&cfg).await?;
                        controller::reset_grace(&cfg)?;
                    }
                    let message: String = format!("{error:#}").chars().take(512).collect();
                    write_health(&cfg, false, &message)?;
                    log_event(
                        "error",
                        "reconcile_failed",
                        json!({
                            "error": message, "code": controller::error_code(&error)
                        }),
                    );
                }
            }
            // Coalesce self-generated and bursty events without delaying shutdown.
            tokio::select! {
                _ = shutdown_rx.changed() => break,
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        }
        Ok(())
    }
    .await;
    // Cancellation may leave a successful connect unobserved; the next snapshot repairs state.
    // Ordinary shutdown never disconnects a network.
    events_task.abort();
    let _ = events_task.await;
    signal_task.abort();
    write_health(&cfg, false, "controller stopped")?;
    log_event("info", "controller_stopped", json!({}));
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_string()).collect()
    }

    #[test]
    fn read_only_commands_reject_mutation_and_unknown_flags() {
        for command in ["status", "doctor", "plan", "capabilities"] {
            for flag in ["--force", "--apply", "--json"] {
                assert!(parse_command(&args(&[command, flag])).is_err());
            }
        }
        assert_eq!(parse_command(&[]).unwrap(), Command::Run);
        assert_eq!(parse_command(&args(&["plan"])).unwrap(), Command::Plan);
        for incomplete in [["prepare"], ["retire"], ["apply"], ["adopt"]] {
            assert!(parse_command(&args(&incomplete)).is_err());
        }
    }

    #[test]
    fn adoption_requires_three_distinct_identity_flags() {
        assert_eq!(
            parse_command(&args(&[
                "adopt",
                "echo",
                "--endpoint-id",
                "e",
                "--target-id",
                "t",
                "--network-id",
                "n"
            ]))
            .unwrap(),
            Command::Adopt {
                app: "echo".into(),
                target: "t".into(),
                network: "n".into(),
                endpoint: "e".into()
            }
        );
        assert!(
            parse_command(&args(&[
                "adopt",
                "echo",
                "--target-id",
                "t",
                "--target-id",
                "t",
                "--endpoint-id",
                "e"
            ]))
            .is_err()
        );
        assert!(
            parse_command(&args(&[
                "adopt",
                "echo",
                "--target-id",
                "t",
                "--network-id",
                "n",
                "--force",
                "e"
            ]))
            .is_err()
        );
    }
}
