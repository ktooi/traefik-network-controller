# Verification and acceptance

**Source:** 0.2.0. **Date:** 2026-11-08 JST / 2026-11-07 UTC.

This is a **partially validated source release**. No production deployment, Docker image build, real Engine lifecycle test, host reboot test, or traffic-isolation acceptance was performed. [SPEC.md](SPEC.md) defines the requirements; this file records actual evidence.

## Public-distribution preparation checks

The publication preparation adds PUB-001–004 and AC-19/AC-20 without changing the Rust runtime/CLI or echo example. Both CI files call `bash scripts/ci/check.sh`, which requires the complete test suite, including listening sockets. The restricted local environment's prior exclusions are not used by either CI.

| Check | Observed result | Evidence and limit |
| --- | --- | --- |
| Cargo license inventory and retained Rust notices | Passed | 98 locked external packages, 278 notice materials / 83 distinct texts; [license-audit.json](verification/license-audit.json) and [license-check.log](verification/license-check.log). This is not an OS-image audit. |
| Notice-drift rejection | Passed | Stale Cargo notices and an altered Rust notice were rejected; [license-negative-checks.json](verification/license-negative-checks.json). |
| Stable release-tag helper | **8 tests passed** | `python3 tests/release_metadata.py`; accepted matching versions/output formats and rejected mismatches, prerelease/leading-zero/injection forms. |
| Optional Docker Hub target guard | **23 setting cases and 2 reference cases passed** | Actual workflow blocks exercised with dummy credentials and temporary files; external-registry/injection forms rejected, push names explicitly qualified with `docker.io/`. [github-registry-guard.json](verification/github-registry-guard.json). No registry call was made. |
| Workflow/configuration syntax and static controls | Passed within static scope | YAML with duplicate-key rejection; embedded shell/Python syntax; full-SHA Action references; protected-tag/source-check dependency and TLS settings; [publication-checks.json](verification/publication-checks.json). |
| EN/JA publication docs and SPEC | Passed within static scope | Matching executable blocks, local links, **40 requirement IDs and 20 acceptance cases**. The earlier documentation result below is the pre-publication baseline. |
| Docker notice/toolchain context | Passed static check only | Dockerfile copies project/dependency/Rust notices and `rust-toolchain.toml`; the allowlist includes them. Copying the pin prevents a builder-image update from silently selecting a different Rust release. |
| Hosted CI, image build/smoke and registry push | **Not executed** | No repository/registry identity or runner was configured here; Docker CLI/socket are absent. Provider CI validation and actual first-run gates remain required. |

`actionlint` and GitLab's server-side CI Lint were not run. YAML parsing does not validate every provider expression or runner capability. The publication files include no test skips, and local source tests do not establish container licensing/source-provision compliance, registry visibility or successful publication. The remaining actual Engine/traffic gates still apply.

## Observed runtime baseline

| Check | Result | Evidence and scope |
| --- | --- | --- |
| Rust 1.99.1 compilation | Passed | `cargo check --offline` after the 0.2.0 lockfile update. |
| Formatting | Passed | [fmt.log](verification/fmt.log). |
| Clippy, all targets, warnings denied | Passed after a fresh package check | [clippy.log](verification/clippy.log). An incremental run replayed obsolete diagnostics despite exit 0; cleaning only this package's dev artifacts and rerunning produced a clean check. Earlier output is retained and labeled. |
| Locked optimized executable build | Passed | [build.log](verification/build.log). This does not build the Docker image. |
| Rust tests without listening sockets | **61 passed** | 59 library tests plus 2 CLI parser tests. [rust-tests.log](verification/rust-tests.log). Nine AF_UNIX cases explicitly filtered; **not counted as passed**. |
| Release-executable CLI suite | **11 passed** | [cli-smoke.log](verification/cli-smoke.log). Missing Docker, duplicate daemon, SIGTERM, invalid arguments, size limits, symlink/FIFO rejection, and 10-second stdin deadline. |
| Python test-file syntax | Passed | `python3 -m py_compile tests/cli_smoke.py tests/mock_docker.py tests/live_acceptance.py`. |
| Compose static resolution | Passed | Official standalone Compose v5.7.0 resolved the previously supplied Traefik base plus the current override with dummy values, and the echo example. [compose-check.json](verification/compose-check.json). No Engine calls. |
| Documentation consistency | Passed within static scope | All 11 commands, 13 decision reasons, 16 report fields and configuration variables documented; 36 SPEC IDs match in EN/JA; paired shell/YAML blocks agree. |
| Operator-skill simulated review | Four scenarios reviewed; wording corrected | [operator-skill-review.md](verification/operator-skill-review.md). No live operations or adoption measurement. |
| Operator-skill format/release check | Passed | Official validator plus actual `--version`, `--help`, `capabilities`; portable copy in `skills/traefik-network-operator/`. |
| Independent core source review | No concrete blocker found within scope | [core-review.md](verification/core-review.md). Read-only review; no independent runtime acceptance. |
| AF_UNIX transport | **Blocked / unexecuted for 0.2.0** | Local socket bind returns `EPERM`; nine Rust cases remain a gate. |
| Mock Docker API suite | **28 supplied scenarios; not executed** | `tests/mock_docker.py` needs an AF_UNIX listener; syntax checked and fixtures reviewed. |
| Real Docker, image and traffic | **Not executed** | Docker CLI and `/var/run/docker.sock` are absent here. |

The full Rust suite contains **70 tests and has not passed as a full suite here**. The eleven Python smoke tests use a deliberately nonexistent Docker socket. They do not replace HTTP transport or Engine tests.

## What was verified

Pure lifecycle cases cover never-used preparation, stopped references, grace and repeated observations, event bursts, returning references, observation gaps, backward clock movement, boot replacement, policy changes, replaced target/network/endpoint IDs, unknown ownership, protected edges, no reconnect loop, explicit preparation and saved-plan freshness. Journal cases cover restrictive persistence, identity/revision round trips, missing/corrupt/symlinked state, scope mismatch, restart reset, dry run, takeover and usage history. Controller unit cases cover inventory consistency, endpoint agreement, ambiguous connections and partial-error evidence.

CLI checks establish that unavailable Docker cannot be reported as a complete successful snapshot; diagnostics do not initialize ownership; capabilities need no deployment configuration; special/oversized input is rejected; and slow stdin exits before taking the operation lock. Daemon missing-Docker health, duplicate refusal and SIGTERM are also exercised.

Compose resolution preserved the prior control/WAF/legacy network keys, added egress priority 100, retained controller `network_mode: none`, defaulted auto-detach to false, and kept Socket Proxy `POST=0` and `NETWORKS=0`. The echo example resolved to `traefik/whoami:v1.12.0`, UID 65532, port 8080, one dedicated edge and no host ports. These are configuration-resolution findings, not runtime proof.

JSON schemas are release-matched structural contracts. Rust report serialization/deserialization is tested. A third-party JSON Schema validator was not installed or run; schema validity is not authorization or freshness verification.

## Reproduce the normal gates

Run on Linux with the pinned Rust toolchain and Unix socket listeners permitted:

```sh
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
python3 tests/cli_smoke.py target/release/traefik-network-controller
python3 tests/mock_docker.py target/release/traefik-network-controller
```

[rust-tests.log](verification/rust-tests.log) retains the exact constrained invocation and all nine skip names. These skips describe the development environment; they are not a replacement for the normal gate.

The blocked Rust transport cases cover Unix authority/filter/body handling, API bounds, optional-404/redaction, redirects, response caps, body-read deadlines, idle event streaming/cancellation, stalled-version cancellation and reconnect negotiation. The Python API double uses a temporary socket with no real-Docker fallback. Its cases include full-ID mutations, scoped preparation, stopped-reference retirement, daemon recovery, read-only operations, guarded apply, adoption, corrupt state and automatic-detach convergence. It remains a model of Docker.

## Disposable-Engine acceptance

Provision a dedicated disposable Linux Engine. The helper requires an explicit non-default local socket, zero pre-existing containers, no pre-existing managed edges, a preloaded image providing `sleep`, and a compatible local controller binary. It does not pull images.

```sh
python3 tests/live_acceptance.py target/release/traefik-network-controller \
  --socket /run/user/1000/tnc-test/docker.sock \
  --image alpine:3.22 \
  --confirm-disposable-engine
```

Replace the socket with the actual dedicated test Engine socket and preload the fixture image there. The helper refuses the default host Docker socket and aliases, and refuses Python optimization that would disable assertions. Cleanup uses recorded fixture IDs only: no prune or volume deletion. A crash can leave fixtures for deliberate cleanup on that Engine.

The helper tests real bridge attachment and EndpointIDs, preparation, stopped references, grace, detach with network preservation, no oscillation, returning applications, read-only journal behavior, current/stale plans and explicit retirement. Its target is a labeled fixture container, **not actual Traefik**. It is part of AC-17, not complete AC-17/AC-18 acceptance.

| Remaining gate | Required evidence |
| --- | --- |
| Image/runtime | Build Dockerfile; accept UID/socket ownership, read-only rootfs, dropped capabilities, no network and persistent volume. |
| Engine compatibility | Run transport/API and live lifecycle tests on the intended Engine. API 1.48–1.56 is a declared contract, not a tested-version matrix. |
| Ownership/failure windows | Manual endpoints stay unowned; replaced identities do not inherit ownership; interruption between connect/commit and lost/corrupt state remain conservative. |
| Restart/recreation | Restart controller, Docker and host; recreate actual Traefik; verify grace reset and repair while fixed connections remain. |
| Coordination | Verify the supported single-management-system deployment workflow. External Docker writes are not atomic with endpoint checks. |
| Actual echo route | Deploy `examples/echo.compose.yaml` with the real TLS/WAF/auth configuration and test the actual hostname. |
| Traffic policy | Verify intended app isolation, published ports, IPv4/IPv6, egress and reachable management listeners from the relevant origins. |

Source compilation, pure tests and mocked API checks do not close the live gates. No result here claims production acceptance.

## Historical baseline

0.1.0 had 22 passing non-socket Rust tests, five CLI smoke tests, nine Unix tests blocked at bind, and 19 unexecuted mock scenarios. Those figures are historical. The current release adds ownership/lifecycle and plan coverage, operator materials, and a disposable echo example.
