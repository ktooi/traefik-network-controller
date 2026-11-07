# Prompt: evaluate or install

Operate Traefik Network Controller 0.2.0 using the `traefik-network-operator` skill when available. Read the matching `SPEC.md`, `docs/cli.md`, `docs/operations.md` and `TESTING.md`. Apply this prompt to the actual installation task; do not assume production deployment is authorized by this file.

## Task inputs

- Desired outcome: `<evaluate | prepare configuration | install>`.
- Source release path: `<path>`.
- Existing Traefik Compose directory/files and project: `<paths and existing project>`.
- Target container/service: `<observed names; usually traefik>`.
- Fixed control/WAF/egress/legacy network physical names: `<observed values>`.
- Authorized environment and changes: `<disposable validation or deployment scope already granted>`.
- Initial auto-detach policy: `false` unless an explicit policy change is included in the task.

## Instructions

1. Inspect missing facts using narrow reads. Verify Docker/Compose compatibility, target name/project/service/full ID, fixed network membership/gateway priority, source version, persistent state and namespace ownership. Do not dump `.env` or raw container environments.
2. Read installed `--version`, `--help`, and `capabilities` if a binary exists. Do not invent commands or flags; JSON is default. No `--json`, `--force`, HTTP listener, MCP endpoint or metrics exporter is implied.
3. Produce a concrete Compose diff from `examples/compose.overwrite.yaml`, preserving the user's services, Socket Proxy restrictions and migration attachments. Use the same project as Traefik. Set physical protected-network names; preserve `controller_state`; explain Docker socket administrative authority and the possible initial Traefik recreation.
4. Validate/build in the authorized environment. Check `TESTING.md`; do not equate compile/mock success with real Docker/traffic acceptance. Run the relevant disposable acceptance when available and authorized; label missing gates explicitly.
5. For authorized installation, apply the reviewed scope, inspect a read-only `plan`, start the controller, inspect `doctor`/`status`, and verify representative existing routes independently. A helper `compose run` creates a container; in a read-only task use existing execution surfaces instead.
6. If a necessary deployment action is outside the granted scope, finish the inspectable configuration/diff and evidence first, then ask only for that unresolved action. Preserve prior authorization; do not restart approval for routine work already granted.

## Completion evidence

Report exact target/configuration identities, files changed, effective auto-detach/protected policy, actual command results, deployment status, traffic result and unexecuted gates. Explain rollback as stopping controller reconciliation without deleting its state or existing attachments. Reference SPEC SCOPE-001–005, AUTH-001–002, OWN-005, OPS-001–005 and AC-01/02/11/15/17/18.
