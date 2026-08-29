#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"
grep -F 'repository = "https://github.com/yelog/lazydb"' Cargo.toml >/dev/null
grep -F 'https://github.com/yelog/lazydb' CHANGELOG.md >/dev/null
cargo metadata --no-deps --format-version 1 >/dev/null
python3 -c 'compile(open("scripts/release/update-changelog.py", encoding="utf-8").read(), "update-changelog.py", "exec")'
