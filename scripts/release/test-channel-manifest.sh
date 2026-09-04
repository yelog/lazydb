#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
GENERATOR="$ROOT/scripts/release/generate-channel-manifest.py"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT HUP INT TERM

make_fixture() {
    dir=$1
    version=$2
    mkdir -p "$dir"
    for target in x86_64-apple-darwin aarch64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
        printf '%s\n' "$target" > "$dir/lazydb_${version}_${target}.tar.xz"
    done
    printf '%s\n' windows > "$dir/lazydb_${version}_x86_64-pc-windows-msvc.zip"
    (cd "$dir" && sha256sum lazydb_*.tar.xz lazydb_*.zip > SHA256SUMS)
}

assert_rejects() {
    if "$@" >/dev/null 2>&1; then
        printf 'expected rejection: %s\n' "$*" >&2
        exit 1
    fi
}

stable="$TMP/stable"
beta="$TMP/beta"
make_fixture "$stable" 1.2.3
make_fixture "$beta" 1.2.3-beta.2
python3 "$GENERATOR" stable 1.2.3 2026-08-31T12:00:00Z "$stable" "$TMP/stable.json"
python3 "$GENERATOR" beta 1.2.3-beta.2 2026-08-31T12:00:00Z "$beta" "$TMP/beta.json"
python3 - "$TMP/stable.json" "$TMP/beta.json" <<'PY'
import json
import sys

for path, channel, version in ((sys.argv[1], "stable", "1.2.3"), (sys.argv[2], "beta", "1.2.3-beta.2")):
    data = json.load(open(path, encoding="utf-8"))
    assert set(data) == {"schema", "product", "channel", "version", "tag", "prerelease", "published_at", "release_url", "assets"}
    assert data["schema"] == 1 and data["product"] == "lazydb"
    assert data["channel"] == channel and data["version"] == version
    assert data["tag"] == "v" + version and data["prerelease"] == (channel == "beta")
    assert data["release_url"].startswith("https://github.com/yelog/lazydb/")
    assert set(data["assets"]) == {"x86_64-apple-darwin", "aarch64-apple-darwin", "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "x86_64-pc-windows-msvc"}
    for asset in data["assets"].values():
        assert asset["url"].startswith("https://github.com/yelog/lazydb/releases/download/")
        assert "evil.example" not in asset["url"]
        assert len(asset["sha256"]) == 64
PY

assert_rejects python3 "$GENERATOR" stable 1.2.3-beta.2 now "$beta" "$TMP/no.json"
assert_rejects python3 "$GENERATOR" beta 1.2.3 now "$stable" "$TMP/no.json"
rm "$stable/lazydb_1.2.3_x86_64-apple-darwin.tar.xz"
assert_rejects python3 "$GENERATOR" stable 1.2.3 now "$stable" "$TMP/no.json"
make_fixture "$stable" 1.2.3
grep -v 'aarch64-unknown-linux-gnu' "$stable/SHA256SUMS" > "$TMP/missing"
mv "$TMP/missing" "$stable/SHA256SUMS"
assert_rejects python3 "$GENERATOR" stable 1.2.3 now "$stable" "$TMP/no.json"
make_fixture "$stable" 1.2.3
printf '%s\n' "$(sha256sum "$stable/lazydb_1.2.3_x86_64-apple-darwin.tar.xz" | awk '{print $1}') lazydb_1.2.3_x86_64-apple-darwin.tar.xz" >> "$stable/SHA256SUMS"
assert_rejects python3 "$GENERATOR" stable 1.2.3 now "$stable" "$TMP/no.json"
assert_rejects python3 "$GENERATOR" stable 1.2.3-beta.0 now "$stable" "$TMP/no.json"
printf '%s\n' 'channel manifest tests: ok'
