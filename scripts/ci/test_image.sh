#!/usr/bin/env bash
# Run only against an explicitly selected disposable CI Docker Engine.
# No socket, host path, credential or state volume is mounted into the image.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."
if [[ $# -ne 1 || -z "$1" || "$1" == -* ]]; then
  printf '%s\n' 'Usage: bash scripts/ci/test_image.sh IMAGE_REF' >&2
  exit 2
fi
image="$1"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
expected="$(python3 -c 'import pathlib,tomllib; print(tomllib.loads(pathlib.Path("Cargo.toml").read_text())["package"]["version"])')"
architecture="$(docker image inspect --format '{{.Os}}/{{.Architecture}}' "$image")"
[[ "$architecture" == 'linux/amd64' ]] || { printf '%s\n' "Unexpected image platform: $architecture" >&2; exit 1; }
run_options=(--rm --pull never --network none --read-only --cap-drop ALL --security-opt no-new-privileges)
version="$(docker run "${run_options[@]}" "$image" --version)"
[[ "$version" == "traefik-network-controller $expected" ]] || { printf '%s\n' 'Image version mismatch' >&2; exit 1; }
docker run "${run_options[@]}" "$image" capabilities > "$work/capabilities.json"
python3 - "$work/capabilities.json" "$expected" <<'PY'
import json
from pathlib import Path
import sys

value = json.loads(Path(sys.argv[1]).read_text())
checks = (
    value.get("controller_version") == sys.argv[2],
    value.get("schema_version") == 1,
    value.get("auto_detach_default") is False,
    value.get("docker_api", {}).get("transport") == "unix_socket_only",
    {"plan", "status", "adopt", "prepare", "retire"}.issubset(value.get("commands", [])),
)
if not all(checks):
    raise SystemExit("Image capabilities do not match the release contract")
PY
docker run "${run_options[@]}" --entrypoint /bin/sh "$image" -ec '
  test -s /usr/share/licenses/traefik-network-controller/LICENSE
  test -s /usr/share/licenses/traefik-network-controller/THIRD-PARTY-NOTICES.txt
  test -s /usr/share/licenses/traefik-network-controller/rust/COPYRIGHT-library.html
'
license="$(docker image inspect --format '{{index .Config.Labels "org.opencontainers.image.licenses"}}' "$image")"
[[ "$license" == 'MIT' ]] || { printf '%s\n' 'Missing project license label' >&2; exit 1; }
printf '%s\n' "Image smoke passed: $image ($architecture, $expected)"
