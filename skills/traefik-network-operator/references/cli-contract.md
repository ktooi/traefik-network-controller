# Release-aware CLI and safety contract

## Contents

- Release discovery and invocation
- Commands
- Saved plan application
- Configuration and network selection
- Ownership and failure boundaries

## Release discovery and invocation

Use the installed binary and release-matched `SPEC.md` and `docs/operations.md` as the authority. This reference covers the v0.1.0 baseline and the v0.2.0 interface; it does not establish that either is installed on a host. With runtime access, verify `--help` before using a command or flag. With files only, prepare commands from the supplied release contract and mark runtime verification pending. No command below takes an invented `--json` option: supported reports are already JSON.

When an existing Compose service is named `controller`, this is the invocation pattern:

```sh
docker compose exec -T controller traefik-network-controller --version
docker compose exec -T controller traefik-network-controller --help
docker compose exec -T controller traefik-network-controller plan
```

Preserve the deployment's actual `-f`, `--project-name`, Docker context, and service selections. Compose commands here select the controller's deployment project; `TNC_TARGET_PROJECT` instead selects the target Traefik container's exact Compose project label. Do not substitute one for the other without evidence that they are the same. These examples do not authorize a connection to an unspecified host. For a local host binary, invoke it directly with the verified configuration and shared state directory. Use one coordinated controller and state directory for the Engine's managed network namespace: the target project selects Traefik, not a separate namespace. Do not create a second unsynchronized controller instance.

The outer Docker CLI context selects where the existing service is executed. The controller itself uses only `TNC_SOCKET_PATH`, an absolute local Unix socket path; it does not follow `DOCKER_HOST`, Docker contexts, or a TCP fallback. Do not assume setting a remote CLI context redirects a separately invoked host binary.

## Commands

| Command | Release | Effect and caution |
| --- | --- | --- |
| `--version`, `--help` | v0.1.0+ | Discover the actual installed interface. |
| `plan` | v0.1.0+ | Inspect current state and report proposed actions without Docker mutations. A plan does not authorize changes. |
| `run` | v0.1.0+ | Run event-driven and periodic reconciliation; this is a long-lived write-capable process. |
| `once` | v0.1.0+ | Reconcile once under current configuration. It is not a synonym for inspection. |
| `prepare APP` | v0.1.0+ | Create or validate the managed edge. In v0.2.0, create/connect only the named application's edge; do not mutate another application's attachment. |
| `retire APP` | v0.1.0+ | Explicitly retire an unused managed network; can disconnect the validated target, including a pre-existing unowned attachment, and delete the network. |
| `healthcheck` | v0.1.0+ | Inspect recent local daemon reconciliation state, not an end-to-end application or isolation probe. |
| `capabilities` | v0.2.0 | Discover machine-readable commands and feature support. Verify availability first. |
| `status` | v0.2.0 | Report current observations and lifecycle decisions without Docker mutations. |
| `doctor` | v0.2.0 | Diagnose configuration and observed state without Docker mutations. Read reason fields and completeness, not only exit status. |
| `apply FILE` or `apply -` | v0.2.0 | Revalidate and apply a saved reconciliation plan. It does not replay arbitrary operations or confer permission. |
| `adopt APP --target-id ID --network-id ID --endpoint-id ID` | v0.2.0 | Explicitly record ownership of the currently matching attachment after policy checks. IDs must come from fresh observations. |

In v0.1.0, reconciliation adds attachments only; it does not implement automatic detach, saved-plan application, or explicit attachment adoption. Do not simulate missing commands with direct Docker mutations. Prepare an upgrade proposal or use the release's documented workflow.

For v0.2.0, use `schemas/report.schema.json` and `schemas/error.schema.json` from the same release when available. Inspect `schema_version`, `controller_version`, `observed_at`, `observation_complete`, `observation.all_containers`, `policy`, `target_id`, `decisions`, `applied`, `pending`, `post_verified`, and `plan_expires_at`. Do not hard-code an expected success phrase or interpret unknown reason codes as permission to mutate. `applied` records a result; `pending` is an eligible action list, not a timer queue. `GRACE_PENDING` may have no pending action. `post_verified` concerns required membership checks, not traffic or every external security goal.

Each decision gives `network`, `network_id`, `phase`, `ownership`, `observed_attached`, `desired_attached`, `endpoint_id`, `reference_ids`, `action`, `reason_code`, `unused_since`, and `empty_observations`. Ownership values are `unowned`, `connected`, and `adopted`. Decision actions are `connect`, `disconnect`, `keep`, and `blocked`; action records use `operation`. Explicit network deletion is serialized as `operation: "remove"`, not `delete`.

In v0.2.0, `plan`, `status`, and `doctor` do not advance the lifecycle journal or acquire attachment ownership. Local locks are permitted. Repeated inspection alone does not advance cleanup grace. An explicitly prepared edge that has never been used remains prepared; it is retired explicitly. Automatic detach applies to a previously used owned edge that becomes unused, and a detached edge must not reconnect merely because its network still exists.

Exit codes are `0` for a completed command, `1` for an operation failure, and `2` for invalid command arguments. A completed diagnostic report may still describe an unhealthy daemon or a blocked lifecycle decision. In the `doctor` output, inspect `checks.daemon_health` and its nested `report`; the command does not perform an HTTP request. `healthcheck` is silent on success and exits nonzero for unavailable, unready, or stale daemon health. Keep JSON result stdout separate from JSON error diagnostics on stderr. On failure, inspect `error.code` and `error.details` for completed actions and re-observe before retrying: failure does not prove that no earlier action succeeded. Keep raw errors out of public reports when they include unrelated host information.

Only the daemon refreshes daemon health. A dry-run daemon remains unready while eligible connect/disconnect actions are pending. Grace or a conservative blocked decision may have no eligible pending action, so a healthy result still does not prove that an application is reachable or the requested network state is achieved.

For v0.2.0 explicit `prepare`, `retire`, or `adopt` preview, set `TNC_DRY_RUN=true` for that command; do not pretend a general `plan` represents those operations. For example, against an existing controller service:

```sh
docker compose exec -T -e TNC_DRY_RUN=true controller traefik-network-controller prepare echo
```

This previews the named preparation without Docker or ownership mutation. It is not an applicable saved reconciliation plan: `apply` accepts only currently allowed connect/disconnect reconciliation actions, never supplied create/remove/adopt operations.

## Saved plan application

With a verified v0.2.0 service, save a successfully generated plan to an operator-controlled local file:

```sh
docker compose exec -T controller traefik-network-controller plan > tnc-plan.json
```

Inspect it and confirm that its entire action scope is already authorized. Then pass that exact file through stdin; a file on the host is not automatically visible inside the container:

```sh
docker compose exec -T controller traefik-network-controller apply - < tnc-plan.json
```

The v0.2.0 reader accepts a regular non-symlink file or bounded stdin, up to 2 MiB; stdin has a 10-second read timeout. The plan lifetime is 300 seconds. If the plan expires or material state changes, regenerate and review it; do not modify timestamps or identity pins to force acceptance. A running daemon may advance the journal revision between plan and apply, so `STALE_PLAN` can be expected even when the operator did nothing else.

After `STALE_PLAN`, re-observe current status and Docker membership first. If the daemon already reached the authorized desired state, report convergence instead of repeatedly applying. Do not edit the plan or stop the daemon just to bypass freshness checks. If a manual change needs a stable review window, coordinate external deployments and an explicitly authorized temporary daemon pause, retaining the same state volume and configuration.

Only after that agreed pause has stopped the daemon, use this one-shot workflow against the verified controller Compose project:

```sh
docker compose run --rm --no-deps -T controller plan > tnc-plan.json
# Review this new plan, then apply within its validity window.
docker compose run --rm --no-deps -T controller apply - < tnc-plan.json
```

These invocations create and remove helper containers; they are not read-only host operations. Re-read membership afterward. Restore a daemon that was previously running with `docker compose up -d --no-deps controller`; preserve its stopped state when it was intentionally stopped before the workflow. A coordinated pause does not serialize unrelated Docker administrators.

## Configuration and network selection

Use configuration keys supported by the actual release. Keep environment values explicit strings in Compose.

| Key | Meaning |
| --- | --- |
| `TNC_TARGET_PROJECT` | Mandatory exact Traefik Compose project label. |
| `TNC_TARGET_NAME`, `TNC_TARGET_SERVICE` | Exact target name and Compose service scope; defaults are `traefik`. |
| `TNC_SOCKET_PATH` | Local Docker Unix socket, default `/var/run/docker.sock`. |
| `TNC_STATE_DIR` | Persistent controller state and locks, default `/run/tnc`. Share it across permitted controller invocations. |
| `TNC_EGRESS_NETWORK` | Fixed egress network, default `traefik_egress`; preserve it at gateway priority 100. |
| `TNC_INTERVAL_SECS` | Reconciliation interval; default 30. |
| `TNC_DRY_RUN` | Preview mode; verify each command's release contract. It does not make a broad Docker credential read-only. |
| `TNC_AUTO_DETACH` | v0.2.0 opt-in automatic detach; default `false`. |
| `TNC_DETACH_GRACE_SECS` | v0.2.0 unused-attachment grace; default 300 seconds, accepted range 1–604800. |
| `TNC_PROTECTED_NETWORKS` | v0.2.0 comma-separated exact physical Docker network names, not Compose logical keys or patterns. Surrounding whitespace is trimmed; empty CSV entries are invalid. Include actual fixed control/WAF/shared migration names. Egress is protected independently. |

The controller selects local default bridge networks with the exact name `traefik_app_<app>` and these labels:

```yaml
com.nofmem.traefik.autojoin: "true"
com.nofmem.traefik.role: "application-edge"
com.nofmem.traefik.owner: "<app>"
```

Use `prepare APP` to create them. The application slug contains 1–48 ASCII characters, starts with a lowercase letter or digit, and otherwise contains lowercase letters, digits, `_`, or `-`. Selection labels alone do not bypass validation of IDs, scope, driver, properties, or attachment ownership. Managed edge networks are not `internal: true`; application DB/backend networks can be internal and must not be selected for Traefik attachment.

The application's Compose uses an external edge network and an exact `traefik.docker.network` label. That label does not connect Traefik to the network. Keep only the application frontend on the edge network and keep DB/cache services on the application's separate internal network. The managed Traefik endpoint uses gateway priority -1; retain the fixed higher-priority egress.

## Ownership and failure boundaries

- Docker socket access remains powerful even when the socket bind mount is `:ro`, capabilities are dropped, and the container filesystem is read-only. Keep the controller isolated with `network_mode: none`, no listener, and its own persistent state volume.
- Do not widen Traefik's Docker Socket Proxy. Its discovery path remains separate; the controller uses its Unix socket for the narrow implemented network operations.
- Network ownership is not attachment ownership. A pre-existing/manual attachment is unowned until explicit validated adoption. A successful-looking endpoint after an ambiguous API failure is not proof that the controller created it.
- Persisted attachment ownership binds controller identity, full target/network IDs, and endpoint ID. Changed IDs, missing state, corrupted state, incomplete inspection, or inconsistent membership prevent unsafe detach. Do not repair these by fabricating journal entries.
- The Linux runtime must expose `/proc/sys/kernel/random/boot_id`. A boot-ID change resets grace, including one-shot execution after a host reboot. Daemon restart, a long observation gap, relevant policy changes, and failed observations also reset grace conservatively; wall-clock elapsed time alone is not sufficient evidence for cleanup.
- References include all non-target containers, running or stopped; only the selected Traefik target is excluded. An empty running-container list, failed inspection, or removed Compose file is not retirement evidence.
- Automatic detach never deletes networks or application containers, never uses forced disconnect, and never disconnects another container. Explicit `retire` has broader network-lifecycle semantics: it can remove a validated unused managed network even when its target attachment is unowned. Scope that explicit authority accordingly; do not perform an unnecessary adoption just to retire it.
- The operation lock coordinates controllers sharing the same state directory. It does not serialize unrelated Docker commands, Compose, or other administrators. Docker disconnect has no conditional endpoint-ID compare-and-swap; use one management authority for each managed attachment and avoid concurrent external edits.
- The controller does not protect against a hostile Docker administrator or hostile holders of the Docker socket. It does not provide L7 authorization, WAF inspection, DNS readiness, firewall enforcement, or complete host isolation.
