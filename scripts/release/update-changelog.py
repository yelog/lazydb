#!/usr/bin/env python3
"""Insert one validated release section into CHANGELOG.md."""
import datetime as dt
import pathlib
import re
import sys

if len(sys.argv) not in (3, 4):
    raise SystemExit("usage: update-changelog.py VERSION BODY_FILE [DATE]")
version, body_file = sys.argv[1:3]
date = sys.argv[3] if len(sys.argv) == 4 else dt.date.today().isoformat()
if not re.fullmatch(r"(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-beta\.[1-9]\d*)?", version):
    raise SystemExit(f"invalid version: {version}")
if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", date):
    raise SystemExit(f"invalid date: {date}")
root = pathlib.Path(__file__).resolve().parents[2]
path = root / "CHANGELOG.md"
text = path.read_text(encoding="utf-8")
heading = f"## [{version}] - {date}"
if heading in text:
    raise SystemExit(f"Changelog section already exists: {version}")
body = pathlib.Path(body_file).read_text(encoding="utf-8").strip()
if not body or not body.startswith("### "):
    raise SystemExit("Changelog body must contain release subsections, not a heading")
section = f"{heading}\n\n{body}\n\n"
marker = "## Unreleased\n"
if marker in text:
    text = text.replace(marker, section + marker, 1)
else:
    text = text.rstrip() + "\n\n" + section
path.write_text(text, encoding="utf-8")
