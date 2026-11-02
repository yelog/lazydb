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
    |          |          |          |          |          |          |
   Postgres   MySQL    MariaDB     Oracle   SQL Server  SQLite     Redis/Valkey   ClickHouse
```

The monitoring dashboard is a separate workspace tab. Its native read-only
queries live in the concrete PostgreSQL/MySQL/MariaDB/Redis-compatible/ClickHouse adapters, while the
dashboard model owns typed snapshots, elapsed-time counter rates, time-bounded
history, and process filtering. Runtime schedules single-flight metric and process
loads and tags every result with the dashboard tab generation and its bound
`ConnectionIdentity`; results are checked against that session, not against the
globally projected active connection.

## Reducer Boundary

`App::update` is the only application-state mutation path. It accepts semantic
actions and returns commands. It does not access the terminal, filesystem, Tokio,
or SQLx. This makes tabs, focus, editor behavior, and stale-event rejection
deterministic unit-test targets.

The runtime executes commands and emits actions through a Tokio channel.
`SessionRegistry` is the App-side authority for connection attempts and connected
sessions, keyed by the complete `ExecutionTarget`. `ConnectionIdentity` scopes
result acceptance to its session, so a later attempt for an unrelated target
cannot invalidate it. `ConnectionState` remains a temporary projection for the
selected connection and is not a liveness check for other sessions. Each console
has its own query generation so a cancelled or old query cannot overwrite a newer
run.

The Explorer is a normalized, UUID-keyed tree. Its top-level roots are ordered
saved or session profiles, and each root carries provenance and connection
status. `EmptyProfiles` is a real empty-state node, not a popup; its action
starts a new profile draft. Catalog requests identify connection, catalog epoch,
request id, target, cursor, and scope. Pages are loaded lazily, validated before
mutation, and stale or mismatched results are discarded. A failed refresh
preserves the previous tree as stale where possible. Relation data, DDL, and
identity-recovery operations also receive the request's immutable scope
snapshot; adapter connection state is not authoritative after a profile's
Visible Objects settings change.

Explorer search has two projections. `/` is a synchronous find over a snapshot of
the normal `visible()` projection, so collapsed descendants and unloaded pages are
never searched and no database command is emitted. It highlights primary labels
in the normal tree and cycles confirmed matches with `n/N`. `f` uses the independent
server-backed catalog contract: debounced requests carry the search session's
connection identity and query generations plus `CatalogScope`, and adapters enumerate native catalog pages
without materializing them in the normal tree. Its results are a temporary,
ancestor-preserving tree projection with highlighted matches; locating one merges
only its real ancestor chain and object into the normalized tree, leaving lazy-page
completion state unchanged.

The Help and Omni views share one top-level interaction layer rendered above the workspace and
the current overlay. Its session owns query generations, stable result IDs, scope
filters, origin tab/profile identity, and a short in-memory navigation history.
Local commands, profiles, consoles, tabs, and loaded catalog entries are merged
without database I/O. Remote catalog search is tagged with a consumer owner and
session ID; Explorer and Omni cancellation are isolated while both continue to
use the validated `CatalogSearchRequest` contract. Opening a relation by
`CatalogId` does not mutate Explorer selection. Cross-profile navigation carries
its final intent and matching connection generation, and only resumes after that
connection succeeds. A failed or stale connection result cannot open a target.

The Help shortcut opens the configured Help or Omni view; `Tab` switches between
them without changing the underlying workspace. F2 opens Omni from any keymap
context, and from Help switches to Omni. Escape unwinds Omni parameter steps and
then closes the unified panel at its root; Ctrl-C dismisses it without quitting.
Idle Profile Manager and Catalog Editor overlays can be moved into an
in-memory, owner-tagged suspended interaction and explicitly resumed. Busy
operations and confirmation dialogs are not suspended. Suspended form state and
credentials are not persisted. During an in-process profile switch, the global
tab/document surface remains installed; connection changes do not duplicate or
move Console documents. The actual `EditorWorkspace` is keyed by console UUID,
preserving cursor, Vim mode, undo history, and editor prompt state; persisted
workspaces remain text-based snapshots and are rebuilt on process restart.

The editor is an App-owned `EditorWorkspace` keyed by console UUID. Modalkit
types stay behind that boundary; actions and UI consume LazyDB-owned editor
snapshots, effects, selections, and UTF-8 byte ranges. SQL scope, risk,
formatting, highlighting, completion, and execution drafts are pure projections
over those snapshots. Confirmation dispatches the immutable SQL snapshot rather
than rereading mutable editor text.

Each SQL console owns an `ExecutionTarget` containing profile UUID, database, and
schema. The target is independent from Explorer focus and may refer to an offline
profile. New consoles resolve their target from the focused Explorer node, the
recent-target list, and then stable profile order; if no profile exists they remain
unbound. The target selector can explicitly rebind a console across profiles
    while preserving its SQL document and editor history; confirmation starts
    or reuses the target connection. Offline profiles contribute only their
    configured default database/schema. A profile without a configured
    database uses the profile manager's discovery flow rather than fabricating
    an empty target.

Runtime connections are keyed by the complete target plus `ConnectionIdentity`.
Different profiles and database/schema targets can connect and execute concurrently;
duplicate attempts for the same target are single-flight. Connected sessions and
new attempts are stored separately so a failed reconnect does not evict a working
session. Query, relation, dashboard, catalog, completion, diagnostic, and
transaction results carry their originating tab/target identity and stale results
are discarded without requiring that target to remain focused. Disconnecting a
profile retires its sessions without affecting unrelated profiles. A disconnected
target leaves its Console document available.

SQL execution captures an immutable Console/request snapshot. Pending executions
are indexed by Console UUID. If the target is offline, the first execution establishes
the connection and resumes each matching, still-current request once; changing tabs
or editing the document does not redirect or mutate its SQL snapshot. Workspace
format v5 stores global Console documents, tab order, target bindings, active tab,
and recent targets. Console labels render the document name with an `@connection`
suffix, while the persisted document name remains unchanged.

The persistence boundary validates each Console UUID and SQL entry globally.
Legacy v1/v2 workspaces retain their historical missing-SQL compatibility during
migration; v3 and later workspaces report a missing SQL file instead of silently
replacing document text with an empty string. Saving after multiple profiles are
opened serializes each Console once, even while compatibility caches for older
profile workspaces are still present in memory.

Console creation and rename share trimmed, non-empty, control-character-free,
case-insensitive name validation. UUID, not the display name, is the stable identity.
Restore preserves persisted closed tabs and offline or invalid target bindings instead
of forcing the first Console open or silently rebinding it. Deleting a Console removes
its SQL file only after the corresponding workspace manifest revision succeeds; a
failed save leaves the file available for retry. SQL paths must be the exact
`<console-uuid>.sql` filename before loading.

Inactive profile snapshots use the shared live `ConsoleRecord` and
`EditorWorkspace` whenever that Console UUID is already installed. Legacy
`ConnectionWorkspace` data is only an import fallback during restoration; it
does not override live names, targets, SQL text, or deletion decisions.

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

`DatabaseConnection` dispatches to concrete PostgreSQL, MySQL/MariaDB, Oracle,
SQL Server, SQLite, and Redis adapters; MariaDB uses the MySQL-compatible
adapter with a distinct product kind. This is intentionally not SQLx `AnyPool`:
native catalog, type, SSL, DDL, cancellation, transaction, and key-value
behavior must remain visible.

Elasticsearch and OpenSearch retain independent profile kinds, connection variants,
URL schemes and authentication options while sharing the search REST adapter. Its
product parameter controls authentication and probing; OpenSearch requires the
server's `version.distribution = "opensearch"` and a nonempty version. Native REST
execution reuses immutable request confirmation, owner/generation validation and
result budgets. Indices use the existing relation catalog identity; SQL-only
editing, transactions, DDL and query controls remain unavailable. Transport rejects
external paths, traversal and redirects, disables retries, and reports potentially
unknown remote write outcomes after network errors.

Catalog requests use bounded keyset pages (maximum page size 500), with separate
targets for databases, schemas, groups, objects, and relation children. The
PostgreSQL adapter requires server version 12 or newer; the Oracle MySQL catalog
adapter requires 8.0.13 or newer; the MariaDB-compatible catalog requires 10.5
or newer; SQL Server requires 2012 or newer; Oracle uses the native client and
targets Oracle 12c or newer; Redis uses a logical database number rather than a
relational catalog. SQLite
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
effects; effects never enter application state. Pane border dragging also keeps
its pointer capture in `UiState`, computes an anchored absolute size, and emits
`SetPaneSize`. The application reducer stores the preference, while
`AppLayout` remains the single owner of minimum and maximum layout constraints.
Inactive effects cause no idle redraw.

Interactive overlays use the shared dialog primitives in `src/ui/dialog.rs`,
`src/ui/dialog_footer.rs`, and `src/ui/shortcut_hints.rs`. Context actions stay
near the content they mutate; window actions use a stable bottom action row;
keyboard help uses a separate low-emphasis row. Button drawing and mouse hit
regions are derived from the same measured rectangles, and single-line shortcut
rows omit complete hint items rather than clipping key/description pairs.
`Theme` exposes semantic button/help styles so plain-color mode still has a
non-color focus indicator. The table catalog editor is the reference
implementation for this hierarchy; connection, Redis, catalog-form, update,
and confirmation surfaces progressively consume the same primitives.
The table editor's column action cells carry the draft column `row_id` through
the mouse hit target and reducer, so a click mutates the row under the pointer
instead of relying on the currently selected row. Its bottom actions are
ordered Add Column, Review SQL, Cancel; change-summary spans use the Theme's
success/action/error colors for added/modified/removed counts.

Database text passes through terminal-control sanitization before it reaches
diagnostic state or display-only editor/SQL-preview projections. Raw SQL remains
unchanged when sent to the database. Completion labels/details and prompt text
are sanitized only for display; their raw insertion/request values remain
separate. Completion detail column types are additionally normalized to compact
display spellings; catalog `native_type` and result-set column types keep their
original text because cell-edit value parsing depends on them.

Workspace tabs are heterogeneous: SQL consoles and relation tabs share identity,
titles, and tab navigation but not behavior. Their tab bar is rendered above the
main content column; Explorer is outside that visual scope. Activating a relation
tab moves focus to Results; SQL-only editor, transaction, execution, and
completion actions are no-ops there. Relation focus cycles between Explorer and
Results.

Explorer catalog mutation help is derived from the selected node and
adapter-owned mutation capabilities. `n` remains profile creation; `a` opens
the supported child-object picker; `e` edits a directly selected profile or
catalog object, never a synthetic group/status row. Plans are guarded by
connection identity, target identity, catalog generation/epoch, and
definition/plan stale checks. PostgreSQL cross-database operations use
temporary target connections where required, then perform targeted refresh and
relation snapshot invalidation.

The implemented PostgreSQL editor scope covers Schema, Table, Column, Index,
Primary Key, Unique/Foreign Key/Check Constraints, View, Materialized View,
Sequence, Database, and Role. Login Role and Role creation are exposed from a
profile anchor. The Role node itself is not represented in the Explorer tree,
so existing roles cannot currently be selected and edited from Explorer.

## Transaction Boundary

AUTO queries carry both `ConnectionIdentity` and the exact `ExecutionTarget`.
Runtime looks up that exact session and rejects a mismatch before any database I/O.
Replacing a session builds a candidate pool before the old pool for that target is
closed. Other profiles and targets remain installed and usable. SQLite keeps the
profile file and reuses its single pool so discovered attached aliases remain
available while the selected target changes.
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

Workspace manifests and SQL files are persisted atomically in the normal save
path. A failed quit-save offers retry, discard-and-quit, or cancel-quit; an
automatic recovery bundle for partially unreadable workspace data remains a
separate follow-up.

## Remaining Architectural Work

- Expand operation-level relation mutation capabilities and round-trip coverage
  for the currently gated drivers and complex table shapes.
- Replace complete result collection with bounded row batches and viewport
  storage for very large arbitrary SQL result sets.
- Add a first-class query-plan/`EXPLAIN` workflow, SSH tunneling, and import/
  export surfaces without weakening the existing write-safety boundaries.
