#!/bin/sh
set -eu
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$SCRIPT_DIR/release-lib.sh"
ROOT=$(release_root)
cd "$ROOT"
[ "$#" -eq 1 ] || release_die "usage: validate-changelog.sh VERSION"
version=$1
count=$(awk -v v="$version" '$0 ~ "^## \\[" v "\\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$" { n++ } END { print n + 0 }' CHANGELOG.md)
[ "$count" -eq 1 ] || release_die "expected exactly one Changelog section for $version"
section=$($SCRIPT_DIR/changelog-section.sh "$version")
printf '%s\n' "$section" | grep '^### ' >/dev/null || release_die "Changelog section has no category: $version"
printf '%s\n' "$section" | grep '^### Commits$' >/dev/null || release_die "Changelog section has no Commits list: $version"
printf '%s\n' "$section" | grep -E '^## \[[0-9]+\.[0-9]+\.[0-9]+(-beta\.[1-9][0-9]*)?\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$' >/dev/null || release_die "invalid Changelog heading: $version"
