# Database Capabilities

This page describes the implemented adapter contract. It is a capability
reference, not a claim that a live server is available on the development
machine.

For connection setup, see [Configuration](configuration.md). Dedicated guides
cover [DuckDB runtime and sessions](duckdb.md), [MongoDB](mongodb.md),
[Snowflake](snowflake.md), and the
[Redis/Valkey browser](redis-browser.md).

## Driver Matrix

The static driver contract is split into interaction model, catalog/DDL,
relation-data editing, transactions, cancellation, and monitoring. `Available`
means the adapter advertises the operation before connection; permissions,
server-version checks, missing metadata, and profile read-only state can still
disable it at runtime.

| Driver | Interaction model | Catalog / DDL | Relation data editing | Transactions | Cancellation | Monitoring |
| --- | --- | --- | --- | --- | --- | --- |
| PostgreSQL | Relational | Available; broad catalog editor | Available when metadata and permissions allow | AUTO/MANUAL | Native cancellation | Metrics and bounded process list |
| CockroachDB | Relational | Database/schema/table/view catalog, column/key/index metadata, search and native DDL inspection | Not advertised | AUTO/MANUAL; transaction retries require manual restart | Manual transaction session close | Not implemented |
| MySQL | Relational | Available; table/view catalog mutation | Insert requires reliable primary-key lookup; update/delete require primary key | AUTO/MANUAL | Supported | Metrics and bounded process list |
| MariaDB | Relational | Available; table/view mutation plus version-gated sequences | Insert supports `RETURNING`, including keyless insert; update/delete require primary key | AUTO/MANUAL | Supported | Metrics and bounded process list |
| TiDB | Relational, MySQL-compatible | Table/view discovery, relation metadata and native DDL display; structured catalog mutations unavailable | Not advertised | Automatic SQL execution only; no manual transaction UI | Not advertised | Not implemented |
| Oracle | Relational | Available; table/view/sequence DDL and catalog mutation slices | Not advertised | AUTO/MANUAL | Not advertised | Not implemented |
| SQL Server | Relational | Available; table/view mutation; complex children gated | Insert supports `OUTPUT`; update/delete require primary key | AUTO/MANUAL | Session close; open transaction rolls back | Not supported |
| SQLite | Relational | Available; table/view DDL and catalog mutation | Rowid-table insert lookup; update/delete require primary key; table shape is gated | AUTO/MANUAL | Progress handler / close | Not applicable |
| DuckDB | Embedded relational | Catalog/search/preview and native DDL; schema/table/view/sequence/ART index create; typed PK/UNIQUE/CHECK/FK constraints on table creation; table/column rename, nullable add, column drop/type/default/nullability and comment change; view rename/query/comment change; authoritative table constraints/index metadata and sequence/index definitions; table constraint edit and table index draft edit gated; macro structured management unavailable | Insert/update/delete for primary-key tables | AUTO/MANUAL | Native operation interrupt | Not applicable |
| Redis | Key-value | No SQL catalog or DDL | Native key/value and collection mutation, separate from relation grid | No SQL transaction UI | Not applicable | Read-only `INFO` metrics |
| Valkey | Key-value, Redis-compatible | No SQL catalog or DDL | Shared Redis-compatible key/value and collection mutation | No SQL transaction UI | Not applicable | Read-only `INFO` metrics |
| MongoDB | Document | Database, collection, and index discovery; native document commands, no SQL DDL | Document operations use a separate BSON/EJSON workflow; no relational grid editing | Bounded console transactions on replica sets or sharded clusters; see [MongoDB](mongodb.md#transactions) | No advertised server-side cancellation | Manual `serverStatus`, `dbStats`, and bounded `currentOp`; see [MongoDB](mongodb.md#monitoring-and-principals) |
| ClickHouse | Relational | Available; database/table/view/MV and bounded native object catalog | Not advertised; supported catalog table-shape edits are separate | No manual transactions | Local request cancellation only; server query may continue | `system.events` counters and bounded `system.processes` |
| Snowflake | Relational | Database/schema/table/view discovery, column metadata, relation previews and DDL reads; no structured catalog mutation | Not advertised | Independent SQL API statements; no manual transactions | Not advertised | Not supported |
| Elasticsearch | Search / REST | Index and mapping discovery; native REST commands, no SQL DDL | Confirmed REST writes; no grid-cell editing | No manual transactions | Local HTTP cancellation; server work may continue | REST APIs only; no dedicated dashboard |
| OpenSearch | Search / REST; independent driver | Index and mapping discovery; shared REST/NDJSON console, no SQL DDL | Confirmed REST writes; no grid-cell editing | No manual transactions | Local HTTP cancellation; server work may continue | REST APIs only; no dedicated dashboard |

Native key/value and document workflows have their own operation checks. The
relational grid and SQL transaction flags do not describe those workflows;
consult the linked database guide for the supported command scope.

## Runtime Requirements

Oracle connections require Oracle Database 12c or newer and a working native
Oracle client. The default build includes the adapter; see
[Oracle client discovery](configuration.md#connection-profile-fields).

DuckDB connections require an optional DuckDB 1.5.6 C API runtime. It is not
bundled with the default binary or downloaded automatically. See the
[DuckDB guide](duckdb.md) for installation, local files, in-memory sessions,
and supported catalog operations.

## Current Limitations

Shared workflows that are not available yet include general SQL query-plan
visualization, SSH tunneling, general-purpose data import/export, and automatic
recovery-bundle export for a corrupted or partially unreadable workspace.
Normal console/tab persistence and console renaming are supported.

Editing, transactions, cancellation, and monitoring differ by database as shown
above. Relation operations can also be disabled by missing row identity,
permissions, unsupported table shapes, or stale snapshots. Oracle relation-grid
editing, advanced LOB metadata, and cancellation are not advertised. SQL Server
authentication and monitoring limits are described in the catalog section below.

See the [issue tracker](https://github.com/yelog/lazydb/issues) and
[release notes](https://github.com/yelog/lazydb/releases) for current priorities
and version-specific changes.

## CockroachDB

CockroachDB uses an independent driver identity over the shared PostgreSQL wire
transport. The pinned integration fixture targets v25.4.6. Implemented workflows
include SQL execution, database/schema/table/view discovery, scoped search,
column/primary-key/index metadata, filtered and paged previews, native
`SHOW CREATE` DDL, and AUTO/MANUAL transactions. Existing MCP query and catalog
tools use the same adapter.

Structured schema editing, relation-grid writes, principal management and
monitoring are not advertised. Manual transactions do not replay SQL after
`40001`; roll back and restart the entire transaction. Cancelling a manual
transaction destroys its dedicated connection. DDL inside multi-statement
transactions remains subject to CockroachDB schema-change limitations.
See [CockroachDB setup and tests](cockroachdb.md).

## TiDB

TiDB is an independent profile using the MySQL transport and SQL dialect. The
initial native adapter targets stable TiDB 8.5.0 or newer, with the pinned 8.5.3
fixture as its baseline. Product/version validation reads the TiDB version suffix
rather than the leading MySQL compatibility number. Discovery, paged catalog and
search, table/view previews, relation metadata, and native `SHOW CREATE` are
supported. Stored procedures, triggers, events, monitoring, principal management,
manual transactions, native cancellation, relation-grid writes and structured
catalog mutations are not advertised. Native SQL still follows normal client
read/write policy. See [TiDB](tidb.md) for setup and verification limits.

## MariaDB

MariaDB shares the MySQL-compatible transport but has its own dialect and
product/version gates, with support starting at MariaDB 10.5. Implemented
workflows include SQL execution, AUTO/MANUAL transactions, monitoring,
database-is-schema catalog browsing, CHECK metadata, native trigger DDL, and
version-gated sequence discovery.

Relation data insertion uses `RETURNING`, including keyless inserts; existing-row
updates and deletes require primary-key metadata. See the relation-data matrix
below for operation-specific checks. Structured column/index/constraint
mutation, sequence mutation, routine/event management, system-versioned history
operations, and MariaDB-specific authentication options remain gated. The
[MariaDB test database guide](mariadb-test-database.md) provides an isolated
server with broad data-type coverage.

## Redis and Valkey

Redis and Valkey are distinct profiles using the same Redis-compatible transport
and key/value browser. Use `redis://` or `rediss://` for Redis and `valkey://`
or `valkeys://` for Valkey. Select the Valkey driver when importing a
Redis-shaped endpoint for a Valkey server so the saved product identity stays
Valkey. Imported credentials follow the configured secret policy and are omitted
from exported profile URLs.

Both drivers use a non-negative, server-configured logical database number.
SQL editing, SQL transactions, and relational catalog DDL do not apply. The
shared browser supports bounded discovery, typed value previews, key/value and
collection mutations, TTL controls, and read-only `INFO` metrics. Valkey-specific
Cluster/Sentinel routing and module types are not implemented.

The isolated fixture verifies Valkey 8.1.10, pinned as
`valkey/valkey@sha256:081c2f5cb575efc901aa80ff9cdbd1ec6a301682fd35e1ebb4b0990a4a4a8507`.
See [Contributing](../CONTRIBUTING.md) for fixture commands, and the
[browser](redis-browser.md) and [value-preview](redis-value-preview.md) guides
for interaction details.

## MongoDB

MongoDB uses a document explorer, native BSON/EJSON values, and a bounded JSON
command console. It has separate document, monitoring, principal-inspection,
and transaction workflows. Console command support is narrower than the native
MongoDB command set; see the [MongoDB guide](mongodb.md) for the current command
scope, topology requirements, permission behavior, and live verification limits.

## Snowflake

Snowflake uses the HTTPS SQL API with certificate verification on port `443`.
Profiles keep database and schema distinct, default schema to `PUBLIC`, and
optionally set warehouse and role for each request. Authentication accepts PAT,
OAuth, or an externally generated key-pair JWT; tokens use the existing secret
credential storage.

Explorer supports database, schema, table, and view discovery, column metadata,
bounded relation previews, catalog search, and `GET_DDL` reads. Identifiers are
quoted and catalog reads obey the profile's visible database/schema scope.
Results preserve SQL API types and use the shared row/byte budgets, including
asynchronous execution and partition retrieval. SQL editor completion,
highlighting, scanning, and parsing use the Snowflake dialect.

SQL API requests do not establish a persistent console session. Each statement
uses the selected database/schema/warehouse/role independently. Requests accept
one statement (`MULTI_STATEMENT_COUNT=1`); execute multi-statement buffers one
statement at a time. Session state such as `USE`, temporary objects, and
transactions must not be relied on across requests. Manual
transactions, structured row/catalog editing, principals, monitoring, and
server cancellation are not advertised. Closing a local request does not
confirm that server execution stopped. The server's grants remain authoritative.
See the [Snowflake guide](snowflake.md) for setup and verification limits.

## OpenSearch

OpenSearch retains its own driver identity and `opensearch://` / `opensearchs://`
URLs while sharing the Elasticsearch REST console, index/mapping catalog and
bounded document previews. Basic/None authentication and verified HTTPS with
optional PEM CA bundles are supported; API keys and AWS SigV4 are unavailable.
OpenSearch Security uses different role/action APIs and permissions. Discovery
may require more grants than document reads. Previews use `from`/`size` within
10,000 results with a 16 MiB preview-page result budget; exceeding it reports an
error rather than a completed truncated page. Cancellation is local and write
results can remain unknown after
a network failure. The official 2.19.4 fixture is the compatibility baseline;
other versions and managed-service deployments require separate verification.
See [OpenSearch](opensearch.md) for permissions, live/PTY commands and the exact
validation status.

## ClickHouse

ClickHouse uses the HTTP(S) interface, with `clickhouse://` defaulting to port
8123 and `clickhouses://` defaulting to verified HTTPS on port 8443. The default
database and user are `default`. TLS is never silently downgraded. Current
adapter and deterministic live fixture coverage targets ClickHouse 25.8 LTS;
other releases may differ in system-table columns and feature support.

The catalog maps a database to LazyDB's schema namespace. It reads databases,
tables, views, materialized views, dictionaries, SQL user-defined functions,
columns, projections, indexes and checks where the server exposes them. Profile
visible-object selections scope catalog and search requests. Type names are
retained alongside values; values that cannot be represented safely by LazyDB's
cell model remain textual instead of being silently narrowed.

The SQL editor supports native ClickHouse statements through the HTTP endpoint.
Grid query execution accepts one statement, normalizes its result format, and
applies row/byte budgets. Multi-statement buffers are not a ClickHouse
transaction: a completed prefix can already have taken effect, and a network
failure after sending a statement may have an unknown remote outcome. The adapter
does not advertise manual transactions. Relation-grid insert/update/delete is
not advertised because ClickHouse sorting keys are not unique row identities.

Structured catalog mutation preserves native engine, sorting, target and query
definitions. Table editing is limited to a single rename, comment change, or
appended simple column; other table shape changes fail closed. Ordinary views
support rename/comment while retaining their query. Materialized-view target and
engine are preserved; unsupported mutation controls remain gated. Dictionary
definitions that could reveal external credentials are not rendered as replay
DDL. User/role identity and grants are read from safe system columns; permission
coverage is Partial, and structured principal GRANT/REVOKE and membership
mutations are unavailable.

Monitoring reads actual cumulative `Query`, `SelectQuery`, and `InsertQuery`
counters from `system.events`; absent values remain absent. The process list reads
at most `MAX_PROCESS_ROWS` (2,000) initial queries from `system.processes`, displays the full native
`query_id`, and reports visibility as unknown because ClickHouse does not expose
whether this user's permissions hide matching sessions. Permission denial is
reported as restricted. It does not synthesize numeric PID values for
ClickHouse queries. Cancelling a running request aborts the local HTTP task;
this does not confirm that ClickHouse stopped executing the remote query.

Agent and MCP query/search reuse the same ClickHouse adapter, catalog scope,
read-only SQL classification, and explicit write policy. `SYSTEM`, `KILL`,
`ALTER`, and unclassified settings statements remain denied by agent SQL policy.
The server's own permissions remain authoritative. See the [ClickHouse test
database guide](clickhouse-test-database.md) for the deterministic 25.8 fixture
and forced-live test command.

## Users & Roles

Every relational connection ends its first explorer level with a `Users & Roles`
group. Expanding it lists users before roles, name-sorted. Enter opens a
read-only, DDL-only tab titled `name@connection`; it has no Data/DDL selector
and shares the relation DDL editor, so syntax highlighting, scrollbars, mouse
selection, and Vim bindings are identical. Passwords, password hashes, and
other secrets are never read or reproduced.

| Driver | Source and scope | Notes |
| --- | --- | --- |
| PostgreSQL | `pg_roles` cluster-wide; users are `rolcanlogin` roles | DDL is adapter-generated from role attributes and direct memberships; passwords cannot be recovered and are documented as omitted |
| MySQL | `mysql.user` accounts plus `mysql.role_edges` for role classification | Accounts keep `user` and `host` as separate identity fields; MySQL has no independent role flag, so roles that were never granted are reported as accounts with a native-kind note; MySQL before 8 has no role metadata |
| MariaDB | `mysql.user` with the native `is_role` flag | The flag is authoritative; account-lock state is not shown because the `mysql.user` view does not expose it |
| TiDB | Not implemented | The group reports unsupported; no account metadata query is issued |
| SQL Server | Bound database's `sys.database_principals`; `sys.database_role_members` and `sys.database_permissions` | Server logins are excluded; built-in fixed roles and users are described rather than emitted as replayable statements; identifiers use bracket quoting |
| Oracle | `all_users` plus `dba_roles` (falling back to `user_role_privs`, marked incomplete) | User and role names are namespaced in the native identity because they can collide; `DBMS_METADATA.GET_DDL` is not used for users because it embeds `IDENTIFIED BY VALUES` |
| SQLite | Not applicable | The group expands to an explicit "SQLite does not support users or roles" notice and issues no account query |
| Redis / Valkey | Not applicable | No `Users & Roles` group is shown |
| ClickHouse | `system.users`, `system.roles`, `system.grants`, and `system.role_grants`; cluster scope | Safe identity/grant columns only; permission and role-inherited coverage is Partial; structured GRANT/REVOKE/membership mutations are unavailable |
| Snowflake | Not advertised | No principal-management workflow |

## Relation Data Mutation Matrix

Relation Data 的行操作能力与 Catalog DDL 能力分开判断。新增一行不要求表有主键；更新和删除已有行仍要求可可靠定位，当前网格使用完整主键。插入结果必须来自数据库返回或可靠回查，不能用草稿值代替服务器生成的默认值、身份值或触发器结果。

| Driver | Insert result strategy | Keyless insert | Existing-row update/delete |
| --- | --- | --- | --- |
| PostgreSQL | `INSERT ... RETURNING` | Available | Primary key plus PostgreSQL row-version rules |
| MariaDB 10.5+ | `INSERT ... RETURNING` | Available | Primary key |
| MySQL | Primary-key lookup / auto-increment result | Requires reliable key metadata | Primary key |
| TiDB | Not implemented | Not advertised | Not advertised |
| SQL Server | `OUTPUT inserted.*` | Available where OUTPUT semantics are supported | Primary key |
| SQLite rowid table | Same-connection `last_insert_rowid()` lookup | Available for ordinary rowid tables | Primary key |
| SQLite `WITHOUT ROWID` / unsupported virtual table shape | Shape-specific strategy required | Capability-gated | Primary key |
| DuckDB | Typed insert for primary-key tables; see [DuckDB](duckdb.md#catalog-and-data) | Not advertised | Primary key |

Review SQL uses the canonical catalog identity and driver identifier quoting. MariaDB/MySQL use backticks and `() VALUES ()` for an empty-column insert; PostgreSQL/SQLite retain `DEFAULT VALUES`. Review SQL is display-only; execution always uses typed, bound mutation requests.

## Catalog Mutation Matrix

Catalog browsing and catalog mutation are separate contracts. A database is
only advertised as mutation-capable after the adapter implements the complete
definition-loading, draft/planning, execution, and catalog-refresh path. The
matrix below records the target contract for the multi-database mutation
rollout; entries marked `planned` must not be exposed by the Explorer until
their adapter tests pass.

| Driver | Namespace target | Planned create targets | Planned edit targets | Important native boundary |
| --- | --- | --- | --- | --- |
| PostgreSQL | Database + schema | Database, schema, table, view, materialized view, sequence, role | Existing PostgreSQL mutation set | Preserve current behavior while moving to the shared contract |
| Oracle | Service + owner/schema | Schema/user, table, view, sequence, index, constraint | Table, view, sequence, index, constraint, user/schema properties | A user/schema is not the same operation as creating an Oracle instance or PDB |
| MySQL | Database is schema | Database, table, view, index, constraint, user/role | Database properties, table, view, index, constraint, user/role | User identity includes host; no independent CREATE SCHEMA layer |
| MariaDB | Database is schema | MySQL core targets plus version-supported objects | MySQL core targets plus version-supported objects | Share safe implementation pieces, but gate MariaDB syntax independently |
| SQL Server | Database + schema | Database, schema, table, view, sequence, principal | Database/schema/table/view/sequence/index/constraint/principal | Login, database user, and role have different scopes; batches matter |
| SQLite | File database + attached aliases | File database, table, view, index, trigger | Table, view, index, trigger | No users/roles; complex table edits require a lossless rebuild plan |
| Redis / Valkey | Logical database + key namespace | Key and supported collection elements | Values, collection elements, and TTL where native semantics permit | Database number is not a creatable SQL catalog; streams are not freely editable |
| ClickHouse | Database is schema | Database, table, view and materialized view slices | Table rename/comment/simple appended columns; view rename/comment; unsupported engine/shape changes gated | Preserve native engine, sorting and target definitions; no drop/create emulation |
| Snowflake | Database + schema | Database, schema, table and view reads | Not advertised | SQL API requests keep explicit database/schema context |

The implementation is deliberately staged. The current implementation status
for the first mutation slice is:

| Driver | Currently advertised catalog mutation |
| --- | --- |
| PostgreSQL | Existing PostgreSQL create/edit contract |
| Oracle | Create and edit table/view/sequence from schema object groups; complex column/index/constraint edits remain gated |
| MySQL/MariaDB | Create and edit table/view from the database-is-schema namespace; column/index/constraint, sequence, principal, and event edits remain gated. Relation-data operations have separate checks in the matrix above |
| SQL Server | Create and edit table/view from a database/schema object group; column/index/constraint edits remain gated |
| SQLite | Create and edit table/view; complex table structure edits use a lossless rebuild plan |
| DuckDB | Native schema/table/view/sequence/index operations with operation-specific limits; see [DuckDB catalog and data](duckdb.md#catalog-and-data) |
| Redis / Valkey | No SQL catalog mutation; native Key-Value mutation supports typed replacement, targeted collection edits, and TTL preservation |
| MongoDB | Separate native document/catalog workflow; no SQL catalog mutation. See [MongoDB](mongodb.md) for the console and principal boundaries |
| ClickHouse | Bounded database/table/view/materialized-view mutations; see [ClickHouse](#clickhouse) for engine and table-shape limits |
| Snowflake | No structured catalog mutation; use explicitly executed SQL where permitted |

Redis mutations are opened from the Redis browser: `a` creates a key under the
selected database/prefix and `e` edits a loaded key value. Collection values are
entered one item per line (or `field<TAB>value` rows); binary fields use the
explicit `hex:` form. Existing-key edits preserve TTL by default, while create
operations default to persistent keys. Partial previews are not writable until
the complete value is loaded.

Until a row's individual object operation is complete, its capability remains
unavailable with a specific reason (`not implemented`, `not applicable`, or a server-version gate). A
read-only profile, missing active connection, permission failure, stale
catalog epoch, and unknown commit outcome are runtime conditions rather than
static driver capabilities.

The minimum integration proof for every advertised operation is:

```text
create → load authoritative definition → edit one property → load again
```

The proof must also cover an invalid permission/baseline case and refresh the
Explorer identity after a rename or rebuild. SQLite and Redis use their native
mutation protocols even though they share the same Explorer action contract.

## Monitoring Dashboard

The connection dashboard is available for PostgreSQL, MySQL, MariaDB, Redis, Valkey, and ClickHouse.
It collects read-only status counters and connection gauges; PostgreSQL, MySQL,
and MariaDB also expose a bounded process list. SQLite and Oracle report
monitoring as unsupported, while SQL Server's dashboard and process metrics are
currently gated. Samples remain in memory for the active workspace tab and are
not persisted. Counter rates use the elapsed time between valid samples;
the first sample, missing fields, resets, restarts, and failed polls create a
history gap rather than a fabricated rate.

The process list is bounded to `MAX_PROCESS_ROWS` (2,000 rows) and is marked
restricted when the database role cannot inspect it; ClickHouse visibility is
otherwise unknown because the server does not expose a completeness flag. Dashboard cancellation
aborts the local request and does not confirm remote query termination.

MongoDB has a separate, manually refreshed monitor for `serverStatus`, `dbStats`,
and `currentOp`. See [MongoDB monitoring](mongodb.md#monitoring-and-principals)
for permission handling and result limits.

## Catalog and Version Reference

| Driver | Server/version gate | Namespace model | Catalog groups | Metadata support |
| --- | --- | --- | --- | --- |
| PostgreSQL | PostgreSQL 12 or newer | Database + schema | Tables, views, materialized views, sequences, functions, procedures, types | Type family, defaults, identity, generated expressions, character length, collation, comments; numeric precision/scale is not advertised |
| MySQL | Oracle MySQL 8.0.13 or newer | Database is schema | Tables, views, functions, procedures, triggers | Type family, defaults, auto-increment, generated expressions, numeric precision/scale, character length, collation, character set, comments |
| MariaDB | MariaDB 10.5 or newer; product-specific version gate and MySQL-compatible transport | Database is schema | Tables, views, functions, procedures, triggers, sequences | Type family, defaults, auto-increment, generated expressions, numeric precision/scale, character length, collation, character set, comments; CHECK expressions are structured relation children; extended TIME values currently fall back to text |
| TiDB | Stable TiDB 8.5.0+; TiDB product version suffix, 8.5.3 fixture baseline | Database is schema | Tables, views, columns, indexes and constraints | MySQL-compatible relation metadata with TiDB-specific index field decoding |
| Oracle | Profile/URL recognition, native connect/probe, basic query, catalog, preview, DDL, and generated result pagination implemented for Oracle 12c+; advanced contract pending | Service + owner/schema | Tables, views, sequences, columns, indexes, primary/unique/check constraints | Type family, defaults, numeric precision/scale, character length; advanced LOB metadata pending |
| SQL Server | SQL Server 2012 or newer | Database + schema | Tables, views, functions, procedures, sequences, triggers; relation children include columns, indexes, keys, and foreign keys | Type family, defaults, identity, computed/generated expressions, numeric precision/scale, character length, collation, comments, and rowversion metadata |
| SQLite | SQLite metadata support through native schema tables; no server-version gate | Database + attached schema aliases | Tables, views, triggers | Default expressions and hidden-column metadata; unsupported fields are represented as unsupported |
| DuckDB | Optional DuckDB 1.5.6 C API runtime | Local file or in-memory database, catalog and schema; attached objects are session-scoped | Schemas, tables, views, sequences, indexes, and searchable native objects | Native types, columns, constraints, indexes, and definitions with the limits in the [DuckDB guide](duckdb.md) |
| Redis | Redis server; logical database selected by profile | Logical database + key namespace | Keys and native collection metadata | Key type, TTL, memory, and value-shape metadata; relational catalog fields are not applicable |
| Valkey | Valkey 8.1.10 verified by the pinned fixture | Logical database + key namespace | Shared Redis-compatible key and collection discovery | Key type, TTL, memory, and typed previews; no SQL catalog |
| MongoDB | Server permissions gate discovery and monitoring; transactions require a replica set or sharded cluster | Database + collection | Databases, collections, and indexes | Native BSON/EJSON document values and dedicated metadata; see [verification limits](mongodb.md#verification-limits) |
| ClickHouse | HTTP interface; supported and tested against ClickHouse 25.8 LTS | Database is schema | Tables, views, materialized views, dictionaries, functions; projections are table children | Names, engine/native kind, comments, columns, defaults, and native definition metadata where available |
| Snowflake | HTTPS SQL API; token and role permissions gate access | Database + schema | Databases, schemas, tables, views, and columns | Column type/nullability and native DDL where available |
| OpenSearch | Official 2.19.4 fixture verified; other releases/deployments require testing | Virtual database/schema namespace | Indices and mapping fields | Nested properties/multifields, JSON documents and original search hits; Basic/None with verified HTTPS |

PostgreSQL, MySQL, MariaDB, TiDB, SQLite, and ClickHouse advertise lazy children. SQLite opens a pool with
exactly one physical connection; SQL Server loads its supported catalog groups
without lazy child requests.

PostgreSQL relation catalog identities include the database, schema, object name,
and `pg_class.oid`. The OID is used only to reconcile a stale relation after an
external rename; the adapter still checks the active database, catalog scope,
relation kind, and permissions before returning the replacement entry. OIDs are
not permanent identifiers: after a relation is dropped, an OID may be reused,
so an unavailable or out-of-scope result is treated as a failed reconciliation
rather than a rename.

SQL Server catalog reads are scoped to databases and schemas and use native
`sys.*` catalog views. SQL Server does not currently use lazy child requests.
The adapter gates connections at SQL Server 2012, supports SQL username/password
authentication only, and requires an explicit TCP host and port. Windows
Integrated Authentication, Kerberos, Entra, Named Instance/SQL Browser discovery,
Dashboard metrics, process metrics, and additional specialized types are
deferred.

SQL Server query support includes multiple result sets, empty-result column
metadata, bounded relation previews, adapter-owned relation DDL, and relation
data editing with bound values. Standalone `GO` batch separator lines are split
before execution; `GO` and `GO 1` are supported, while other repeat counts are
rejected. The value layer decodes NULL, booleans, signed and unsigned integers,
floating point, text, bytes, dates, times, date-times, timestamps,
`uniqueidentifier` as text, and XML as text. Types that do not have a safe
normalized representation, including `money`/`smallmoney` and `sql_variant`, are
shown as unsupported values rather than being silently coerced.

## Profile Discovery

`Test Connection` probes the draft connection, then performs read-only database
and schema discovery for the hierarchical scope picker. It does not save
metadata, change the active connection, or create a persisted profile. Results
are fingerprinted against connection fields and credential revision; a result
for a draft that has changed is ignored.

The picker presents `All` or `Selected` databases, with schemas nested under
each discovered database. PostgreSQL and SQLite can select `All schemas` or
individual schemas. MySQL and MariaDB mirror each selected database as its schema; those
rows are read-only and cannot be toggled separately. If discovery is stale or
unavailable, saved selections remain visible with a warning.

## Catalog Paging

SQL Editor result pages and relation previews use Oracle's `OFFSET ... ROWS
FETCH NEXT ... ROWS ONLY` syntax and therefore require Oracle Database 12c or
newer. Result paging does not add a default ordering: queries without an
`ORDER BY` may have an unstable row order between pages. Complex queries that
cannot be safely parsed as one read-only statement remain on the unwrapped
execution path.

### PostgreSQL Schema Owners

The New Schema form uses PostgreSQL's effective `current_user` as the initial
owner. Its owner picker discovers both `LOGIN` users and `NOLOGIN` roles from
`pg_roles`. Roles that are visible but cannot be assumed by the active user are
shown as disabled rather than hidden. PostgreSQL 16 and newer use `SET ROLE`
privilege checks, as required by `CREATE SCHEMA ... AUTHORIZATION`; this same
check is used on older supported PostgreSQL versions. If role discovery is unavailable, the Owner field remains a
manual text input and PostgreSQL remains authoritative when the mutation runs.

Explorer pages are lazy, bounded to a maximum page size of 500, and use
versioned keyset cursors rather than offsets. A request includes connection
identity, catalog epoch, request id, target, cursor, and scope. Pages are
validated against that complete key before updating the tree or completion
cache. Refresh advances the catalog epoch; late, duplicate, malformed, or
cross-connection pages are ignored. Existing data is retained as stale when a
refresh fails. Empty targets render an empty state and partial targets render
`Load more...`.

SQLite loads each catalog page inside one transaction and rolls it back
afterward. The single physical connection gives the page a stable snapshot and
prevents races with another pooled SQLite connection; the catalog operations do
not write database state.

## Relations

### Refresh And Catalog Synchronization

Explorer `r` refreshes the selected catalog target and advances the catalog
epoch. Requests from an older epoch, duplicate pages, malformed pages, and
responses for another connection are ignored. Existing rows remain visible as
stale until the replacement page succeeds; a failed refresh does not erase the
last known snapshot.

After SQL changes that can alter catalog structure, LazyDB conservatively
refreshes the active profile's database catalog. The refresh is deduplicated,
completion data is rebuilt from accepted pages, and the UI reports either
`Catalog synchronized` or `SQL succeeded, but catalog synchronization failed;
refresh to retry`. Ambiguous or multi-statement SQL is not used to infer a
smaller target. Catalog synchronization does not claim that unrelated external
changes have been detected; use Explorer `r` when another client changes the
database.

For a selected PostgreSQL relation or relation child, `r` first resolves the
stored OID. A successful resolution replaces the stale catalog entry, rebinds
an open relation tab in place, invalidates its pending requests, and reloads
the relation children. This path preserves the tab's view and local state, but
a dirty relation remains write-blocked until its new identity is verified.

Opening a table, view, materialized view, or supported relation child creates or
activates a relation workspace tab. It has independent `Data` and `DDL` pages.
Data is an adapter-owned read-only preview with a hard `LIMIT 500`; callers do
not append an arbitrary limit. The adapter quotes the relation and applies the
optional WHERE/ORDER BY preview clauses before applying that limit. Statement
metadata is collected before rows, so zero-row relations still expose their
columns.

The `DDL` page is also adapter-owned end to end. The adapter validates the
catalog identity, reads the relation and its children, assembles complete
display SQL, and returns the SQL plus its provenance. The UI only renders and
scrolls that result; it never reconstructs DDL from generic catalog rows.

Driver-specific DDL behavior is:

- PostgreSQL uses a read-only `REPEATABLE READ` transaction and native catalog
  functions such as `pg_get_viewdef`, `pg_get_expr`, `pg_get_constraintdef`,
  `pg_get_indexdef`, and `pg_get_triggerdef`. It assembles tables, views, and
  materialized views with columns, identity/default/generated clauses,
  constraints, comments, indexes, and non-internal triggers where available.
- PostgreSQL relation rename reconciliation requires access to `pg_class` and
  `pg_namespace` for the stored OID and the active profile's selected schema.
  It does not require database or role creation privileges. A dropped object,
  an OID reused for another relation kind, or an object hidden by permissions
  is not treated as a rename.
- Oracle MySQL and MariaDB read the main table/view and each trigger through `SHOW CREATE`,
  discover triggers and CHECK expressions through `information_schema`, and assemble the native
  object statement with sorted trigger statements. MariaDB additionally discovers sequences
  from `information_schema.sequences`; sequence mutation is not advertised yet.
- SQLite reads the main table/view and related indexes/triggers from each
  schema's `sqlite_schema` table. The complete read runs on the single SQLite
  connection inside a transaction that is rolled back afterward. A relation
  with only its native statement has `NativeCatalog` provenance; adding related
  statements makes the assembled result `AdapterGenerated`.

`NativeCatalog` means the returned DDL is one native server/catalog statement.
`AdapterGenerated` means the adapter combined native statements into a stable,
sectioned result (and, for PostgreSQL, reconstructed the main statement from
catalog metadata). Both are read-only display results and remain owned by the
adapter.

Every relation request carries tab UUID, tab generation, request id, connection
identity, relation key, request kind, and catalog scope. Cancellation and stale
responses cannot overwrite a newer request. A loading, failed, or cancelled
request may continue displaying its previous owned Data or DDL snapshot. Each
owned snapshot records connection identity, profile UUID, and catalog scope.
The UI attributes snapshots as `LIVE`, `OFFLINE SNAPSHOT`, `PROFILE DELETED
SNAPSHOT`, or `OUT OF SCOPE SNAPSHOT`; these labels describe the snapshot's
relationship to the current connection/profile/scope, not whether its SQL was
native or adapter-generated.

## Completion Cache

Completion is served from an in-memory catalog cache while typing. Lazy pages
append stable, deduplicated entries filtered to the active scope; connection or
catalog resets clear it. Scheduled completion carries console UUID, document
revision, connection identity, and catalog generation. If any component is
stale, the result is discarded. Explicit completion performs no database I/O,
returns ranked context-aware candidates, and is capped at ten results.
PostgreSQL and MySQL routine entries can contribute function and procedure
completion; SQLite does not advertise routine completion.

Cancelling SQL Server execution closes the active session because the TDS driver
does not expose a cancellation primitive. SQL Server rolls back an open
transaction when that session closes; LazyDB does not claim commit or rollback
acknowledgement after the forced close.

## Coding Agent Boundary

LazyDB's agent interface uses the same native adapters as the TUI, but each
operation is headless and target-attributed. Connections are resolved from the
current project plus global profiles. Other project-scoped profiles are hidden.

The first release has no shared agent transaction handles. Each operation owns
its connection lifecycle, and schema/query results include the selected profile,
environment, database, and effective read-only status. The database role is the
final read/write boundary; client-side MCP approval is not a replacement for
database grants.
