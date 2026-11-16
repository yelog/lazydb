#!/bin/bash
set +x
set -euo pipefail
umask 077

if [[ "${1:-}" == "--help" ]]; then
    printf 'Usage: bash tests/fixtures/data_ops/run.sh [postgres] [mysql] [mariadb]\n'
    printf 'Defaults to all three cached server images; removes its own fixture on exit.\n'
    exit 0
fi

engines=("$@")
if [[ ${#engines[@]} -eq 0 ]]; then
    engines=(postgres mysql mariadb)
fi
selected=" "
for engine in "${engines[@]}"; do
    case "$engine" in
        postgres|mysql|mariadb) ;;
        *) printf 'Unknown engine: %s\n' "$engine" >&2; exit 2 ;;
    esac
    if [[ "$selected" == *" $engine "* ]]; then
        printf 'Duplicate engine: %s\n' "$engine" >&2
        exit 2
    fi
    selected="$selected$engine "
done

fixture_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repository="$(cd "$fixture_directory/../../.." && pwd -P)"
docker_binary="$(command -v docker)"
command -v python3 >/dev/null
command -v cargo >/dev/null
docker_host="${DOCKER_HOST:-$("$docker_binary" context inspect --format '{{.Endpoints.docker.Host}}')}"
case "$docker_host" in
    unix://*) ;;
    *) printf 'This same-path mount fixture requires a local Unix Docker endpoint.\n' >&2; exit 2 ;;
esac
docker_command=("$docker_binary" --host "$docker_host")
"${docker_command[@]}" info --format '{{.ServerVersion}}' >/dev/null
for engine in "${engines[@]}"; do
    case "$engine" in
        postgres) image=postgres:16 ;;
        mysql) image=mysql:8.4 ;;
        mariadb) image=mariadb:11.4 ;;
    esac
    "${docker_command[@]}" image inspect "$image" >/dev/null
done

fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/lazydb-data-ops.XXXXXXXX")"
fixture_root="$(cd "$fixture_root" && pwd -P)"
fixture_id="$(python3 -c 'import uuid; print(uuid.uuid4().hex)')"
printf '%s\n' "$fixture_id" > "$fixture_root/fixture-id"
containers=()
cleanup() {
    result=$?
    trap - EXIT INT TERM
    for container_id in "${containers[@]-}"; do
        [[ -n "$container_id" ]] || continue
        owner="$("${docker_command[@]}" inspect --format '{{index .Config.Labels "lazydb.data-ops.fixture"}}' "$container_id" 2>/dev/null || true)"
        if [[ "$owner" == "$fixture_id" ]]; then
            if ! "${docker_command[@]}" rm --force --volumes "$container_id" >/dev/null; then
                printf 'Could not remove owned fixture container %s\n' "$container_id" >&2
                result=1
            fi
        fi
    done
    if [[ -f "$fixture_root/fixture-id" && "$(< "$fixture_root/fixture-id")" == "$fixture_id" ]]; then
        rm -rf -- "$fixture_root"
    fi
    printf 'Cleaned isolated data-ops fixture %s\n' "$fixture_id"
    exit "$result"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

export LAZYDB_DATA_OPS_FIXTURE_ROOT="$fixture_root"
export TMPDIR="$fixture_root"
unset LAZYDB_DATA_OPS_POSTGRES_URL LAZYDB_DATA_OPS_POSTGRES_PASSWORD
unset LAZYDB_DATA_OPS_MYSQL_URL LAZYDB_DATA_OPS_MYSQL_PASSWORD
unset LAZYDB_DATA_OPS_MARIADB_URL LAZYDB_DATA_OPS_MARIADB_PASSWORD
mkdir "$fixture_root/bin"
config="$fixture_root/bin/client-config.sh"
printf 'DATA_OPS_DOCKER=%q\nDATA_OPS_DOCKER_HOST=%q\n' "$docker_binary" "$docker_host" > "$config"
for engine in postgres mysql mariadb; do
    printf 'DATA_OPS_%s_CONTAINER=%q\n' "$(printf '%s' "$engine" | tr '[:lower:]' '[:upper:]')" \
        "lazydb-data-ops-$fixture_id-$engine" >> "$config"
done
for client in pg_dump pg_restore mysqldump mysql mariadb-dump mariadb; do
    cp "$fixture_directory/client.sh" "$fixture_root/bin/$client"
    chmod 700 "$fixture_root/bin/$client"
done

ports=($(python3 -c 'import socket
sockets = [socket.socket() for _ in range(3)]
for listener in sockets:
    listener.bind(("127.0.0.1", 0))
print(" ".join(str(listener.getsockname()[1]) for listener in sockets))
for listener in sockets:
    listener.close()'))
index=0
for engine in "${engines[@]}"; do
    port="${ports[$index]}"
    index=$((index + 1))
    printf '%s\n' "$port" > "$fixture_root/$engine.port"
    password="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
    database="ldops_${fixture_id}_source"
    container_name="lazydb-data-ops-$fixture_id-$engine"
    environment_file="$fixture_root/$engine.env"
    case "$engine" in
        postgres)
            image=postgres:16
            printf 'POSTGRES_USER=lazydb_fixture\nPOSTGRES_DB=%s\nPOSTGRES_PASSWORD=%s\n' \
                "$database" "$password" > "$environment_file"
            server=(postgres -p "$port" -c "listen_addresses=*")
            export LAZYDB_DATA_OPS_POSTGRES_URL="postgresql://lazydb_fixture@127.0.0.1:$port/$database"
            export LAZYDB_DATA_OPS_POSTGRES_PASSWORD="$password"
            export LAZYDB_DATA_OPS_POSTGRES_DUMP="$fixture_root/bin/pg_dump"
            export LAZYDB_DATA_OPS_POSTGRES_RESTORE="$fixture_root/bin/pg_restore"
            ;;
        mysql)
            image=mysql:8.4
            printf 'MYSQL_ROOT_HOST=%%\nMYSQL_DATABASE=%s\nMYSQL_ROOT_PASSWORD=%s\n' \
                "$database" "$password" > "$environment_file"
            server=(mysqld "--port=$port" --bind-address=0.0.0.0 --event-scheduler=OFF)
            export LAZYDB_DATA_OPS_MYSQL_URL="mysql://root@127.0.0.1:$port/$database"
            export LAZYDB_DATA_OPS_MYSQL_PASSWORD="$password"
            export LAZYDB_DATA_OPS_MYSQL_DUMP="$fixture_root/bin/mysqldump"
            export LAZYDB_DATA_OPS_MYSQL_RESTORE="$fixture_root/bin/mysql"
            ;;
        mariadb)
            image=mariadb:11.4
            printf 'MARIADB_ROOT_HOST=%%\nMARIADB_DATABASE=%s\nMARIADB_ROOT_PASSWORD=%s\n' \
                "$database" "$password" > "$environment_file"
            server=(mariadbd "--port=$port" --bind-address=0.0.0.0 --event-scheduler=OFF)
            export LAZYDB_DATA_OPS_MARIADB_URL="mariadb://root@127.0.0.1:$port/$database"
            export LAZYDB_DATA_OPS_MARIADB_PASSWORD="$password"
            export LAZYDB_DATA_OPS_MARIADB_DUMP="$fixture_root/bin/mariadb-dump"
            export LAZYDB_DATA_OPS_MARIADB_RESTORE="$fixture_root/bin/mariadb"
            ;;
    esac
    container_id="$("${docker_command[@]}" create --pull=never \
        --name "$container_name" --label "lazydb.data-ops.fixture=$fixture_id" \
        --mount "type=bind,source=$fixture_root,target=$fixture_root" \
        --publish "127.0.0.1:$port:$port" --env-file "$environment_file" \
        "$image" "${server[@]}")"
    containers+=("$container_id")
    "${docker_command[@]}" start "$container_id" >/dev/null
    ready=false
    for attempt in {1..90}; do
        if [[ "$engine" == postgres ]]; then
            if "${docker_command[@]}" exec "$container_id" pg_isready \
                -h 127.0.0.1 -p "$port" -U lazydb_fixture -d "$database" >/dev/null 2>&1; then
                ready=true
                break
            fi
        elif "${docker_command[@]}" exec "$container_id" sh -c \
            'export MYSQL_PWD="${MYSQL_ROOT_PASSWORD:-${MARIADB_ROOT_PASSWORD:-}}"; exec "$1" --protocol=TCP --host=127.0.0.1 --port="$2" --user=root --batch --skip-column-names --execute="SELECT 1"' \
            sh "$engine" "$port" >/dev/null 2>&1; then
            ready=true
            break
        fi
        state="$("${docker_command[@]}" inspect --format '{{.State.Status}}' "$container_id")"
        if [[ "$state" == exited || "$state" == dead ]]; then
            printf '%s fixture container exited during startup.\n' "$engine" >&2
            exit 1
        fi
        sleep 1
    done
    if [[ "$ready" != true ]]; then
        printf '%s fixture did not become ready on port %s.\n' "$engine" "$port" >&2
        exit 1
    fi
    printf 'Ready: %s (%s), loopback/internal port %s\n' "$engine" "$image" "$port"
done

printf 'Running isolated native acceptance; fixture root: %s\n' "$fixture_root"
cd "$repository"
cargo test --locked --test data_ops_servers -- --nocapture --test-threads=1
