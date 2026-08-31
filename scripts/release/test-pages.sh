#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT HUP INT TERM

make_assets() {
    dir=$1
    version=$2
    mkdir -p "$dir"
    for target in x86_64-apple-darwin aarch64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
        printf '%s\n' "$target" > "$dir/lazydb_${version}_${target}.tar.xz"
    done
    (cd "$dir" && sha256sum lazydb_*.tar.xz > SHA256SUMS)
    if [ "$version" = 1.2.3 ]; then
        for name in lazydb_1.2.3_amd64.deb lazydb_1.2.3_arm64.deb lazydb-1.2.3-amd64.rpm lazydb-1.2.3-arm64.rpm lazydb-1.2.3-x86_64.pkg.tar.zst lazydb-1.2.3-aarch64.pkg.tar.zst; do
            printf '%s\n' package > "$dir/$name"
        done
        printf '%s\n' installer > "$dir/lazydb-installer.sh"
    fi
    printf '%s\n' '{}' > "$dir/lazydb-${version}-sbom.spdx.json"
}

mkdir -p "$TMP/existing" "$TMP/source" "$TMP/stable-assets" "$TMP/beta-assets"
cp "$ROOT/pages/install.sh" "$ROOT/pages/install-beta.sh" "$ROOT/pages/install-core.sh" "$ROOT/pages/CNAME" "$TMP/source/"
printf '%s\n' preserved > "$TMP/existing/beta.json"
make_assets "$TMP/stable-assets" 1.2.3
sh "$ROOT/scripts/release/assemble-pages.sh" stable 1.2.3 2026-08-31T12:00:00Z "$TMP/stable-assets" "$TMP/source" "$TMP/existing" "$TMP/output"
sh "$ROOT/scripts/release/assemble-pages.sh" stable 1.2.3 2026-08-31T12:00:00Z "$TMP/stable-assets" "$TMP/source" "$TMP/existing" "$TMP/output-again"
cmp "$TMP/output/channels/stable.json" "$TMP/output-again/channels/stable.json"
[ -f "$TMP/output/channels/stable.json" ]
[ "$(cat "$TMP/output/channels/beta.json")" = preserved ]
[ "$(cat "$TMP/output/CNAME")" = lazydb.yelog.org ]
[ -x "$TMP/output/install.sh" ] && [ -x "$TMP/output/install-beta.sh" ] && [ -x "$TMP/output/install-core.sh" ]
[ "$(find "$TMP/output" -type f | sort | sed "s#^$TMP/output/##")" = "channels/beta.json
channels/stable.json
CNAME
install-beta.sh
install-core.sh
install.sh" ]
if find "$TMP/output" -type f -name '*.tar.xz' | grep . >/dev/null 2>&1; then exit 1; fi

cp "$TMP/output/channels/stable.json" "$TMP/existing/stable.json"
make_assets "$TMP/beta-assets" 1.2.3-beta.1
sh "$ROOT/scripts/release/assemble-pages.sh" beta 1.2.3-beta.1 2026-08-31T12:00:00Z "$TMP/beta-assets" "$TMP/source" "$TMP/existing" "$TMP/output-beta"
[ -f "$TMP/output-beta/channels/beta.json" ]
[ "$(cat "$TMP/output-beta/channels/stable.json")" = "$(cat "$TMP/existing/stable.json")" ]
[ "$(cat "$TMP/output-beta/CNAME")" = lazydb.yelog.org ]
if find "$TMP/output-beta" -type f \( -name '*.tar.xz' -o -name '*.deb' -o -name '*.rpm' \) | grep . >/dev/null 2>&1; then exit 1; fi
printf '%s\n' 'Pages tests: ok'
