# Database Explorer and Connection Integration Design

**Status:** Approved

**Date:** 2026-08-25

## Summary

LazyDB will replace the Profile Manager connection-list popup with connection
roots in the left Explorer. The application will continue to have one active
database connection at a time, while every saved or session-only profile remains
visible as a root with an explicit connection state.

The Explorer will use a capability-aware, lazily loaded catalog hierarchy:

```text
Connection profile
  Database
    Schema
      Object group and count
        Object
          Column and other object details
```

Connection editing will gain a hierarchical database/schema visibility picker.
The selection will be persisted as an explicit database-to-schema scope, pushed
into catalog requests, and shared by Explorer and SQL completion. Opening a table,
view, or one of its descendants will create or reuse a relation page containing
Data and Structure views. Data opens first and previews at most 500 rows.

## Confirmed Decisions

- Keep one active connection and one active runtime database pool.
- Show all profiles as Explorer roots; do not keep inactive catalog trees cached.
- Support database discovery according to each driver's real capabilities.
- PostgreSQL initially browses only the database named by the profile.
- Use one relation page with Data and Structure views rather than separate tabs.
- Replace the current flat `include_databases` and `include_schemas` fields with
  an unambiguous hierarchical scope model.
- The project is still early; the new profile shape does not need to migrate
  existing connection files.
- Preserve the existing reducer/runtime boundary and concrete database adapters.

## Goals

- Make the Explorer the sole place for listing, selecting, connecting, editing,
  creating, and deleting connection profiles.
- Represent offline, pending, online, catalog-syncing, and failed states without
  relying on color alone.
- Match the required hierarchy and show exact or explicitly partial child counts.
- Persist user-selected database/schema visibility and restore it on restart.
- Keep hidden objects out of both the Explorer and SQL completion.
- Expose structured column type, default, nullability, generated/identity, and
  comment metadata where the backend supports it.
- Open table data and structure from a table, view, or any descendant node.
- Reject stale asynchronous catalog and preview responses deterministically.
- Remain responsive with at least 10,000 catalog objects.

## Non-goals

- Multiple simultaneously active database workspaces or pools.
- Persisting inactive catalog snapshots to disk.
- Treating visibility filters as database authorization.
- Browsing other PostgreSQL databases through hidden auxiliary pools.
- A generic least-common-denominator catalog that advertises unsupported objects.
- Editable result grids, full result paging, or schema migration tooling in this
  increment.

## Current Constraints

The current implementation has a singular connection and singular flat catalog:

- `App` owns one `ConnectionState` and one `ExplorerState` in `src/app.rs`.
- `ExplorerState` stores `Vec<CatalogNode>`, a positional selection, and one
  expansion set in `src/model/workspace.rs`.
- `CatalogKind` contains database-native concepts only in `src/db/catalog.rs`.
- `ProfileManagerState` independently owns the popup list and form state in
  `src/model/profile_manager.rs`.
- Every adapter eagerly loads its complete supported catalog.
- Catalog refreshes are protected by connection generation but not by a distinct
  catalog request generation, so two same-connection refreshes can complete out
  of order.
- Explorer rendering ignores `native_kind` and `detail`, and repeatedly scans the
  flat node vector while projecting visible rows.
- `include_databases` and `include_schemas` are persisted but are not editable or
  applied to catalog loading.
- Table preview already generates a safely quoted `LIMIT 500` query, but it opens
  a SQL result tab rather than a relation page, and completion events do not carry
  the full connection identity.

This design changes those boundaries only where required. It does not turn the
runtime into a per-profile connection map.

## Architecture

The existing unidirectional flow remains authoritative:

```text
Crossterm event or async result
              |
            Action
              |
         App::update
              |
           Command
              |
           Runtime
              |
   concrete DatabaseConnection
```

Only `App::update` mutates application state. UI code renders projections and
emits semantic actions. Runtime performs connection, persistence, secret-store,
catalog, and query side effects. Concrete adapters retain backend-specific SQL,
qualification, capabilities, and metadata behavior.

### Explorer Identity

Connection and presentation-group nodes must not be added to `CatalogKind`.
Introduce an Explorer-owned identity:

```rust
enum ExplorerNodeId {
    Profile(Uuid),
    Catalog(CatalogId),
    Group {
        parent: CatalogId,
        group: ObjectGroup,
    },
}
```

`CatalogId` continues to identify native catalog objects. `ObjectGroup` is a
presentation concept derived from adapter capabilities and loaded catalog
summaries. Stable IDs replace visible-row indices as the source of truth for
selection, expansion, mouse targets, and refresh restoration.

### Explorer State

The Explorer owns ordered profile roots and per-profile presentation state:

```rust
struct ExplorerState {
    selected: Option<ExplorerNodeId>,
    expanded: HashSet<ExplorerNodeId>,
    scroll: usize,
    profiles: HashMap<Uuid, ExplorerProfileState>,
}

struct ExplorerProfileState {
    status: ExplorerConnectionStatus,
    catalog: CatalogTree,
    catalog_request_generation: u64,
    load_state: LoadState,
    last_error: Option<String>,
    expand_after_connect: bool,
}
```

Profile display order continues to come from the profile registry. `CatalogTree`
uses parent-to-children adjacency indexes so visible-row projection is linear in
the visible subtree rather than repeatedly scanning every catalog entry.

Only the active profile retains a catalog. On successful switching, the previous
root becomes offline, collapses, and drops its catalog. Relation pages may retain
explicitly labeled snapshots, but Explorer never presents an inactive snapshot
as live metadata.

### Connection Status

Every profile root renders one of these textual states:

- `OFFLINE`: saved or session profile with no active or pending connection.
- `LINKING`: the target of the current connection attempt.
- `ONLINE`: active pool installed and catalog ready.
- `SYNCING`: active pool installed and the requested catalog level is loading.
- `FAILED`: the most recent attempt for this profile failed.

The old active root remains `ONLINE` while another root is `LINKING`, matching the
existing safe-switch behavior. A failed switch leaves the old active root online
and marks only the target root failed. Retrying, editing, or successfully
connecting clears the target's failure.

### Catalog Contract

The database boundary gains explicit catalog capabilities and scoped requests:

```rust
struct CatalogCapabilities {
    namespace_model: NamespaceModel,
    top_level_groups: Vec<ObjectGroup>,
    column_metadata: ColumnMetadataCapabilities,
    supports_lazy_children: bool,
}

struct CatalogRequest {
    connection: ConnectionIdentity,
    request_generation: u64,
    parent: Option<CatalogId>,
    group: Option<ObjectGroup>,
    scope: CatalogScope,
    cursor: Option<CatalogCursor>,
    page_size: usize,
    depth: CatalogDepth,
}

struct CatalogPage {
    entries: Vec<CatalogEntry>,
    total_count: Option<usize>,
    next_cursor: Option<CatalogCursor>,
    completeness: CatalogCompleteness,
}
```

The exact Rust shape may vary, but every request and response must carry enough
identity to reject work from another profile, connection generation, parent, or
newer catalog request. A count is never guessed: exact counts render normally,
lower bounds render with `+`, and unavailable counts render as an ellipsis while
loading or no suffix when unsupported.

## Persisted Catalog Scope

The existing two flat inclusion lists cannot represent schemas belonging to
different databases and overload an empty list to mean `All`. Replace them with
an explicit hierarchical model:

```rust
struct CatalogScope {
    databases: ScopeSelection<DatabaseScope>,
}

struct DatabaseScope {
    name: String,
    schemas: ScopeSelection<String>,
}

enum ScopeSelection<T> {
    All,
    Selected(Vec<T>),
}
```

The serialized representation must be explicitly tagged rather than relying on
empty-array conventions. Profile files move to a new configuration version and
old versions are rejected with an actionable error; no migration is required for
this early-development increment.

### Defaults

For a new profile:

- Select the configured/current database.
- If `default_schema` is present, select only that schema under the database.
- If `default_schema` is absent, select `All schemas` under the database.
- For a backend whose schema is the database namespace, mirror and lock the schema
  selection instead of presenting two independent controls.

`All` means all non-system objects that the adapter considers visible. It does not
override adapter-level system-catalog exclusions. A default schema excluded by a
custom selection is a form validation error because it would make scope and
completion defaults contradictory.

### Scope Discovery

Opening a profile form never initiates a network connection by itself. A
successful `Test Connection` also performs read-only scope discovery before
closing the temporary connection. Editing the active profile may reuse compatible
active discovery data.

The scope picker remains usable when discovery is unavailable: it displays saved
selections and an instruction to test or refresh. Changes to driver, host, port,
user, database, SSL, or credential inputs invalidate discovered options while
preserving the user's draft selection with a stale warning. Unavailable saved
identifiers are retained and warned about rather than silently deleted.

## Explorer User Experience

### Tree Rows

The standard layout is:

```text
v [PG] local-app                         ONLINE
  v app                                  Database · 3 schemas
    v public                             Schema
      v Tables                           42
        v users                          Table
          · id                           bigint · NOT NULL · DEFAULT nextval(...)
          · email                        varchar(255) · "login email"
      > Views                            6
      > Sequences                        4
> [MY] staging                           OFFLINE
! [SQ] broken-demo                       FAILED
```

The driver badge is an ASCII-safe fallback for environments without special icon
fonts. Theme symbols and colors supplement, but never replace, status text.

Database rows show the filtered schema count. Group rows show the filtered object
count. Object rows show the native object name. A relation's loaded children
include columns and available index/key/constraint metadata without introducing
fake native objects.

Column row metadata follows this priority:

1. Native or fully formatted type.
2. Nullability and identity/generated flags.
3. Default expression.
4. Comment.

Rows remain single-line. At constrained widths, comment, default, then secondary
flags are truncated in that order; type remains visible. The Explorer detail area
shows the selected row's complete sanitized metadata. Standard and wide layouts
allocate more width to Explorer than the current 28/34 columns, while sub-100
column focus mode continues to give the selected panel the full terminal width.

### Keyboard and Mouse

Explorer bindings become directional rather than mapping every key to toggle:

| Key | Behavior |
| --- | --- |
| `j/k`, arrows | Move by visible row |
| `Home/End`, `gg/G` | Move to first/last visible row |
| `l`, Right | Expand; an offline profile first connects safely |
| `h`, Left | Collapse, or move to the parent when already collapsed |
| `Enter`, `o` | Execute the selected node's primary open action |
| `r` | Refresh the selected subtree, not the entire catalog |
| `n` | Create a profile |
| `e` | Edit the selected profile root or owning profile |
| `d` | Delete the selected profile after confirmation |
| `c` | Connect or switch to the selected profile |
| `x` | Disconnect the active profile |
| `p` | Open or focus the relation Data view |
| `D` | Open available structure/DDL information |

For a profile, database, schema, or group, `Enter` connects or expands as
appropriate. For a table, view, column, index, key, constraint, or trigger owned
by a relation, `Enter` resolves the nearest relation ancestor and opens that
relation page. Non-relation objects use capability-aware detail or DDL behavior.

A single mouse click focuses Explorer and selects the stable node ID. A double
click executes the same primary action as `Enter`. Mouse wheel movement changes
the viewport while keeping selection visible.

### Connection Management

The Profile Manager list page is removed. Explorer roots expose all profile
operations. The existing form and delete-confirmation overlays remain because
they are focused transactional tasks rather than redundant navigation.

When no profile exists, Explorer displays one actionable empty-state row and `n`
opens a new PostgreSQL draft, preserving current first-run behavior. An ad-hoc
`--url` profile appears as a `SESSION` root and is not persisted implicitly.

The form adds a `Visible objects` row whose summary is such as
`1 database / 2 schemas`. Activating it opens a hierarchical checkbox page. `All
schemas` is mutually exclusive with individual schema selections under the same
database. Unsupported levels are shown as derived and read-only, not as controls
that cannot work.

## Driver Capabilities

Initial capability declarations and catalog coverage are:

| Driver | Database scope | Schema model | Top-level groups |
| --- | --- | --- | --- |
| PostgreSQL | Configured database only | Multiple user-visible schemas | Tables, Views, Materialized Views, Sequences, Functions, Procedures, Types |
| MySQL | Databases visible to the current account | Database and schema are the same namespace; UI mirrors the schema row | Tables, Views, Functions, Procedures, Triggers |
| SQLite | Configured file or memory database | `main`, `temp`, and attached aliases from `PRAGMA database_list` | Tables, Views, Triggers |

Adapters must not declare a group until they can return correct entries and
stable identities for it. Unsupported groups remain absent rather than appearing
empty or failing when opened. Relation child support, such as indexes and foreign
keys, is likewise capability-aware.

## Lazy Loading and Counts

Catalog loading follows these levels:

1. Connecting or expanding the active root discovers databases allowed by the
   persisted scope.
2. Expanding a database loads filtered schemas and their exact count when the
   adapter supports one.
3. Expanding a schema loads group summaries and filtered counts.
4. Expanding a group loads paged object summaries.
5. Expanding a relation loads columns, indexes, keys, and constraints.

Group summary and child loading are separate so a schema can show useful counts
without materializing every column. Pagination prevents large schemas from
blocking the event loop or allocating an unbounded response.

Visibility is pushed into adapter queries wherever possible. A pure catalog
scope check also validates returned entries before they enter `CatalogTree`. This
second check is an invariant guard, not a substitute for query pushdown.

Refresh allocates a new per-profile catalog request generation. Older responses
for the same connection are ignored. Successful replacement preserves selected
and expanded stable IDs that still exist; missing IDs fall back to the nearest
existing ancestor. A failed refresh retains the previous subtree and marks it
stale.

## Structured Column Metadata

Replace overloaded display strings with structured relation and column metadata.
A column model includes, where supported:

- Ordinal position and native name.
- Native/declared type and optional normalized type family.
- Nullability.
- Default expression.
- Identity, auto-increment, generated expression, and hidden state.
- Precision, scale, length, collation, or character set when useful.
- Comment.
- Structured primary, unique, and foreign-key memberships.

PostgreSQL uses native catalogs such as `pg_attribute`, `pg_type`, and
`pg_description` together with information-schema data. MySQL uses
`information_schema.columns`, including `column_type`, `column_default`, `extra`,
`generation_expression`, and `column_comment`. SQLite uses
`pragma_table_xinfo`, including `dflt_value`; native column comments are marked
unsupported rather than represented as an empty supported value.

Indexes and constraints are grouped native objects, not one fake object per
component column. This is required for correct object counts and composite key
display.

Database-provided text remains raw for identity and execution but is sanitized in
all display projections. Long default expressions and comments are bounded before
rendering.

## Relation Page

Add a profile-scoped relation tab alongside SQL console tabs:

```rust
enum TabKind {
    SqlConsole,
    Relation(RelationTab),
}

struct RelationTab {
    profile_id: Uuid,
    object_id: CatalogId,
    view: RelationView,
    data: RelationDataState,
    structure: RelationStructureState,
}

enum RelationView {
    Data,
    Structure,
}
```

Opening uses `(profile_id, object_id)` to focus an existing tab instead of
creating duplicates. The default view is Data. Structure is populated from the
same typed catalog metadata used by Explorer and contains columns, indexes, keys,
constraints, comments, and available DDL with provenance.

The adapter owns preview SQL generation and receives a stable object ID rather
than schema/name strings inferred from path offsets. The initial request has a
hard limit of 500 and uses fully qualified, safely quoted identifiers. The
generated SQL is inspectable but is not confused with a user-owned SQL console.

Preview responses retain column metadata even when no rows are returned. They
carry `ConnectionIdentity`, relation-tab generation, and request ID. Cancellation
and late-response rejection use the same rules as normal query work.

After switching connections, an existing relation page may keep its returned data
as an `OFFLINE SNAPSHOT`. Refresh and new data requests are disabled until its
owning profile becomes active again. The snapshot can never be relabeled as data
from the newly active connection.

## Data Flows

### Startup

1. Runtime loads ordered profiles and chooses the startup profile as today.
2. App creates an Explorer root for every saved and ad-hoc profile.
3. The selected startup profile enters `LINKING`; other roots remain `OFFLINE`.
4. Connection success installs the active identity and begins root discovery.
5. Root discovery success marks the root `ONLINE` and expands it according to the
   pending navigation intent.

### Connect From Explorer

1. `l`, `Enter`, or `c` on an offline root emits a profile-ID-bearing connect
   action.
2. Existing query and transaction guards decide whether switching can proceed.
3. Target root becomes `LINKING`; old active root remains usable but new commands
   are blocked during the switch.
4. Runtime installs only a generation-current successful connection.
5. App retires and clears the old root, activates the new root, and starts scoped
   discovery.

### Edit and Save Scope

1. The form derives the initial scope from the profile defaults or persisted
   selection.
2. Test Connection optionally returns discovered databases/schemas and adapter
   capabilities.
3. The user chooses databases and schema selections in the scope page.
4. Validation rejects contradictory default-schema selections.
5. Runtime atomically saves the complete profile file.
6. If only catalog scope changed on the active profile, App increments the
   catalog request generation, clears completion, and reloads catalog scope
   without reconnecting.
7. Connection-affecting fields retain the existing Save versus Save & Connect
   semantics.

### Open a Relation

1. App resolves the selected catalog node to its owning table or view.
2. App focuses or creates the profile/object-scoped relation tab.
3. If the owning profile is active, Runtime loads missing structure and executes
   an adapter-generated preview with limit 500.
4. Result actions must match profile, connection generation, tab generation, and
   request ID before App applies them.
5. Data becomes visible while Structure remains available without another SQL
   console.

## Error and Stale-State Handling

- A connection failure marks only the target root failed and preserves the prior
  active connection.
- A first-load catalog failure renders a retryable child error under the owning
  node.
- A refresh failure keeps prior data and marks the subtree stale.
- Permission denied is represented separately from an empty result.
- Scope-discovery failure leaves the draft and saved selections intact.
- Duplicate catalog requests for the same node are coalesced or the older request
  is ignored by generation.
- Deleting the active root uses existing running-query and manual-transaction
  guards, disconnects, closes relation refresh paths, and selects the nearest
  remaining root.
- Saving only catalog scope does not access or mutate the keyring.
- Every database error and metadata value is sanitized before rendering.

## Testing Strategy

### Pure Model Tests

- Scope defaults for every driver and default-schema combination.
- Tagged scope serialization and new profile-file version rejection.
- Per-database schema selection, `All`, contradiction validation, and stale
  discovered options.
- Stable Explorer selection, parent navigation, expansion, collapse, scrolling,
  and removal fallback.
- Capability-derived groups and exact/partial count formatting.
- Structured column metadata formatting and terminal sanitization.
- Relation ancestor resolution from columns, indexes, keys, and constraints.

### Reducer and Runtime Tests

- All profile roots and statuses across connect, safe switch, failure, retry,
  disconnect, save, and delete.
- Catalog request generation rejects same-connection out-of-order responses.
- Subtree refresh preserves expansion and retains stale data on failure.
- Filter-only save hot-reloads catalog without reconnecting or touching secrets.
- Hidden nodes never enter completion.
- Relation tabs deduplicate by stable identity and reject stale preview,
  structure, and DDL responses after switching.
- Existing query and manual-transaction switch guards remain effective.

### Adapter Tests

- SQLite fixtures assert hierarchy, attached schemas, group counts, defaults,
  generated/hidden metadata, grouped indexes/FKs, and zero-row preview columns.
- PostgreSQL CI fixtures assert multiple schemas, materialized views, sequences,
  routines, native types, defaults, comments, and composite keys.
- MySQL CI fixtures assert database discovery, routines, triggers, defaults,
  comments, generated columns, and grouped composite indexes/FKs.
- Every adapter test verifies scope pushdown and safe native qualification.

### UI and Input Tests

- Ratatui test-backend coverage at 80x24, 120x36, and 180x50.
- Root status text, tree indentation, counts, loading, stale, failed, and empty
  states.
- Column metadata priority and truncation.
- Connection forms, scope picker, and delete confirmation.
- Directional keyboard behavior, CRUD shortcuts, table-child open behavior,
  focus-on-click, double-click, and wheel viewport behavior.
- Relation Data/Structure views, offline snapshot labeling, errors, and zero-row
  results.

### Performance Gate

A synthetic catalog with at least 10,000 objects must keep movement, expansion,
collapse, selection lookup, and visible-row rendering indexed by stable IDs and
bounded by the visible subtree. The test should assert structural operation
counts or a generous non-flaky upper bound rather than a machine-specific tiny
timing threshold.

## Delivery Sequence

### Phase 1: Domain Contracts

- Add catalog scope, capabilities, request/response identity, stable Explorer
  identity, and typed metadata models.
- Replace profile scope persistence and update the profile file version.
- Add focused serialization, scope-default, identity, and tree-state tests.

### Phase 2: Adapter Catalog APIs

- Implement scope discovery and capability declarations for all three adapters.
- Implement level-based/paged catalog queries, counts, and structured metadata.
- Fix grouped composite index/FK handling while establishing truthful counts.
- Add real-backend fixture assertions before exposing capabilities in the UI.

### Phase 3: Connection-Root Explorer

- Render every profile as a root and map global active/pending connection state to
  per-root status.
- Replace positional tree selection and repeated flat scans with stable indexed
  state.
- Move profile list actions into Explorer and remove the Profile Manager list
  page while retaining form and confirmation overlays.
- Implement directional navigation, subtree refresh, focus-aware mouse behavior,
  and responsive row details.

### Phase 4: Scope Editing and Filtering

- Add scope discovery to Test Connection.
- Add the hierarchical scope picker and default derivation.
- Push scope into catalog requests and validate all returned entries.
- Rebuild completion only from active filtered catalog state.
- Implement filter-only hot reload with stale-request rejection.

### Phase 5: Relation Page

- Add relation tabs with Data and Structure views.
- Resolve relation ownership from table descendants and deduplicate open tabs.
- Move preview generation behind the adapter with a hard default limit of 500.
- Preserve zero-row column metadata and bind all completions to connection/tab
  identities.
- Add offline snapshot behavior, refresh, and cancellation.

### Phase 6: Hardening and Documentation

- Complete responsive UI, help, README, configuration, keybinding, architecture,
  and capability documentation.
- Run focused model, reducer, UI, adapter, integration, Clippy, format, and full
  regression checks.
- Enforce the 10,000-object Explorer performance gate.

## Acceptance Criteria

- Explorer lists every saved/session connection as a root and no separate
  connection-list popup remains.
- Every root has an unambiguous non-color-only connection state.
- The rendered hierarchy is connection, database with schema count, schema,
  capability-derived group with object count, object, and object children.
- New profile scope defaults to current database plus the configured default
  schema, or all schemas when no default is configured.
- Custom database/schema selections survive save and restart.
- Hidden catalog entries are neither queried unnecessarily nor exposed through
  Explorer or completion.
- PostgreSQL, MySQL, and SQLite render supported column type, default, generated,
  nullability, and comment metadata truthfully.
- Opening a table/view or any descendant creates or reuses one relation page,
  defaults to Data, and requests no more than 500 rows.
- Structure and preview results cannot cross profile, connection, request, or tab
  generations.
- A failed switch, scope discovery, catalog load, refresh, or preview preserves
  the prior usable state and offers a local retry path.
- Supported terminal sizes remain usable, and a 10,000-object catalog remains
  navigable without repeated full-vector scans.
