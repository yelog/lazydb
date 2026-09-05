#!/bin/sh
set -eu
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
for test in test-installer test-channel-manifest test-pages test-release-metadata test-online-install; do
    printf 'Running %s\n' "$test"
    sh "$SCRIPT_DIR/$test.sh"
done
