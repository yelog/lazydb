#!/bin/sh
set -eu

[ "$#" -eq 1 ] || {
    printf 'Usage: check-binary-size.sh BINARY\n' >&2
    exit 2
}

binary=$1
[ -f "$binary" ] || {
    printf 'binary not found: %s\n' "$binary" >&2
    exit 1
}

size_bytes() {
    wc -c < "$1" | tr -d '[:space:]'
}

printf 'binary=%s raw_bytes=%s\n' "$binary" "$(size_bytes "$binary")"

compressed=$(mktemp "${TMPDIR:-/tmp}/lazydb-binary.XXXXXX.xz")
trap 'rm -f "$compressed"' EXIT HUP INT TERM
xz -9 -c "$binary" > "$compressed"
printf 'binary=%s xz9_bytes=%s\n' "$binary" "$(size_bytes "$compressed")"

case "$(file -b "$binary" 2>/dev/null || true)" in
    *Mach-O*)
        if command -v otool >/dev/null 2>&1; then
            printf '%s\n' 'dynamic_dependencies=otool -L'
            otool -L "$binary" || true
        else
            printf '%s\n' 'dynamic_dependencies=otool unavailable'
        fi
        ;;
    *ELF*)
        if command -v ldd >/dev/null 2>&1; then
            printf '%s\n' 'dynamic_dependencies=ldd'
            ldd "$binary" || true
        else
            printf '%s\n' 'dynamic_dependencies=ldd unavailable'
        fi
        ;;
    *)
        printf '%s\n' 'dynamic_dependencies=unrecognized binary format'
        ;;
esac
