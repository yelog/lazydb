#!/bin/sh
set -eu
[ "$#" -eq 2 ] || { printf '%s\n' 'usage: smoke-online-install.sh CHANNEL VERSION' >&2; exit 1; }
channel=$1
version=$2
case "$channel" in
    stable) entry=install.sh ;;
    beta) entry=install-beta.sh ;;
    *) printf '%s\n' 'invalid channel' >&2; exit 1 ;;
esac
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT HUP INT TERM
# Do not inherit a developer's installation paths or channel overrides.
export HOME="$TMP/home" LAZYDB_CONFIG_HOME="$TMP/config"
export LAZYDB_INSTALL_DIR="$TMP/bin" LAZYDB_CHANNEL="$channel"
export LAZYDB_CHANNEL_BASE_URL=https://lazydb.yelog.org/channels
unset LAZYDB_CHANNEL_LOCKED LAZYDB_VERSION
mkdir -p "$HOME"
# Pages/CDN propagation is asynchronous. Retry the normal public URL, not a
# cache-busted URL that could pass while the documented command still fails.
attempt=1
while [ "$attempt" -le 6 ]; do
    if curl -fsSL --proto '=https' --tlsv1.2 --max-time 60 \
        "https://lazydb.yelog.org/$entry" -o "$TMP/installer.sh" &&
        sh "$TMP/installer.sh" --version "$version" --install-dir "$LAZYDB_INSTALL_DIR" &&
        "$LAZYDB_INSTALL_DIR/lazydb" version --json > "$TMP/version.json" &&
        python3 - "$TMP/version.json" "$version" <<'PY'
import json, sys
with open(sys.argv[1], encoding='utf-8') as stream:
    actual = json.load(stream).get('version')
if actual != sys.argv[2]:
    raise SystemExit('installed version mismatch: %r' % actual)
PY
    then
        printf 'Online installation verified: %s %s\n' "$channel" "$version"
        exit 0
    fi
    [ "$attempt" -eq 6 ] || sleep 15
    attempt=$((attempt + 1))
done
printf 'Online installation failed: %s %s\n' "$channel" "$version" >&2
exit 1
