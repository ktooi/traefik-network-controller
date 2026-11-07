# Changelog

## 0.2.0

### Public distribution preparation

- Add an MIT license for original project contributions, reviewed notices for locked Cargo dependencies and the pinned Rust runtime, and a fail-closed notice-drift check. Preserve base-image licensing separately.
- Add commit-pinned GitHub Actions for the full quality gate, container smoke, GHCR publication and optional Docker Hub publication; add GitLab CI for the same source gate and protected-branch/tag container jobs using GitLab Container Registry.
- Add strict version-tag validation, least-privilege/provider-specific setup instructions, digest-based deployment guidance and an agent release-preparation prompt in English/Japanese.
- Keep the runtime/API version and echo example unchanged. Publishing files is not an actual registry push, hosted-CI pass or production acceptance; see TESTING.md.

### Behavior

- Add opt-in automatic Traefik attachment cleanup (`TNC_AUTO_DETACH=false` by default), with grace, repeated complete observations and stopped-container references.
- Persist exact endpoint ownership and lifecycle in `attachments.json`. Pre-existing/manual/ambiguous attachments remain unowned; explicit `adopt` requires target/network/endpoint pins.
- Preserve prepared edges before first use, keep automatically detached networks detached until needed, and reconnect when the application returns. No automatic network deletion or forced disconnect.
- Add protected network names, with required egress always protected.
- Add `status`, `doctor`, `capabilities`, guarded expiring `plan`/`apply`, structured decisions/reasons and JSON schemas.
- Limit `prepare APP` mutations to the named application edge. Explicit `retire APP` remains the path to disconnect and remove a validated empty managed network; the operation name in JSON is `remove`.

### Documentation and integration

- Add a normative English specification and synchronized Japanese copy, requirement IDs and acceptance mapping.
- Add concise diagram-led READMEs, detailed bilingual operations/CLI/security/agent guidance, development/usage prompts and an operator skill.
- Replace the application example with the disposable `echo` service using `traefik/whoami:v1.12.0` and a dedicated edge.
- Preserve 0.1.0 network names/labels and the existing read-only Traefik Socket Proxy integration. New report consumers must use the 0.2.0 schema contract.

### Migration and validation

Keep the state volume and initially leave auto-detach disabled. Existing attachments require intentional ID-pinned adoption before automatic cleanup. Review all existing eligible owned edges before enabling cleanup; the switch applies to the configured namespace, not one app.

See [TESTING.md](TESTING.md) for executed checks and unavailable gates. Source delivery is not a claim of completed real-Docker or routed-traffic acceptance.

## 0.1.0

Initial connect-only Rust controller with startup/event/periodic reconciliation, explicit preparation/retirement, strict target/network/gateway policy, JSON plans, local health and Compose examples. See the versioned source history or prior release artifact for its exact contract.
