#!/bin/bash
set -euo pipefail

install -d -m 0755 -o postgres -g postgres /run/lazydb-tls
install -m 0600 -o postgres -g postgres \
    /run/lazydb-tls-src/postgres.key /run/lazydb-tls/postgres.key
install -m 0644 -o postgres -g postgres \
    /run/lazydb-tls-src/postgres.crt /run/lazydb-tls/postgres.crt
install -m 0644 -o postgres -g postgres \
    /run/lazydb-tls-src/ca.pem /run/lazydb-tls/ca.pem
install -m 0644 -o postgres -g postgres \
    /run/lazydb-tls-src/pg_hba.conf /run/lazydb-tls/pg_hba.conf

exec /usr/local/bin/docker-entrypoint.sh "$@"
