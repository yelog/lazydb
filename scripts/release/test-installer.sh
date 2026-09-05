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
for target in x86_64-apple-darwin aarch64-apple-darwin aarch64-unknown-linux-gnu; do
    cp "$TMP/server/assets/lazydb_1.2.3_x86_64-unknown-linux-gnu.tar.xz" "$TMP/server/assets/lazydb_1.2.3_$target.tar.xz"
done
printf '%s\n' windows > "$TMP/server/assets/lazydb_1.2.3_x86_64-pc-windows-msvc.zip"
(cd "$TMP/server/assets" && sha256sum lazydb_*.tar.xz lazydb_*.zip > SHA256SUMS)
python3 "$ROOT/scripts/release/generate-channel-manifest.py" stable 1.2.3 2026-09-05T00:00:00Z "$TMP/server/assets" "$TMP/server/channels/stable.json"
cat > "$TMP/assets/beta-lazydb" <<'SH'
#!/bin/sh
[ "${1:-}" = version ] && printf '%s\n' '{"version":"1.2.3-beta.1"}'
SH
chmod 755 "$TMP/assets/beta-lazydb"
mkdir -p "$TMP/package/lazydb_1.2.3-beta.1_x86_64-unknown-linux-gnu"
cp "$TMP/assets/beta-lazydb" "$TMP/package/lazydb_1.2.3-beta.1_x86_64-unknown-linux-gnu/lazydb"
(cd "$TMP/package" && COPYFILE_DISABLE=1 tar -cJf "$TMP/server/assets/lazydb_1.2.3-beta.1_x86_64-unknown-linux-gnu.tar.xz" lazydb_1.2.3-beta.1_x86_64-unknown-linux-gnu)
for target in x86_64-apple-darwin aarch64-apple-darwin aarch64-unknown-linux-gnu; do
    cp "$TMP/server/assets/lazydb_1.2.3-beta.1_x86_64-unknown-linux-gnu.tar.xz" "$TMP/server/assets/lazydb_1.2.3-beta.1_$target.tar.xz"
done
printf '%s\n' windows > "$TMP/server/assets/lazydb_1.2.3-beta.1_x86_64-pc-windows-msvc.zip"
(cd "$TMP/server/assets" && sha256sum lazydb_*.tar.xz lazydb_*.zip > SHA256SUMS)
python3 "$ROOT/scripts/release/generate-channel-manifest.py" beta 1.2.3-beta.1 2026-09-05T00:00:00Z "$TMP/server/assets" "$TMP/server/channels/beta.json"
export TMPDIR_TEST="$TMP" PATH="$TMP/bin:$PATH" LAZYDB_CHANNEL_BASE_URL=https://fixture/channels
export LAZYDB_CONFIG_HOME="$TMP/home/config"
# Consume the same five-target manifests as production, including Windows.
if ! HOME="$TMP/home" sh "$TMP/pages/install.sh" --install-dir "$TMP/install" >/dev/null; then
    printf '%s\n' 'installer fixture failed' >&2
    exit 1
fi
[ -L "$TMP/home/config/current" ]
[ -L "$TMP/install/lazydb" ]
[ -f "$TMP/home/config/install.json" ]
[ "$(python3 -c 'import json; print(json.load(open("'$TMP'/home/config/install.json"))["channel"])')" = stable ]
[ -e "$TMP/install/lazydb" ]
HOME="$TMP/home" sh "$TMP/pages/install.sh" --install-dir "$TMP/install" >/dev/null
[ -d "$TMP/home/config/releases/1.2.3" ]
if HOME="$TMP/home" sh "$TMP/pages/install.sh" --channel invalid --install-dir "$TMP/install" >/dev/null 2>&1; then
    printf '%s\n' 'invalid channel was accepted' >&2
    exit 1
fi
if HOME="$TMP/home" sh "$TMP/pages/install.sh" --channel beta --install-dir "$TMP/install" >/dev/null 2>&1; then
    printf '%s\n' 'stable entrypoint allowed beta channel' >&2
    exit 1
fi
beta_output=$(HOME="$TMP/beta-home" LAZYDB_CONFIG_HOME="$TMP/beta-home/config" sh "$TMP/pages/install-beta.sh" --install-dir "$TMP/beta-install")
case "$beta_output" in
    *'LAZYDB BETA installer'*'lazydb 1.2.3-beta.1 installed (beta)'*) ;;
    *) printf 'unexpected beta output: %s\n' "$beta_output" >&2; exit 1 ;;
esac
[ "$(python3 -c 'import json; print(json.load(open("'$TMP'/beta-home/config/install.json"))["channel"])')" = beta ]
HOME="$TMP/home" XDG_DATA_HOME="$TMP/root-data" sh "$ROOT/install.sh" --install-dir "$TMP/root-install" >/dev/null
for mutation in missing extra; do
    python3 - "$TMP/server/channels/stable.json" "$mutation" <<'PY'
import json, sys
path, mutation = sys.argv[1:]
with open(path) as stream:
    data = json.load(stream)
if mutation == 'missing':
    del data['assets']['x86_64-unknown-linux-gnu']
else:
    data['assets']['unsupported-target'] = next(iter(data['assets'].values()))
with open(path, 'w') as stream:
    json.dump(data, stream)
PY
    for installer in "$TMP/pages/install.sh" "$ROOT/install.sh"; do
        if HOME="$TMP/home" XDG_DATA_HOME="$TMP/root-data" sh "$installer" --install-dir "$TMP/install" >"$TMP/error" 2>&1; then
            printf 'installer accepted %s target set\n' "$mutation" >&2
            exit 1
        fi
        grep -q 'manifest target set mismatch' "$TMP/error"
    done
    python3 "$ROOT/scripts/release/generate-channel-manifest.py" stable 1.2.3 2026-09-05T00:00:00Z "$TMP/server/assets" "$TMP/server/channels/stable.json"
done
printf '%s\n' 'installer tests: ok'
