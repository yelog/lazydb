#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)

[ "$#" -eq 7 ] || {
    printf '%s\n' 'usage: assemble-pages.sh CHANNEL VERSION PUBLISHED_AT ASSET_DIR SOURCE_PAGES EXISTING_PAGES OUTPUT_DIR' >&2
    exit 1
}

channel=$1
version=$2
published_at=$3
assets=$4
source_pages=$5
existing_pages=$6
output=$7

case "$channel" in stable|beta) ;; *) printf 'pages: invalid channel: %s\n' "$channel" >&2; exit 1 ;; esac
[ -d "$source_pages" ] || { printf '%s\n' 'pages: source Pages directory is missing' >&2; exit 1; }
[ -d "$existing_pages" ] || { printf '%s\n' 'pages: existing Pages directory is missing' >&2; exit 1; }

"$SCRIPT_DIR/check-assets.sh" "$channel" "$version" "$assets"

for name in install.sh install-beta.sh install-core.sh CNAME; do
    [ -f "$source_pages/$name" ] || { printf 'pages: missing source file: %s\n' "$name" >&2; exit 1; }
done
[ "$(cat "$source_pages/CNAME")" = 'lazydb.yelog.org' ] || {
    printf '%s\n' 'pages: CNAME must be lazydb.yelog.org' >&2
    exit 1
}
opposite=stable
[ "$channel" = stable ] && opposite=beta
[ -f "$existing_pages/$opposite.json" ] || {
    printf 'pages: missing existing %s manifest\n' "$opposite" >&2
    exit 1
}

rm -rf "$output"
mkdir -p "$output/channels"
cp "$source_pages/install.sh" "$source_pages/install-beta.sh" "$source_pages/install-core.sh" "$output/"
cp "$source_pages/CNAME" "$output/CNAME"
chmod 0755 "$output/install.sh" "$output/install-beta.sh" "$output/install-core.sh"

python3 "$ROOT/scripts/release/generate-channel-manifest.py" \
    "$channel" "$version" "$published_at" "$assets" "$output/channels/$channel.json"
cp "$existing_pages/$opposite.json" "$output/channels/$opposite.json"

# The allow-list above is deliberate: release archives never become Pages files.
unexpected=$(find "$output" -type f ! -name 'install.sh' ! -name 'install-beta.sh' \
    ! -name 'install-core.sh' ! -name 'CNAME' ! -name 'stable.json' ! -name 'beta.json' -print)
[ -z "$unexpected" ] || { printf 'pages: unexpected output file: %s\n' "$unexpected" >&2; exit 1; }
