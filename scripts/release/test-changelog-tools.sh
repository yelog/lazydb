#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"
body=$(mktemp)
original=$(mktemp)
cp CHANGELOG.md "$original"
trap 'cp "$original" CHANGELOG.md; rm -f "$body" "$original"' EXIT HUP INT TERM
printf '%s\n' '### Added' '' '- test feature' '' '### Commits' '' '- [`abc1234`](https://github.com/yelog/lazydb/commit/abc1234) test' >"$body"
python3 scripts/release/update-changelog.py 9.9.9-beta.1 "$body" 2026-08-29
grep -F '## [9.9.9-beta.1] - 2026-08-29' CHANGELOG.md >/dev/null
grep -F '### Commits' CHANGELOG.md >/dev/null
printf '%s\n' 'changelog tools ok'
