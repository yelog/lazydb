# Database Capabilities

This page describes the implemented adapter contract. It is a capability
reference, not a claim that a live server is available on the development
machine.

## Driver Matrix

| Driver | Server/version gate | Namespace model | Catalog groups | Metadata support |
| --- | --- | --- | --- | --- |
| PostgreSQL | PostgreSQL 12 or newer | Database + schema | Tables, views, materialized views, sequences, functions, procedures, types | Type family, defaults, identity, generated expressions, character length, collation, comments; numeric precision/scale is not advertised |
| Oracle MySQL | Oracle MySQL 8.0.13 or newer; MariaDB is rejected for this catalog contract | Database is schema | Tables, views, functions, procedures, triggers | Type family, defaults, auto-increment, generated expressions, numeric precision/scale, character length, collation, character set, comments |
| SQLite | SQLite metadata support through native schema tables; no server-version gate | Database + attached schema aliases | Tables, views, triggers | Default expressions and hidden-column metadata; unsupported fields are represented as unsupported |

All three adapters advertise lazy children. SQLite opens a pool with exactly
one physical connection.

## Profile Discovery

`Test Connection` probes the draft connection, then performs read-only database
and schema discovery for the hierarchical scope picker. It does not save
metadata, change the active connection, or create a persisted profile. Results
are fingerprinted against connection fields and credential revision; a result
for a draft that has changed is ignored.

The picker presents `All` or `Selected` databases, with schemas nested under
each discovered database. PostgreSQL and SQLite can select `All schemas` or
individual schemas. MySQL mirrors each selected database as its schema; those
rows are read-only and cannot be toggled separately. If discovery is stale or
unavailable, saved selections remain visible with a warning.

## Catalog Paging

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
- Oracle MySQL reads the main table/view and each trigger through `SHOW CREATE`,
  discovers triggers through `information_schema`, and assembles the native
  object statement with sorted trigger statements. MariaDB is not part of this
  contract.
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
## Coding Agent Boundary

LazyDB's agent interface uses the same native adapters as the TUI, but each
operation is headless and target-attributed. Connections are resolved from the
current project plus global profiles. Other project-scoped profiles are hidden.

The first release has no shared agent transaction handles. Each operation owns
its connection lifecycle, and schema/query results include the selected profile,
environment, database, and effective read-only status. The database role is the
final read/write boundary; client-side MCP approval is not a replacement for
database grants.
