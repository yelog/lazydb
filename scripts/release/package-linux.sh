#!/bin/sh
set -eu
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$SCRIPT_DIR/release-lib.sh"
[ "$#" -eq 4 ] || release_die "usage: package-linux.sh VERSION ARCH BINARY OUTPUT_DIR"
version=$1; arch=$2; binary=$3; output=$4
case "$version" in *-beta.*) release_die 'native packages are stable-only' ;; esac
case "$arch" in x86_64) nfpm_arch=amd64 ;; aarch64) nfpm_arch=arm64 ;; *) release_die "unsupported architecture: $arch" ;; esac
[ -x "$binary" ] || release_die "binary is not executable: $binary"
mkdir -p "$output"
NFPM_VERSION=$version NFPM_ARCH=$nfpm_arch NFPM_BINARY=$binary nfpm pkg --config "$SCRIPT_DIR/../../packaging/nfpm.yaml" --packager deb --target "$output/lazydb_${version}_${nfpm_arch}.deb"
NFPM_VERSION=$version NFPM_ARCH=$nfpm_arch NFPM_BINARY=$binary nfpm pkg --config "$SCRIPT_DIR/../../packaging/nfpm.yaml" --packager rpm --target "$output/lazydb-${version}-${nfpm_arch}.rpm"
NFPM_VERSION=$version NFPM_ARCH=$arch NFPM_BINARY=$binary nfpm pkg --config "$SCRIPT_DIR/../../packaging/nfpm.yaml" --packager archlinux --target "$output/lazydb-${version}-${arch}.pkg.tar.zst"
