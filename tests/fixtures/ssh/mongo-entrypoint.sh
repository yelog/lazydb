#!/bin/bash
set -euo pipefail

member="${MONGO_MEMBER:?MONGO_MEMBER is required}"
runtime_dir=/run/lazydb-mongo-runtime

install -d -m 0700 -o mongodb -g mongodb "$runtime_dir"
install -m 0400 -o mongodb -g mongodb \
    /run/lazydb-mongo/ca.pem "$runtime_dir/ca.pem"
install -m 0400 -o mongodb -g mongodb \
    "/run/lazydb-mongo/${member}.pem" "$runtime_dir/server.pem"
install -m 0400 -o mongodb -g mongodb \
    /run/lazydb-mongo/keyfile "$runtime_dir/keyfile"

exec gosu mongodb "$@"
