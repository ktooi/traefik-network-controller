#!/usr/bin/env python3
"""Release publication guard regression tests; no Docker or registry calls."""

import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[1]
HELPER = ROOT / "scripts/ci/release_metadata.py"
SPEC = importlib.util.spec_from_file_location("release_metadata_helper", HELPER)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
VERSION = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]


class ReleaseMetadataTests(unittest.TestCase):
    def test_exact_stable_tag(self):
        self.assertEqual(MODULE.validate("v0.2.0", "0.2.0"), {"version": "0.2.0", "image_tag": "0.2.0"})

    def test_different_source_version_is_rejected(self):
        with self.assertRaises(ValueError):
            MODULE.validate("v0.2.1", "0.2.0")

    def test_untrusted_tag_forms_are_rejected(self):
        invalid = ("0.2.0", "v01.2.0", "v1.02.0", "v1.2.00", "v1.2", "v1.2.3.4",
                   "v0.2.0-rc.1", "v0.2.0+build.1", "v0.2.0\n", "v0.2.0\nBAD=value",
                   "v0.2.0;echo injected", "v０.2.0", "refs/tags/v0.2.0", "../v0.2.0", "")
        for value in invalid:
            with self.subTest(tag=value), self.assertRaises(ValueError):
                MODULE.validate(value, "0.2.0")

    def command(self, *args):
        return subprocess.run([sys.executable, str(HELPER), *args], text=True, capture_output=True, timeout=5)

    def test_json_contract(self):
        result = self.command(f"v{VERSION}")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout), {"version": VERSION, "image_tag": VERSION})

    def test_github_contract(self):
        result = self.command(f"v{VERSION}", "--format", "github")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, f"version={VERSION}\nimage_tag={VERSION}\n")

    def test_gitlab_contract(self):
        result = self.command(f"v{VERSION}", "--format", "dotenv")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, f"TNC_VERSION={VERSION}\nTNC_IMAGE_TAG={VERSION}\n")

    def test_invalid_tag_writes_no_success_stdout(self):
        result = self.command("v0.2.0-rc.1", "--format", "github")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "")
        self.assertIn("rejected", result.stderr)

    def test_invalid_format_is_rejected(self):
        self.assertNotEqual(self.command(f"v{VERSION}", "--format", "shell").returncode, 0)


if __name__ == "__main__":
    unittest.main()
