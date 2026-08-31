#!/bin/sh
printf '%s\n' 'LAZYDB BETA installer'
export LAZYDB_CHANNEL=beta
export LAZYDB_CHANNEL_LOCKED=beta
exec sh "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/install-core.sh" "$@"
