#!/usr/bin/env python3
"""Check reviewed Cargo notices and pinned Rust notices; no third-party modules."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tomllib


ROOT = Path(__file__).resolve().parents[2]
NOTICE_PATH = ROOT / "THIRD-PARTY-NOTICES.txt"
# Exact reviewed expressions, not a general SPDX compatibility solver.
LICENSE_CHOICES = {
    "MIT": "MIT",
    "MIT OR Apache-2.0": "MIT",
    "Apache-2.0 OR MIT": "MIT",
    "MIT/Apache-2.0": "MIT (legacy dual-license expression)",
    "Apache-2.0": "Apache-2.0",
    "Apache-2.0 OR BSL-1.0": "BSL-1.0",
    "Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT": "MIT",
    "(MIT OR Apache-2.0) AND Unicode-3.0": "MIT AND Unicode-3.0",
    "Unicode-3.0": "Unicode-3.0",
    "Unlicense OR MIT": "MIT",
}
LICENSE_NAMES = ("license", "licence", "copying", "notice", "copyright", "authors")
COMMENT = re.compile(r"/\*[\s\S]*?\*/|(?m:^[ \t]*//[^\n]*(?:\n[ \t]*//[^\n]*)*)")
LEGAL_COMMENT = re.compile(
    r"copyright|SPDX-License-Identifier|licensed? under|"
    r"permission is hereby granted|redistribution and use",
    re.IGNORECASE,
)
EXTRA_LICENSES = {
    "futures-channel": "Includes a BSD-2-Clause notice in src/mpsc/queue.rs.",
    "icu_provider": "Includes an MIT OR Apache-2.0 notice in src/marker.rs.",
    "tower": "Includes Apache-2.0 attribution in src/load/peak_ewma.rs; "
    "the notice is retained even if that optional module is not compiled.",
}


def run(*args):
    return subprocess.run(args, cwd=ROOT, check=True, capture_output=True, text=True).stdout


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def check_rust_notices():
    directory = ROOT / "third_party" / "rust"
    origin = json.loads((directory / "ORIGIN.json").read_text(encoding="utf-8"))
    toolchain = tomllib.loads((ROOT / "rust-toolchain.toml").read_text(encoding="utf-8"))
    version = toolchain["toolchain"]["channel"]
    if version != origin["rust_version"]:
        raise ValueError("Rust notice version differs from rust-toolchain.toml; review and refresh notices")
    rust = dict(line.split(": ", 1) for line in run("rustc", "--version", "--verbose").splitlines() if ": " in line)
    if rust.get("release") != version or rust.get("commit-hash") != origin["rust_commit"]:
        raise ValueError("Active Rust compiler does not match the reviewed notice version/commit")
    if not origin.get("files") or "COPYRIGHT-library.html" not in origin["files"]:
        raise ValueError("Missing Rust standard-library notice inventory")
    for relative, expected in origin["files"].items():
        path = directory / relative
        if path.is_symlink() or not path.resolve().is_relative_to(directory.resolve()):
            raise ValueError("Rust notice path must be a local regular file")
        if not path.is_file() or sha256(path.read_bytes()) != expected:
            raise ValueError(f"Rust notice hash mismatch: {relative}")


def collect_notices(package):
    directory = Path(package["manifest_path"]).parent
    items = []
    license_file_count = 0
    explicit = package.get("license_file")
    for path in sorted(directory.rglob("*")):
        if not path.is_file():
            continue
        relative = path.relative_to(directory).as_posix()
        if path.is_symlink():
            raise ValueError(f"Unexpected symlink in crate source: {package['name']}/{relative}")
        # Preserve supplied texts and nested attributions, including copyright
        # files that provide information missing from the package SPDX field.
        if any(part in path.name.lower() for part in LICENSE_NAMES) or relative == explicit:
            content = path.read_bytes().decode("utf-8")
            if not content.strip():
                raise ValueError(f"Empty license material: {package['name']}/{relative}")
            items.append((relative, content))
            license_file_count += 1
        elif path.suffix in (".rs", ".c", ".h"):
            content = path.read_bytes().decode("utf-8")
            for match in COMMENT.finditer(content):
                if LEGAL_COMMENT.search(match.group(0)):
                    line = content.count("\n", 0, match.start()) + 1
                    items.append((f"{relative} (comment at line {line})", match.group(0)))
    if not license_file_count:
        raise ValueError(f"No license text found for {package['name']} {package['version']}")
    return items


def generate(offline):
    project_license = (ROOT / "LICENSE").read_text(encoding="utf-8")
    if not project_license.startswith("MIT License\n") or "Copyright (c)" not in project_license:
        raise ValueError("Project LICENSE is missing its MIT title or copyright notice")
    args = ["cargo", "metadata", "--locked", "--format-version", "1"]
    if offline:
        args.append("--offline")
    metadata = json.loads(run(*args))
    root_id = metadata["resolve"]["root"]
    root_package = next(p for p in metadata["packages"] if p["id"] == root_id)
    if root_package["name"] != "traefik-network-controller" or root_package["license"] != "MIT":
        raise ValueError("Project package must declare the reviewed MIT license")
    lock_bytes = (ROOT / "Cargo.lock").read_bytes()
    locked = tomllib.loads(lock_bytes.decode("utf-8"))["package"]
    key = lambda p: (p["name"], p["version"], p.get("source"))
    if {key(p) for p in locked} != {key(p) for p in metadata["packages"]}:
        raise ValueError("Cargo metadata does not cover every locked package; review dependency scope")
    packages = sorted((p for p in metadata["packages"] if p["id"] != root_id), key=key)
    texts = {}
    package_sections = []
    material_count = 0
    for package in packages:
        if package.get("source") != "registry+https://github.com/rust-lang/crates.io-index":
            raise ValueError(f"Unreviewed dependency source: {package['name']}")
        declared = package.get("license")
        if declared not in LICENSE_CHOICES:
            raise ValueError(f"Unreviewed license expression for {package['name']}: {declared!r}")
        materials = collect_notices(package)
        checksum = next(p["checksum"] for p in locked if key(p) == key(package))
        lines = [
            f"{package['name']} {package['version']}",
            f"  Crate: https://crates.io/crates/{package['name']}/{package['version']}",
            f"  Registry package SHA-256: {checksum}",
            f"  Declared license: {declared}",
            f"  Chosen package license: {LICENSE_CHOICES[declared]}",
        ]
        if package.get("authors"):
            lines.append("  Authors (package metadata): " + "; ".join(package["authors"]))
        if package["name"] in EXTRA_LICENSES:
            lines.append("  Additional material: " + EXTRA_LICENSES[package["name"]])
        for name, content in materials:
            digest = sha256(content.encode("utf-8"))
            texts.setdefault(digest, content)
            lines.append(f"  Notice: {name} -> SHA-256 {digest}")
            material_count += 1
        package_sections.append("\n".join(lines))
    header = f"""THIRD-PARTY NOTICES — TRAEFIK NETWORK CONTROLLER

Generated from Cargo.lock and the corresponding downloaded crate sources.
Cargo.lock SHA-256: {sha256(lock_bytes)}
External packages: {len(packages)}
Notice files/comment blocks: {material_count}
Distinct notice texts: {len(texts)}

This inventory conservatively includes all locked platforms and build/test
dependencies for the project's configured features. It is not a statement
that every listed file or package is linked into the Linux executable.

The project's LICENSE applies to its original code and documentation.
Third-party materials retain their own licenses. Package license choices
below do not override additional per-file notices. All supplied alternative
license texts are retained for reference; this does not impose all license
alternatives simultaneously or relicense third-party code as MIT.

Rust standard-library notices are separately retained in third_party/rust/.
Container base-image packages retain their own licenses and source obligations;
see docs/licensing.md. This Cargo inventory does not cover the operating system.

Regenerate after reviewing a dependency change:
  python3 scripts/ci/check_licenses.py --write
Verify without modifying files:
  python3 scripts/ci/check_licenses.py

PACKAGE INVENTORY
=================

"""
    sections = [header, "\n\n".join(package_sections), "\n\nFULL NOTICE TEXTS\n=================\n"]
    for digest, content in sorted(texts.items()):
        sections.append(f"\n--- BEGIN SHA-256 {digest} ---\n{content}")
        if not content.endswith("\n"):
            sections.append("\n")
        sections.append(f"--- END SHA-256 {digest} ---\n")
    return "".join(sections), len(packages), material_count


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="regenerate Cargo notices after review")
    parser.add_argument("--offline", action="store_true", help="require dependencies in Cargo's local cache")
    args = parser.parse_args()
    try:
        check_rust_notices()
        expected, count, materials = generate(args.offline)
        if args.write:
            NOTICE_PATH.write_text(expected, encoding="utf-8")
            print(f"Updated THIRD-PARTY-NOTICES.txt: {count} packages, {materials} notice materials")
        elif not NOTICE_PATH.is_file() or NOTICE_PATH.read_bytes() != expected.encode("utf-8"):
            raise ValueError("THIRD-PARTY-NOTICES.txt is stale; review changes and regenerate with --write")
        else:
            print(f"License notices match: {count} Cargo packages; {materials} notice materials; pinned Rust notices")
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print(f"License check failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
