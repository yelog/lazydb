#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"
set +e
scripts/release/validate-version.sh --pre-tag v0.1.0 >/dev/null 2>&1
status=$?
set -e
[ "$status" -ne 0 ] || { printf 'validator unexpectedly accepted missing Changelog section\n' >&2; exit 1; }
for tag in v0.1.0 v0.1.0-beta.1 v1.2.3 v1.2.3-beta.9; do
    version=$(sh -c '. scripts/release/release-lib.sh; release_version_from_tag "$1"' sh "$tag")
    [ -n "$version" ] || exit 1
done
for tag in 0.1.0 v0.1 v0.1.0-alpha.1 v0.1.0-beta v0.1.0-beta.0; do
    if sh -c '. scripts/release/release-lib.sh; release_version_from_tag "$1"' sh "$tag" >/dev/null 2>&1; then
        printf 'validator unexpectedly accepted %s\n' "$tag" >&2
        exit 1
    fi
done
