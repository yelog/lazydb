#!/bin/sh
set -eu

REPO=${LAZYDB_INSTALL_REPO:-yelog/lazydb}
VERSION=${LAZYDB_VERSION:-latest}
INSTALL_DIR=${LAZYDB_INSTALL_DIR:-"$HOME/.local/bin"}

usage() { printf '%s\n' 'Usage: install.sh [--version VERSION] [--install-dir PATH]'; }
die() { printf 'lazydb installer: %s\n' "$*" >&2; exit 1; }
while [ "$#" -gt 0 ]; do
    case "$1" in
        --version) [ "$#" -gt 1 ] || die '--version needs a value'; VERSION=$2; shift 2 ;;
        --install-dir) [ "$#" -gt 1 ] || die '--install-dir needs a value'; INSTALL_DIR=$2; shift 2 ;;
        --help) usage; exit 0 ;;
        *) die "unknown argument: $1" ;;
    esac
done

OS=$(uname -s); ARCH=$(uname -m)
case "$OS:$ARCH" in
    Darwin:x86_64) TARGET=x86_64-apple-darwin ;;
    Darwin:arm64|Darwin:aarch64) TARGET=aarch64-apple-darwin ;;
    Linux:x86_64|Linux:amd64) TARGET=x86_64-unknown-linux-gnu ;;
    Linux:aarch64|Linux:arm64) TARGET=aarch64-unknown-linux-gnu ;;
    *) die "unsupported platform: $OS/$ARCH" ;;
esac

command -v curl >/dev/null 2>&1 || die 'curl is required'
command -v tar >/dev/null 2>&1 || die 'tar is required'
if command -v sha256sum >/dev/null 2>&1; then HASH=sha256sum; else HASH='shasum -a 256'; fi
[ "$VERSION" = latest ] && TAG_PATH=latest || TAG_PATH="v$VERSION"
BASE="https://github.com/$REPO/releases/download/$TAG_PATH"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT HUP INT TERM
ARCHIVE="lazydb_${VERSION}_${TARGET}.tar.xz"
if [ "$VERSION" = latest ]; then
    command -v awk >/dev/null 2>&1 || die 'awk is required'
    release_json=$(curl --fail --location --proto '=https' --tlsv1.2 --max-time 60 "https://api.github.com/repos/$REPO/releases/latest") || die 'failed to resolve latest stable release'
    VERSION=$(printf '%s' "$release_json" | awk -F'"' '/"tag_name"/ { sub(/^v/, "", $4); print $4; exit }')
    case "$VERSION" in
        [0-9]*.[0-9]*.[0-9]*) ;;
        *) die 'GitHub latest release has no valid stable tag' ;;
    esac
    ARCHIVE="lazydb_${VERSION}_${TARGET}.tar.xz"
    TAG_PATH="v$VERSION"
    BASE="https://github.com/$REPO/releases/download/$TAG_PATH"
fi
curl --fail --location --proto '=https' --tlsv1.2 --max-time 60 -o "$TMP/archive.tar.xz" "$BASE/$ARCHIVE" || die 'download failed'
curl --fail --location --proto '=https' --tlsv1.2 --max-time 60 -o "$TMP/SHA256SUMS" "$BASE/SHA256SUMS" || die 'checksum download failed'
EXPECTED=$(awk -v name="$ARCHIVE" '$2 == name || $2 == "*" name { print $1; exit }' "$TMP/SHA256SUMS")
[ -n "$EXPECTED" ] || die "checksum entry missing for $ARCHIVE"
ACTUAL=$($HASH "$TMP/archive.tar.xz" | awk '{print $1}')
[ "$ACTUAL" = "$EXPECTED" ] || die 'checksum mismatch'
mkdir -p "$TMP/unpack" "$INSTALL_DIR"
tar -xJf "$TMP/archive.tar.xz" -C "$TMP/unpack" --no-same-owner
BINARY=$(find "$TMP/unpack" -type f -name lazydb -perm -u+x -print -quit)
[ -n "$BINARY" ] || die 'archive does not contain an executable lazydb'
install -m 0755 "$BINARY" "$INSTALL_DIR/lazydb"
INSTALLED="$INSTALL_DIR/lazydb"
$INSTALLED version --json >/dev/null || die 'installed binary failed version check'
