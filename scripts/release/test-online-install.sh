#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT HUP INT TERM
mkdir -p "$TMP/bin"
cat > "$TMP/bin/curl" <<'SH'
#!/bin/sh
set -eu
out=
url=
while [ "$#" -gt 0 ]; do
    case "$1" in
        -o) shift; out=$1 ;;
        https:*) url=$1 ;;
    esac
    shift
done
printf '%s\n' "$url" >> "$TEST_LOG"
cp "$TEST_INSTALLER" "$out"
SH
cat > "$TMP/bin/sleep" <<'SH'
#!/bin/sh
exit 0
SH
cat > "$TMP/installer.sh" <<'SH'
#!/bin/sh
set -eu
[ "$1" = --version ] && [ "$2" = "$TEST_VERSION" ]
[ "$3" = --install-dir ] && [ "$4" = "$LAZYDB_INSTALL_DIR" ]
[ "$LAZYDB_CHANNEL_BASE_URL" = https://lazydb.yelog.org/channels ]
[ -z "${LAZYDB_VERSION:-}" ] && [ -z "${LAZYDB_CHANNEL_LOCKED:-}" ]
printf '%s\n' "$LAZYDB_CONFIG_HOME" >> "$TEST_PATHS"
mkdir -p "$LAZYDB_INSTALL_DIR"
cat > "$LAZYDB_INSTALL_DIR/lazydb" <<'BIN'
#!/bin/sh
printf '{"version":"%s"}\n' "$TEST_ACTUAL"
BIN
chmod 755 "$LAZYDB_INSTALL_DIR/lazydb"
SH
chmod 755 "$TMP/bin/curl" "$TMP/bin/sleep"
export PATH="$TMP/bin:$PATH" TEST_INSTALLER="$TMP/installer.sh"
export TEST_LOG="$TMP/requests" TEST_PATHS="$TMP/paths"
export LAZYDB_VERSION=ignored LAZYDB_CHANNEL_LOCKED=ignored
for channel in stable beta; do
    TEST_VERSION=1.2.3
    entry=install.sh
    if [ "$channel" = beta ]; then TEST_VERSION=1.2.3-beta.1; entry=install-beta.sh; fi
    export TEST_VERSION TEST_ACTUAL="$TEST_VERSION"
    sh "$ROOT/scripts/release/smoke-online-install.sh" "$channel" "$TEST_VERSION"
    grep -Fx "https://lazydb.yelog.org/$entry" "$TEST_LOG" >/dev/null
done
export TEST_ACTUAL=0.0.0
if sh "$ROOT/scripts/release/smoke-online-install.sh" beta "$TEST_VERSION" >"$TMP/error" 2>&1; then
    printf '%s\n' 'online smoke test accepted the wrong version' >&2
    exit 1
fi
grep -q 'installed version mismatch' "$TMP/error"
[ "$(wc -l < "$TEST_LOG" | tr -d ' ')" = 8 ]
while IFS= read -r path; do
    [ ! -e "${path%/config}" ]
done < "$TEST_PATHS"
printf '%s\n' 'online installer tests: ok'
