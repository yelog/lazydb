# TiDB

TiDB is a separate relational driver using the MySQL protocol and SQL dialect.
Select **TiDB** in the Relational connection category, or import a native URL:

```text
tidb://root@127.0.0.1:4000/test
tidb://cloud.user@cluster.example:4000/app?sslMode=verify-full
```

The default port is `4000` and the default user is `root`. Database selection also
selects the schema; there is no separate schema field. Credentials use the same
secret-storage policy as other profiles and are omitted from exported URLs.
A MySQL-shaped endpoint should be entered with the `tidb://` scheme to preserve
TiDB product identity. TLS options use the existing MySQL spelling, including
`sslMode=disable`, `require`, `verify-ca`, and `verify-full` (also
`VERIFY_IDENTITY`). Cloud connections should use verified TLS.

## Supported workflows

The initial adapter targets stable TiDB **8.5.0 or newer**, using TiDB **8.5.3**
as its fixed test baseline. The TiDB product suffix in `VERSION()` determines
compatibility; `8.0.11-TiDB-v8.5.3` is TiDB 8.5.3, not MySQL 8.0.11. Profiles
selecting a different product are rejected by product/version checks.

- SQL execution through the MySQL transport, including normal SQL writes when
  allowed by the profile, client policy, and database grants.
- Database discovery and scoped, paged table/view catalog browsing and search.
- Column, index, primary/unique/foreign key and CHECK metadata where available.
- Bounded table/view data previews, filtering, ordering and result pagination.
- Native table/view definitions through `SHOW CREATE`.
- Existing SQL editor, completion, result grid and headless query infrastructure.

## Current limits

The adapter does not advertise structured catalog create/edit/drop, relation-grid
insert/update/delete, principal management, server monitoring, native query
cancellation, or manual transaction UI. Stored procedures, triggers and events
are not exposed in its catalog. Execute supported native SQL in the console when
appropriate; its database-side privileges remain authoritative.

TiDB profiles marked `read_only` use LazyDB's SQL risk classifier before executing
user SQL: every statement must be recognized as read-only, otherwise the whole
batch is rejected before execution. Sequence-changing `NEXTVAL` and `SETVAL`
expressions are also rejected, including nested queries and preview filters or
sorting expressions. TiDB's compatibility `READ ONLY` transaction
flag is not treated as server-side enforcement. Use a database account with
read-only grants for the authorization boundary. The shared policy also rejects
sequence changes through the read-only agent query endpoint on writable profiles,
and recognizes locking reads inside expression subqueries.

Native batch execution reads the connection's current `sql_mode` and refreshes it
after non-query statements, including `SET` and executable comments. This supports
`ANSI_QUOTES` and `NO_BACKSLASH_ESCAPES` when splitting a writable batch. Read-only
profiles conservatively reject these quote modes because their risk parser uses
default MySQL quoting. Automatic current-statement selection also rejects ranges
whose boundaries depend on those modes; use an explicit selection or full-buffer
execution for ambiguous SQL.

TiDB's distributed engine and compatibility catalog differ from MySQL. Shared
transport does not imply identical metadata types, InnoDB metrics, transaction
semantics, or cancellation behavior. The local fixture below uses a single
TiDB server with **unistore**, intended for adapter tests. It does not verify
TiKV/PD topology, distributed failures, or production load. TiDB Cloud validation
is recorded separately below. The declared 8.5+ compatibility gate is not a
certification of every newer release.

## TiDB Cloud validation

On 2026-10-31, the adapter was tested against a TiDB Cloud **Starter, MySQL
Compatible** instance on AWS Tokyo. `VERSION()` returned
`8.0.11-TiDB-v8.5.3-serverless`. Connections used `sslMode=verify-full`, the native
system trust store, and the gateway hostname; the session reported a nonempty
TLS cipher. No certificate-verification bypass or custom CA was needed.

The strict TiDB suite passed all 25 tests with the Cloud endpoint configured,
including its four live adapter tests. It verified native values, multiple result
sets, empty-result columns, scoped catalog paging/search, table/view metadata and
DDL, previews, normal SQL writes, and client read-only sequence/batch protection.
Additional live checks verified ordered preview pages of 10, 10, and 3 rows,
database discovery for a SELECT-only account, server rejection of its INSERT,
and `Authentication` categorization for an invalid password. The server's denied
INSERT currently surfaces as category `Sql` with SQLSTATE `42000`; its original
permission-denied message is preserved.

Use the complete username supplied by the console, including the instance prefix
(for example, `instance_prefix.root`). Creating a SQL user on Starter also
requires that prefix, and the complete name must fit the server's 32-character
limit. These requirements apply to SQL user creation as well as login.

The validation instance had a **$0 monthly spending limit**. Temporary databases,
tables, views, sequences, and the SELECT-only account were removed after testing.
The instance was retained with access restricted to the browser and SQL clients'
individual public IPs; their egress addresses differed during this run. Update
its firewall rules if your public IP changes. The generated test credential was
rotated at cleanup; use the console's password reset flow before connecting again.
One connection attempt timed
out at the adapter's 10-second pool acquisition limit; a subsequent attempt
succeeded. This run does not establish latency or availability guarantees, and
it does not cover Dedicated, private endpoints, other regions, PostgreSQL
compatibility mode, failover, or production load.

## TUI acceptance

The release TUI was exercised through a PTY against the same Cloud Starter
instance on 2026-10-31 and 2026-11-01. A connection was created in the actual form
with TiDB selected, `VERIFY FULL`, and local encrypted password storage. Table
and view previews, native table/view DDL, SQL console creation and execution,
and restored relation tabs and SQL text were checked from terminal screen output.

This acceptance found that saving changed connection settings could reuse an old
session. Saving a profile with changed settings or credentials now retires its
old sessions and connects again. The live writable-to-read-only switch was
retested: INSERT was rejected by the adapter and SELECT remained available.
Temporary Cloud objects and local acceptance credentials were cleaned up.

## Isolated local test server

The compose file pins the TiDB 8.5.3 image by digest, binds only loopback ports,
and uses container-local disposable storage. It does not mount user databases.
Unistore is a test backend and is not a production deployment recommendation.

```bash
docker compose -p lazydb-tidb-test -f docker-compose.tidb.yml up -d
curl --fail --retry 60 --retry-delay 2 --retry-all-errors --retry-max-time 180 http://127.0.0.1:18080/status
```

Run the adapter suite with missing service configuration treated as an error:

```bash
LAZYDB_REQUIRE_DATABASE_TESTS=1 \
LAZYDB_TEST_TIDB_URL='tidb://root@127.0.0.1:14000/test?sslMode=disable' \
cargo test --locked --test tidb_profile --test tidb_adapter --test tidb_contract --test tidb_ui -- --nocapture --test-threads=1
```

Without `LAZYDB_TEST_TIDB_URL`, live tests print an explicit skip message. This
must not be interpreted as successful server verification. Tests create and
remove uniquely named fixture objects; use an isolated service.

Stop and remove only this test project's containers and disposable storage:

```bash
docker compose -p lazydb-tidb-test -f docker-compose.tidb.yml down
```

See [database capabilities](database-capabilities.md) for the shared contract and
[TiDB MySQL compatibility](https://docs.pingcap.com/tidb/v8.5/mysql-compatibility/)
for upstream feature differences.
