# Operations guide

**Release:** 0.2.0 · [日本語](operations.ja.md) · [CLI contract](cli.md) · [Normative requirements](../SPEC.md)

This guide uses one existing Traefik Compose project and a disposable `echo` application. It distinguishes Docker state, controller evidence, and actual traffic. Check [TESTING.md](../TESTING.md) before rollout; real-Engine and routed-traffic acceptance remain separate gates.

## 1. Install or upgrade

### Establish the target and layout

Use these paths:

- `~/traefik/compose.yaml`: existing Traefik stack.
- `~/traefik/compose.overwrite.yaml`: merged integration example.
- `~/traefik/traefik-network-controller/`: this complete source release, including the lockfile.

`compose.overwrite.yaml` is not an automatically discovered Compose filename. Always supply both files. If an integration file already exists, merge [the example](../examples/compose.overwrite.yaml); do not overwrite unrelated services or network definitions.

Read only the target identity fields you need:

```sh
docker inspect traefik --format 'id={{.Id}} name={{.Name}} project={{index .Config.Labels "com.docker.compose.project"}} service={{index .Config.Labels "com.docker.compose.service"}}'
docker version --format '{{json .Server}}'
docker compose version
```

Set the existing Compose project name, not a newly invented project:

```sh
cd ~/traefik
export TNC_TARGET_PROJECT=your_existing_traefik_project

tnc_compose() {
  docker compose -p "$TNC_TARGET_PROJECT" \
    -f compose.yaml -f compose.overwrite.yaml "$@"
}
```

Persist the same setting in deployment tooling. Match `TNC_TARGET_NAME` and `TNC_TARGET_SERVICE` to the inspected values. The target must be running and have the configured project/service labels. One controller deployment manages this Engine-wide `traefik_app_` namespace; a different project setting does not create a second namespace.

### Establish fixed networks and controller state

Retain existing control and WAF networks and the shared legacy network during migration. Add the non-internal egress network with `gw_priority: 100`. The controller connects application edges at priority `-1`; using ordinary Compose `priority` does not configure the default gateway.

Set `TNC_PROTECTED_NETWORKS` to the **physical Docker network names** of fixed control/WAF/legacy networks. The required egress network is always protected. A protected managed edge is preserved and explicit `prepare`, `retire`, and `adopt` refuse it.

Keep `TNC_AUTO_DETACH=false` for initial acceptance. Preserve the named `controller_state` volume mounted at `/run/tnc`. Do not share it with applications or create a second independently managed copy. Traefik's existing Socket Proxy keeps `POST="0"` and `NETWORKS="0"`; the controller has its own local socket mount.

```sh
tnc_compose config -q
tnc_compose build --pull controller
# This initial fixed-network change may recreate Traefik.
tnc_compose up -d --no-deps traefik
tnc_compose run --rm --no-deps controller --version
tnc_compose run --rm --no-deps controller capabilities
tnc_compose run --rm --no-deps controller plan
```

Resolve target, egress, protection and network-policy errors before starting the daemon. Plan is Docker-read-only, while its `compose run` wrapper creates/removes a helper container and may create the volume. In a strictly read-only engagement, use an already-running controller with `exec` or an already-installed local binary with approved read access; do not silently create a helper.

```sh
tnc_compose up -d --no-deps controller
tnc_compose exec -T controller traefik-network-controller doctor
tnc_compose logs --tail=100 controller
```

The example runs the controller without an application network or HTTP port, with a read-only filesystem, dropped capabilities and a writable state volume. Its usual rootful socket access still confers Docker administrative power. Rootless Docker, user-namespace remapping, unusual socket ownership and non-Linux hosts require separate acceptance.

### Upgrade from 0.1.0

Keep the existing labels, names and state volume. Build the new source and recreate only the controller after configuration validation. Existing Traefik attachments have no 0.2.0 endpoint ownership evidence and remain **unowned** until explicitly adopted. Enabling auto-detach does not implicitly take them over.

0.2.0 narrows `prepare APP` to creating/connecting that named edge; it does not reconcile unrelated edges. `plan` now provides the versioned saved-plan contract. Update clients that assumed the old JSON layout. See [the changelog](../CHANGELOG.md).

## 2. Add the echo application

Prepare its edge before deployment:

```sh
cd ~/traefik
tnc_compose exec -T controller traefik-network-controller prepare echo
```

This creates or validates `traefik_app_echo` and attaches the verified target. Repeating it is safe within the same validated scope; it is not an ownership takeover for a pre-existing attachment. An empty edge that has never been used stays prepared until explicitly retired.

Copy [examples/echo.compose.yaml](../examples/echo.compose.yaml) into the application's own directory. It uses `traefik/whoami:v1.12.0`, listens on 8080 as UID/GID 65532, publishes no host port, and joins the prepared external edge. Set `ECHO_HOSTNAME` to a hostname you control and preserve the existing deployment's TLS resolver and WAF/authentication middleware names. `echo.example.test` is a placeholder and does not obtain a publicly trusted certificate.

```sh
cd ~/echo
# Replace this placeholder with the hostname and TLS setup under test.
export ECHO_HOSTNAME=echo.example.test
docker compose config -q
docker compose up -d
```

The echo response includes request headers and network details. Do not pass secrets, authentication cookies, or sensitive forwarded headers to a public diagnostic demo. Remove the demo when its test is complete.

Verify both topology and the intended route:

```sh
docker network inspect traefik_app_echo --format '{{json .Containers}}'
docker inspect traefik --format '{{json .NetworkSettings.Networks}}'
# Use the actual hostname and its configured certificate trust.
curl --fail --silent --show-error "https://$ECHO_HOSTNAME/"
```

An application still attached to the legacy shared bridge retains that shared path. During migration, change its `traefik.docker.network` to the dedicated edge and remove its shared-network attachment together. After every application has moved and traffic is accepted, removal of Traefik's legacy attachment is a separate reviewed infrastructure change. The controller does not perform that migration.

## 3. Plan and apply

`plan` returns a full JSON reconciliation plan. It does not save ownership, count a new persistent empty observation, create a network, or plan explicit retirement. A valid plan may contain pending actions and still exit 0.

```sh
cd ~/traefik
tnc_compose exec -T controller traefik-network-controller plan > tnc-plan.json
# Review the target, policy, decisions and pending actions.
tnc_compose exec -T controller traefik-network-controller apply - < tnc-plan.json
```

The plan expires after 300 seconds. Apply re-reads Docker and checks current policy, identities, journal revision and allowed actions. Normal daemon reconciliation can advance the journal revision before expiry; external deployment can change the snapshot. A stale plan is rejected. Regenerate and review it; do not edit the JSON or bypass the rejection.

For a coordinated manual change where a background daemon would continually invalidate the reviewed plan, record its initial state and stop only the controller temporarily. Existing Traefik connections remain. Then use one-shot operations sharing the same state volume:

```sh
tnc_compose stop controller
tnc_compose run --rm --no-deps -T controller plan > tnc-plan.json
# Review, then apply promptly within the plan's validity window.
tnc_compose run --rm --no-deps -T controller apply - < tnc-plan.json
# Restore the daemon if it was running before this workflow.
tnc_compose up -d --no-deps controller
```

Stopping the controller is a change requiring the existing operation's authority. It does not stop external Compose deployments; coordinate those separately. Plans contain infrastructure identifiers and policy paths and should be stored with appropriate access controls. They are not signed authorization tokens.

## 4. Enable automatic detach

First accept the disposable-engine lifecycle cases in [SPEC.md](../SPEC.md): stopped references, grace cancellation, endpoint replacement, restart, journal failure and no reconnect loop. Inspect current decisions:

```sh
tnc_compose exec -T controller traefik-network-controller status
```

For a pre-existing attachment that should be owned, capture its current full IDs immediately before takeover:

```sh
target_id=$(docker inspect traefik --format '{{.Id}}')
network_id=$(docker network inspect traefik_app_echo --format '{{.Id}}')
endpoint_id=$(docker inspect traefik --format '{{(index .NetworkSettings.Networks "traefik_app_echo").EndpointID}}')

tnc_compose exec -T controller traefik-network-controller adopt echo \
  --target-id "$target_id" --network-id "$network_id" --endpoint-id "$endpoint_id"
```

Do this only for the exact edge whose lifecycle is intended to be managed. `adopt` changes ownership state, not Docker attachment membership. If IDs change, review the new attachment instead of automatically retrying against it. Empty/missing IDs, protected edges and invalid policy are refused.

Set and persist `TNC_AUTO_DETACH=true` and the chosen `TNC_DETACH_GRACE_SECS` (default 300), validate configuration, then recreate only the controller. The controller does not edit its own Compose file. Inspect `status` again to verify the effective policy.

| Observation | Result |
| --- | --- |
| Running or stopped application container references the edge | Keep the Traefik attachment; cancel pending detach. |
| Prepared edge has never been used | Keep it. Use explicit retirement to abandon preparation. |
| Previously used, controller-owned edge has no references | Begin/continue grace under complete observations. |
| Grace and repeated complete checks pass | Revalidate and disconnect Traefik only; keep the network. |
| Empty detached edge remains | Keep it detached; network existence alone does not reconnect it. |
| Application returns to that edge | Connect the verified Traefik target again. |
| Ownership, inventory, endpoint identity or state is uncertain | Preserve or fail; report the reason. |

Auto-detach waits for at least two successful observations at different seconds and the grace window. A long observation gap, daemon restart, kernel boot-ID change (including one-shot use after a reboot), or relevant policy change resets grace conservatively. The implementation reads the Linux boot ID from `/proc/sys/kernel/random/boot_id`; the controller runtime must provide it. API failure is never interpreted as no references. `docker compose stop` does not retire an application because its containers still exist.

## 5. Retire an application

Pause deployments and other connection changes for this app. Remove its containers using its own existing Compose project; preserve volumes when keeping data:

```sh
cd ~/echo
docker compose down

cd ~/traefik
tnc_compose exec -T controller traefik-network-controller retire echo
```

Do not add `-v` unless data deletion is separately intended. Explicit `retire` validates the network's management labels, target and all references, disconnects only Traefik without force, verifies membership, then removes the empty managed network. Unlike automatic detach, this explicit retirement may remove a valid unused edge even when its current Traefik attachment is unowned. It still does not bypass protection, identity, reference or network-property checks.

If any running or stopped container refers to the edge, retire refuses it. Resolve the application's lifecycle deliberately; do not respond with `docker network disconnect --force`, network prune, or unrelated container deletion.

## 6. Diagnose and monitor

Use three separate observations:

| Layer | Observe | Interpretation |
| --- | --- | --- |
| Controller | Docker health, `healthcheck`, structured reconciliation errors | Whether the daemon recently reconciled its configured target. |
| Docker desired state | `status`/`plan` decisions, full network membership, ownership/protection and reference IDs | Whether actual attachments agree with policy and why a change is held. |
| User traffic | An external HTTPS request per representative route; DNS, TLS, WAF/auth behavior | Whether clients can use the service. |

There is no Prometheus endpoint or built-in HTTP exporter in 0.2.0. A host-side monitoring job can read JSON with the documented schema, collect Docker health, and expose its own metrics or logs. Give it read-only credentials appropriate to its probes; do not add an application network to the controller solely for monitoring. Suggested alerts are sustained reconciliation failure, stale daemon health, required attachment missing, an unexpected protected/unowned change, and route failure. A pending grace window or deliberately stopped controller is not automatically an incident.

```sh
tnc_compose exec -T controller traefik-network-controller status
tnc_compose exec -T controller traefik-network-controller doctor
tnc_compose exec -T controller traefik-network-controller healthcheck
tnc_compose logs --tail=100 controller
```

`doctor` reports snapshot/policy checks and local daemon health separately. It can exit successfully while `daemon_health` is `NOT_READY_OR_UNAVAILABLE`; it always marks routed-request checks `NOT_CHECKED`. Use `healthcheck` for a failing exit status on unhealthy daemon state. Health identity includes auto-detach, grace and protected-network configuration; a record from a different effective policy is not accepted.

| Symptom | First evidence and response |
| --- | --- |
| All Traefik routes return 404 | Inspect Traefik router/middleware/plugin/provider logs. Missing router configuration is different from an absent backend edge. Do not assume the controller caused it. |
| Plugin download/DNS failure after host reboot | Verify the actual DNS and plugin errors, upstream readiness and Traefik plugin initialization. Network reconciliation does not reload a disabled plugin. |
| Backend connection errors | Check the selected `traefik.docker.network`, actual full-ID attachment and backend port; then test the route. |
| `OWNERSHIP_UNKNOWN` | Preserve the attachment; decide whether explicit ID-pinned takeover is intended. |
| `STALE_PLAN` | Re-observe, regenerate and review; no force option exists. |
| `PROTECTED_NETWORK` | Confirm intended fixed-network policy; do not remove protection to make a cleanup command succeed. |
| Target/gateway validation failure | Correct the actual Compose identity or fixed egress configuration through infrastructure management. |

## 7. State, recovery and rollback

`attachments.json` in `TNC_STATE_DIR` contains controller identity, full resource/endpoint IDs and lifecycle evidence. Back it up together with the matching release/configuration using an atomic or quiesced volume snapshot. Do not print the full state unnecessarily or hand-edit it to authorize cleanup.

Missing state means existing attachments are unowned; it does not authorize detach. Corrupt, unsafe or incompatible state causes failure. Stop the controller, preserve the damaged evidence, and restore a known matching backup or deliberately rebuild state with explicit validation/adoption of each attachment. State recovery is an administrative operation, not a normal diagnostic command. Copying a journal does not safely create a second controller.

To stop automatic detach, set/persist `TNC_AUTO_DETACH=false` and recreate the controller. To stop all reconciliation, stop the controller; it leaves existing attachments intact. Neither operation automatically restores previously detached edges or a deleted network. Use `prepare APP` for an intentionally required edge, deploy the app, and verify its traffic. Reverting a source release alone does not revert Docker state.

The controller's locks cover its shared state directory. Docker's disconnect API cannot condition the write on an expected EndpointID, so independent concurrent connect/disconnect/deployment operations require external coordination. See [the security limits](security.md) and [SPEC AUTH-001 / LIFE-006](../SPEC.md).
