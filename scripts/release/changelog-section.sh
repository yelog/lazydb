#!/bin/sh
set -eu
[ "$#" -eq 1 ] || { printf 'Usage: changelog-section.sh VERSION\n' >&2; exit 2; }
version=$1
awk -v version="$version" '
    $0 ~ "^## \\[" version "\\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$" { found=1; print; next }
    found && /^## / { exit }
    found { print }
' CHANGELOG.md
