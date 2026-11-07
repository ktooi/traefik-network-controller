# Operational workflows

## Contents

- Assess fit
- Install or upgrade
- Add an application
- Enable automatic detach
- Retire an application
- Recover attachment ownership
- Diagnose and verify
- Report the result

## Assess fit

1. Confirm a standalone Docker Compose host and one intended Traefik target. Kubernetes and Swarm need a different design.
2. Map fixed control/WAF/egress networks, shared migration networks, application edges, and internal DB/cache networks. Mark published application ports and other shared paths separately.
3. Check that the operator accepts the controller's Docker Unix socket authority and can persist its state. Rootless Docker or a custom socket path requires matching access and release validation; do not silently relax permissions.
4. Compare requirements with the actual release. Verify Docker API negotiation and Compose `gw_priority` support from release documentation and installed versions. Do not claim an entire version matrix was tested because a minimum is documented.
5. Produce a compact topology and a concrete adoption diff. Explain that applications on their own edge networks lose the common shared path only after they actually leave shared networks and other exposure paths are addressed.

## Install or upgrade

1. Read release-matched `SPEC.md`, `docs/operations.md`, and the example Compose overlay if available. Use `examples/echo.compose.yaml` (`traefik/whoami:v1.12.0`) for a demonstration application. Its response includes request headers; avoid injecting secrets or leaving a diagnostic endpoint exposed beyond its intended test. Preserve the user's existing images, credentials, storage, WAF/OIDC middleware, router rules, and unrelated services when preparing production changes.
2. Distinguish the controller's Compose project from the target Traefik project's exact label used in `TNC_TARGET_PROJECT`. Set target project/service/name, fixed egress, protected network names, and a persistent state volume. Keep automatic detach off during initial adoption unless it is explicitly within the authorized deployment scope and ownership has been checked.
3. Preserve controller restrictions: no network namespace connectivity (`network_mode: none`), no host ports or HTTP listener, read-only filesystem with the dedicated writable state mount, dropped capabilities, and no-new-privileges. Do not describe these as making the Docker socket safe for untrusted code.
4. Preserve Traefik's discovery proxy restriction and fixed networks. Configure fixed egress gateway priority 100 before dynamic edge attachment. Retain shared migration networks until the corresponding applications have moved.
5. Validate the configuration without printing expanded secret values. Build the delivered source or select an authorized pinned image using a verified reference; do not invent a public registry image or repository URL. When runtime access is available, inspect its help/capabilities and collect a plan; otherwise prepare those verification commands and mark them unperformed. A local build or configuration file is not a completed deployment.
6. Apply authorized installation changes. Compose environment changes require service recreation, such as `docker compose up -d --no-deps controller` with the established project/files; `docker compose restart` does not load changed environment values. Check controller health/status, target identity, actual Docker memberships, and fixed egress after startup. Existing attachments remain unowned until documented adoption; an upgrade must not silently take ownership.

## Add an application

1. Select a valid application slug and review `traefik_app_<app>` for collisions, conflicting ownership, and protection policy. The demonstration uses `echo` and `traefik_app_echo`. Do not reuse a same-named network merely because its name matches.
2. Prepare the application Compose diff: frontend uses the external edge network and exact `traefik.docker.network`; DB/cache retain only their internal network; remove the old shared edge attachment only as part of the authorized migration. Keep routing and middleware settings intact.
3. With authority for deployment and runtime access, run the installed release's `prepare APP` workflow and inspect its result. In v0.2.0, `prepare echo` validates the global snapshot but creates/connects only `traefik_app_echo`; it does not mutate another application's attachment. With files only, provide this as an unexecuted deployment step.
4. Deploy only the intended application scope using the established deployment method. Do not issue project-wide teardown, `down -v`, image pruning, or unrelated restarts.
5. Verify the target and frontend share the intended edge network, the backend services do not, and unwanted shared memberships have been removed. Probe the expected route from an appropriate host. State separately any paths not tested, including published ports and host routing.

## Enable automatic detach

1. Confirm v0.2.0 or another verified release with automatic-detach capability. Inspect existing attachment ownership, complete references from all non-target containers including stopped containers, protected networks, and the persistence of the journal.
2. Prepare a bounded configuration diff with `TNC_AUTO_DETACH: "true"`, a deliberate grace interval, and the actual protected network names. Keep fixed egress protected. Do not set grace to an unsupported value or shorten it to work around uncertain state.
3. Explain which owned attachments could become eligible and which remain manual/unowned. Enabling the policy affects already-existing eligible owned edges as well as future ones. An application restart or stop is not a detach trigger because stopped-container references remain. A prepared but never-used edge is retained until explicit retirement; network existence by itself must not reconnect a previously detached edge.
4. Apply the authorized configuration and recreate only the controller using the established Compose project/files and `up -d --no-deps controller`. A plain `restart` retains the old container environment. Inspect `status.policy` after recreation to verify effective values. Expect conservative grace resets after controller restarts or incomplete observations; do not claim an immediate detach is guaranteed.
5. Verify decisions over complete observations and confirm eventual membership when an eligible network becomes unused. A grace-pending decision is an expected intermediate state, not a verified disconnection.

## Retire an application

1. Establish whether the user wants to stop the application, remove its deployment, detach Traefik, or delete the unused edge network. These have different effects; use existing task context instead of asking unnecessarily.
2. Stop alone is insufficient. If authorized, remove the specific application's container deployment while preserving its volumes/data unless data deletion was explicitly requested. Never remove unrelated stopped containers to make a network appear unused.
3. Obtain complete observations of references from all non-target containers, including stopped containers; exclude only the selected Traefik target. If any remain, report them and keep the attachment. API errors or partial observations are not proof of zero references.
4. For automatic cleanup, inspect the policy and wait for the configured complete-observation grace; verify that only the owned Traefik attachment disappears and the network remains.
5. For explicit network retirement, use `retire APP` only when disconnect plus deletion of the unused managed network is in the authorized scope. This explicit command can retire a validated network with an unowned target attachment; it is not automatic cleanup and does not require unnecessary adoption. If policy, protection, reference, or identity checks refuse, diagnose without bypassing them.
6. Re-read actual Docker membership and network existence. Preserve the persistent journal; deleting the journal is not a reset or a rollback. Re-adding an application uses the documented prepare/deploy workflow.

## Recover attachment ownership

1. Identify whether the attachment was manual, created by an older release, or lost its ownership evidence after state-volume loss/corruption. Preserve evidence and stop cleanup attempts against the uncertain attachment.
2. Check release-matched recovery instructions. Prefer restoration of a verified matching backup when appropriate; do not merge, hand-edit, or manufacture ownership records. A backup with old target/network/endpoint IDs does not establish current ownership.
3. If explicit takeover is authorized and supported, obtain fresh full target, network, and endpoint IDs and review the selected application and network policy. Use the exact `adopt APP --target-id ... --network-id ... --endpoint-id ...` command.
4. Re-read the result and current status. Adoption is a journal ownership operation, not evidence of a network disconnect or application health. It resets grace; an empty adopted edge with no observed previous use remains prepared. Do not couple takeover to bulk retirement or zero-grace cleanup.
5. If a safe documented path is unavailable, prepare a concrete recovery plan and explain the unresolved evidence. Do not erase state, force detach, or convert an observation failure into permission.

## Diagnose and verify

| Observation | First evidence to collect | Interpretation boundary |
| --- | --- | --- |
| Expected edge attachment missing | Exact target labels/ID, managed network properties, current plan decisions, fixed egress | A valid Traefik label alone does not establish attachment or controller eligibility. |
| Unused attachment remains | Complete non-target-container references, ownership, protection, grace and observation history | Stopped references, unowned endpoints, or incomplete inspection legitimately block detach. |
| Controller unhealthy | Freshness of local health, supported `doctor`/`status`, scoped recent controller logs, socket access | A process running or old successful health file does not prove recent reconciliation. |
| Every route returns 404 | Traefik router/middleware errors and provider/plugin initialization logs, then network observations | Invalid or absent routers can cause 404; do not assume a network detach caused it. |
| Plugins disabled after host restart | Startup DNS and outbound HTTPS evidence from Traefik's environment; current resolver readiness | Later healthy DNS does not make an earlier failed plugin initialization disappear. |
| Docker provider unavailable | Discovery proxy membership/readiness and restricted API access | Do not enable broad proxy write access to repair controller behavior. |
| Plan apply refused | Schema/version, expiry, target/network/endpoint identities, policy and current journal, fresh plan | Refusal is not a reason to replay the old action with raw Docker. |
| App is reachable but isolation uncertain | Shared networks, frontend/backend memberships, published ports, host/firewall paths | Reachability and isolation require separate evidence. |

Use the smallest relevant observations. When container/network inspection is needed, select identity and network fields with Docker formatting rather than exporting complete objects containing environment variables. Do not install diagnostic utilities in the production controller or attach it to a network merely to run a probe.

## Report the result

Present four items:

1. The requested outcome and its verified scope.
2. The exact configuration or lifecycle changes made, or the concrete prepared change when execution was unavailable.
3. Evidence: installed release, observation completeness/time, relevant object identities, resulting memberships, protected egress, and route checks if performed. Redact unrelated or sensitive data.
4. Pending grace, refusals, unresolved risks, and checks not performed. Distinguish a passing unit test, a mock Docker API test, an actual host inspection, and a real end-to-end application probe.

Keep a prepared plan reviewable. Where additional authority is truly needed, finish preparation first and ask only about the exact remaining action, explaining why it needs that authority.
