# CLI and machine contract

**Release:** 0.2.0 · **Report schema:** 1 · [日本語](cli.ja.md) · [Operations](operations.md)

Use the installed binary as the first source of available commands:

```sh
traefik-network-controller --version
traefik-network-controller --help
traefik-network-controller capabilities
```

These three invocations do not require `TNC_TARGET_PROJECT` or a Docker connection. The remaining commands require the configured target scope. JSON is the default; there is no `--json`, `--force`, `detach`, `prune`, or implicit remote Docker fallback.

## Commands and effects

| Invocation | Docker effect | Persistent ownership effect |
| --- | --- | --- |
| `run` or no command | Reconcile on startup, events and interval; connect and, if enabled and eligible, disconnect | Persist observations, confirmed ownership and lifecycle; reset grace at daemon start |
| `once` | One reconciliation under the same policy | Persist the same evidence; does not refresh daemon health |
| `plan` | Read only; output a guarded reconciliation plan | Does not advance the journal |
| `status` | Read only; output current attachments, ownership and decisions | Does not advance the journal |
| `doctor` | Read only; validate snapshot/policy and report local daemon health | Does not advance the journal |
| `capabilities` | No Docker access | None |
| `healthcheck` | No Docker access; read local daemon health | None |
| `prepare APP` | Create/validate and attach **only the named** edge | Record a confirmed new attachment and preparation; does not adopt an existing attachment |
| `retire APP` | Explicitly disconnect Traefik and remove a validated unused managed edge | Remove retired state; may retire an unowned attachment after the explicit checks |
| `apply PLAN_FILE_OR_DASH` | Revalidate and apply only the still-current reconciliation plan | Commit only through current policy; reject stale/tampered preconditions |
| `adopt APP --target-id ID --network-id ID --endpoint-id ID` | Inspect only; no connect/disconnect | Explicitly claim the exact current attachment after full policy checks |

`APP` matches `[a-z0-9][a-z0-9_-]{0,47}`; IDs must be full 64-character lowercase hexadecimal Docker identities. Adopt accepts its three flags in any order, exactly once each. No other subcommand has flags in this release.

All non-daemon operations use the shared operation lock where state coordination is needed. Read-only commands may create/check local coordination files and restrict state-directory permissions. This is distinct from changing Docker or the ownership journal. A `compose run` wrapper has its own container/volume effects; see [operations](operations.md).

`TNC_DRY_RUN=true` prevents controller mutation, including ownership changes; reports describe intended operations. Run/once/prepare/retire/apply/adopt still validate their scope. Prefer `plan` for a read-only reconciliation preview. There is no saved-plan workflow for explicit `prepare`, `retire`, or `adopt`; `apply` accepts reconciliation plans containing only eligible connect/disconnect operations.

## Environment

Boolean values must be exactly lowercase `true` or `false`. Seconds are positive decimal integers within the specified range.

| Variable | Default | Meaning / accepted range |
| --- | --- | --- |
| `TNC_TARGET_PROJECT` | Required | Exact existing Compose project label |
| `TNC_TARGET_NAME` | `traefik` | Exact container name |
| `TNC_TARGET_SERVICE` | `traefik` | Exact Compose service label |
| `TNC_EGRESS_NETWORK` | `traefik_egress` | Physical fixed network name; always protected; target attachment priority must be 100 |
| `TNC_SOCKET_PATH` | `/var/run/docker.sock` | Absolute local Unix socket path |
| `TNC_STATE_DIR` | `/run/tnc` | Absolute shared persistent directory |
| `TNC_INTERVAL_SECS` | `30` | Reconciliation interval, 1–86400 |
| `TNC_API_TIMEOUT_SECS` | `10` | Docker API timeout, 1–300 |
| `TNC_HEALTH_MAX_AGE_SECS` | `90` | Maximum daemon health age, 1–604800 |
| `TNC_AUTO_DETACH` | `false` | Enable automatic detach of eligible owned attachments |
| `TNC_DETACH_GRACE_SECS` | `300` | Required unused grace, 1–604800 |
| `TNC_PROTECTED_NETWORKS` | Empty | Comma-separated exact physical names; whitespace around names is trimmed; empty entries are invalid |
| `TNC_DRY_RUN` | `false` | Additional controller mutation guard |

The Compose example sets additional protected control/WAF/legacy names; those are example configuration, not built-in universal network names. The required egress is protected even when the CSV is empty. `attachments.json` is the ownership journal; `health.json` is separate daemon health. Do not edit either to bypass policy.

## Report schema

[schemas/report.schema.json](../schemas/report.schema.json) describes report schema 1. It covers `plan`, `status`, `once`, `prepare`, `retire`, `apply`, and `adopt` reports. `doctor` wraps a report in its own object, and capabilities is a separate discovery document. Reject unsupported schema/controller versions before using a saved plan.

| Field | Meaning |
| --- | --- |
| `schema_version`, `controller_version` | Machine contract and exact producing release |
| `target_id` | Verified full target ID when observed; nullable before identification |
| `dry_run` | This report represents a non-mutating evaluation |
| `managed_networks` | Number of managed candidates represented by the snapshot |
| `observed_at`, `plan_expires_at` | Unix seconds; saved-plan validity is 300 seconds |
| `observation_complete` | Whether a complete validated observation was obtained |
| `observation` | `complete`, `all_containers`, counts, and `started_at`/`finished_at` Unix seconds |
| `policy` | Effective target/socket/state/egress, interval/timeout, dry-run, detach and protection configuration |
| `journal` | `present`, `controller_id`, `revision`; the state-generation guard |
| `inputs` | API version, target attachments, network/reference observations and journal guard; used for freshness checks |
| `decisions` | Per-edge observed state, ownership, lifecycle and reason |
| `pending` | Concrete eligible operations not applied in this report; not a timer queue |
| `applied` | Operations completed by this invocation |
| `post_verified` | Required post-action membership verification completed; not a traffic health assertion |

A decision contains `network`, `network_id`, `phase`, `ownership`, `observed_attached`, `desired_attached`, nullable `endpoint_id`, `reference_ids`, `action`, `reason_code`, nullable `unused_since`, and `empty_observations`. Reference IDs include stopped non-target containers. Arrays derived from sets are deterministic. Phases are `prepared`, `in_use`, `unused_pending`, `detached`. Ownership is `unowned`, `connected`, or `adopted`.

Decision actions are `connect`, `disconnect`, `keep`, and `blocked`. Action records in `pending`/`applied` have `operation`, `network`, nullable `network_id`, nullable `endpoint_id`, and `reason_code`. Explicit operations additionally use `create`, **`remove`**, and `adopt`; network retirement is serialized as `remove`, not `delete`.

`GRACE_PENDING` can appear with no pending operation because disconnect is not yet eligible. A read-only report has `applied=[]` and does not certify post-mutation success. A successful command may still include a conservative `blocked` decision. Inspect decisions and effective policy rather than treating exit 0 or `post_verified` as proof that every external goal is satisfied.

### Decision reasons

| Reason | Interpretation / appropriate response |
| --- | --- |
| `PROTECTED_NETWORK` | Preserve its observed attachment; no explicit lifecycle command may override protection. |
| `REFERENCED` | A non-target container references it; retain attachment. |
| `ATTACHMENT_MISSING` | Policy requires a connection which is absent. |
| `PREPARED_NEVER_USED` | Keep the initial empty preparation. |
| `ALREADY_DETACHED` | Keep the unused edge detached. |
| `AUTO_DETACH_DISABLED` | Preserve/connect under connect-only policy. |
| `OWNERSHIP_UNKNOWN` | Do not auto-detach; investigate or explicitly adopt the intended exact attachment. |
| `ENDPOINT_ID_UNAVAILABLE` | Identity evidence cannot authorize cleanup. |
| `GRACE_PENDING` | Wait for grace and additional complete observations. |
| `UNUSED_GRACE_ELAPSED` | Eligible for revalidated automatic detach. |
| `PREPARE_REQUESTED` | Explicit preparation action. |
| `RETIRE_REQUESTED` | Explicit retirement action. |
| `ADOPT_REQUESTED` | Explicit ownership change. |

## Saved plans

Use an unmodified `plan` report from the same release and policy. `apply -` accepts piped JSON; a file path refers to the filesystem **inside the process/container**, not automatically the host file. Input is limited to 2 MiB. Regular plan files must not be symlinks or special files. Stdin must be piped and finish within 10 seconds.

Apply compares fresh identities, policy, inventory, journal revision and currently allowed actions. It rejects expiry and material changes even if the input JSON has valid syntax. A daemon pass may advance the revision within the nominal 300-second lifetime. The plan is not signed authority and editing the action list cannot bypass current policy. Follow the coordinated workflow in [operations](operations.md#3-plan-and-apply).

## Doctor and health

Doctor returns `schema_version`, `controller_version`, `command: "doctor"`, `checks`, and `report`. The following is an illustrative `checks` object after a valid snapshot with unavailable local daemon health:

```json
{
  "docker_snapshot": "PASSED",
  "target_policy": "PASSED",
  "daemon_health": "NOT_READY_OR_UNAVAILABLE",
  "routed_request": "NOT_CHECKED"
}
```

With valid local daemon health, `daemon_health` is `READY`. Snapshot/policy failure exits nonzero. Unready/missing local daemon health is reported separately and does not alone fail `doctor`. `healthcheck` is silent on success and exits nonzero when the recent, identity-matched daemon health state is unavailable/unready/stale. It makes no Docker or HTTP call.

Only daemon reconciliation updates daemon health. A successful `once`, `status` or `doctor` does not make a stopped or stale daemon healthy. A dry-run daemon remains unready when eligible connect/disconnect actions are pending. Grace or conservative blocked decisions may have no eligible pending action, so a healthy result still does not prove an external goal or route is satisfied. Inspect decisions and effective policy.

## Exit codes, errors and logs

| Exit | Meaning |
| --- | --- |
| `0` | Command completed according to its contract; pending/grace/blocked decisions may remain. |
| `1` | Configuration, input, policy, Docker, state, freshness or execution failure. |
| `2` | Invalid command name/argument shape. |

Successful one-shot operations print one JSON document to stdout. Help/version are text; healthcheck succeeds without output. Daemon logs and command failures go to stderr. Do not mix stderr into a saved plan. Error envelopes follow [schemas/error.schema.json](../schemas/error.schema.json), including `error.code`, bounded `error.message` and, when available, structured details about completed actions. Diagnostics are not a raw Docker-environment dump.

Match documented codes and fields, not English message fragments. Important error groups are:

| Codes | Response |
| --- | --- |
| `INVALID_ARGUMENT` | Correct the documented command syntax; exit 2. |
| `INVALID_PLAN`, `INPUT_TIMEOUT` | Correct the bounded file/stdin input; do not alter a plan to bypass policy. |
| `STALE_PLAN` | Obtain and review a new plan. Rejection itself does not advance the journal. |
| `OBSERVATION_INCOMPLETE` | Restore complete Docker observation. A mutating command conservatively breaks prior grace continuity. |
| `JOURNAL_INVALID`, `JOURNAL_WRITE_FAILED` | Preserve evidence; follow state recovery. Do not hand-edit authority into the journal. |
| `TARGET_CHANGED`, `NETWORK_CHANGED`, `OWNERSHIP_CHANGED` | Inspect the new generation; old IDs or ownership do not carry over automatically. |
| `PROTECTED_NETWORK`, `REFERENCES_PRESENT`, `NETWORK_NOT_MANAGED` | Respect the configured boundary and real references; do not force cleanup. |
| `DETACH_REFUSED`, `ADOPT_REFUSED`, `ACTION_REFUSED` | Review the exact intended action and current policy before any retry. |
| `CONNECT_AMBIGUOUS`, `CONNECT_FAILED`, `DISCONNECT_AMBIGUOUS` | Inspect actual membership and completed actions; do not infer ownership or absence from the failed response. |
| `POST_VERIFY_FAILED`, `ENDPOINT_ID_UNAVAILABLE` | Preserve uncertainty and inspect identity/membership; success is not established. |
| `OPERATION_FAILED` | Read the bounded diagnostic and current state; no more specific typed classification was provided. |

For an unknown code, retain the complete bounded diagnostic and stop that mutation; do not guess a bypass. In a partial failure, inspect `error.details.applied` and fresh state before deciding on a retry. Error details mark `post_verified: false`; a nonzero exit does not prove that no earlier action succeeded.
