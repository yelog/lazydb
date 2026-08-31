#!/bin/sh
export LAZYDB_CHANNEL=stable
export LAZYDB_CHANNEL_LOCKED=stable
exec sh "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/install-core.sh" "$@"
