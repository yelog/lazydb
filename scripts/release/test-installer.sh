#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT HUP INT TERM

mkdir -p "$TMP/bin" "$TMP/server/channels" "$TMP/server/assets" "$TMP/home" "$TMP/install" "$TMP/beta-home" "$TMP/beta-install" "$TMP/pages"
cp "$ROOT/pages/install.sh" "$ROOT/pages/install-beta.sh" "$ROOT/pages/install-core.sh" "$TMP/pages/"
cat > "$TMP/bin/curl" <<'SH'
#!/bin/sh
set -eu
out=
url=
while [ "$#" -gt 0 ]; do
    case "$1" in
        -o) shift; out=$1 ;;
        http*) url=$1 ;;
    esac
    shift
done
case "$url" in
  *channels/*) src="$TMPDIR_TEST/server/${url##*/}"; src="$TMPDIR_TEST/server/channels/${url##*/}" ;;
  *releases/download/*) src="$TMPDIR_TEST/server/assets/${url##*/}" ;;
  *) exit 1 ;;
esac
cp "$src" "$out"
SH
chmod 755 "$TMP/bin/curl"
cat > "$TMP/bin/uname" <<'SH'
#!/bin/sh
case "${1:-}" in -s) printf '%s\n' Linux ;; -m) printf '%s\n' x86_64 ;; esac
SH
chmod 755 "$TMP/bin/uname"
# Use deterministic fake binaries so the test does not require a release build.
mkdir -p "$TMP/assets/stable" "$TMP/assets/beta"
cat > "$TMP/assets/lazydb" <<'SH'
#!/bin/sh
[ "${1:-}" = version ] && printf '%s\n' '{"version":"1.2.3"}'
SH
chmod 755 "$TMP/assets/lazydb"
mkdir -p "$TMP/package/lazydb_1.2.3_x86_64-unknown-linux-gnu"
cp "$TMP/assets/lazydb" "$TMP/package/lazydb_1.2.3_x86_64-unknown-linux-gnu/lazydb"
(cd "$TMP/package" && COPYFILE_DISABLE=1 tar -cJf "$TMP/server/assets/lazydb_1.2.3_x86_64-unknown-linux-gnu.tar.xz" lazydb_1.2.3_x86_64-unknown-linux-gnu)
digest=$(sha256sum "$TMP/server/assets/lazydb_1.2.3_x86_64-unknown-linux-gnu.tar.xz" | awk '{print $1}')
for target in x86_64-apple-darwin aarch64-apple-darwin aarch64-unknown-linux-gnu; do
    cp "$TMP/server/assets/lazydb_1.2.3_x86_64-unknown-linux-gnu.tar.xz" "$TMP/server/assets/lazydb_1.2.3_$target.tar.xz"
done
python3 - "$TMP/server/channels/stable.json" "$digest" <<'PY'
import json, sys
targets = ('x86_64-apple-darwin', 'aarch64-apple-darwin', 'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu')
assets = {target: {'url': 'https://github.com/yelog/lazydb/releases/download/v1.2.3/lazydb_1.2.3_%s.tar.xz' % target, 'sha256': sys.argv[2]} for target in targets}
json.dump({'schema': 1, 'product': 'lazydb', 'channel': 'stable', 'version': '1.2.3', 'tag': 'v1.2.3', 'prerelease': False, 'published_at': 'now', 'release_url': 'https://github.com/yelog/lazydb/releases/tag/v1.2.3', 'assets': assets}, open(sys.argv[1], 'w'))
PY
cat > "$TMP/assets/beta-lazydb" <<'SH'
#!/bin/sh
[ "${1:-}" = version ] && printf '%s\n' '{"version":"1.2.3-beta.1"}'
SH
chmod 755 "$TMP/assets/beta-lazydb"
mkdir -p "$TMP/package/lazydb_1.2.3-beta.1_x86_64-unknown-linux-gnu"
cp "$TMP/assets/beta-lazydb" "$TMP/package/lazydb_1.2.3-beta.1_x86_64-unknown-linux-gnu/lazydb"
(cd "$TMP/package" && COPYFILE_DISABLE=1 tar -cJf "$TMP/server/assets/lazydb_1.2.3-beta.1_x86_64-unknown-linux-gnu.tar.xz" lazydb_1.2.3-beta.1_x86_64-unknown-linux-gnu)
beta_digest=$(sha256sum "$TMP/server/assets/lazydb_1.2.3-beta.1_x86_64-unknown-linux-gnu.tar.xz" | awk '{print $1}')
for target in x86_64-apple-darwin aarch64-apple-darwin aarch64-unknown-linux-gnu; do
    cp "$TMP/server/assets/lazydb_1.2.3-beta.1_x86_64-unknown-linux-gnu.tar.xz" "$TMP/server/assets/lazydb_1.2.3-beta.1_$target.tar.xz"
done
python3 - "$TMP/server/channels/beta.json" "$beta_digest" <<'PY'
import json, sys
targets = ('x86_64-apple-darwin', 'aarch64-apple-darwin', 'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu')
assets = {target: {'url': 'https://github.com/yelog/lazydb/releases/download/v1.2.3-beta.1/lazydb_1.2.3-beta.1_%s.tar.xz' % target, 'sha256': sys.argv[2]} for target in targets}
json.dump({'schema': 1, 'product': 'lazydb', 'channel': 'beta', 'version': '1.2.3-beta.1', 'tag': 'v1.2.3-beta.1', 'prerelease': True, 'published_at': 'now', 'release_url': 'https://github.com/yelog/lazydb/releases/tag/v1.2.3-beta.1', 'assets': assets}, open(sys.argv[1], 'w'))
PY
export TMPDIR_TEST="$TMP" PATH="$TMP/bin:$PATH" LAZYDB_CHANNEL_BASE_URL=https://fixture/channels
# The fixture contains the host target; production manifests contain all four.
if ! HOME="$TMP/home" XDG_DATA_HOME="$TMP/home/data" sh "$TMP/pages/install.sh" --install-dir "$TMP/install" >/dev/null; then
    printf '%s\n' 'installer fixture failed' >&2
    exit 1
fi
[ -L "$TMP/home/data/lazydb/current" ]
[ -L "$TMP/install/lazydb" ]
[ -f "$TMP/home/data/lazydb/install.json" ]
[ "$(python3 -c 'import json; print(json.load(open("'$TMP'/home/data/lazydb/install.json"))["channel"])')" = stable ]
[ -e "$TMP/install/lazydb" ]
HOME="$TMP/home" XDG_DATA_HOME="$TMP/home/data" sh "$TMP/pages/install.sh" --install-dir "$TMP/install" >/dev/null
[ -d "$TMP/home/data/lazydb/releases/1.2.3" ]
if HOME="$TMP/home" XDG_DATA_HOME="$TMP/home/data" sh "$TMP/pages/install.sh" --channel invalid --install-dir "$TMP/install" >/dev/null 2>&1; then
    printf '%s\n' 'invalid channel was accepted' >&2
    exit 1
fi
if HOME="$TMP/home" XDG_DATA_HOME="$TMP/home/data" sh "$TMP/pages/install.sh" --channel beta --install-dir "$TMP/install" >/dev/null 2>&1; then
    printf '%s\n' 'stable entrypoint allowed beta channel' >&2
    exit 1
fi
beta_output=$(HOME="$TMP/beta-home" XDG_DATA_HOME="$TMP/beta-home/data" sh "$TMP/pages/install-beta.sh" --install-dir "$TMP/beta-install")
case "$beta_output" in
    *'LAZYDB BETA installer'*'lazydb 1.2.3-beta.1 installed (beta)'*) ;;
    *) printf 'unexpected beta output: %s\n' "$beta_output" >&2; exit 1 ;;
esac
[ "$(python3 -c 'import json; print(json.load(open("'$TMP'/beta-home/data/lazydb/install.json"))["channel"])')" = beta ]
printf '%s\n' 'installer tests: ok'
