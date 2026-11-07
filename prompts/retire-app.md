# Prompt: retire an application edge

Retire the named application edge using the installed controller contract and operator skill when available. Read `SPEC.md`, `docs/cli.md` and `docs/operations.md`. Distinguish optional automatic Traefik detach from explicit `retire APP`, which removes a validated empty network.

## Task inputs

- Application slug/edge: `<app / traefik_app_<app>; echo in the demo>`.
- App and controller Compose paths/projects: `<values>`.
- Desired result: `<remove only unused edge | retire application containers and edge>`.
- Data retention: `<preserve volumes unless explicitly stated otherwise>`.
- Authorized removals and deployment coordination: `<scope already granted>`.

## Instructions

1. Discover installed version/help/capabilities and inspect target/network IDs, protection, actual membership and all references, including stopped containers. Do not infer retirement from an HTTP failure, `stop`, `die`, or unhealthy status.
2. Present the exact intended network/container effects, data retention and coordination needs. Preserve existing authorization. If only edge retirement was requested and application containers remain, report that blocker instead of deleting them without authority.
3. Coordinate against new deployments/starts/connects for this app. When app removal is in scope, use its own existing Compose project and preserve data (`down` without `-v` unless data deletion is explicitly intended). Do not remove unrelated containers.
4. Call `retire APP`. This explicit command may disconnect a valid unused edge even when that attachment is unowned, but must satisfy target/network/protection/reference checks. It never requires a forced disconnect. The report operation for network deletion is `remove`.
5. Verify the exact edge is absent, unrelated fixed/application networks remain, and preserved data was not deleted. Read partial-failure details and fresh state before retrying.

Never bypass a refusal using prune, direct forced disconnect, journal editing or removal of protection. If the task is only to enable automatic cleanup, use the documented policy/ownership/grace workflow instead; auto-detach leaves the network present. Map to SCOPE-002–005, OWN-002–004, LIFE-002–007, PLAN-005 and AC-04/06/09/13/17.
