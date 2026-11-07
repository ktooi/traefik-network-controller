# Prompt: add an application

Add the application using the installed Traefik Network Controller contract. Use the operator skill if available. Read this release's `SPEC.md`, `docs/cli.md`, `docs/operations.md` and `examples/echo.compose.yaml`. The demo is `echo` on `traefik_app_echo`; use the actual application slug supplied below.

## Task inputs

- Application slug: `<app; echo for the demo>`.
- Application Compose path/project and intended hostname: `<values>`.
- Existing Traefik/controller Compose path/project: `<values>`.
- Existing routing/TLS/WAF/authentication configuration: `<inspect actual names>`.
- Authorized changes/deployment: `<scope already granted>`.

## Instructions

1. Verify installed `--version`, `--help`, `capabilities`, target identity and effective policy. Inspect the existing app file and network references with secrets redacted. Use the running controller through the documented Compose wrapper when possible.
2. Read `status`/`plan`. Check that the desired exact edge is managed, unprotected and compatible; a conflicting existing name is not permission to adopt or rename it. Do not weaken a refusal.
3. Prepare a concrete app Compose diff: only its intended web service joins external `traefik_app_<app>`; `traefik.docker.network` selects that physical name; private databases/caches retain their internal networks; no unnecessary host ports or Docker socket. Remove the old shared application attachment as part of the intended migration. Preserve volumes, credentials, image approvals and actual TLS/WAF/auth names.
4. Execute `prepare APP` within granted authority, then deploy using the app's existing Compose project. Preparation is limited to that named edge; it does not change unrelated edges or take ownership of a pre-existing endpoint. Do not edit Traefik's central network list for the new app.
5. Verify full-ID Docker membership, current controller decisions, backend port and actual intended HTTPS route. If the app is the echo demo, use `traefik/whoami:v1.12.0` with explicit port8080/nonroot configuration and no sensitive request headers or environment.
6. Report actual results. An intact network plus failed routing is not success; investigate provider/middleware/plugin/DNS/backend evidence without stripping security to make the check pass.

## Completion evidence

Give the changed files, app/project/edge identity, prepared attachment result, separate traffic result and remaining checks. A prepared edge that has never been used is retained; if deployment is abandoned, propose explicit retirement within scope. Map to SCOPE-002–005, OWN-001–002, LIFE-005, PLAN-001, DOC-005 and AC-01/02/03/08/17/18.
