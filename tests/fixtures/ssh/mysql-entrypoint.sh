#!/bin/bash
set -euo pipefail

install -d -m 0755 -o mysql -g mysql /run/lazydb-tls
install -m 0600 -o mysql -g mysql \
    /run/lazydb-tls-src/mysql.key /run/lazydb-tls/mysql.key
install -m 0644 -o mysql -g mysql \
    /run/lazydb-tls-src/mysql.crt /run/lazydb-tls/mysql.crt
install -m 0644 -o mysql -g mysql \
    /run/lazydb-tls-src/ca.pem /run/lazydb-tls/ca.pem

exec /usr/local/bin/docker-entrypoint.sh "$@"
