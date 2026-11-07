#!/usr/bin/env python3
"""Core lifecycle acceptance on a dedicated, initially empty Docker Engine.

This test MUTATES Docker. Never run it on a production Engine. It refuses the
default host socket, existing containers, and existing managed edge networks.
It needs a preloaded Linux image providing `sleep` and a compatible local binary.
It does not build an image, deploy Traefik, test WAF/TLS, or prove isolation.

Example (only after provisioning a disposable Engine and loading the image):
  python3 tests/live_acceptance.py target/release/traefik-network-controller \
    --socket /run/user/1000/tnc-test/docker.sock --image alpine:3.22 \
    --confirm-disposable-engine
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile
import time


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--socket", required=True, type=Path)
    parser.add_argument("--image", required=True)
    parser.add_argument("--confirm-disposable-engine", action="store_true")
    args = parser.parse_args()
    if not __debug__:
        parser.error("run without Python optimization; acceptance assertions must remain enabled")
    if not args.confirm_disposable_engine:
        parser.error("explicit --confirm-disposable-engine is required")
    if not args.socket.is_absolute() or not args.socket.is_socket():
        parser.error("--socket must identify an existing absolute Unix socket")
    for forbidden in [Path("/var/run/docker.sock"), Path("/run/docker.sock")]:
        if args.socket.resolve() == forbidden.resolve() or (
            forbidden.exists() and os.path.samefile(args.socket, forbidden)
        ):
            parser.error("the default host Docker socket is forbidden")
    binary = args.binary.resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK) or not shutil.which("docker"):
        parser.error("an executable controller binary and Docker CLI are required")
    clean_env = {
        key: value for key, value in os.environ.items()
        if not key.startswith(("DOCKER_", "TNC_"))
    }
    base = ["docker", "--host", f"unix://{args.socket}"]

    def docker(*words: str, check: bool = True) -> subprocess.CompletedProcess:
        return subprocess.run(base + list(words), env=clean_env, text=True,
                              capture_output=True, timeout=45, check=check)

    # All admission checks occur before any Docker write.
    docker("info", "--format", "{{.ServerVersion}}")
    if docker("container", "ls", "-aq").stdout.strip():
        parser.error("disposable Engine must initially contain zero containers")
    if docker("network", "ls", "-q", "--filter",
              "label=com.nofmem.traefik.autojoin=true").stdout.strip():
        parser.error("disposable Engine already contains managed edge networks")
    docker("image", "inspect", args.image)  # No automatic image pull.

    token = secrets.token_hex(5)
    project = f"tnc-test-{token}"
    target_name = f"{project}-target"
    app = f"test_{token}"
    app_name = f"{project}-app"
    egress = f"tnc_test_egress_{token}"
    edge = f"traefik_app_{app}"
    created_containers: list[str] = []
    created_egress: str | None = None
    edge_id: str | None = None
    checks: list[str] = []
    with tempfile.TemporaryDirectory(prefix="tnc-live-acceptance-") as directory:
        state = Path(directory) / "state"
        env = clean_env | {
            "TNC_SOCKET_PATH": str(args.socket), "TNC_STATE_DIR": str(state),
            "TNC_TARGET_NAME": target_name, "TNC_TARGET_PROJECT": project,
            "TNC_TARGET_SERVICE": "traefik", "TNC_EGRESS_NETWORK": egress,
            "TNC_INTERVAL_SECS": "1", "TNC_API_TIMEOUT_SECS": "5",
            "TNC_AUTO_DETACH": "true", "TNC_DETACH_GRACE_SECS": "2",
        }

        def cli(*words: str, override: dict | None = None, check: bool = True):
            result = subprocess.run([str(binary), *words], env=env | (override or {}),
                                    text=True, capture_output=True, timeout=45, check=False)
            if check and result.returncode != 0:
                raise RuntimeError(f"controller {' '.join(words)} failed: {result.stderr[-4000:]}")
            return result

        def attached() -> bool:
            raw = docker("container", "inspect", target_name).stdout
            return edge in json.loads(raw)[0]["NetworkSettings"]["Networks"]

        def mark(name: str) -> None:
            checks.append(name)
            print(f"PASS {name}", file=sys.stderr, flush=True)

        def start_app() -> str:
            result = docker("run", "-d", "--name", app_name, "--label",
                            f"tnc.test.run={token}", "--network", edge,
                            "--pull=never", args.image, "sleep", "3600")
            identifier = result.stdout.strip()
            created_containers.append(identifier)
            return identifier

        try:
            created_egress = docker("network", "create", "--label",
                                    f"tnc.test.run={token}", egress).stdout.strip()
            target_id = docker(
                "run", "-d", "--name", target_name, "--label", f"tnc.test.run={token}",
                "--label", f"com.docker.compose.project={project}",
                "--label", "com.docker.compose.service=traefik", "--network",
                f"name={egress},gw-priority=100", "--pull=never", args.image, "sleep", "3600",
            ).stdout.strip()
            created_containers.append(target_id)
            cli("prepare", app)
            edge_id = json.loads(docker("network", "inspect", edge).stdout)[0]["Id"]
            assert attached(), "prepare did not attach the target"
            mark("AC-03 verified first attachment")

            cli("once")
            time.sleep(2.1)
            cli("once")
            assert attached(), "never-used prepared edge was detached"
            mark("AC-08 prepared edge retained")

            first_app = start_app()
            cli("once")
            docker("stop", "--time", "1", first_app)
            cli("once")
            time.sleep(2.1)
            cli("once")
            assert attached(), "stopped container reference failed to protect attachment"
            mark("AC-06 stopped reference retained")

            docker("rm", first_app)
            created_containers.remove(first_app)
            cli("once")
            deadline = time.monotonic() + 12
            while attached() and time.monotonic() < deadline:
                time.sleep(1.1)
                cli("once")
            assert not attached(), "unused owned edge did not detach after grace"
            docker("network", "inspect", edge_id)
            cli("once")
            assert not attached(), "detached empty edge immediately reattached"
            mark("AC-07/08/09 detach, network preservation and no oscillation")

            second_app = start_app()
            cli("once")
            assert attached(), "returning application did not reconnect"
            mark("AC-08 returning application reconnects")

            before = (state / "attachments.json").read_bytes()
            for command in ["plan", "status", "doctor"]:
                result = cli(command)
                json.loads(result.stdout)
                assert (state / "attachments.json").read_bytes() == before
            mark("AC-11 diagnostic commands preserve ownership journal")

            plan = Path(directory) / "plan.json"
            plan.write_text(cli("plan").stdout)
            rejected = cli("apply", str(plan), override={"TNC_AUTO_DETACH": "false"}, check=False)
            assert rejected.returncode != 0, "changed-policy plan was accepted"
            error = json.loads(rejected.stderr.strip().splitlines()[-1])
            assert error["error"]["code"] == "STALE_PLAN", error
            cli("apply", str(plan))
            mark("AC-12 current plan applies; changed policy rejects")

            refusal = cli("retire", app, check=False)
            assert refusal.returncode != 0 and attached(), "retire removed a referenced edge"
            docker("rm", "-f", second_app)
            created_containers.remove(second_app)
            cli("retire", app)
            assert docker("network", "inspect", edge_id, check=False).returncode != 0
            edge_id = None
            mark("explicit retirement refuses references then removes empty edge")

            print(json.dumps({"status": "passed", "checks": checks,
                              "not_tested": ["controller image", "actual Traefik", "TLS/WAF",
                                             "traffic isolation", "host reboot", "all race cases"]}, indent=2))
        finally:
            # Cleanup only recorded fixture IDs. No prune, volume deletion, or name-wide removal.
            for identifier in reversed(created_containers):
                docker("rm", "-f", identifier, check=False)
            if edge_id:
                docker("network", "rm", edge_id, check=False)
            if created_egress:
                docker("network", "rm", created_egress, check=False)


if __name__ == "__main__":
    main()
