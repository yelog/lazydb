# SSH tunnel protocol fixture

This fixture verifies the transport assumptions behind LazyDB's managed SSH
tunnels without changing a database connection's logical hostname or relaxing
TLS verification.

Each run creates a fresh Compose project containing:

- an Alpine 3.22 OpenSSH server (`openssh-server=10.0_p1-r11`) and a separate
  jump service;
- PostgreSQL 17.6, MySQL 8.4.6, Valkey 8.1.10, an nginx HTTP endpoint, and a
  three-member MongoDB 7.0 `rs0` replica set with TLS and SCRAM-SHA-256 on
  distinct container aliases and ports;
- an optional, pinned SQL Server 2022 profile under the Compose `sqlserver`
  profile;
- a runtime CA, server certificates, a PostgreSQL client certificate, SSH host
  keys, client keys, passwords, and `known_hosts` files.

Only SSH ports are published, on ephemeral loopback addresses. Database and
HTTP service ports are not published. The script must never touch an unrelated
Compose project, container, volume, SSH agent, or user `known_hosts` file.

## Prerequisites

- Docker with Compose v2
- Docker support for `linux/amd64` containers when running the optional SQL
  Server profile on another architecture
- a local OpenSSH client and `ssh-keygen`
- Python 3.9 or newer
- `openssl` on `PATH`

The recorded run used macOS arm64, Docker 29.8.2, Compose 5.5.0, Python 3.9.6,
and `OpenSSH_10.5p1, LibreSSL 3.3.6` as the client.

## Run

Run the isolated protocol suites serially:

```sh
unset SSL_CERT_FILE
python3 scripts/ci/test-ssh-integration.py
```

The default run provisions the fixture, runs the ignored Cargo suites serially,
then runs the real PTY check. When the preceding stages pass, it runs the
Redis-compatible suite in a separate child process. The order is:

1. `ssh_transport`
2. `ssh_integration`
3. `ssh_http`
4. `ssh_mongo`
5. `ssh_services`, covering CLI, Agent, MCP, and LSP service processes
6. `ssh_sqlserver` only when `--with-sqlserver` is supplied
7. `scripts/ci/test-ssh-tui.py`, a real PTY check for a saved PostgreSQL profile
8. `ssh_redis` in a separate child process with `SSL_CERT_FILE` set to the
   fixture's Redis-compatible TLS CA

The PTY binary is selected from Cargo's resolved `target_directory`, not from a
hard-coded `target/` path. The runner uses `cargo metadata` by default;
`--cargo-target-dir` sets `CARGO_TARGET_DIR` for the Cargo tests, build, and PTY
child.

The Cargo and PTY subprocesses run with `SSL_CERT_FILE` removed from their
environment. For manual runs, keep it unset for every non-Redis suite; the SQLx
wrong-CA negative tests depend on that. If the fixture CA remains in the default
trust path, the negative test can accept that CA through the global store and no
longer prove that the per-connection CA was rejected. The runner sets
`SSL_CERT_FILE` only for the later `ssh_redis` child because Redis profiles have
no per-profile CA field.

Useful runner options:

```sh
python3 scripts/ci/test-ssh-integration.py --prepare-only
python3 scripts/ci/test-ssh-integration.py --keep
python3 scripts/ci/test-ssh-integration.py --keep-on-failure
python3 scripts/ci/test-ssh-integration.py --timeout 600
python3 scripts/ci/test-ssh-integration.py --cargo-target-dir /path/to/target
python3 scripts/ci/test-ssh-integration.py --with-sqlserver
```

- `--prepare-only` provisions the fixture, validates the raw forwarding path,
  writes the manifest and environment file, prints the follow-up commands, and
  exits without running Cargo tests. The fixture is retained.
- `--keep` retains a successfully prepared fixture after the test run, whether
  the tests pass or fail. A provisioning failure still needs
  `--keep-on-failure`.
- `--keep-on-failure` retains the fixture when provisioning or a test run
  fails. It does not retain a successful run.
- `--timeout` controls fixture provisioning waits. It is not the
  `cargo test` timeout.
- `--cargo-target-dir` sets `CARGO_TARGET_DIR` for Cargo tests, the binary
  build, and the PTY child. Without it, the runner uses the `target_directory`
  reported by `cargo metadata`.
- `--with-sqlserver` enables the Compose `sqlserver` profile, starts the pinned
  `linux/amd64` SQL Server 2022 image, and adds `ssh_sqlserver`. The database
  password is passed through a temporary file and environment variable; the
  database port is not published.

Without `--prepare-only`, `--keep`, or a matching failure retention option, the
script removes the fixture when it exits.

## Manual test runs

Provision once and keep the fixture:

```sh
unset SSL_CERT_FILE
python3 scripts/ci/test-ssh-integration.py --prepare-only
```

The output prints the manifest and environment paths. Source the generated
environment file, then run the non-Redis suites serially:

```sh
FIXTURE_ENV=/path/printed/by/prepare-only/fixture.env
source "$FIXTURE_ENV"
cargo test --locked \
  --test ssh_transport \
  --test ssh_integration \
  --test ssh_http \
  --test ssh_mongo \
  --test ssh_services \
  -- --ignored --nocapture --test-threads=1
```

To include SQL Server, provision with its optional profile and run its suite:

```sh
unset SSL_CERT_FILE
python3 scripts/ci/test-ssh-integration.py \
  --prepare-only \
  --with-sqlserver
FIXTURE_ENV=/path/printed/by/prepare-only/fixture.env
source "$FIXTURE_ENV"
cargo test --locked --test ssh_sqlserver -- \
  --ignored --nocapture --test-threads=1
```

Run the real PTY check against the kept fixture after building the binary:

```sh
cargo build --locked --bin lazydb
TARGET_DIR="$(cargo metadata --locked --no-deps --format-version=1 |
  python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"
python3 scripts/ci/test-ssh-tui.py \
  --manifest "$LAZYDB_TEST_SSH_FIXTURE_MANIFEST" \
  --env-file "$FIXTURE_ENV" \
  --binary "$TARGET_DIR/debug/lazydb"
```

Run the Redis-compatible suite separately with the fixture CA:

```sh
SSL_CERT_FILE="$LAZYDB_TEST_SSH_REDIS_TLS_CA_FILE" \
  cargo test --locked --test ssh_redis -- \
  --ignored --nocapture --test-threads=1
```

All generated paths are randomized. Use the paths printed by the current
`--prepare-only` run; do not infer ownership from a previous directory name.

`fixture.env` is mode 0600 and contains throwaway passwords. Source it only in
the test shell. Do not print it, paste it into logs, or enable shell tracing
while it is loaded.

## Cleanup

The manifest contains the validated cleanup command. To remove a retained
fixture:

```sh
python3 scripts/ci/test-ssh-integration.py \
  --cleanup-manifest "$LAZYDB_TEST_SSH_FIXTURE_MANIFEST"
```

Before removing anything, `--cleanup-manifest` validates the manifest schema,
Compose project name, Compose file, runtime directory, SSH endpoint, and Docker
labels. It stops only the generated Compose project, then removes its generated
volumes and private runtime directory.

Do not delete a fixture directory by name pattern. A running fixture may still
own a Compose project, and the manifest is the cleanup authorization record.

## Environment contract

The runner writes non-secret descriptions to `manifest.json` and exports these
runtime values:

| Variable | Meaning |
| --- | --- |
| `LAZYDB_TEST_SSH_FIXTURE_MANIFEST` | Path to the non-secret manifest |
| `LAZYDB_TEST_SSH_POSTGRES_PASSWORD` | Throwaway PostgreSQL password |
| `LAZYDB_TEST_SSH_MYSQL_PASSWORD` | Throwaway MySQL password |
| `LAZYDB_TEST_SSH_REDIS_PASSWORD` | Throwaway Valkey default-user password |
| `LAZYDB_TEST_SSH_MONGO_USERNAME` | Throwaway MongoDB SCRAM username |
| `LAZYDB_TEST_SSH_MONGO_PASSWORD` | Throwaway MongoDB SCRAM password |
| `LAZYDB_TEST_SSH_SQLSERVER_PASSWORD` | Throwaway SQL Server password for `--with-sqlserver` |
| `LAZYDB_REQUIRE_DATABASE_TESTS` | Set to `1`; prevents required fixture tests from being skipped |
| `LAZYDB_TEST_SSH_REDIS_URL` | Logical plain Redis URL using `redis.internal` |
| `LAZYDB_TEST_SSH_REDIS_TLS_URL` | Logical TLS Redis URL using `redis.internal` |
| `LAZYDB_TEST_SSH_REDIS_ROUTED_ADDR` | Loopback plain forwarding address |
| `LAZYDB_TEST_SSH_REDIS_TLS_ROUTED_ADDR` | Loopback TLS forwarding address |
| `LAZYDB_TEST_SSH_REDIS_DB` / `_DB2` | Available logical databases |
| `LAZYDB_TEST_SSH_REDIS_TLS_CA_FILE` | CA used by the managed Redis TLS test |

The manifest also contains:

- the SSH and jump endpoints;
- client, wrong-client, `known_hosts`, and wrong-host-key paths;
- PostgreSQL CA, wrong CA, client certificate, and client key paths;
- MySQL CA and wrong CA paths;
- ClickHouse HTTP, Elasticsearch HTTPS, and OpenSearch HTTPS URLs;
- MongoDB URL, database, CA, replica-set name, SCRAM environment names, and the
  three member host/port pairs used by the discovery and explicit-seed tests;
- SQL Server enablement, logical host, port, and password environment name;
- the cleanup command.

The runtime directory is mode 0700. Secret-bearing generated files are mode
0600; public certificates and keys intended for the containers are readable by
the fixture containers.

## Forwarding readiness

For Unix, TCP, and SOCKS forwarding, the client is accepted as ready only after
ordered evidence appears in the same managed SSH process log:

1. `debug1: Local forwarding listening on <exact requested endpoint>.`
2. `debug1: channel ... new unix-listener` for a Unix socket, or
   `debug1: channel ... new port-listener` for TCP/SOCKS.
3. `debug1: Entering interactive session.`

The earlier "Local forwarding listening" line alone is not sufficient. The test
also requires the exact requested path or port and verifies that the SSH child
is still alive before the database or Redis probe starts.

The fixture has exercised:

- Unix local stream forwarding to a remote TCP service;
- loopback TCP forwarding;
- OpenSSH dynamic SOCKS5 forwarding;
- failure on an occupied TCP port with `ExitOnForwardFailure=yes`;
- rejection of a wrong private key;
- rejection of a wrong host key.

## Recorded acceptance

The complete recorded Docker acceptance was run on macOS arm64 with the OpenSSH
client version above. It passed 47 Rust integration tests plus the real PTY
check:

| Area | Recorded result |
| --- | --- |
| OpenSSH process and forwarding | `tests/ssh_integration.rs` passed 6/6 for Unix, TCP, and SOCKS forwarding, an occupied port, a wrong private key, and a wrong host key. |
| PostgreSQL and MySQL managed transport | `tests/ssh_transport.rs` passed 18/18. PostgreSQL kept `postgres.internal` for `verify-full`, the fixture CA, and the client certificate; MySQL kept `mysql.internal` for strict identity verification. The suite also covered TLS negatives, required-TLS Unix forwarding, reuse, pool-saturation cancellation, transactions, database targets, an agent-only key, 20 concurrent waiters, and rejection of rewiring an old SQLx route to a replacement tunnel. |
| SQL Server | `tests/ssh_sqlserver.rs` passed 2/2 for pool reuse, transactions, database targets, and stale-endpoint rejection. The pinned image ran as `linux/amd64` under Docker emulation. |
| Redis-compatible managed routing | `tests/ssh_redis.rs` passed 6/6 for TLS, AUTH, SELECT, scan/delete, ProxyJump, connector shutdown, and tunnel-crash handling. |
| HTTP/HTTPS through SOCKS | `tests/ssh_http.rs` passed 5/5 for ClickHouse HTTP and Elasticsearch/OpenSearch HTTPS with the original endpoints, strict HTTPS checks, and a wrong CA. |
| MongoDB through SOCKS | `tests/ssh_mongo.rs` passed 6/6 against a three-member TLS/SCRAM-SHA-256 `rs0` replica set through managed SOCKS. |
| CLI, Agent, MCP, and LSP | `tests/ssh_services.rs` passed 4/4 for `connections test`, Agent query, MCP tool discovery/query, and LSP catalog/stdio shutdown. |
| Real TUI PTY | The saved SSH PostgreSQL profile executed SQL, persisted history, exited cleanly, and reaped its managed tunnel. |

The MongoDB 6/6 result covers three members with different hostnames and ports,
TLS with SCRAM-SHA-256 authentication, wrong-password rejection, discovery and
an explicit three-seed connection, a primary stepdown that discovers a new
primary without replaying an uncertain write, transaction read and abort, and a
tunnel crash with an active session. In the crash test the background client
terminates and a replacement listener on the released loopback port is
rejected.

The service 4/4 result is from subprocess protocol tests; the PTY result is
recorded separately. The MySQL positive CA proof configured SQLx's `ssl_ca`
directly. The current MySQL/MariaDB/TiDB profile schema has no CA or
client-certificate field, so this does not establish a per-profile custom-CA
workflow for those drivers.

The SQLx assertions use the repository-locked dependency graph. A standalone
upstream SQLx proof resolved newer Rustls transitives and demonstrated TLS 1.3;
the locked suite therefore asserts a TLS 1.x session rather than a specific
minor version.

Other ignored tests cover tunnel reuse and release, shutdown, additional wrong
credential cases, ProxyJump, SCAN/delete, wrong HTTP CA, and related failure
paths. They remain useful follow-up coverage, but are not additional platform
or topology claims beyond the results above.

## Explicitly unverified

The following must not be described as accepted until separately executed and
recorded:

- Linux and Windows native clients, including Windows OpenSSH variants, SQLx
  TLS restrictions, process-tree cleanup, and Job Object cleanup.
- SQL Server on a native Linux or Windows host. The recorded run used an
  emulated `linux/amd64` container on macOS arm64.
- HTTP IP-SAN and IPv6 endpoints.
- MongoDB SRV/TXT discovery, sharded clusters, and failover beyond the recorded
  three-member primary-stepdown scenario.
- CockroachDB, MariaDB, and TiDB through their shared SQLx transports.
- Broad ProxyJump and multi-hop process-tree coverage. The recorded Redis test
  uses one configured jump service, but this is not a complete platform matrix.
- Redis Cluster, Sentinel, and other multi-node Redis topologies.
- Any driver not represented by the recorded rows above. Oracle is not
  supported by the managed SSH implementation.

Unverified combinations remain implementation work and test coverage, not
documented acceptance.

## Reproducible commands

```sh
python3 -m py_compile scripts/ci/test-ssh-integration.py
LAZYDB_SSH_RUNTIME_DIR=/path/to/private/runtime \
  docker compose -p lazydb-ssh-example \
  -f docker-compose.ssh.yml config --quiet

unset SSL_CERT_FILE
python3 scripts/ci/test-ssh-integration.py
```

For a retained manual run, use the `--prepare-only` and per-suite commands in
[Manual test runs](#manual-test-runs), then remove it with
`--cleanup-manifest` and the manifest path printed by the runner.
