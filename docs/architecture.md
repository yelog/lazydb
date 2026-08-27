# Architecture

LazyDB is currently a modular Rust monolith plus an independent thin Neovim
wrapper.

```text
Crossterm input / DB events
           |
         Action
           |
      App::update
           |
        Command
           |
        Runtime
           |
DatabaseConnection enum
  |          |          |
Postgres   MySQL      SQLite
```

## Reducer Boundary

`App::update` is the only application-state mutation path. It accepts semantic
actions and returns commands. It does not access the terminal, filesystem, Tokio,
or SQLx. This makes tabs, focus, editor behavior, and stale-event rejection
deterministic unit-test targets.

The runtime executes commands and emits actions through a Tokio channel. A
connection generation prevents a late connection result from replacing a newer
connection. Each console has its own generation so a cancelled or old query
cannot overwrite a newer run.

The Explorer is a normalized, UUID-keyed tree. Its top-level roots are ordered
saved or session profiles, and each root carries provenance and connection
status. `EmptyProfiles` is a real empty-state node, not a popup; its action
starts a new profile draft. Catalog requests identify connection, catalog epoch,
request id, target, cursor, and scope. Pages are loaded lazily, validated before
mutation, and stale or mismatched results are discarded. A failed refresh
preserves the previous tree as stale where possible.

The editor is an App-owned `EditorWorkspace` keyed by console UUID. Modalkit
types stay behind that boundary; actions and UI consume LazyDB-owned editor
snapshots, effects, selections, and UTF-8 byte ranges. SQL scope, risk,
formatting, highlighting, completion, and execution drafts are pure projections
over those snapshots. Confirmation dispatches the immutable SQL snapshot rather
than rereading mutable editor text.

Each SQL console owns an `ExecutionTarget` containing profile UUID, database, and
schema. `Space d` derives stable, sorted candidates from the active profile's
normalized catalog and `CatalogScope`. App keeps a target change pending until a
generation-matched connection succeeds, then updates and persists the console;
failure preserves both the old console target and old active pool.

## Profile and Credential Boundary

`ProfileStore` atomically persists versioned connection metadata and authenticated
local credential ciphertext in TOML. Profiles
use an explicit credential policy: passwordless, process-local prompt, authenticated
local encryption with a separate device key, or a native system credential
provider. `Runtime` owns profile CRUD, local cipher operations, native credential
operations, session-only secrets, and connection identity validation. Native and
local credential calls run in blocking tasks. Native references use
`keyring:dev.lazydb.lazydb/<profile-uuid>`; provider availability is runtime state
and is never persisted.
`App` applies profile state only after matching runtime completion actions, so a
failed save or switch can compensate without exposing a password.

The Profile Manager owns one draft form. `Test Connection` uses a temporary
connection, probes it, and performs read-only database/schema discovery for the
hierarchical scope picker without persistence or active-connection mutation.
Scope is `All` or `Selected`; MySQL's database-is-schema namespace is represented
as a mirrored, non-toggleable schema row. Discovery is fingerprinted by the
connection fields and credential revision; edits make a previous discovery
stale and late test results are ignored.

Structured profile fields are the sole connection source of truth. The form URL
is a secret-backed editing projection with a persisted format preference. URL
parsing applies fields atomically; field changes only format a new URL and never
trigger reverse parsing. Parsed passwords move to `SecretString` storage and are
omitted from the canonical URL.

`default_schema` controls the default namespace and completion ranking, while
`CatalogScope` remains the sole Explorer and metadata visibility policy. A draft
tracks whether scope is derived or explicitly customized so default database and
schema changes only rewrite derived visibility.

## Database Boundary

`DatabaseConnection` dispatches to concrete `PostgresAdapter`, `MySqlAdapter`, or
`SqliteAdapter`. This is intentionally not SQLx `AnyPool`: native catalog, type,
SSL, DDL, cancellation, and transaction behavior must remain visible.

Catalog requests use bounded keyset pages (maximum page size 500), with separate
targets for databases, schemas, groups, objects, and relation children. The
PostgreSQL adapter requires server version 12 or newer; the Oracle MySQL catalog
adapter requires 8.0.13 or newer and rejects MariaDB for this contract. SQLite
supports metadata from native schema tables and loads each page inside a
transaction that is rolled back afterward. SQLite deliberately uses a single
physical pool connection, and catalog operations do not write database state.

Relation tabs are workspace tabs distinct from SQL consoles. Their Data and
Structure loads retain owned snapshots attributed to connection identity,
profile UUID, and catalog scope. Data previews are adapter-owned `SELECT * ...
LIMIT 500` requests and keep column metadata when there are zero rows. Relation
responses must match tab UUID, tab generation, request id, relation key, scope,
and active connection. Snapshot provenance is Live, OfflineSnapshot,
ProfileDeletedSnapshot, or OutOfScopeSnapshot.

Dynamic user SQL consumes SQLx `raw_sql().fetch_many()` so statement result
markers are retained. Each adapter decodes its concrete row type into owned
`CellValue` values. NULL, empty text, and empty binary remain distinct. Unknown
types become `Unsupported` values rather than failing the complete query.

## Rendering Boundary

The UI is immediate-mode Ratatui. `AppLayout` selects too-small, focus, standard,
or wide composition. `UiState` owns mouse hit regions and transient TachyonFX
effects; effects never enter application state. Inactive effects cause no idle
redraw.

Database text passes through terminal-control sanitization before it reaches
diagnostic state or display-only editor/SQL-preview projections. Raw SQL remains
unchanged when sent to the database. Completion labels/details and prompt text
are sanitized only for display; their raw insertion/request values remain
separate.

Workspace tabs are heterogeneous: SQL consoles and relation tabs share identity,
titles, and tab navigation but not behavior. Their tab bar is rendered above the
main content column; Explorer is outside that visual scope. Activating a relation
tab moves focus to Results; SQL-only editor, transaction, execution, and
completion actions are no-ops there. Relation focus cycles between Explorer and
Results.

## Transaction Boundary

AUTO queries carry both `ConnectionIdentity` and the exact `ExecutionTarget`.
Runtime records both on the active connection and rejects a mismatch before any
database I/O. PostgreSQL/MySQL target changes build a candidate pool before the
old pool is closed. SQLite keeps the profile file and reuses its single pool so
discovered attached aliases remain available while the active target changes.
MANUAL mode owns one serial worker and one physical connection per console.
Adapters drive SQLx's concrete `TransactionManager` directly on that connection;
the worker never sends transaction controls through a random pool connection.
An armed guard detaches and backend-closes uncertain sessions. PostgreSQL and
MySQL use database-native cancellation metadata; SQLite uses a progress handler
and awaited connection close. Commit/rollback acknowledgement loss is represented
as `OutcomeUnknown` and is never retried automatically.

Completion is an in-memory index populated from accepted catalog entries and
updated as lazy pages arrive. It filters to the active catalog scope,
deduplicates stable IDs, and replaces entries when the connection/catalog is
reset. Scheduled completion work carries console UUID, document revision,
connection identity, and catalog generation as one stale-check key. Typing,
switching connections, or refreshing the catalog therefore invalidates old
results; completion performs no database I/O while typing.

## Neovim Boundary

`lazydb.nvim` owns only a scratch terminal buffer, floating window, argv/cwd,
one process per Neovim tab handle, and lifecycle/health checks. It does not parse
SQL, connect to databases, or store credentials. The embedded LazyDB TUI
provides the same Profile Manager and keyring behavior when launched by Neovim.

## Remaining Architectural Work

- Persist console manifests and SQL files atomically.
- Replace complete result collection with bounded row batches and viewport storage.
- Add a conservative mutation planner for stable-key single-table previews.
