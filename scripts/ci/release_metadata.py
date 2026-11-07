#!/usr/bin/env python3
"""Validate a stable version tag against the source before any registry login.

Default stdout is JSON. --format github emits GITHUB_OUTPUT records;
--format dotenv emits GitLab dotenv records. Requires Python 3.11+.
No shell evaluation or arbitrary source path is accepted.
"""

import argparse
import json
from pathlib import Path
import re
import sys
import tomllib


ROOT = Path(__file__).resolve().parents[2]
TAG = re.compile(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", re.ASCII)


def validate(tag: str, package_version: str) -> dict[str, str]:
    if TAG.fullmatch(tag) is None:
        raise ValueError("release tag must be stable vMAJOR.MINOR.PATCH (no leading zeroes)")
    version = tag[1:]
    if version != package_version:
        raise ValueError(f"tag version {version} does not match Cargo.toml {package_version}")
    return {"version": version, "image_tag": version}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tag")
    parser.add_argument("--format", choices=("json", "github", "dotenv"), default="json")
    args = parser.parse_args()
    try:
        package = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]
        metadata = validate(args.tag, package["version"])
    except (ValueError, OSError, KeyError) as error:
        print(f"release metadata rejected: {error}", file=sys.stderr)
        return 1
    if args.format == "github":
        print(f"version={metadata['version']}\nimage_tag={metadata['image_tag']}")
    elif args.format == "dotenv":
        print(f"TNC_VERSION={metadata['version']}\nTNC_IMAGE_TAG={metadata['image_tag']}")
    else:
        print(json.dumps(metadata, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
