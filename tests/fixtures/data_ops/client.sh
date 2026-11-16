#!/bin/bash
set +x
set -euo pipefail

client_directory="${BASH_SOURCE[0]%/*}"
source "$client_directory/client-config.sh"
client="${BASH_SOURCE[0]##*/}"
case "$client" in
    pg_dump|pg_restore) container="$DATA_OPS_POSTGRES_CONTAINER" ;;
    mysqldump|mysql) container="$DATA_OPS_MYSQL_CONTAINER" ;;
    mariadb-dump|mariadb) container="$DATA_OPS_MARIADB_CONTAINER" ;;
    *) printf 'Unknown fixture client: %s\n' "$client" >&2; exit 2 ;;
esac

if [[ -f "$client_directory/fail-$client" ]]; then
    for argument in "$@"; do
        case "$argument" in
            --dbname=*|--database=*)
                expected_manifest="$(< "$client_directory/fail-$client")"
                if [[ ! -f "$expected_manifest" ]]; then
                    printf 'Completed backup manifest is missing before restore.\n' >&2
                    exit 87
                fi
                printf 'Injected fixture restore failure.\n' >&2
                exit 86
                ;;
        esac
    done
fi

arguments=(--host "$DATA_OPS_DOCKER_HOST" exec -i --user 0 --workdir "$PWD")
arguments+=(--env "HOME=$HOME" --env LC_ALL=C --env LANG=C)
for variable in PGHOST PGPORT PGDATABASE PGUSER PGPASSFILE PGSSLMODE PGSSLROOTCERT PGHOSTADDR; do
    if declare -p "$variable" >/dev/null 2>&1; then
        arguments+=(--env "$variable=${!variable}")
    fi
done
exec "$DATA_OPS_DOCKER" "${arguments[@]}" "$container" "$client" "$@"
