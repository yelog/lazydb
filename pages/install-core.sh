#!/bin/sh
set -eu

DEFAULT_CHANNEL=stable
REPO=${LAZYDB_INSTALL_REPO:-yelog/lazydb}
CHANNEL=${LAZYDB_CHANNEL:-$DEFAULT_CHANNEL}
VERSION=${LAZYDB_VERSION:-}
INSTALL_DIR=${LAZYDB_INSTALL_DIR:-"$HOME/.local/bin"}
BASE_URL=${LAZYDB_CHANNEL_BASE_URL:-https://lazydb.yelog.org/channels}

usage() { printf '%s\n' 'Usage: install.sh [--channel stable|beta] [--version VERSION] [--install-dir PATH]'; }
die() { printf 'lazydb installer: %s\n' "$*" >&2; exit 1; }
while [ "$#" -gt 0 ]; do
    case "$1" in
        --channel) [ "$#" -gt 1 ] || die '--channel needs a value'; CHANNEL=$2; shift 2 ;;
        --version) [ "$#" -gt 1 ] || die '--version needs a value'; VERSION=$2; shift 2 ;;
        --install-dir) [ "$#" -gt 1 ] || die '--install-dir needs a value'; INSTALL_DIR=$2; shift 2 ;;
        --help) usage; exit 0 ;;
        *) die "unknown argument: $1" ;;
    esac
done
case "$CHANNEL" in stable|beta) ;; *) die "invalid channel: $CHANNEL" ;; esac
[ -z "${LAZYDB_CHANNEL_LOCKED:-}" ] || [ "$CHANNEL" = "$LAZYDB_CHANNEL_LOCKED" ] || die "channel is fixed to $LAZYDB_CHANNEL_LOCKED"
[ -n "${HOME:-}" ] || die 'HOME is required'

OS=$(uname -s); ARCH=$(uname -m)
case "$OS:$ARCH" in
    Darwin:x86_64) TARGET=x86_64-apple-darwin ;;
    Darwin:arm64|Darwin:aarch64) TARGET=aarch64-apple-darwin ;;
    Linux:x86_64|Linux:amd64) TARGET=x86_64-unknown-linux-gnu ;;
    Linux:aarch64|Linux:arm64) TARGET=aarch64-unknown-linux-gnu ;;
    *) die "unsupported platform: $OS/$ARCH" ;;
esac

for tool in curl tar awk cat cp mkdir mv ln chmod; do command -v "$tool" >/dev/null 2>&1 || die "$tool is required"; done
command -v python3 >/dev/null 2>&1 || die 'python3 is required'
if command -v sha256sum >/dev/null 2>&1; then HASH=sha256sum; else command -v shasum >/dev/null 2>&1 || die 'sha256sum or shasum is required'; HASH='shasum -a 256'; fi

DATA_HOME=${LAZYDB_CONFIG_HOME:-"$HOME/.config/lazydb"}
RELEASES=$DATA_HOME/releases
TMP=$(mktemp -d "${TMPDIR:-/tmp}/lazydb-install.XXXXXX")
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT HUP INT TERM

mkdir -p "$DATA_HOME"
LOCK="$DATA_HOME/.install.lock"
acquire_lock() {
    if mkdir "$LOCK" 2>/dev/null; then printf '%s\n' "$$" > "$LOCK/pid"; return; fi
    pid=$(cat "$LOCK/pid" 2>/dev/null || true)
    case "$pid" in ''|*[!0-9]*) rm -rf "$LOCK" ;; *) kill -0 "$pid" 2>/dev/null && die "another installation is running (pid $pid)"; rm -rf "$LOCK" ;; esac
    mkdir "$LOCK" || die 'could not acquire install lock'
    printf '%s\n' "$$" > "$LOCK/pid"
}
release_lock() { rm -rf "$LOCK"; }
trap 'release_lock; cleanup' EXIT HUP INT TERM
acquire_lock

MANIFEST="$TMP/manifest.json"
curl --fail --location --proto '=https' --tlsv1.2 --max-time 60 -o "$MANIFEST" "$BASE_URL/$CHANNEL.json" || die 'manifest download failed'
python3 - "$MANIFEST" "$CHANNEL" "$TARGET" "$VERSION" "$TMP/metadata" <<'PY'
import json, re, sys
from urllib.parse import urlparse
path, channel, target, requested, output = sys.argv[1:]
try:
    data = json.load(open(path, encoding='utf-8'))
    if data.get('schema') != 1 or data.get('product') != 'lazydb' or data.get('channel') != channel:
        raise ValueError('manifest identity mismatch')
    version = data['version']
    if not re.fullmatch(r'(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-beta\.[1-9][0-9]*)?', version): raise ValueError('invalid version')
    if (channel == 'beta') != ('-beta.' in version) or data.get('tag') != 'v' + version or bool(data.get('prerelease')) != (channel == 'beta'): raise ValueError('manifest version mismatch')
    if requested and requested != version: raise ValueError('requested version is not the channel version')
    supported = {'x86_64-apple-darwin', 'aarch64-apple-darwin', 'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu'}
    if set(data.get('assets', {})) != supported: raise ValueError('manifest target set mismatch')
    asset = data['assets'][target]
    url = asset['url']; parsed = urlparse(url)
    if parsed.scheme != 'https' or parsed.netloc not in {'github.com', 'lazydb.yelog.org'} or not re.fullmatch(r'[0-9a-f]{64}', asset['sha256']): raise ValueError('invalid asset')
    name = url.rsplit('/', 1)[-1]
    if name != 'lazydb_%s_%s.tar.xz' % (version, target): raise ValueError('invalid asset name')
    with open(output, 'w', encoding='utf-8') as stream: stream.write('%s\n%s\n%s\n' % (version, url, asset['sha256']))
except (OSError, KeyError, TypeError, ValueError, json.JSONDecodeError) as exc:
    print('manifest: %s' % exc, file=sys.stderr); raise SystemExit(1)
PY
[ -s "$TMP/metadata" ] || die 'invalid channel manifest'
RELEASE_VERSION=$(awk 'NR == 1 { print; exit }' "$TMP/metadata")
ARCHIVE_URL=$(awk 'NR == 2 { print; exit }' "$TMP/metadata")
EXPECTED=$(awk 'NR == 3 { print; exit }' "$TMP/metadata")
ARCHIVE_NAME="lazydb_${RELEASE_VERSION}_${TARGET}.tar.xz"
curl --fail --location --proto '=https' --tlsv1.2 --max-time 60 -o "$TMP/archive.tar.xz" "$ARCHIVE_URL" || die 'archive download failed'
ACTUAL=$($HASH "$TMP/archive.tar.xz" | awk '{print $1}')
[ "$ACTUAL" = "$EXPECTED" ] || die 'checksum mismatch'

mkdir -p "$TMP/unpack"
python3 - "$TMP/archive.tar.xz" "$TMP/unpack" "$ARCHIVE_NAME" <<'PY'
import sys, tarfile
archive, output, expected = sys.argv[1:]
with tarfile.open(archive, 'r:xz') as tar:
    members = tar.getmembers(); root = expected[:-7]; names = set()
    for member in members:
        name = member.name
        if name.startswith('/') or name in names or any(part in ('', '..') for part in name.split('/')) or (name != root and not name.startswith(root + '/')): raise SystemExit('unsafe archive entry: ' + name)
        if member.issym() or member.islnk() or not (member.isdir() or member.isreg()): raise SystemExit('unsupported archive entry: ' + name)
        names.add(name)
    binary = root + '/lazydb'
    if binary not in names or not any(tarinfo.name == binary and tarinfo.isreg() for tarinfo in members): raise SystemExit('archive does not contain the expected executable')
    tar.extractall(output)
PY
chmod 0755 "$TMP/unpack"/*/lazydb
STAGED="$TMP/release"; mkdir -p "$STAGED"; cp -R "$TMP/unpack"/*/. "$STAGED/"
"$STAGED/lazydb" version --json > "$TMP/version.json" || die 'staged binary failed version check'
python3 - "$TMP/version.json" "$RELEASE_VERSION" <<'PY'
import json,sys
data=json.load(open(sys.argv[1], encoding='utf-8'))
if data.get('version') != sys.argv[2]: raise SystemExit('binary reported version %r' % data.get('version'))
PY
DEST="$RELEASES/$RELEASE_VERSION"
if [ ! -e "$DEST" ]; then mkdir -p "$RELEASES"; mv "$STAGED" "$DEST"; fi
ln -sfn "$DEST" "$TMP/current.new"; mv -f "$TMP/current.new" "$DATA_HOME/current"
mkdir -p "$INSTALL_DIR"; ln -sfn "$DATA_HOME/current/lazydb" "$INSTALL_DIR/lazydb"
STATE="$DATA_HOME/install.json"
python3 - "$STATE" "$TMP/state" "$CHANNEL" "$RELEASE_VERSION" "$TARGET" "$INSTALL_DIR/lazydb" <<'PY'
import json, os, sys, tempfile
state = {'schema': 1, 'product': 'lazydb', 'manager': 'native', 'channel': sys.argv[3], 'version': sys.argv[4], 'target': sys.argv[5], 'path': sys.argv[6]}
fd, path = tempfile.mkstemp(prefix='.install.json.', dir=os.path.dirname(sys.argv[1]))
with os.fdopen(fd, 'w', encoding='utf-8') as stream: json.dump(state, stream, indent=2); stream.write('\n'); stream.flush(); os.fsync(stream.fileno())
os.replace(path, sys.argv[1])
PY
printf 'lazydb %s installed (%s)\n' "$RELEASE_VERSION" "$CHANNEL"
