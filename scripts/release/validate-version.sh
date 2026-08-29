#!/bin/sh
set -eu
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
# shellcheck source=release-lib.sh
. "$SCRIPT_DIR/release-lib.sh"
ROOT=$(release_root)
cd "$ROOT"

pre_tag=0
tag=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --pre-tag) pre_tag=1 ;;
        --help) printf 'Usage: validate-version.sh [--pre-tag] vMAJOR.MINOR.PATCH[-beta.N]\n'; exit 0 ;;
        v*) tag=$1 ;;
        *) release_die "unknown argument: $1" ;;
    esac
    shift
done
[ -n "$tag" ] || release_die "a release tag is required"
version=$(release_version_from_tag "$tag")
[ "$(release_cargo_version)" = "$version" ] || release_die "Cargo.toml version does not match $version"
[ "$(release_lock_version)" = "$version" ] || release_die "Cargo.lock version does not match $version"
"$SCRIPT_DIR/validate-changelog.sh" "$version"

if [ "$pre_tag" -eq 0 ]; then
    git rev-parse --verify --quiet "refs/tags/$tag" >/dev/null || release_die "tag does not exist: $tag"
    git merge-base --is-ancestor "refs/tags/$tag" "origin/main" || release_die "tag is not reachable from origin/main: $tag"
fi
printf '%s %s\n' "$tag" "$(release_channel "$tag")"
