#!/bin/sh
set -eu
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$SCRIPT_DIR/release-lib.sh"
ROOT=$(release_root)
cd "$ROOT"
[ "$#" -eq 1 ] || release_die "usage: set-version.sh VERSION"
version=$1
release_version_from_tag "v$version" >/dev/null

if command -v cargo-set-version >/dev/null 2>&1; then
    cargo set-version --workspace "$version"
else
    tmp=$(mktemp)
    trap 'rm -f "$tmp"' EXIT HUP INT TERM
    awk -v version="$version" 'BEGIN { done=0 } /^version = "/ && !done { print "version = \"" version "\""; done=1; next } { print }' Cargo.toml >"$tmp"
    mv "$tmp" Cargo.toml
fi
cargo check >/dev/null
[ "$(release_cargo_version)" = "$version" ] || release_die "failed to update Cargo.toml"
[ "$(release_lock_version)" = "$version" ] || release_die "failed to update Cargo.lock"
