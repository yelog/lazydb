#!/bin/sh
set -eu

release_die() {
    printf '%s\n' "release: $*" >&2
    exit 1
}

release_root() {
    git rev-parse --show-toplevel
}

release_version_from_tag() {
    tag=$1
    printf '%s\n' "$tag" | grep -E '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-beta\.[1-9][0-9]*)?$' >/dev/null ||
        release_die "invalid tag: $tag (expected vMAJOR.MINOR.PATCH[-beta.N])"
    version=${tag#v}
    printf '%s\n' "$version"
}

release_channel() {
    case "$1" in
        *-beta.[1-9]*) printf '%s\n' beta ;;
        *) printf '%s\n' stable ;;
    esac
}

release_cargo_version() {
    cargo metadata --no-deps --format-version 1 |
        python3 -c 'import json, sys; data=json.load(sys.stdin); print(next(package["version"] for package in data["packages"] if package["name"] == "lazydb"))'
}

release_lock_version() {
    awk '/^name = "lazydb"$/ { found=1; next } found && /^version = / { gsub(/"/, "", $3); print $3; exit }' Cargo.lock
}

release_sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

release_require_tools() {
    for tool in "$@"; do
        command -v "$tool" >/dev/null 2>&1 || release_die "required tool is missing: $tool"
    done
}
