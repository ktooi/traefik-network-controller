# Prompt: specification-driven review

Review `<change, diff or release>` in Traefik Network Controller against `AGENTS.md`, canonical `SPEC.md`, schemas, installed/buildable CLI and `TESTING.md`. This is a review of actual behavior and evidence, not a request to approve the design from its prose alone.

## Review procedure

1. Map changed code and claims to requirement IDs and acceptance cases. Identify implementation/specification/documentation disagreements rather than silently choosing the most convenient version.
2. Trace writes from exact target and network validation through ownership, grace, complete all-container references, pre-action checks, Docker mutation, journal persistence and post-check. Test rejection cases and partial failure where they materially affect risk.
3. Check target/network/endpoint replacement, unknown/manual attachments, corrupt/missing/symlink state, stopped containers, interrupted observations, protected networks, prepared-but-never-used edges, post-detach convergence, app return, stale/tampered plans and external concurrency limits.
4. Verify explicit `prepare` is bounded to the named app and `retire` is intentionally separate from automatic detach. No automatic delete, `Force=true`, other-container disconnect, remote fallback or incidental Socket Proxy broadening is allowed by the current contract.
5. Compare actual help/capabilities/JSON schemas/exit semantics with every operator example, prompt and skill. Check English/Japanese consistency and the echo sample. Ensure a fresh agent can complete the supported task without guessed commands, unauthorized side effects or secret disclosure.
6. Distinguish observed tests from supplied-but-unexecuted tests. Use only an authorized disposable Engine for live acceptance. Do not call compilation/mock success proof of Docker semantics, route availability or all-path isolation.

## Report

List concrete findings by severity, with the affected file/behavior, applicable SPEC ID, trigger and consequence, and a targeted correction. State what was actually verified and any blocked/unexecuted gate. If no defect is found, say so with the verification scope and limits. If the task authorizes fixes, implement confirmed fixes and recheck affected cases; do not rewrite requirements merely to make a failed implementation look compliant.
