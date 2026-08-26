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
activates a relation workspace tab. It has independent `Data` and `Structure`
pages. Data is an adapter-owned read-only preview with a hard `LIMIT 500`, and
callers do not append an arbitrary limit. Statement metadata is collected before
rows, so zero-row relations still expose their columns. Structure includes
relation, column, index/constraint, trigger, and available DDL metadata.

Every relation request carries tab UUID, tab generation, request id, connection
identity, relation key, request kind, and catalog scope. Cancellation and stale
responses cannot overwrite a newer request. A loading, failed, or cancelled
request may continue displaying its previous owned snapshot. The UI attributes
snapshots as `LIVE`, `OFFLINE SNAPSHOT`, `PROFILE DELETED SNAPSHOT`, or `OUT OF
SCOPE SNAPSHOT`.

## Completion Cache

Completion is served from an in-memory catalog cache while typing. Lazy pages
append stable, deduplicated entries filtered to the active scope; connection or
catalog resets clear it. Scheduled completion carries console UUID, document
revision, connection identity, and catalog generation. If any component is
stale, the result is discarded. Explicit completion performs no database I/O,
returns ranked context-aware candidates, and is capped at ten results.
PostgreSQL and MySQL routine entries can contribute function and procedure
completion; SQLite does not advertise routine completion.
