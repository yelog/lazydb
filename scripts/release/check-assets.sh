#!/bin/sh
set -eu
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$SCRIPT_DIR/release-lib.sh"
[ "$#" -eq 3 ] || release_die "usage: check-assets.sh CHANNEL VERSION ASSET_DIR"
channel=$1
version=$2
dir=$3
case "$channel" in beta|stable) ;; *) release_die "invalid channel: $channel" ;; esac
for suffix in x86_64-apple-darwin aarch64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
    found=$(find "$dir" -maxdepth 1 -type f -name "lazydb_${version}_${suffix}.tar.xz" -print -quit)
    [ -n "$found" ] || release_die "missing archive for $suffix"
done
if [ "$channel" = stable ]; then
    for name in \
        "lazydb_${version}_amd64.deb" "lazydb_${version}_arm64.deb" \
        "lazydb-${version}-amd64.rpm" "lazydb-${version}-arm64.rpm" \
        "lazydb-${version}-x86_64.pkg.tar.zst" "lazydb-${version}-aarch64.pkg.tar.zst"; do
        [ -f "$dir/$name" ] || release_die "missing stable package: $name"
    done
    [ -f "$dir/lazydb-installer.sh" ] || release_die "missing stable installer"
fi
[ -f "$dir/SHA256SUMS" ] || release_die "missing SHA256SUMS"
[ -f "$dir/lazydb-${version}-sbom.spdx.json" ] || release_die "missing SBOM"
(cd "$dir" && sha256sum -c SHA256SUMS >/dev/null 2>&1 || shasum -a 256 -c SHA256SUMS >/dev/null 2>&1) || release_die "checksum verification failed"
