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

The monitoring dashboard is a separate workspace tab. Its native read-only
queries live in the concrete PostgreSQL/MySQL adapters, while the dashboard
model owns typed snapshots, elapsed-time counter rates, time-bounded history,
and process filtering. Runtime schedules single-flight metric and process
loads and tags every result with the dashboard tab generation and active
connection identity, so stale results cannot update a switched or closed tab.

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

Explorer search has two projections. `/` is a synchronous find over a snapshot of
the normal `visible()` projection, so collapsed descendants and unloaded pages are
never searched and no database command is emitted. It highlights primary labels
in the normal tree and cycles confirmed matches with `n/N`. `f` uses the independent
server-backed catalog contract: debounced requests carry active connection and
query generations plus `CatalogScope`, and adapters enumerate native catalog pages
without materializing them in the normal tree. Its results are a temporary,
ancestor-preserving tree projection with highlighted matches; locating one merges
only its real ancestor chain and object into the normalized tree, leaving lazy-page
completion state unchanged.

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

Coding-agent access uses a headless boundary above `DatabaseConnection`:

```text
JSON CLI / stdio MCP
        |
    AgentService
    - project visibility
    - deterministic selection
    - credential resolution
    - SQL/write policy
    - bounded results
        |
 DatabaseConnection
```

Agent sessions do not reuse TUI active pools or transaction workers. Global and
current-project profiles are visible to agents; profiles assigned only to other
projects are not exposed. MCP client permissions control tool visibility and
approval, while LazyDB profile policy and database grants remain authoritative.

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

Saved profiles also carry mutually exclusive global or project-scoped access.
Project roots are canonical absolute paths and may be shared by many projects.
The current Git-root-first project context filters Explorer placement; it does
not restrict database authorization. Access mutations use the same serialized
profile mutation lock and atomic ProfileStore save as profile CRUD, without
reconnecting active pools or changing workspace state.

The Profile Manager owns one draft form. `Test Connection` uses a temporary
connection, probes it, and performs read-only database/schema discovery for the
hierarchical scope picker without persistence or active-connection mutation.
Scope is `All` or `Selected`; MySQL's database-is-schema namespace is represented
as a mirrored, non-toggleable schema row. Discovery is fingerprinted by the
connection fields and credential revision; edits make a previous discovery
stale and late test results are ignored.

Profile fields are the sole connection source of truth. The form URL
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

Relation tabs are workspace tabs distinct from SQL consoles. Their Data and DDL
loads retain owned snapshots attributed to connection identity, profile UUID,
and catalog scope. Data previews are adapter-owned `SELECT * ... LIMIT 500`
requests: identifier quoting, preview filters/order, and the hard limit are all
owned by the concrete adapter. Column metadata is retained when there are zero
rows.

DDL has the same ownership boundary. `DatabaseConnection` dispatches relation
DDL to the concrete adapter, which validates the catalog identity, reads the
relation and children in a consistent read, and returns a complete sectioned
SQL string plus `DdlProvenance`. PostgreSQL reconstructs relation DDL from
`pg_class`/`pg_attribute` and `pg_get_*` catalog functions in a read-only
repeatable-read transaction. MySQL obtains the main object and triggers with
`SHOW CREATE` plus `information_schema`. SQLite reads `sqlite_schema` on its
single physical connection inside a transaction rolled back after the read.
The shared DDL assembler only normalizes sections and statement terminators;
the UI does not infer or generate DDL.

`NativeCatalog` identifies a single native catalog/server statement;
`AdapterGenerated` identifies a result assembled by the adapter from native
statements or metadata. This DDL provenance is independent of relation snapshot
provenance. Relation responses must match tab UUID, tab generation, request id,
relation key, scope, and active connection. A previous owned Data or DDL
snapshot can remain visible while a request loads, fails, or is cancelled.
Snapshot provenance is `Live`, `OfflineSnapshot`, `ProfileDeletedSnapshot`, or
`OutOfScopeSnapshot`, based on the current connection identity, profile, and
catalog scope.

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

The standalone
[`lazydb.nvim`](https://github.com/yelog/lazydb.nvim) frontend owns only a
scratch terminal buffer, floating window, argv/cwd, one process per Neovim tab
handle, and lifecycle/health checks. It does not parse SQL, connect to
databases, or store credentials. The launched LazyDB process provides the same
Profile Manager and keyring behavior as a direct CLI session.

## Remaining Architectural Work

- Persist console manifests and SQL files atomically.
- Replace complete result collection with bounded row batches and viewport storage.
- Add a conservative mutation planner for stable-key single-table previews.
