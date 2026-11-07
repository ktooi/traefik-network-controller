# Prompt: specification-driven implementation

Implement the requested change in Traefik Network Controller. Read `AGENTS.md`, canonical `SPEC.md`, the current CLI/schema, affected code, and `TESTING.md` before editing. Use English code/comments/identifiers and keep Japanese documentation synchronized. The requested change is: `<concrete problem and desired behavior>`.

## Required workflow

1. Identify the existing behavior, affected SPEC requirement IDs and acceptance cases. State whether this is a bug fix within the contract or a contract change. Inspect rather than assume the current implementation.
2. For a contract change, update SPEC first with precise conditions, authority, failure behavior, concurrency limits and migration impact. Do not weaken an invariant simply because an implementation is inconvenient. Keep intended requirements distinct from verified implementation.
3. Implement shared policy for daemon/once/plan/apply and explicit operations. Preserve target/network/full-ID checks, protected paths, gateway priorities, stopped references, durable endpoint ownership, conservative failures, grace and no reconnect oscillation. Automatic cleanup must not delete networks, force-disconnect or touch another container.
4. Add or adjust meaningful verification for changed decisions and negative paths. Map test names/results to SPEC acceptance IDs. Use a disposable Engine for live tests; never fall back to the user's production socket. Run required formatting/build/test/Clippy gates as described in AGENTS.md and TESTING.md. Report environmental blocks instead of removing or faking a test.
5. Align `--help`, `capabilities`, JSON schemas, docs/cli, examples, English/Japanese README/spec/operations, task prompts and the operator skill for every changed public contract. Preserve established names unless migration is explicit. Demo examples use `echo` and `traefik_app_echo`.
6. Review the actual diff and required evidence. Complete authorized local fixes. Do not deploy, publish or broaden permissions merely because source development was requested; prepare concrete artifacts before any necessary unresolved authorization.

For dependency/toolchain/CI changes, apply PUB-001–004 and AC-19/AC-20. Review updated upstream and nested notices before regeneration, run `scripts/ci/check_licenses.py`, preserve the full shared CI gate, and synchronize the provider setup docs. Use `prompts/prepare-release.md` for publication preparation; an existing source version is not a registry digest or proof of a successful push.

## Final report

Give the resulting behavior, requirement/case mapping, files changed, exact checks and outcomes, compatibility/migration impact, known residual risk and unexecuted gates. Do not present the specification, source compilation, mocks and real Docker traffic as interchangeable proof. No production-readiness claim beyond TESTING.md evidence.
