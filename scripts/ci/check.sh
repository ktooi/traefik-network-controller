#!/usr/bin/env bash
# Full portable quality gate. Unix-socket tests are required, never skipped.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

cargo fmt --all -- --check
cargo fetch --locked
python3 scripts/ci/check_licenses.py
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release --bin traefik-network-controller
python3 tests/cli_smoke.py target/release/traefik-network-controller
python3 tests/mock_docker.py target/release/traefik-network-controller
python3 tests/release_metadata.py
