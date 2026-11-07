#!/usr/bin/env python3
"""CLI and missing-daemon checks; never creates a socket or contacts real Docker.

Usage: python3 tests/cli_smoke.py target/release/traefik-network-controller
"""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest


BINARY = Path(sys.argv.pop(1)).resolve() if len(sys.argv) > 1 else None


class CliSmokeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="tnc-cli-smoke-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.env = {key: value for key, value in os.environ.items() if not key.startswith("TNC_")}
        self.env.update({
            "TNC_SOCKET_PATH": str(self.root / "nonexistent-docker.sock"),
            "TNC_STATE_DIR": str(self.root / "state"),
            "TNC_TARGET_PROJECT": "test",
            "TNC_INTERVAL_SECS": "1",
            "TNC_API_TIMEOUT_SECS": "1",
        })

    def command(self, *args, env=None):
        return subprocess.run([str(BINARY), *args], env=env or self.env,
                              capture_output=True, text=True, timeout=5, check=False)

    def test_help_and_version_do_not_require_project_configuration(self):
        env = self.env.copy()
        del env["TNC_TARGET_PROJECT"]
        self.assertEqual(self.command("--help", env=env).returncode, 0)
        result = self.command("--version", env=env)
        self.assertEqual(result.returncode, 0)
        self.assertIn("traefik-network-controller 0.2.0", result.stdout)

    def test_capabilities_are_available_without_configuration_or_docker(self):
        env = self.env.copy()
        del env["TNC_TARGET_PROJECT"]
        result = self.command("capabilities", env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        contract = json.loads(result.stdout)
        self.assertEqual(contract["schema_version"], 1)
        self.assertIn("plan", result.stdout)
        self.assertIn("adopt", result.stdout)
        self.assertFalse((self.root / "state").exists())

    def test_invalid_usage_has_exit_code_two(self):
        for args in [("not-a-command",), ("prepare",), ("retire",), ("apply",),
                     ("status", "--force"), ("plan", "--apply"), ("doctor", "--json")]:
            with self.subTest(args=args):
                result = self.command(*args)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(json.loads(result.stderr)["error"]["code"], "INVALID_ARGUMENT")

    def test_apply_rejects_invalid_and_oversized_input_before_locking(self):
        for payload in [b"not JSON", b"{}", b"x" * (2 * 1024 * 1024 + 1)]:
            with self.subTest(length=len(payload)):
                path = self.root / "invalid-plan.json"
                path.write_bytes(payload)
                result = self.command("apply", str(path))
                self.assertEqual(result.returncode, 1)
                self.assertEqual(json.loads(result.stderr)["error"]["code"], "INVALID_PLAN")
                self.assertFalse((self.root / "state").exists())

    def test_apply_rejects_symlinks_and_non_regular_files_without_hanging(self):
        target = self.root / "plan.json"
        target.write_text("{}")
        link = self.root / "link.json"
        link.symlink_to(target)
        fifo = self.root / "fifo"
        os.mkfifo(fifo)
        for path in [link, fifo, self.root]:
            with self.subTest(path=path.name):
                result = self.command("apply", str(path))
                self.assertEqual(result.returncode, 1)
                self.assertEqual(json.loads(result.stderr)["error"]["code"], "INVALID_PLAN")
                self.assertFalse((self.root / "state").exists())

    def test_apply_accepts_piped_input_and_rejects_malformed_schema(self):
        result = subprocess.run([str(BINARY), "apply", "-"], input="{}", env=self.env,
                                capture_output=True, text=True, timeout=5, check=False)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(json.loads(result.stderr)["error"]["code"], "INVALID_PLAN")
        self.assertFalse((self.root / "state").exists())

    def test_slow_plan_pipe_times_out_before_taking_operation_lock(self):
        process = subprocess.Popen([str(BINARY), "apply", "-"], env=self.env,
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True)
        try:
            # Keep the writer open without bytes. The application must stop its
            # stdin worker and exit, not leave a runtime shutdown hang.
            process.wait(timeout=14)
            self.assertEqual(process.returncode, 1)
            self.assertEqual(json.loads(process.stderr.read())["error"]["code"], "INPUT_TIMEOUT")
            self.assertFalse((self.root / "state").exists())
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=2)
            process.stdin.close()
            process.stdout.close()
            process.stderr.close()

    def test_diagnostics_never_claim_complete_snapshot_without_docker(self):
        for name in ["plan", "status", "doctor"]:
            with self.subTest(command=name):
                result = self.command(name)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, "")
                self.assertIn("code", json.loads(result.stderr)["error"])
                self.assertFalse((self.root / "state" / "attachments.json").exists())

    def test_invalid_configuration_fails_before_a_docker_request(self):
        env = self.env.copy()
        del env["TNC_TARGET_PROJECT"]
        self.assertEqual(self.command("once", env=env).returncode, 1)
        env = self.env.copy()
        env["TNC_DRY_RUN"] = "TRUE"
        self.assertEqual(self.command("once", env=env).returncode, 1)
        self.assertFalse((self.root / "state").exists())

    def test_missing_daemon_one_shot_does_not_claim_success_or_health(self):
        self.assertEqual(self.command("once").returncode, 1)
        self.assertFalse((self.root / "state" / "health.json").exists())
        self.assertEqual(self.command("healthcheck").returncode, 1)

    def test_daemon_is_unhealthy_without_docker_and_stops_on_sigterm(self):
        process = subprocess.Popen([str(BINARY), "run"], env=self.env,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            health_path = self.root / "state" / "health.json"
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                if health_path.exists():
                    health = json.loads(health_path.read_text())
                    if health["detail"] != "initializing":
                        break
                self.assertIsNone(process.poll(), "daemon exited before retrying")
                time.sleep(0.05)
            else:
                self.fail("daemon did not report its missing-Docker failure")
            self.assertFalse(health["ready"])
            self.assertEqual(self.command("healthcheck").returncode, 1)
            duplicate = self.command("run")
            self.assertEqual(duplicate.returncode, 1)
            self.assertIn("another daemon", duplicate.stderr)
            process.terminate()
            _stdout, stderr = process.communicate(timeout=5)
            self.assertEqual(process.returncode, 0, stderr)
            health = json.loads(health_path.read_text())
            self.assertFalse(health["ready"])
            self.assertEqual(health["detail"], "controller stopped")
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate(timeout=5)


if __name__ == "__main__":
    if BINARY is None or not BINARY.is_file():
        raise SystemExit(__doc__)
    unittest.main(verbosity=2)
