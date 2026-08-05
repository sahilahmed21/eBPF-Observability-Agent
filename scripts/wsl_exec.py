#!/usr/bin/env python3
"""Strip CR from a script under scripts/ and exec bash with remaining args."""
import os
import pathlib
import subprocess
import sys

repo = pathlib.Path("/mnt/c/projects/eBPF-Observability-Agent")
name = sys.argv[1]
src = repo / "scripts" / name
dst = pathlib.Path(f"/tmp/obs-{os.getuid()}-{name}")
dst.write_bytes(src.read_bytes().replace(b"\r", b""))
raise SystemExit(subprocess.call(["bash", str(dst), *sys.argv[2:]]))
