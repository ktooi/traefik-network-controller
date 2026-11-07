#!/usr/bin/env python3
"""Black-box controller tests against an isolated AF_UNIX Docker API double.

Usage:
    python3 tests/mock_docker.py target/debug/traefik-network-controller

The executable only receives this test's temporary socket. No real Docker
daemon, containers, external network, or third-party Python packages are used.
"""

from __future__ import annotations

import copy
import http.server
import json
import os
from pathlib import Path
import re
import socketserver
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from urllib.parse import parse_qs, unquote, urlsplit


BINARY: Path
TARGET_ID = "b" * 64
EDGE_ID = "a" * 64
EGRESS_ID = "c" * 64
CONTROL_ID = "d" * 64
APP_ID = "e" * 64
SECOND_EDGE_ID = "f" * 64
REPLACEMENT_ID = "1" * 64
FULL_ID = re.compile(r"[0-9a-f]{64}\Z")
LABEL_AUTOJOIN = "com.nofmem.traefik.autojoin"
LABEL_ROLE = "com.nofmem.traefik.role"
LABEL_OWNER = "com.nofmem.traefik.owner"


def owned_network(app: str = "echo", network_id: str = EDGE_ID) -> dict:
    return {
        "Id": network_id,
        "Name": f"traefik_app_{app}",
        "Driver": "bridge",
        "Scope": "local",
        "Internal": False,
        "Ingress": False,
        "ConfigOnly": False,
        "Attachable": False,
        "EnableIPv6": False,
        "Options": {},
        "Labels": {
            LABEL_AUTOJOIN: "true",
            LABEL_ROLE: "application-edge",
            LABEL_OWNER: app,
        },
        "Containers": {},
    }


def target() -> dict:
    return {
        "Id": TARGET_ID,
        "Name": "/traefik",
        "Config": {
            "Labels": {
                "com.docker.compose.project": "test",
                "com.docker.compose.service": "traefik",
            }
        },
        "State": {"Running": True, "Status": "running"},
        "NetworkSettings": {
            "Networks": {
                "traefik_egress": {"NetworkID": EGRESS_ID, "GwPriority": 100, "EndpointID": "8" * 64},
                "test_traefik_control": {"NetworkID": CONTROL_ID, "GwPriority": 0, "EndpointID": "8" * 64},
            }
        },
    }


def fixed_network(name: str, network_id: str, internal: bool = False) -> dict:
    network = owned_network(network_id=network_id)
    network["Name"] = name
    network["Labels"] = {}
    network["Internal"] = internal
    network["Containers"][TARGET_ID] = {"Name": "traefik", "EndpointID": "8" * 64}
    return network


class Model:
    """Stateful API model; all request handling is serialized by its lock."""

    def __init__(self) -> None:
        self.lock = threading.RLock()
        self.api_version = "1.56"
        self.minimum_version = "1.44"
        self.target = target()
        self.target_available = True
        self.networks = {
            EDGE_ID: owned_network(),
            EGRESS_ID: fixed_network("traefik_egress", EGRESS_ID),
            CONTROL_ID: fixed_network("test_traefik_control", CONTROL_ID, internal=True),
        }
        self.other_containers: list[dict] = []
        self.requests: list[dict] = []
        self.mutations: list[dict] = []
        self.successful_mutations: list[dict] = []
        self.target_reads = 0
        self.recreate_after_first_target_read = False
        self.fail_network_list = False
        self.next_network_id = 2

    def find_network(self, name_or_id: str) -> dict | None:
        return next(
            (
                network
                for network in self.networks.values()
                if name_or_id in (network["Id"], network["Name"])
            ),
            None,
        )

    def attach(self, network_id: str = EDGE_ID, priority: int = -1) -> None:
        network = self.networks[network_id]
        self.target["NetworkSettings"]["Networks"][network["Name"]] = {
            "NetworkID": network_id,
            "GwPriority": priority,
            "EndpointID": "9" * 64,
        }
        network["Containers"][self.target["Id"]] = {
            "Name": "traefik",
            "EndpointID": "9" * 64,
            "IPv4Address": "172.30.90.2/24",
            "IPv6Address": "",
        }

    def target_summary(self) -> dict:
        return {
            "Id": self.target["Id"],
            "Names": [self.target["Name"]],
            "State": "running" if self.target["State"]["Running"] else "exited",
            "Labels": copy.deepcopy(self.target["Config"]["Labels"]),
            "NetworkSettings": copy.deepcopy(self.target["NetworkSettings"]),
        }

    @staticmethod
    def filter_matches(network: dict, filters: dict) -> bool:
        labels = network.get("Labels") or {}
        for expression in filters.get("label", []):
            key, sep, value = expression.partition("=")
            if key not in labels or (sep and labels[key] != value):
                return False
        for name in filters.get("name", []):
            if name not in network["Name"]:
                return False
        return True

    def dispatch(self, method: str, raw_path: str, body: object) -> tuple[int, object]:
        with self.lock:
            parsed = urlsplit(raw_path)
            path = unquote(parsed.path)
            path = re.sub(r"^/v[0-9]+\.[0-9]+(?=/)", "", path)
            query = parse_qs(parsed.query)
            request = {"method": method, "path": path, "query": query, "body": body}
            self.requests.append(copy.deepcopy(request))
            if method in ("POST", "DELETE", "PUT", "PATCH"):
                self.mutations.append(copy.deepcopy(request))

            if method == "GET" and path == "/version":
                return 200, {
                    "ApiVersion": self.api_version,
                    "MinAPIVersion": self.minimum_version,
                    "Version": "29.0.0",
                    "Os": "linux",
                }
            if method in ("GET", "HEAD") and path == "/_ping":
                return 200, "OK"

            if method == "GET" and path == "/networks":
                if self.fail_network_list:
                    return 500, {"message": "injected network inventory failure"}
                try:
                    filters = json.loads(query.get("filters", ["{}"]) [0])
                except (ValueError, TypeError):
                    return 400, {"message": "invalid filters"}
                return 200, copy.deepcopy(
                    [network for network in self.networks.values() if self.filter_matches(network, filters)]
                )

            if method == "GET" and path == "/containers/json":
                rows = ([self.target_summary()] if self.target_available else [])
                rows.extend(copy.deepcopy(self.other_containers))
                if query.get("all", ["false"])[0].lower() not in ("1", "true"):
                    rows = [row for row in rows if row.get("State") == "running"]
                return 200, rows

            match = re.fullmatch(r"/containers/([^/]+)/json", path)
            if method == "GET" and match:
                identifier = match.group(1)
                if not self.target_available or identifier not in ("traefik", self.target["Id"]):
                    return 404, {"message": "No such container"}
                response = copy.deepcopy(self.target)
                self.target_reads += 1
                if self.recreate_after_first_target_read and self.target_reads == 1:
                    # Simulate replacement immediately after the caller's
                    # snapshot. The new generation is deliberately foreign.
                    self.target["Id"] = REPLACEMENT_ID
                    self.target["Config"]["Labels"]["com.docker.compose.project"] = "foreign"
                return 200, response

            match = re.fullmatch(r"/networks/([^/]+)", path)
            if method == "GET" and match:
                network = self.find_network(match.group(1))
                return (200, copy.deepcopy(network)) if network else (404, {"message": "No such network"})

            if method == "POST" and path == "/networks/create":
                if not isinstance(body, dict):
                    return 400, {"message": "invalid create body"}
                if self.find_network(body.get("Name", "")):
                    return 409, {"message": "network already exists"}
                created_id = f"{self.next_network_id:064x}"
                self.next_network_id += 1
                network = {
                    "Id": created_id,
                    "Name": body.get("Name"),
                    "Driver": body.get("Driver", "bridge"),
                    "Scope": body.get("Scope", "local"),
                    "Internal": body.get("Internal", False),
                    "Ingress": body.get("Ingress", False),
                    "ConfigOnly": body.get("ConfigOnly", False),
                    "Options": copy.deepcopy(body.get("Options", {})),
                    "Labels": copy.deepcopy(body.get("Labels", {})),
                    "Containers": {},
                }
                self.networks[created_id] = network
                self.successful_mutations.append(copy.deepcopy(request))
                return 201, {"Id": created_id, "Warning": ""}

            match = re.fullmatch(r"/networks/([^/]+)/(connect|disconnect)", path)
            if method == "POST" and match:
                network_id, action = match.groups()
                network = self.networks.get(network_id)
                if not FULL_ID.fullmatch(network_id) or network is None:
                    return 404, {"message": "network mutation requires existing full ID"}
                if not isinstance(body, dict) or not FULL_ID.fullmatch(str(body.get("Container", ""))):
                    return 400, {"message": "container mutation requires full ID"}
                if body["Container"] != self.target["Id"]:
                    return 404, {"message": "No such container generation"}
                if action == "connect":
                    endpoint = body.get("EndpointConfig", {})
                    if endpoint.get("GwPriority") != -1:
                        return 400, {"message": "expected GwPriority=-1"}
                    if network["Name"] in self.target["NetworkSettings"]["Networks"]:
                        return 403, {"message": "endpoint already exists"}
                    self.attach(network_id, endpoint["GwPriority"])
                else:
                    if body.get("Force", False):
                        return 400, {"message": "forced disconnect is forbidden"}
                    self.target["NetworkSettings"]["Networks"].pop(network["Name"], None)
                    network["Containers"].pop(self.target["Id"], None)
                self.successful_mutations.append(copy.deepcopy(request))
                return 200, None

            match = re.fullmatch(r"/networks/([^/]+)", path)
            if method == "DELETE" and match:
                network_id = match.group(1)
                if not FULL_ID.fullmatch(network_id):
                    return 400, {"message": "delete requires full ID"}
                network = self.networks.get(network_id)
                if network is None:
                    return 404, {"message": "No such network"}
                if network["Containers"]:
                    return 409, {"message": "network has active endpoints"}
                del self.networks[network_id]
                self.successful_mutations.append(copy.deepcopy(request))
                return 204, None

            return 404, {"message": f"unsupported mock request: {method} {path}"}


class UnixHTTPServer(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True

    def __init__(self, path: str, model: Model) -> None:
        self.model = model
        super().__init__(path, Handler)


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, _format: str, *_args: object) -> None:
        pass

    def _request(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        payload = self.rfile.read(length) if length else b""
        try:
            body = json.loads(payload) if payload else None
        except ValueError:
            self._respond(400, {"message": "invalid JSON"})
            return
        status, response = self.server.model.dispatch(self.command, self.path, body)
        self._respond(status, response)

    def _respond(self, status: int, response: object) -> None:
        if response is None:
            data = b""
        elif isinstance(response, str):
            data = response.encode()
        else:
            data = json.dumps(response).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Connection", "close")
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(data)
        self.close_connection = True

    do_GET = _request
    do_HEAD = _request
    do_POST = _request
    do_DELETE = _request
    do_PUT = _request
    do_PATCH = _request


class Daemon:
    def __init__(self, process: subprocess.Popen, stdout_path: Path, stderr_path: Path) -> None:
        self.process = process
        self.stdout_path = stdout_path
        self.stderr_path = stderr_path

    def diagnostics(self) -> str:
        return (
            f"returncode={self.process.poll()}\n"
            f"stdout={self.stdout_path.read_text(errors='replace')[-8000:]}\n"
            f"stderr={self.stderr_path.read_text(errors='replace')[-8000:]}"
        )

    def cleanup(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=4)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=2)


class ControllerBlackBoxTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="tnc-api-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.model = Model()
        self.socket_path = self.root / "docker.sock"
        self.server = UnixHTTPServer(str(self.socket_path), self.model)
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True)
        self.thread.start()
        self.addCleanup(self.close_server)
        self.daemon_count = 0

    def close_server(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)

    def environment(self, extra_env: dict | None = None) -> dict:
        env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith("TNC_")
            and key.lower() not in ("http_proxy", "https_proxy", "all_proxy", "no_proxy")
        }
        env.update({
            "TNC_SOCKET_PATH": str(self.socket_path),
            "TNC_STATE_DIR": str(self.root / "state"),
            "TNC_TARGET_NAME": "traefik",
            "TNC_TARGET_PROJECT": "test",
            "TNC_TARGET_SERVICE": "traefik",
            "TNC_EGRESS_NETWORK": "traefik_egress",
            "TNC_API_TIMEOUT_SECS": "2",
            "TNC_DRY_RUN": "false",
            "DOCKER_HOST": "unix:///no-real-docker-for-this-test.sock",
        })
        env.update(extra_env or {})
        return env

    def cli(self, *args: str, extra_env: dict | None = None) -> subprocess.CompletedProcess:
        return subprocess.run(
            [str(BINARY), *args],
            env=self.environment(extra_env),
            capture_output=True,
            text=True,
            timeout=12,
            check=False,
        )

    def start_daemon(self, extra_env: dict | None = None) -> Daemon:
        self.daemon_count += 1
        stdout_path = self.root / f"daemon-{self.daemon_count}.stdout"
        stderr_path = self.root / f"daemon-{self.daemon_count}.stderr"
        stdout_file = stdout_path.open("wb")
        stderr_file = stderr_path.open("wb")
        self.addCleanup(stdout_file.close)
        self.addCleanup(stderr_file.close)
        options = {
            "TNC_INTERVAL_SECS": "1",
            "TNC_HEALTH_MAX_AGE_SECS": "10",
        }
        options.update(extra_env or {})
        process = subprocess.Popen(
            [str(BINARY), "run"],
            env=self.environment(options),
            stdout=stdout_file,
            stderr=stderr_file,
        )
        daemon = Daemon(process, stdout_path, stderr_path)
        self.addCleanup(daemon.cleanup)
        return daemon

    def health(self) -> dict | None:
        try:
            return json.loads((self.root / "state" / "health.json").read_text())
        except (FileNotFoundError, json.JSONDecodeError):
            return None

    def wait_until(self, predicate, description: str, daemon: Daemon, timeout: float = 8.0) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return
            if daemon.process.poll() is not None:
                self.fail(f"daemon exited while waiting for {description}\n{daemon.diagnostics()}")
            time.sleep(0.05)
        self.fail(f"timed out waiting for {description}; health={self.health()}\n{daemon.diagnostics()}")

    def is_ready(self) -> bool:
        return (self.health() or {}).get("ready") is True

    def is_unhealthy_after_reconciliation(self) -> bool:
        health = self.health() or {}
        return (
            health.get("ready") is False
            and health.get("detail") not in (None, "initializing", "controller stopped")
        )

    def edge_attached_to(self, container_id: str) -> bool:
        with self.model.lock:
            endpoint = self.model.target["NetworkSettings"]["Networks"].get("traefik_app_echo")
            return (
                self.model.target["Id"] == container_id
                and endpoint == {"NetworkID": EDGE_ID, "GwPriority": -1, "EndpointID": "9" * 64}
                and container_id in self.model.networks[EDGE_ID]["Containers"]
            )

    def assert_graceful_stop(self, daemon: Daemon) -> None:
        daemon.process.terminate()  # SIGTERM on the supported Unix test platform.
        try:
            daemon.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.fail(f"SIGTERM did not stop daemon in five seconds\n{daemon.diagnostics()}")
        self.assertEqual(daemon.process.returncode, 0, daemon.diagnostics())
        self.assertFalse((self.health() or {}).get("ready", True), daemon.diagnostics())
        self.assertFalse(
            any(
                request["method"] == "DELETE" or request["path"].endswith("/disconnect")
                for request in self.model.mutations
            ),
            self.model.mutations,
        )

    def assert_success(self, result: subprocess.CompletedProcess) -> None:
        self.assertEqual(result.returncode, 0, f"stdout={result.stdout}\nstderr={result.stderr}")

    def assert_failure_without_mutation(self, result: subprocess.CompletedProcess) -> None:
        self.assertNotEqual(result.returncode, 0, f"unexpected success: {result.stdout}")
        self.assertEqual(self.model.mutations, [], self.model.mutations)

    def assert_mutation_ids_are_full(self) -> None:
        for request in self.model.mutations:
            if request["path"] == "/networks/create":
                continue
            parts = request["path"].split("/")
            self.assertEqual(parts[1], "networks", request)
            self.assertIsNotNone(FULL_ID.fullmatch(parts[2]), request)
            if request["method"] == "POST":
                self.assertIsNotNone(FULL_ID.fullmatch(request["body"]["Container"]), request)

    def test_plan_reads_snapshot_without_post_or_delete(self) -> None:
        self.assert_success(self.cli("plan"))
        self.assertEqual(self.model.mutations, [])
        self.assertNotIn("traefik_app_echo", self.model.target["NetworkSettings"]["Networks"])

    def test_once_connects_with_negative_gateway_priority_and_second_once_is_idempotent(self) -> None:
        self.assert_success(self.cli("once"))
        self.assertEqual(len(self.model.mutations), 1, self.model.mutations)
        operation = self.model.mutations[0]
        self.assertEqual(operation["path"], f"/networks/{EDGE_ID}/connect")
        self.assertEqual(operation["body"]["Container"], TARGET_ID)
        self.assertEqual(operation["body"]["EndpointConfig"], {"GwPriority": -1})
        self.assert_success(self.cli("once"))
        self.assertEqual(len(self.model.mutations), 1, self.model.mutations)
        self.assert_mutation_ids_are_full()

    def test_dry_run_once_does_not_mutate(self) -> None:
        self.assert_success(self.cli("once", extra_env={"TNC_DRY_RUN": "true"}))
        self.assertEqual(self.model.mutations, [])

    def test_wrong_target_project_blocks_all_writes(self) -> None:
        self.model.target["Config"]["Labels"]["com.docker.compose.project"] = "foreign"
        self.assert_failure_without_mutation(self.cli("once"))

    def test_invalid_later_network_blocks_even_the_valid_earlier_network(self) -> None:
        invalid = owned_network("second", SECOND_EDGE_ID)
        invalid["Options"] = {"com.docker.network.bridge.gateway_mode_ipv4": "nat-unprotected"}
        self.model.networks[SECOND_EDGE_ID] = invalid
        self.assert_failure_without_mutation(self.cli("once"))

    def test_candidate_with_missing_role_label_is_not_silently_ignored(self) -> None:
        invalid = owned_network("second", SECOND_EDGE_ID)
        del invalid["Labels"][LABEL_ROLE]
        self.model.networks[SECOND_EDGE_ID] = invalid
        self.assert_failure_without_mutation(self.cli("once"))

    def test_target_recreation_is_not_an_authority_to_mutate_the_new_target(self) -> None:
        self.model.recreate_after_first_target_read = True
        result = self.cli("once")
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertEqual(self.model.successful_mutations, [], self.model.successful_mutations)
        self.assert_mutation_ids_are_full()

    def test_unsupported_api_cannot_mutate(self) -> None:
        self.model.api_version = "1.47"
        self.assert_failure_without_mutation(self.cli("once"))

    def test_inventory_failure_cannot_be_treated_as_empty_success(self) -> None:
        self.model.fail_network_list = True
        self.assert_failure_without_mutation(self.cli("once"))

    def test_prepare_cannot_adopt_a_foreign_existing_name(self) -> None:
        self.model.networks[EDGE_ID]["Labels"] = {}
        self.assert_failure_without_mutation(self.cli("prepare", "echo"))

    def test_prepare_creates_a_new_owned_network_then_attaches_by_full_id(self) -> None:
        self.model.networks.pop(EDGE_ID)
        self.assert_success(self.cli("prepare", "echo"))
        operations = self.model.mutations
        self.assertEqual(len(operations), 2, operations)
        self.assertEqual(operations[0]["path"], "/networks/create")
        self.assertEqual(operations[0]["body"]["Name"], "traefik_app_echo")
        self.assertEqual(operations[0]["body"]["Labels"][LABEL_AUTOJOIN], "true")
        self.assertEqual(operations[0]["body"]["Labels"][LABEL_ROLE], "application-edge")
        self.assertEqual(operations[0]["body"]["Labels"][LABEL_OWNER], "echo")
        self.assertTrue(operations[1]["path"].endswith("/connect"))
        self.assert_mutation_ids_are_full()

    def test_dry_run_prepare_limits_pending_changes_to_named_application(self) -> None:
        result = self.cli("prepare", "echo_two", extra_env={"TNC_DRY_RUN": "true"})
        self.assert_success(result)
        report = json.loads(result.stdout)
        pending = {(action["operation"], action["network"]) for action in report["pending"]}
        self.assertEqual(
            pending,
            {
                ("create", "traefik_app_echo_two"),
                ("connect", "traefik_app_echo_two"),
            },
        )
        self.assertEqual(self.model.mutations, [])

    def test_existing_wrong_priority_fails_without_detach_repair(self) -> None:
        self.model.attach(priority=0)
        self.assert_failure_without_mutation(self.cli("once"))

    def test_retire_refuses_a_stopped_container_reference(self) -> None:
        self.model.attach()
        self.model.other_containers.append({
            "Id": APP_ID,
            "Names": ["/echo-old"],
            "State": "exited",
            "NetworkSettings": {"Networks": {"traefik_app_echo": {"NetworkID": EDGE_ID}}},
        })
        self.assert_failure_without_mutation(self.cli("retire", "echo"))
        lists = [request for request in self.model.requests if request["path"] == "/containers/json"]
        self.assertTrue(lists, "retire must inspect all container references")
        self.assertIn(lists[0]["query"].get("all", [""])[0].lower(), ("true", "1"))

    def test_retire_disconnects_only_target_and_deletes_only_owned_network(self) -> None:
        self.model.attach()
        self.assert_success(self.cli("retire", "echo"))
        self.assertEqual(
            [(request["method"], request["path"]) for request in self.model.mutations],
            [("POST", f"/networks/{EDGE_ID}/disconnect"), ("DELETE", f"/networks/{EDGE_ID}")],
        )
        self.assertEqual(self.model.mutations[0]["body"]["Container"], TARGET_ID)
        self.assertIs(self.model.mutations[0]["body"].get("Force", False), False)
        self.assertNotIn(EDGE_ID, self.model.networks)
        self.assertIn("traefik_egress", self.model.target["NetworkSettings"]["Networks"])
        self.assert_mutation_ids_are_full()

    def test_daemon_restores_recreated_target_with_unavailable_event_stream(self) -> None:
        # /events deliberately returns 404: recovery works without an event stream.
        # Both the periodic timer and failed-stream hints may trigger reconciliation.
        daemon = self.start_daemon()
        self.wait_until(lambda: self.is_ready() and self.edge_attached_to(TARGET_ID), "initial attachment", daemon)
        self.assert_success(self.cli("healthcheck", extra_env={"TNC_HEALTH_MAX_AGE_SECS": "10"}))
        initial_health_timestamp = self.health()["updated_at"]
        with self.model.lock:
            self.model.target["Id"] = REPLACEMENT_ID
            self.model.target["NetworkSettings"]["Networks"].pop("traefik_app_echo")
            self.model.networks[EDGE_ID]["Containers"].pop(TARGET_ID)
            for network_id in (EGRESS_ID, CONTROL_ID):
                member = self.model.networks[network_id]["Containers"].pop(TARGET_ID)
                self.model.networks[network_id]["Containers"][REPLACEMENT_ID] = member
        self.wait_until(
            lambda: self.is_ready()
            and self.edge_attached_to(REPLACEMENT_ID)
            and (self.health() or {}).get("updated_at", 0) > initial_health_timestamp,
            "attachment and fresh healthy state for the replacement target generation",
            daemon,
        )
        connections = [
            request["body"]["Container"]
            for request in self.model.successful_mutations
            if request["path"].endswith("/connect")
        ]
        self.assertEqual(connections, [TARGET_ID, REPLACEMENT_ID])
        self.assert_mutation_ids_are_full()
        self.assert_graceful_stop(daemon)
        self.assertTrue(self.edge_attached_to(REPLACEMENT_ID))

    def test_live_daemon_allows_plan_rejects_duplicate_and_stops_without_disconnect(self) -> None:
        daemon = self.start_daemon()
        self.wait_until(self.is_ready, "healthy daemon", daemon)
        mutation_count = len(self.model.mutations)
        self.assert_success(self.cli("plan"))
        self.assertEqual(len(self.model.mutations), mutation_count)
        duplicate = self.cli("run", extra_env={"TNC_INTERVAL_SECS": "1"})
        self.assertNotEqual(duplicate.returncode, 0, duplicate.stdout)
        self.assertIsNone(daemon.process.poll(), daemon.diagnostics())
        self.assert_success(self.cli("healthcheck"))
        self.assert_graceful_stop(daemon)
        self.assertNotEqual(self.cli("healthcheck").returncode, 0)
        self.assertTrue(self.edge_attached_to(TARGET_ID))

    def test_daemon_becomes_unhealthy_for_missing_or_foreign_target_then_recovers(self) -> None:
        daemon = self.start_daemon()
        self.wait_until(self.is_ready, "healthy daemon", daemon)
        with self.model.lock:
            self.model.target_available = False
        self.wait_until(self.is_unhealthy_after_reconciliation, "missing-target failure", daemon)
        self.assertNotEqual(self.cli("healthcheck").returncode, 0)
        mutation_count = len(self.model.mutations)
        with self.model.lock:
            self.model.target_available = True
            self.model.target["Config"]["Labels"]["com.docker.compose.project"] = "foreign"
            self.model.target["NetworkSettings"]["Networks"].pop("traefik_app_echo")
            self.model.networks[EDGE_ID]["Containers"].pop(TARGET_ID)
        self.wait_until(
            lambda: self.is_unhealthy_after_reconciliation()
            and "project" in str((self.health() or {}).get("detail", "")).lower(),
            "foreign-project failure",
            daemon,
        )
        self.assertEqual(len(self.model.mutations), mutation_count)
        with self.model.lock:
            self.model.target["Config"]["Labels"]["com.docker.compose.project"] = "test"
        self.wait_until(
            lambda: self.is_ready() and self.edge_attached_to(TARGET_ID),
            "recovery after the intended target returns",
            daemon,
        )
        self.assert_success(self.cli("healthcheck"))
        self.assert_graceful_stop(daemon)

    def test_dry_run_daemon_with_pending_attachments_is_unhealthy_without_writes(self) -> None:
        daemon = self.start_daemon({"TNC_DRY_RUN": "true"})
        self.wait_until(
            lambda: self.is_unhealthy_after_reconciliation()
            and "pending" in str((self.health() or {}).get("detail", "")).lower(),
            "dry-run pending health status",
            daemon,
        )
        self.assertNotEqual(self.cli("healthcheck", extra_env={"TNC_DRY_RUN": "true"}).returncode, 0)
        self.assertEqual(self.model.mutations, [])
        self.assertFalse(self.edge_attached_to(TARGET_ID))
        self.assert_graceful_stop(daemon)
        self.assertEqual(self.model.mutations, [])

    def add_stopped_reference(self) -> None:
        self.model.other_containers.append({
            "Id": APP_ID, "Names": ["/echo"], "State": "exited",
            "NetworkSettings": {"Networks": {"traefik_app_echo": {"NetworkID": EDGE_ID}}},
        })

    def test_read_only_commands_preserve_journal_and_docker_state(self) -> None:
        self.assert_success(self.cli("once"))
        before = (self.root / "state" / "attachments.json").read_bytes()
        mutations = copy.deepcopy(self.model.mutations)
        for command in ["plan", "status", "doctor"]:
            with self.subTest(command=command):
                result = self.cli(command)
                self.assert_success(result)
                report = json.loads(result.stdout)
                if command == "doctor":
                    self.assertEqual(report["checks"]["routed_request"], "NOT_CHECKED")
                    report = report["report"]
                self.assertTrue(report["observation_complete"])
                self.assertEqual((self.root / "state" / "attachments.json").read_bytes(), before)
                self.assertEqual(self.model.mutations, mutations)

    def test_saved_plan_applies_only_unchanged_material_state(self) -> None:
        result = self.cli("plan")
        self.assert_success(result)
        path = self.root / "plan.json"
        path.write_text(result.stdout)
        self.assert_success(self.cli("apply", str(path)))
        self.assertTrue(self.edge_attached_to(TARGET_ID))
        count = len(self.model.mutations)
        stale = self.cli("apply", str(path))
        self.assertNotEqual(stale.returncode, 0)
        self.assertEqual(json.loads(stale.stderr.splitlines()[-1])["error"]["code"], "STALE_PLAN")
        self.assertEqual(len(self.model.mutations), count)

    def test_tampered_plan_actions_are_not_executed(self) -> None:
        result = self.cli("plan")
        self.assert_success(result)
        plan = json.loads(result.stdout)
        plan["pending"] = []
        path = self.root / "tampered.json"
        path.write_text(json.dumps(plan))
        self.assert_failure_without_mutation(self.cli("apply", str(path)))

    def test_auto_detach_lifecycle_keeps_stopped_reference_and_does_not_oscillate(self) -> None:
        options = {"TNC_AUTO_DETACH": "true", "TNC_DETACH_GRACE_SECS": "1"}
        self.assert_success(self.cli("once", extra_env=options))
        self.add_stopped_reference()
        self.assert_success(self.cli("once", extra_env=options))
        time.sleep(1.1)
        self.assert_success(self.cli("once", extra_env=options))
        self.assertTrue(self.edge_attached_to(TARGET_ID))
        self.model.other_containers.clear()
        self.assert_success(self.cli("once", extra_env=options))
        self.assertTrue(self.edge_attached_to(TARGET_ID))
        time.sleep(1.1)
        self.assert_success(self.cli("once", extra_env=options))
        self.assertFalse(self.edge_attached_to(TARGET_ID))
        self.assertIn(EDGE_ID, self.model.networks)
        self.assert_success(self.cli("once", extra_env=options))
        self.assertFalse(self.edge_attached_to(TARGET_ID))
        self.add_stopped_reference()
        self.assert_success(self.cli("once", extra_env=options))
        self.assertTrue(self.edge_attached_to(TARGET_ID))
        self.assertFalse(any(r["method"] == "DELETE" for r in self.model.mutations))
        self.assert_mutation_ids_are_full()

    def test_auto_detach_does_not_claim_preexisting_attachment(self) -> None:
        self.model.attach()
        self.add_stopped_reference()
        options = {"TNC_AUTO_DETACH": "true", "TNC_DETACH_GRACE_SECS": "1"}
        self.assert_success(self.cli("once", extra_env=options))
        self.model.other_containers.clear()
        self.assert_success(self.cli("once", extra_env=options))
        time.sleep(1.1)
        result = self.cli("once", extra_env=options)
        self.assert_success(result)
        self.assertEqual(self.model.mutations, [])
        decision = json.loads(result.stdout)["decisions"][0]
        self.assertEqual(decision["ownership"], "unowned")
        self.assertEqual(decision["reason_code"], "OWNERSHIP_UNKNOWN")

    def test_explicit_adopt_requires_matching_endpoint_and_records_provenance(self) -> None:
        self.model.attach()
        wrong = self.cli("adopt", "echo", "--target-id", TARGET_ID,
                         "--network-id", EDGE_ID, "--endpoint-id", "7" * 64)
        self.assert_failure_without_mutation(wrong)
        self.assert_success(self.cli("adopt", "echo", "--target-id", TARGET_ID,
                                     "--network-id", EDGE_ID, "--endpoint-id", "9" * 64))
        report = json.loads(self.cli("status").stdout)
        self.assertEqual(report["decisions"][0]["ownership"], "adopted")
        self.assertEqual(self.model.mutations, [])  # Adoption changes only validated local ownership.

    def test_malformed_all_container_inventory_blocks_connect(self) -> None:
        self.model.other_containers.append({"Id": APP_ID, "Names": ["/broken"]})
        self.assert_failure_without_mutation(self.cli("once"))

    def test_protected_candidate_cannot_be_attached_or_retired(self) -> None:
        options = {"TNC_PROTECTED_NETWORKS": "traefik_app_echo"}
        self.assert_success(self.cli("once", extra_env=options))
        self.assertEqual(self.model.mutations, [])
        self.assert_failure_without_mutation(self.cli("retire", "echo", extra_env=options))

    def test_corrupt_journal_is_not_recreated_to_authorize_mutation(self) -> None:
        state = self.root / "state"
        state.mkdir(mode=0o700)
        journal = state / "attachments.json"
        journal.write_text("{truncated")
        journal.chmod(0o600)
        self.assert_failure_without_mutation(self.cli("once"))
        self.assertEqual(journal.read_text(), "{truncated")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    BINARY = Path(sys.argv.pop(1)).resolve()
    if not BINARY.is_file():
        raise SystemExit(f"Controller binary not found: {BINARY}")
    unittest.main(verbosity=2)
