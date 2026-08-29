#!/bin/sh
set -eu
[ "$#" -eq 2 ] || { printf 'Usage: smoke-artifact.sh BINARY VERSION\n' >&2; exit 2; }
