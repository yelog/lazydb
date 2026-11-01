# CockroachDB

LazyDB uses SQLx's PostgreSQL wire transport with an independent CockroachDB
adapter. The pinned integration fixture targets CockroachDB v25.4.6.

## Connections

Choose **CockroachDB** in the connection form. The default SQL port is `26257`,
the default database is `defaultdb`, and the default schema is `public`.

Use the explicit URL format to preserve the driver identity:

```text
cockroachdb://user@host:26257/defaultdb?sslmode=verify-full
```

PostgreSQL URL formats (`postgres://`, `postgresql://`, and
`jdbc:postgresql://`) are also accepted when the CockroachDB driver is selected.
Pasting them into a CockroachDB form keeps that driver and its default port.
For non-interactive import, choose the driver explicitly:

```sh
lazydb connections add --name cockroach --driver cockroachdb \
  --url 'postgresql://user@host:26257/defaultdb?sslmode=verify-full'
```

Without an explicit driver, a PostgreSQL URL continues to create a PostgreSQL
profile. Use `cockroachdb://` for an ad-hoc `lazydb --url` connection.

TLS URL parameters `sslrootcert`, `sslcert`, and `sslkey` store certificate file
paths. Certificate and private-key contents are not copied into profiles.
`options` preserves PostgreSQL startup options, including routing options when
required by a deployment. Percent-encode query values containing spaces, `&`,
`+`, or `=`. The shared optional profile field is:

```toml
[pg_options]
ssl_root_cert = "/path/to/ca.crt"
ssl_client_cert = "/path/to/client.crt"
ssl_client_key = "/path/to/client.key"
```

Use `sslmode=verify-full` for secure deployments. Explicit root certificate
files must contain at least one PEM certificate; unreadable or malformed files
are rejected when certificate verification is enabled. SQLx adds these CA
certificates to the system trust store, so `sslrootcert` does not restrict trust
to that file alone. Hostname verification still applies with `verify-full`.
The local fixture below uses explicit `sslmode=disable`. Exported connection
URLs omit passwords.

## Supported operations

- SQL execution and PostgreSQL-based syntax highlighting/completion.
- Database and schema selection; table and view discovery with lazy pages.
- Column, primary-key and index metadata; scoped catalog search.
- Filtered, sorted and paged relation previews without PostgreSQL `xmin`.
- Native table/view DDL using `SHOW CREATE`.
- AUTO/MANUAL transactions and existing MCP query/catalog tools.

Structured schema editing, relation-grid insert/update/delete, principal
management and monitoring are unavailable in this first version. CockroachDB
specific syntax may execute even when the PostgreSQL-based parser cannot
provide completion or diagnostics for it; MCP write policy still applies.

CockroachDB transaction retry errors (`40001`) require `ROLLBACK` and a manual
restart of the whole transaction. LazyDB never automatically replays interactive
SQL. A rejected commit retains its session only for rollback and blocks further
SQL until the transaction is rolled back. A network error during commit or
SQLSTATE `40003` (`statement_completion_unknown`) leaves its outcome unknown;
inspect the database before issuing the operation again. Cancelling a manual
transaction drops its dedicated socket immediately, without waiting for a TLS
shutdown handshake, so the session cannot return to the pool with pending work.
This also applies to manual pagination and its row-count query.

AUTO query cancellation discards the executing pooled session and sends a bounded
server cancellation request through a separate connection. Cancellation can race
a completed commit; it does not undo writes that have already committed.
DDL in multi-statement transactions follows CockroachDB's online schema-change
rules; LazyDB does not make such changes atomic.

## Cloud validation

On 2026-11-01, all five adapter integration tests passed against CockroachDB
Cloud Standard v26.2.9 using password authentication over `verify-full` TLS.
These tests cover catalog discovery, metadata, native DDL, stable identities,
previews, value decoding, read-only enforcement, AgentService, query budgets,
manual commit/rollback, real concurrent serialization failure (`40001`), and
cancellation with rollback of uncommitted writes. The retry test uses two real
sessions and does not require access to internal retry-injection functions.

Saved encrypted profiles reconnect successfully. Separate connection checks
confirmed rejection of missing/malformed CA files and hostname mismatches.
Client-certificate authentication has not been validated by this run.

## Local acceptance and commit faults

The v25.4.6 local fixture was also driven through the real TUI using an isolated
application home and a pseudo-terminal. Acceptance covers profile creation,
connection testing and editing, persisted-profile reconnect, catalog search,
native DDL, filtering/sorting/pagination, disabled grid writes, database/schema
target selection, and manual transaction commit/rollback and serialization
failure handling. Cancelling a running manual query through `Ctrl-c` and its
confirmation rolls back earlier uncommitted writes, returns the console to
MANUAL idle, and allows a fresh query to succeed. Selected visible databases are
offered by the workspace database selector before their catalogs are loaded;
switching databases refreshes the Explorer without rebinding existing consoles.

`cockroach_commit_faults` uses a local PostgreSQL wire relay to disconnect before
COMMIT is forwarded and after the server has confirmed COMMIT but before the
client receives its response. Independent connections verify the different
server outcomes while the client reports unknown outcome in both cases, closes
the affected session, and never replays SQL. Ordinary competing transactions
also verify real commit rejection (`40001`) and rollback-only runtime behavior.
The relay suite requires a password-free loopback URL with `sslmode=disable`;
it is separate from Cloud TLS validation.

## Local integration tests

The fixture keeps its data in memory, exposes only its SQL port on localhost,
and uses a dedicated Compose project. Starting it does not affect other
database containers.

```sh
docker compose -p lazydb-cockroach-tests -f docker-compose.cockroachdb.yml up -d --wait

LAZYDB_TEST_COCKROACHDB_URL='cockroachdb://root@127.0.0.1:26257/defaultdb?sslmode=disable' \
LAZYDB_REQUIRE_DATABASE_TESTS=1 \
cargo test --locked --test cockroach_adapter --test cockroach_commit_faults --test cockroach_profile -- --nocapture --test-threads=1

docker compose -p lazydb-cockroach-tests -f docker-compose.cockroachdb.yml down
```

Set `COCKROACHDB_HOST_PORT` when port `26257` is already occupied and update the
test URL accordingly. Tests create unique schemas and clean them up on success.
The required-test flag makes a missing test URL fail instead of skipping live
checks. The fixture uses a separate SQL listener inside the container because
this server's insecure RPC listener is restricted to loopback; the host still
uses the standard SQL port.
