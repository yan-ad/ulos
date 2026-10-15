#!/usr/bin/env python3
"""Read the desktop Cargo version and reject mismatched release identities."""
import json
import re
import subprocess
import sys

metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], text=True))
version = next(p["version"] for p in metadata["packages"] if p["name"] == "ulos-desktop")
if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", version):
    sys.exit("Unsupported Cargo version")
if len(sys.argv) > 1 and version != sys.argv[1]:
    sys.exit(f"Cargo version {version!r} does not match release {sys.argv[1]!r}")
print(version)
