# Database Explorer Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the connection-list popup with a connection-root Explorer, add persisted database/schema visibility, lazy capability-aware catalogs, rich column metadata, and profile-scoped relation Data/Structure pages with a 500-row preview.

**Architecture:** Preserve `Action -> App::update -> Command -> Runtime -> DatabaseConnection` and the single active runtime connection. Add stable Explorer identities and an indexed per-profile catalog tree, keep presentation groups outside the database catalog domain, push explicit hierarchical scope into adapter-owned catalog requests, and bind every asynchronous catalog/relation result to connection, catalog, tab, and request generations.

**Tech Stack:** Rust 1.94, Ratatui 0.30, Crossterm 0.29, Tokio 1.47, SQLx 0.9 concrete PostgreSQL/MySQL/SQLite drivers, Serde/TOML, existing reducer and integration-test infrastructure.

---

## Execution Preconditions

- Read `docs/plans/2026-08-25-database-explorer-design.md` before starting.
- Execute in a dedicated worktree based on `ee3270e` or a later commit containing
  the current SQL editor and transaction work. Verify the chosen baseline and a
  clean worktree before implementation.
- Do not add compatibility behavior for version-1 connection files. Parse their version and reject them with an actionable error.
- Keep every intermediate commit compiling. Add new catalog APIs alongside the eager API, switch all consumers, and only then remove the eager path.
- Follow TDD for every task: focused failing test, observed failure, minimum implementation, focused green test, broader regression.
- Commit commands below are logical checkpoints. Run them only when commit authorization is present.
- Do not add a production dependency unless the existing standard library and current crates cannot implement the requirement.
- Never expose a driver capability before its query, stable identity, count, and integration test are implemented.
- Never put passwords, secret values, or full sensitive DSNs in tests, logs, actions, debug output, or snapshots.

## Shared Invariants

- One active database connection and pool remains authoritative.
- `CatalogKind` contains native database objects only; connection roots, groups,
  loading rows, errors, and pagination rows are Explorer presentation nodes.
- `CatalogSelection::All` is distinct from `Selected`. Empty `Selected` lists are
  rejected so a malformed custom selection cannot silently mean all or none.
- Persist exact native names. Do not lowercase PostgreSQL identifiers.
- Visibility affects catalog loading, Explorer, completion, and tool-generated
  object entry points. It is not an authorization boundary for user SQL.
- Every catalog response echoes profile, connection generation, catalog epoch,
  request ID, target, and cursor.
- Every relation response echoes profile, connection generation, tab generation,
  request ID, object ID, and request kind.
- Never append `LIMIT` to user-authored SQL. Only adapter-owned relation preview
  SQL uses the hard limit of 500.
- Display projections sanitize database text; identity and execution data remain
  raw.

### Task 1: Add the Hierarchical Catalog Scope Model

**Files:**
- Modify: `src/profile.rs:1-66` (`ConnectionProfile`, database-kind definitions)
- Create: `tests/catalog_scope.rs`

**Step 1: Write failing scope-default tests**

Create `tests/catalog_scope.rs` with focused tests for PostgreSQL defaults:

```rust
use lazydb::profile::{
    CatalogScope, CatalogSelection, DatabaseKind, DatabaseScope,
};

#[test]
fn scope_defaults_to_current_database_and_default_schema() {
    let scope = CatalogScope::for_profile(
        DatabaseKind::Postgres,
        "app",
        Some("public"),
    );

    assert_eq!(
        scope,
        CatalogScope {
            databases: CatalogSelection::Selected(vec![DatabaseScope {
                name: "app".to_owned(),
                schemas: CatalogSelection::Selected(vec!["public".to_owned()]),
            }]),
        }
    );
}

#[test]
fn missing_default_schema_selects_all_schemas() {
    let scope = CatalogScope::for_profile(DatabaseKind::Postgres, "app", None);
    let CatalogSelection::Selected(databases) = scope.databases else {
        panic!("current database must be selected")
    };
    assert_eq!(databases[0].schemas, CatalogSelection::All);
}
```

Add equivalent tests for MySQL's mirrored namespace and SQLite's configured
database with all discovered schemas.

**Step 2: Run the focused test and observe the expected failure**

Run: `cargo test --test catalog_scope scope_defaults -- --nocapture`

Expected: FAIL because `CatalogScope`, `CatalogSelection`, and `DatabaseScope` do
not exist.

**Step 3: Add the serializable domain types**

Add to `src/profile.rs`:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CatalogScope {
    pub databases: CatalogSelection<DatabaseScope>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DatabaseScope {
    pub name: String,
    pub schemas: CatalogSelection<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "items", rename_all = "snake_case")]
pub enum CatalogSelection<T> {
    All,
    Selected(Vec<T>),
}
```

Implement `CatalogScope::for_profile`, `allows_database`, `allows_schema`, and a
pure `validate` method. Validation must reject empty names, duplicate exact native
names, an empty `Selected`, and a configured `default_schema` excluded by scope.
MySQL canonicalizes its mirrored schema selection to `All`; the UI will render it
read-only later.

**Step 4: Add validation and serialization tests**

Cover:

- Different databases can select different schema sets.
- `All` and `Selected` serialize with explicit tags.
- `Selected(Vec::new())` is invalid at either level.
- Exact case-sensitive names round-trip unchanged.
- An excluded default schema is invalid.
- `allows_schema` never permits a schema below a rejected database.

**Step 5: Run focused scope tests**

Run: `cargo test --test catalog_scope -- --nocapture`

Expected: PASS.

**Step 6: Run profile unit tests**

Run: `cargo test --lib profile::tests -- --nocapture`

Expected: PASS. Task 1 is additive; Task 2 performs the profile-shape cutover.

**Step 7: Logical commit checkpoint**

```bash
git add src/profile.rs tests/catalog_scope.rs
git commit -m "feat(profiles): add hierarchical catalog scope"
```

### Task 2: Persist Version-2 Scope and Update Profile Drafts

**Files:**
- Modify: `src/profile.rs:38-60,116-247`
- Modify: `src/persistence/profiles.rs:14-109`
- Modify: `src/model/profile_manager.rs:147-245,360-460,544-568`
- Modify: `tests/persistence.rs`
- Modify: `tests/profile_draft.rs`
- Modify: `tests/startup_profiles.rs`

**Step 1: Write failing version and round-trip tests**

Add tests asserting:

```rust
#[test]
fn version_two_profiles_round_trip_hierarchical_scope() {
    // Build a profile with two database selections and distinct schema modes.
    // Save it, reload it, and compare the complete ConnectionProfile.
}

#[test]
fn version_one_is_rejected_before_profile_shape_decoding() {
    // Write a valid old version-1 file containing include_databases and
    // include_schemas. Assert UnsupportedVersion { found: 1, expected: 2 }.
}
```

Also assert that serialized TOML contains `catalog_scope` and contains neither
`include_databases` nor `include_schemas`.

**Step 2: Run persistence tests and observe failure**

Run: `cargo test --test persistence version_ -- --nocapture`

Expected: FAIL because the file version is still 1 and the old fields remain.

**Step 3: Parse the file header before decoding profiles**

In `src/persistence/profiles.rs`, set the current version to 2 and add:

```rust
#[derive(Deserialize)]
struct ProfileFileHeader {
    version: u16,
}

let header: ProfileFileHeader = toml::from_str(&contents)?;
if header.version != PROFILE_FILE_VERSION {
    return Err(ProfileStoreError::UnsupportedVersion {
        found: header.version,
        expected: PROFILE_FILE_VERSION,
    });
}
let file: ProfileFile = toml::from_str(&contents)?;
```

This ordering is required so an old profile missing `catalog_scope` reports an
unsupported version instead of a misleading field-decode error.

**Step 4: Replace the old profile fields**

Remove `include_databases` and `include_schemas` from `ConnectionProfile`. Add:

```rust
pub catalog_scope: CatalogScope,
```

Update PostgreSQL/MySQL URL imports and SQLite imports to call
`CatalogScope::for_profile`. Add `#[serde(deny_unknown_fields)]` to the version-2
profile shape after the header-first decode is in place.

**Step 5: Update `ProfileDraft` without exposing scope as comma-separated text**

Store a structured `catalog_scope` plus a discovery state in the draft. Do not
add schema lists to `TextInput`. `ProfileDraft::validate` must preserve the UUID,
scope, and credential intent while applying the pure scope validation from Task
1.

**Step 6: Rewrite draft tests**

Replace the old "preserves include arrays" assertion with:

- Editing preserves hierarchical scope.
- New profiles derive the approved defaults once.
- Changing unrelated fields does not reset a custom scope.
- Excluding the default schema reports a field-level validation error.
- Debug output contains no credential and does not dump discovered server data.

**Step 7: Run focused tests**

Run:

```bash
cargo test --test catalog_scope --test persistence --test profile_draft \
  --test startup_profiles -- --nocapture
```

Expected: PASS.

**Step 8: Run all profile lifecycle tests**

Run:

```bash
cargo test --test profile_runtime --test profile_reducer \
  --test profile_lifecycle -- --nocapture
```

Expected: PASS after fixture constructors use `catalog_scope`.

**Step 9: Logical commit checkpoint**

```bash
git add src/profile.rs src/persistence/profiles.rs \
  src/model/profile_manager.rs tests/catalog_scope.rs tests/persistence.rs \
  tests/profile_draft.rs tests/startup_profiles.rs tests/profile_runtime.rs \
  tests/profile_reducer.rs tests/profile_lifecycle.rs
git commit -m "feat(profiles): persist versioned catalog scope"
```

### Task 3: Add Neutral Connection Identity and Catalog Contracts

**Files:**
- Create: `src/identity.rs`
- Modify: `src/lib.rs`
- Modify: `src/model/workspace.rs:94-134`
- Modify: `src/db/catalog.rs`
- Create: `tests/catalog_contract.rs`
- Update exhaustive matches in: `src/sql/completion.rs`, `src/ui/mod.rs`

**Step 1: Write failing catalog-contract tests**

Cover these invariants in `tests/catalog_contract.rs`:

- A catalog object ID remains the same after reconnecting the same profile.
- Identical native paths in two profiles are different IDs.
- Materialized views are relations.
- Presentation groups are not `CatalogKind` values.
- Unsupported metadata differs from supported-but-absent metadata.
- Counts distinguish exact, lower-bound, and unknown values.
- A target encodes only a valid hierarchy level.
- A column can resolve its owning relation without path-offset arithmetic.

**Step 2: Run the contract tests and observe failure**

Run: `cargo test --test catalog_contract -- --nocapture`

Expected: FAIL on missing types and `MaterializedView`.

**Step 3: Move `ConnectionIdentity` to a neutral module**

Create `src/identity.rs`:

```rust
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ConnectionIdentity {
    pub profile_id: Uuid,
    pub generation: u64,
}
```

Export it from `src/lib.rs` and re-export it from `model::workspace` temporarily
so existing imports compile. Do not put a connection generation inside
`CatalogId`; IDs must survive reconnects.

**Step 4: Add additive catalog types**

Keep `CatalogNode` temporarily and add new contracts beside it:

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ObjectGroup {
    Tables,
    Views,
    MaterializedViews,
    Sequences,
    Functions,
    Procedures,
    Types,
    Triggers,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogCount {
    Exact(u64),
    AtLeast(u64),
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OptionalMetadata<T> {
    Unsupported,
    Supported(Option<T>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QualifiedName {
    pub database: Option<String>,
    pub schema: Option<String>,
    pub object: String,
}
```

Add `CatalogKind::MaterializedView`, `CatalogKind::is_relation`, structured
`ColumnMetadata`, grouped `IndexMetadata` and `ConstraintMetadata`,
`CatalogCapabilities`, `NamespaceModel`, `CatalogTarget`, `CatalogRequestKey`,
`CatalogPage`, `CatalogCursor`, and `CatalogCompleteness`.

**Step 5: Add the new `CatalogEntry` without removing `CatalogNode`**

`CatalogEntry` must hold stable ID, parent ID, kind, raw native kind, qualified
name, optional comment, typed metadata, and an expandable flag. Add explicit
constructors for databases, schemas, relations, and relation children. Constructors
must validate matching profile IDs and parent hierarchy.

**Step 6: Run focused and compile checks**

Run:

```bash
cargo test --test catalog_contract -- --nocapture
cargo test --lib db::catalog::tests -- --nocapture
cargo check --all-targets --all-features
```

Expected: PASS; the old eager path still compiles.

**Step 7: Logical commit checkpoint**

```bash
git add src/identity.rs src/lib.rs src/model/workspace.rs src/db/catalog.rs \
  src/sql/completion.rs src/ui/mod.rs tests/catalog_contract.rs
git commit -m "feat(catalog): add scoped catalog contracts"
```

### Task 4: Build the Indexed Stable-ID Explorer Model

**Files:**
- Create: `src/model/explorer.rs`
- Modify: `src/model/mod.rs`
- Modify: `src/model/workspace.rs:136-230`
- Create: `tests/explorer_state.rs`
- Create: `tests/explorer_performance.rs`

**Step 1: Write failing stable-tree tests**

Use fixed UUIDs and a synthetic two-profile tree. Assert:

- `profile_order` controls root order independently of hash-map order.
- Selection and expansion use stable IDs, not visible indices.
- Refresh preserves selected and expanded IDs that still exist.
- Removing a selected column falls back to its table, then schema, then profile.
- `expand`, `collapse`, and `move_to_parent` are distinct operations.
- Collapsed subtrees are not visited during visible projection.
- Empty, loading, retry, and load-more rows have deterministic synthetic IDs.
- Scrolling always keeps the selected visible row in the viewport.

**Step 2: Run focused tests and observe failure**

Run: `cargo test --test explorer_state -- --nocapture`

Expected: FAIL because `model::explorer` does not exist.

**Step 3: Add Explorer-owned identities and states**

Create types equivalent to:

```rust
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ExplorerNodeId {
    EmptyProfiles,
    Profile(Uuid),
    Catalog(CatalogId),
    Group { parent: CatalogId, group: ObjectGroup },
    Status { owner: ExplorerOwnerId, kind: StatusRowKind },
    LoadMore { parent: ExplorerOwnerId, cursor: CatalogCursor },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorerConnectionStatus {
    Offline,
    Linking,
    Online,
    Syncing,
    Failed,
}
```

Add `ExplorerState { profile_order, profiles, selected, expanded, scroll }` and
`ExplorerProfileState { status, catalog, catalog_epoch, next_request_id,
load_states, last_error, expand_after_connect }`.

**Step 4: Implement `CatalogTree` adjacency indexes**

Use maps for entries and parent-to-child vectors. Group state is keyed by
`(schema_id, ObjectGroup)`. Reject duplicate IDs and wrong-profile parentage.
Expose `get`, `parent`, `children`, `owning_relation`, `replace_page`, and
`remove_subtree`.

**Step 5: Implement visible projection and viewport updates**

Projection starts from `profile_order` and visits only expanded children. It must
not scan every entry to find each parent. Keep a test-only visit counter so the
performance test can assert structural complexity without flaky wall-clock
timings.

**Step 6: Add the 10,000-object structural test**

Build 100 collapsed schemas with 100 relations each. Expand one schema and assert
that projection visits the roots plus the expanded subtree, not all 10,000
relations. Then expand all and assert the exact visible count.

**Step 7: Run focused tests**

Run:

```bash
cargo test --test explorer_state -- --nocapture
cargo test --test explorer_performance -- --nocapture
cargo check --all-targets --all-features
```

Expected: PASS.

**Step 8: Logical commit checkpoint**

```bash
git add src/model/explorer.rs src/model/mod.rs src/model/workspace.rs \
  tests/explorer_state.rs tests/explorer_performance.rs
git commit -m "feat(explorer): add indexed stable tree state"
```

### Task 5: Add Truthful Capabilities and Scope Discovery

**Files:**
- Modify: `src/db/mod.rs:94-155`
- Modify: `src/db/postgres.rs`
- Modify: `src/db/mysql.rs`
- Modify: `src/db/sqlite.rs`
- Modify: `src/action.rs` profile-test variants
- Modify: `src/runtime.rs:261-308`
- Modify: `src/app.rs` profile-test reducer branches
- Modify: `src/model/profile_manager.rs`
- Modify: `tests/profile_runtime.rs`
- Modify: `tests/profile_reducer.rs`
- Modify: `tests/postgres_adapter.rs`
- Modify: `tests/mysql_adapter.rs`
- Modify: `tests/sqlite_adapter.rs`

**Step 1: Write failing pure capability tests**

Assert each adapter declares exactly the approved namespace model and groups. Do
not assert a group that its queries do not yet support; initially capabilities may
be a truthful subset and expand in Tasks 7-9.

**Step 2: Write failing profile-test discovery tests**

Cover:

- Opening/editing a form emits no network command.
- Test Connection probes and then discovers scope on the same temporary database.
- A successful probe plus discovery failure remains a test success with a warning.
- A stale test request cannot replace newer discovery.
- Editing connection-defining fields marks discovery stale but preserves scope.
- Discovery never mutates persisted scope automatically.

**Step 3: Run focused tests and observe failure**

Run:

```bash
cargo test --test profile_runtime profile_test -- --nocapture
cargo test --test profile_reducer profile_test -- --nocapture
```

Expected: FAIL because test success has no capability/discovery payload.

**Step 4: Add dispatch methods**

Add to `DatabaseConnection`:

```rust
pub fn catalog_capabilities(&self) -> CatalogCapabilities;

pub async fn discover_catalog_scope(
    &self,
) -> Result<CatalogDiscovery, DatabaseError>;
```

`CatalogDiscovery` contains ordered databases and schemas with stable native
names and a discovery fingerprint. PostgreSQL returns only `current_database()`
plus user-visible schemas. MySQL returns account-visible non-system databases.
SQLite returns the configured database and aliases from `PRAGMA database_list`.

**Step 5: Extend profile-test results**

Return server information, capabilities, and a separate discovery result. The
temporary connection closes after both probe and discovery. Store discovery only
in the current generation's draft state.

**Step 6: Make discovery invalidation explicit**

Compute a non-secret `DiscoveryFingerprint` from driver, endpoint, user,
database, SSL mode, SQLite path/memory mode, and credential-presence revision.
Changing any component marks discovery stale. Never hash or retain a password.

**Step 7: Run focused tests**

Run:

```bash
cargo test --test profile_runtime profile_test -- --nocapture
cargo test --test profile_reducer profile_test -- --nocapture
cargo test --test sqlite_adapter discovery -- --nocapture
cargo check --all-targets --all-features
```

Expected: PASS.

**Step 8: Logical commit checkpoint**

```bash
git add src/db/mod.rs src/db/postgres.rs src/db/mysql.rs src/db/sqlite.rs \
  src/action.rs src/runtime.rs src/app.rs src/model/profile_manager.rs \
  tests/profile_runtime.rs tests/profile_reducer.rs \
  tests/postgres_adapter.rs tests/mysql_adapter.rs tests/sqlite_adapter.rs
git commit -m "feat(catalog): discover driver catalog scopes"
```

### Task 6: Add the Paged Catalog API Beside the Eager API

**Files:**
- Modify: `src/db/catalog.rs`
- Modify: `src/db/mod.rs:149-176`
- Modify: `src/db/postgres.rs`
- Modify: `src/db/mysql.rs`
- Modify: `src/db/sqlite.rs`
- Expand: `tests/catalog_contract.rs`

**Step 1: Write failing request-validation tests**

Assert:

- Page size must be `1..=MAX_CATALOG_PAGE_SIZE`.
- A page echoes the complete request key.
- A page cannot contain wrong-profile or out-of-scope entries.
- Unsupported targets return an explicit error, not an empty page.
- First and final pages report `Partial` and `Complete` correctly.
- Applying a page with a mismatched target, cursor, or request ID is invalid.

**Step 2: Run contract tests and observe failure**

Run: `cargo test --test catalog_contract page_ -- --nocapture`

Expected: FAIL on missing validation and dispatch.

**Step 3: Define the exact request key**

Use:

```rust
pub struct CatalogRequestKey {
    pub connection: ConnectionIdentity,
    pub catalog_epoch: u64,
    pub request_id: u64,
    pub target: CatalogTarget,
    pub cursor: Option<CatalogCursor>,
}

pub struct CatalogRequest {
    pub key: CatalogRequestKey,
    pub scope: CatalogScope,
    pub page_size: usize,
}
```

Targets are `Databases`, `Schemas { database }`, `Groups { schema }`,
`Objects { schema, group }`, and `RelationChildren { relation }`. Invalid
parent/group combinations must not be representable.

**Step 4: Add an additive adapter method**

```rust
pub async fn load_catalog_page(
    &self,
    request: &CatalogRequest,
) -> Result<CatalogPage, DatabaseError>;
```

Keep `load_catalog()` active until Task 10. Stub unsupported targets with typed
errors while each driver is migrated.

**Step 5: Implement keyset pagination helpers**

Use `LIMIT page_size + 1`, retain `page_size`, and derive an opaque cursor from
the last retained row's normalized sort key plus stable native tie-breaker. Do
not use offset pagination.

**Step 6: Run focused tests and checks**

Run:

```bash
cargo test --test catalog_contract -- --nocapture
cargo check --all-targets --all-features
```

Expected: PASS with the eager runtime still unchanged.

**Step 7: Logical commit checkpoint**

```bash
git add src/db/catalog.rs src/db/mod.rs src/db/postgres.rs \
  src/db/mysql.rs src/db/sqlite.rs tests/catalog_contract.rs
git commit -m "feat(catalog): add paged catalog requests"
```

### Task 7: Implement SQLite Catalog Pages and Rich Metadata

**Files:**
- Modify: `src/db/sqlite.rs`
- Modify: `tests/sqlite_adapter.rs`

**Step 1: Write a complete failing SQLite fixture**

Create a temporary database containing multiple tables, a view, a trigger,
defaults, a generated column, a composite key, a composite foreign key, a
multi-column index, and more than one table for page-size-one cursor tests. Attach
a second database and create an excluded object there.

Assert scope discovery, filtered schemas, exact group counts, stable repeated IDs,
keyset pages without duplicates, defaults/generated/hidden metadata, one grouped
index per native index, one grouped FK per native FK, and schema-qualified DDL.

**Step 2: Run the SQLite catalog tests and observe failure**

Run: `cargo test --test sqlite_adapter catalog_page -- --nocapture`

Expected: FAIL because SQLite implements only the eager `main` catalog.

**Step 3: Make SQLite connection-local namespace state truthful**

Set the SQLite pool to one physical connection before advertising `temp` and
attached aliases. `PRAGMA database_list`, `ATTACH`, and `temp` are connection
local; a four-connection pool can return a schema from one connection and query a
different connection that does not know it.

Add a regression test proving an alias attached through normal execution is
visible to discovery and catalog loading.

**Step 4: Implement database, schema, and group pages**

Discover aliases with `PRAGMA database_list`. Prove every interpolated schema
identifier came from that discovery and passed scope validation, then quote it.
Query each alias's `sqlite_schema` with binary keyset ordering. Return only Tables,
Views, and Triggers at schema group level.

**Step 5: Implement relation children**

Use schema-aware table-valued pragmas:

```sql
SELECT cid, name, type, "notnull" AS not_null,
       dflt_value, pk, hidden
FROM pragma_table_xinfo(?, ?)
ORDER BY cid;
```

Group index parts by native index identity. Group FK rows by FK `id`, ordered by
`seq`; do not create one object per column. Represent comments as
`OptionalMetadata::Unsupported`.

**Step 6: Run SQLite tests**

Run:

```bash
cargo test --test sqlite_adapter catalog_ -- --nocapture
cargo test --test sqlite_adapter -- --nocapture
cargo test --lib db::sqlite::tests -- --nocapture
```

Expected: PASS.

**Step 7: Logical commit checkpoint**

```bash
git add src/db/sqlite.rs tests/sqlite_adapter.rs
git commit -m "feat(sqlite): add scoped lazy catalog metadata"
```

### Task 8: Implement PostgreSQL Catalog Pages and Rich Metadata

**Files:**
- Modify: `src/db/postgres.rs`
- Modify: `tests/postgres_adapter.rs`

**Step 1: Expand the environment-gated fixture**

Create a UUID-named schema containing tables, a view, materialized view, sequence,
function, procedure, enum/domain type, identity/generated/default/commented
columns, composite PK/unique/FK/check constraints, and at least two same-group
objects. Create a second excluded schema. Clean up with `DROP SCHEMA ... CASCADE`.

**Step 2: Run the focused live test and observe failure**

Run:

```bash
LAZYDB_TEST_POSTGRES_URL='postgresql://user:password@localhost:5432/database' \
  cargo test --test postgres_adapter catalog_page -- --nocapture
```

Expected: FAIL on absent groups, metadata, counts, and scope pushdown. If the env
variable is unavailable locally, the test must still compile and CI supplies the
real execution evidence.

**Step 3: Implement PostgreSQL hierarchy and keyset ordering**

Return only `current_database()` at database level. Exclude
`information_schema` and `pg_%` schemas. Use `pg_class.relkind` for tables, views,
materialized views, and sequences; `pg_proc.prokind` for functions/procedures;
and stable OIDs/signatures as native tie-breakers. Use deterministic `COLLATE "C"`
ordering where names participate in cursors.

**Step 4: Implement column metadata**

Use `pg_attribute`, `format_type`, `pg_attrdef`, and `col_description` so custom
types, defaults, identity/generated flags, and comments remain accurate. Separate
a generated expression from a default using `attgenerated`.

**Step 5: Fix composite constraints while grouping native objects**

Use `pg_constraint.conkey/confkey` with matching ordinality. Group rows by
constraint OID. Do not retain the current information-schema join that can
cross-product composite FK columns.

**Step 6: Expand capabilities only after each group passes**

Advertise Tables, Views, Materialized Views, Sequences, Functions, Procedures,
and Types only when the corresponding page/count test is green.

**Step 7: Run PostgreSQL and compile tests**

Run:

```bash
cargo test --test postgres_adapter quotes_ -- --nocapture
LAZYDB_TEST_POSTGRES_URL='postgresql://user:password@localhost:5432/database' \
  cargo test --test postgres_adapter -- --nocapture
cargo check --all-targets --all-features
```

Expected: PASS.

**Step 8: Logical commit checkpoint**

```bash
git add src/db/postgres.rs tests/postgres_adapter.rs
git commit -m "feat(postgres): add scoped lazy catalog metadata"
```

### Task 9: Implement MySQL Catalog Pages and Rich Metadata

**Files:**
- Modify: `src/db/mysql.rs`
- Modify: `tests/mysql_adapter.rs`

**Step 1: Expand the environment-gated fixture**

Create two UUID-named databases when the CI account permits it. In the selected
database create tables, a view, function, procedure, trigger, auto-increment
column, default, comment, generated column, composite PK/unique/index/FK, and at
least two same-group objects. Clean both databases after the test.

**Step 2: Run the focused live test and observe failure**

Run:

```bash
LAZYDB_TEST_MYSQL_URL='mysql://user:password@localhost:3306/database' \
  cargo test --test mysql_adapter catalog_page -- --nocapture
```

Expected: FAIL because current queries are fixed to `DATABASE()` and composite
objects are emitted per component.

**Step 3: Implement database discovery and mirrored schemas**

Read visible databases from `information_schema.schemata`, excluding
`information_schema`, `mysql`, `performance_schema`, and `sys`. Bind the selected
database in every object/count query. Emit a derived same-name schema node and
canonicalize its scope to `All`.

**Step 4: Implement deterministic pages and group counts**

Use `BINARY` name comparisons/order for keyset cursors because information-schema
collations are commonly case-insensitive. Add schema-level Tables, Views,
Functions, Procedures, and Triggers.

**Step 5: Implement column metadata and grouped constraints**

Select `column_type`, nullability, `column_default`, `extra`,
`generation_expression`, `column_comment`, lengths, precision/scale, character
set, and collation. Treat `DEFAULT_GENERATED` separately from generated columns.
Group indexes by `(schema, table, index_name)` and FKs by constraint identity,
ordered by ordinal position.

**Step 6: Run MySQL and compile tests**

Run:

```bash
cargo test --test mysql_adapter quotes_ -- --nocapture
LAZYDB_TEST_MYSQL_URL='mysql://user:password@localhost:3306/database' \
  cargo test --test mysql_adapter -- --nocapture
cargo check --all-targets --all-features
```

Expected: PASS.

**Step 7: Logical commit checkpoint**

```bash
git add src/db/mysql.rs tests/mysql_adapter.rs
git commit -m "feat(mysql): add scoped lazy catalog metadata"
```

### Task 10: Replace Eager Runtime Catalog Loading

**Files:**
- Modify: `src/action.rs` catalog actions and commands
- Modify: `src/runtime.rs:150-258,534-567`
- Modify: `src/app.rs:713-829`
- Modify: `src/model/explorer.rs`
- Modify: `src/sql/completion.rs`
- Create: `tests/catalog_reducer.rs`
- Modify: `tests/connection_switch.rs`
- Modify: `tests/app_flow.rs`
- Modify: `tests/sql_completion.rs`

**Step 1: Write failing reducer request-identity tests**

Feed actions directly in deterministic order. Assert:

- Same connection/target request 2 wins over a late request 1.
- Wrong profile or connection generation is ignored.
- Wrong epoch, target, cursor, or request ID is ignored.
- A failed refresh retains the old subtree and marks it stale.
- First-load failure creates a local retry row, not a connection failure.
- Accepted next pages append stable IDs without duplicates.
- Selection/expansion survive a replacement containing the same IDs.

**Step 2: Run reducer tests and observe failure**

Run: `cargo test --test catalog_reducer -- --nocapture`

Expected: FAIL because current catalog actions carry only connection generation.

**Step 3: Add semantic actions and commands**

Add equivalents of:

```rust
Action::ExplorerLoadTarget(CatalogTarget)
Action::CatalogPageLoaded(CatalogPage)
Action::CatalogPageFailed {
    key: CatalogRequestKey,
    category: ErrorCategory,
    message: String,
}

Command::LoadCatalogPage(CatalogRequest)
```

The reducer allocates checked monotonically increasing request IDs and catalog
epochs. Runtime verifies the active connection identity before adapter dispatch
and echoes the request key unchanged.

**Step 4: Keep old data visible during refresh**

Represent initial load, refresh-with-previous, loaded, stale, and failed states
per target. Never clear a valid subtree before a replacement succeeds.

**Step 5: Load completion summaries independently of expansion**

After accepted schema/group discovery, schedule paged in-scope object summaries
for completion even if the user leaves the group collapsed. Add relation columns
to completion when relation details load. This avoids making completion depend on
which Explorer nodes the user happened to open.

Hidden/out-of-scope entries must fail the pure page validator before they enter
either `CatalogTree` or `CompletionIndex`.

**Step 6: Switch connection startup and refresh to lazy targets**

Connection success requests `Databases`. Expanding nodes requests their precise
target. `r` refreshes the selected target and increments its request ID; a profile
scope replacement increments the whole catalog epoch.

**Step 7: Remove the eager API after every consumer migrates**

Delete `DatabaseConnection::load_catalog`, the three eager loaders, old
`CatalogLoaded` actions, and the flat `ExplorerState::set_nodes` flow only after
the new app/runtime tests are green.

**Step 8: Run focused regressions**

Run:

```bash
cargo test --test catalog_reducer --test connection_switch \
  --test app_flow --test sql_completion -- --nocapture
cargo check --all-targets --all-features
```

Expected: PASS.

**Step 9: Logical commit checkpoint**

```bash
git add src/action.rs src/runtime.rs src/app.rs src/model/explorer.rs \
  src/sql/completion.rs src/db/mod.rs src/db/postgres.rs src/db/mysql.rs \
  src/db/sqlite.rs tests/catalog_reducer.rs tests/connection_switch.rs \
  tests/app_flow.rs tests/sql_completion.rs
git commit -m "feat(catalog): load scoped explorer pages safely"
```

### Task 11: Make Connection Profiles Explorer Roots

**Files:**
- Modify: `src/app.rs`
- Modify: `src/runtime.rs` startup initialization
- Modify: `src/action.rs` profile actions
- Modify: `src/model/profile_manager.rs`
- Modify: `src/ui/profiles.rs`
- Modify: `tests/startup_profiles.rs`
- Modify: `tests/profile_reducer.rs`
- Modify: `tests/connection_switch.rs`
- Modify: `tests/profile_lifecycle.rs`

**Step 1: Write failing profile-root lifecycle tests**

Assert:

- Every saved profile becomes one ordered root.
- An ad-hoc URL becomes a `SESSION` root.
- No-profile startup selects an actionable empty row and does not open a list
  overlay.
- The startup target is `LINKING`; others are `OFFLINE`.
- During a safe switch, old root remains `ONLINE` and target is `LINKING`.
- Failed switching marks only the target `FAILED`.
- Successful switching clears/collapses the old root and activates the target.
- Deleting/disconnecting the active profile selects the nearest remaining root.

**Step 2: Run focused tests and observe failure**

Run:

```bash
cargo test --test startup_profiles --test connection_switch \
  profile_root -- --nocapture
```

Expected: FAIL because profiles are still rendered only by the manager list.

**Step 3: Make profile actions UUID-targeted**

Replace manager-selection-dependent actions with payload-bearing actions such as:

```rust
ProfileStartEdit { profile_id: Uuid }
ProfileRequestDelete { profile_id: Uuid }
RequestProfileConnect { profile_id: Uuid }
RequestProfileDisconnect { profile_id: Uuid }
```

All reducer/runtime paths must use the payload, never a mutable list index.

**Step 4: Remove `ProfileManagerPage::List`**

Keep Form, Scope, and ConfirmDelete states. Remove list rendering, list keymaps,
list hit targets, and positional selection. Opening the overlay without a target
means New; opening with a profile ID means Edit.

**Step 5: Initialize and synchronize Explorer roots**

Startup, save, and delete results update `profile_order` and profile root state
only after persistence succeeds. Derive `Saved` versus `Session` from the runtime
registry's persisted IDs.

**Step 6: Map global connection state to per-root status**

Reserve root `FAILED` for connection or credential failures. Catalog failures are
local status rows below the relevant node. `SYNCING` means the active root is
loading catalog data while its pool is valid.

**Step 7: Run profile regressions**

Run:

```bash
cargo test --test startup_profiles --test profile_reducer \
  --test connection_switch --test profile_lifecycle -- --nocapture
```

Expected: PASS with rewritten tests that no longer expect a profile list page.

**Step 8: Logical commit checkpoint**

```bash
git add src/app.rs src/runtime.rs src/action.rs \
  src/model/profile_manager.rs src/ui/profiles.rs \
  tests/startup_profiles.rs tests/profile_reducer.rs \
  tests/connection_switch.rs tests/profile_lifecycle.rs
git commit -m "feat(explorer): show connections as root nodes"
```

### Task 12: Add the Scope Picker and Filter-Only Hot Reload

**Files:**
- Modify: `src/model/profile_manager.rs`
- Modify: `src/ui/profiles.rs`
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/runtime.rs` profile-save result payload
- Modify: `tests/profile_draft.rs`
- Modify: `tests/profile_reducer.rs`
- Modify: `tests/profile_runtime.rs`
- Modify: `tests/ui_render.rs`
- Modify: `tests/keymap.rs`
- Modify: `tests/mouse.rs`

**Step 1: Write failing scope-picker model tests**

Cover:

- The form has one `VisibleObjects` field with a stable summary.
- Entering it opens a hierarchical picker, not a comma-separated text input.
- Database and schema rows use stable native names.
- `All schemas` is mutually exclusive with individual schemas.
- MySQL's mirrored schema row is selected and read-only.
- Saved unavailable names remain visible with a warning.
- A stale discovery cannot silently clear a custom selection.

**Step 2: Run focused tests and observe failure**

Run:

```bash
cargo test --test profile_draft scope -- --nocapture
cargo test --test profile_reducer scope -- --nocapture
```

Expected: FAIL because no scope-picker page or field exists.

**Step 3: Add pure picker state**

Add `ProfileManagerPage::Scope`, a stable selected picker row, expanded database
set, viewport, current `CatalogScope`, discovered options, freshness state, and
warning. Reuse the draft's request generation for discovery results.

**Step 4: Render and map the picker**

Add database and schema checkbox rows, `All schemas`, stale/unavailable markers,
and Test/Refresh guidance. Generic form scrolling must continue to work at compact
sizes. Mouse hit targets carry stable picker IDs.

**Step 5: Classify successful profile changes in Runtime**

Return a non-secret change summary:

```rust
pub struct ProfileChange {
    pub connection_settings_changed: bool,
    pub catalog_scope_changed: bool,
    pub display_only_changed: bool,
}
```

Runtime compares prior and saved profiles after credential handling. App must not
infer credential change after `CredentialUpdate` has been consumed.

**Step 6: Implement filter-only hot reload**

When only scope changes on the active profile:

- Keep the active connection.
- Increment catalog epoch.
- Clear active completion immediately.
- Retain old Explorer subtree as stale until the filtered replacement arrives.
- Never call keyring get/set/delete.

Connection-affecting fields retain Save versus Save & Connect behavior.

**Step 7: Add hot-reload and secret-store assertions**

Use the existing fake secret-store call log. Assert scope-only save persists,
emits catalog reload, emits no disconnect/reconnect, and performs zero secret
operations.

**Step 8: Run focused tests**

Run:

```bash
cargo test --test profile_draft --test profile_reducer \
  --test profile_runtime --test ui_render --test keymap --test mouse \
  scope -- --nocapture
```

Expected: PASS.

**Step 9: Logical commit checkpoint**

```bash
git add src/model/profile_manager.rs src/ui/profiles.rs src/action.rs \
  src/app.rs src/runtime.rs tests/profile_draft.rs tests/profile_reducer.rs \
  tests/profile_runtime.rs tests/ui_render.rs tests/keymap.rs tests/mouse.rs
git commit -m "feat(profiles): edit visible catalog scope"
```

### Task 13: Render and Navigate the New Explorer

**Files:**
- Modify: `src/ui/mod.rs` Explorer renderer and hit targets
- Modify: `src/ui/layout.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/input/mouse.rs`
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `tests/ui_render.rs`
- Modify: `tests/keymap.rs`
- Modify: `tests/mouse.rs`
- Modify: `tests/explorer_state.rs`

**Step 1: Write failing directional-navigation tests**

Assert `l/Right` expands, `h/Left` collapses or selects the parent, `Enter`
executes the node's primary action, `r` refreshes one subtree, and
`n/e/d/c/x` operate on the owning profile. `Space c` and header profile click now
focus/select the active Explorer root instead of opening a list popup.

**Step 2: Write failing mouse tests**

Assert one click focuses Explorer and selects a stable node ID, wheel movement
updates the viewport while keeping selection visible, and two clicks on the same
node within 500 ms emit the same primary action as Enter. Test the click tracker
with injected timestamps; do not sleep.

**Step 3: Write failing render tests at all target sizes**

At 80x24, 120x36, and 180x50 assert:

- `[PG] name ONLINE`, `[MY] name OFFLINE`, and failed text render without color.
- Database rows show schema counts.
- Group rows show exact, partial, or loading counts.
- Column rows prioritize name, type, flags, default, then comment.
- Narrow rows truncate comment/default first and show complete selected metadata
  in the Explorer detail area.
- Loading, retry, stale, empty, and permission states render locally.

**Step 4: Run focused tests and observe failure**

Run:

```bash
cargo test --test keymap explorer -- --nocapture
cargo test --test mouse explorer -- --nocapture
cargo test --test ui_render explorer -- --nocapture
```

Expected: FAIL on old toggle-only and positional behavior.

**Step 5: Implement semantic navigation and CRUD mapping**

Keymap emits expand, collapse/parent, open, refresh-target, and UUID-targeted
profile actions. App performs connection guards before expanding an offline root
and uses `expand_after_connect` to finish the navigation after success.

**Step 6: Implement adaptive Explorer layout and row formatting**

For split layouts use approximately one third of terminal width, clamped to a
safe range such as 34-56 columns. Keep focus mode full width. Render metadata as
separate spans so priority truncation is deterministic and control sequences are
sanitized.

**Step 7: Implement stable hit targets and double-click tracking**

Store the transient click tracker in `UiState`, not `App`. It contains node ID and
timestamp only. Clear it when the target changes or the timeout elapses.

**Step 8: Run focused UI/input tests**

Run:

```bash
cargo test --test explorer_state --test keymap --test mouse \
  --test ui_render -- --nocapture
```

Expected: PASS.

**Step 9: Logical commit checkpoint**

```bash
git add src/ui/mod.rs src/ui/layout.rs src/input/keymap.rs \
  src/input/mouse.rs src/action.rs src/app.rs tests/ui_render.rs \
  tests/keymap.rs tests/mouse.rs tests/explorer_state.rs
git commit -m "feat(explorer): render and navigate connection catalog tree"
```

### Task 14: Introduce Heterogeneous Workspace Tabs

**Files:**
- Modify: `src/model/tab.rs`
- Create: `src/model/relation.rs`
- Modify: `src/model/mod.rs`
- Modify: `src/app.rs` tab helpers and console-only loops
- Modify: `src/ui/mod.rs` common tab-bar access
- Modify: `src/input/keymap.rs` active-tab identity
- Create: `tests/workspace_tabs.rs`

**Step 1: Write failing mixed-tab model tests**

Assert:

- Every tab exposes common `id`, `title`, and kind.
- Console-only operations return `None` on relation tabs instead of panicking.
- Closing a relation tab never enters transaction-exit logic.
- Tab cycling works across mixed kinds.
- Editor focus normalizes to relation content when activating a relation tab.

**Step 2: Run tests and observe failure**

Run: `cargo test --test workspace_tabs -- --nocapture`

Expected: FAIL because `App.tabs` is `Vec<ConsoleTab>`.

**Step 3: Add `WorkspaceTab` and shared grid state**

Add:

```rust
pub enum WorkspaceTab {
    Sql(ConsoleTab),
    Relation(RelationTab),
}

impl WorkspaceTab {
    pub fn id(&self) -> Uuid { /* exhaustive match */ }
    pub fn title(&self) -> &str { /* exhaustive match */ }
    pub fn as_console(&self) -> Option<&ConsoleTab> { /* match */ }
    pub fn as_console_mut(&mut self) -> Option<&mut ConsoleTab> { /* match */ }
}
```

Extract result-cell selection into a reusable `GridState`. Wrap every existing
console in `WorkspaceTab::Sql` without changing SQL behavior yet.

**Step 4: Replace panic-prone active-console assumptions**

`active_console` and `active_console_mut` return `Option`. Transaction, editor,
completion, confirmation, and query paths explicitly no-op or report the correct
context when a relation tab is active.

**Step 5: Run existing editor, transaction, and tab tests**

Run:

```bash
cargo test --test workspace_tabs -- --nocapture
cargo test --test sql_execution --test transaction_reducer \
  --test app_flow -- --nocapture
cargo check --all-targets --all-features
```

Expected: PASS with no relation behavior exposed yet.

**Step 6: Logical commit checkpoint**

```bash
git add src/model/tab.rs src/model/relation.rs src/model/mod.rs src/app.rs \
  src/ui/mod.rs src/input/keymap.rs tests/workspace_tabs.rs \
  tests/sql_execution.rs tests/transaction_reducer.rs tests/app_flow.rs
git commit -m "refactor(tabs): support heterogeneous workspace tabs"
```

### Task 15: Add Relation State and Open/Deduplicate Semantics

**Files:**
- Modify: `src/model/relation.rs`
- Modify: `src/model/tab.rs`
- Modify: `src/model/explorer.rs`
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Create: `tests/relation_tabs.rs`

**Step 1: Write failing relation-ancestor tests**

Test a table/view itself and each supported descendant: column, index, primary
key, unique key, foreign key, check constraint, and trigger. Missing parents and
cycles return `None` without hanging.

**Step 2: Write failing open/deduplication tests**

Assert:

- Enter or `o` on a table/view/descendant opens Data.
- `p` opens Data and `D` opens Structure.
- Same `(profile_id, object_id)` focuses the existing relation tab.
- Same native path in two profiles creates distinct tabs.
- Opening does not mutate any SQL console document.
- Reopening a ready view does not refetch automatically.

**Step 3: Run tests and observe failure**

Run: `cargo test --test relation_tabs opening -- --nocapture`

Expected: FAIL because no relation model/open action exists.

**Step 4: Add relation identity and load states**

```rust
pub struct RelationKey {
    pub profile_id: Uuid,
    pub object_id: CatalogId,
}

pub enum RelationView {
    Data,
    Structure,
}

pub enum RelationLoad<T> {
    Empty,
    Loading { request: RelationRequest, previous: Option<OwnedSnapshot<T>> },
    Ready(OwnedSnapshot<T>),
    Failed { message: String, previous: Option<OwnedSnapshot<T>> },
    Cancelled { previous: Option<OwnedSnapshot<T>> },
}
```

`RelationDescriptor` stores raw profile/object identity, qualified name, kind, and
title so a tab remains attributable after the Explorer catalog is dropped.

**Step 5: Add semantic open actions**

Replace old preview behavior with `ExplorerOpenSelected`,
`OpenSelectedRelation { view }`, and `SetRelationView`. App resolves the owning
relation through `CatalogTree`, focuses/creates the tab, and records missing Data
and Structure loads without generating SQL in the reducer.

**Step 6: Remove SQL-console preview tab creation**

Delete the behavior that creates a console titled `<name> data`, inserts a
loading comment, and later replaces editor text with generated SQL. Do not remove
runtime `PreviewTable` until Task 17 switches transport.

**Step 7: Run relation and app-flow model tests**

Run:

```bash
cargo test --test relation_tabs -- --nocapture
cargo test --test app_flow relation -- --nocapture
```

Expected: relation model/open tests PASS; adapter/runtime requests remain pending
until Tasks 16-17.

**Step 8: Logical commit checkpoint**

```bash
git add src/model/relation.rs src/model/tab.rs src/model/explorer.rs \
  src/action.rs src/app.rs tests/relation_tabs.rs tests/app_flow.rs
git commit -m "feat(relations): open deduplicated relation tabs"
```

### Task 16: Add Adapter-Owned Relation Preview and Structure

**Files:**
- Modify: `src/db/query.rs`
- Modify: `src/db/catalog.rs`
- Modify: `src/db/mod.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/db/mysql.rs`
- Modify: `src/db/sqlite.rs`
- Modify: `tests/sqlite_adapter.rs`
- Modify: `tests/postgres_adapter.rs`
- Modify: `tests/mysql_adapter.rs`

**Step 1: Write failing SQLite relation-preview tests**

Create an empty table and a 501-row table. Assert:

- Empty preview retains column names and native types.
- Preview returns exactly 500 rows from the larger table.
- Generated SQL contains a literal adapter-owned `LIMIT 500`.
- Hostile identifiers are safely quoted.
- Views are supported.
- Foreign-profile and non-relation IDs are rejected.

**Step 2: Run tests and observe failure**

Run: `cargo test --test sqlite_adapter relation_preview -- --nocapture`

Expected: FAIL because runtime currently creates preview SQL and empty results
lose metadata.

**Step 3: Add relation result contracts**

Add `RELATION_PREVIEW_LIMIT: usize = 500`, `RelationPreview { sql, result, stats }`,
typed `RelationStructure`, and DDL provenance to database-owned models.

**Step 4: Add adapter dispatch methods**

```rust
pub async fn preview_relation(
    &self,
    relation: &CatalogId,
) -> Result<RelationPreview, DatabaseError>;

pub async fn relation_structure(
    &self,
    relation: &CatalogId,
) -> Result<RelationStructure, DatabaseError>;
```

Each adapter validates profile, relation kind, qualification, and active scope.
The caller never supplies schema/name strings or a limit.

**Step 5: Preserve zero-row columns from prepared statements**

Use SQLx 0.9's explicit prepare API before fetching. The locked local SQLx source
documents that `Executor::prepare` exposes statement metadata before the first row
and `Statement::columns()` returns expected columns. In each concrete adapter:

```rust
use sqlx::{AssertSqlSafe, Executor, SqlSafeStr, Statement};

let mut connection = pool.acquire().await?;
let statement = (&mut *connection)
    .prepare(AssertSqlSafe(sql.clone()).into_sql_str())
    .await?;
let columns = statement.columns().iter().map(column_meta).collect();
let rows = statement.query().fetch_all(&mut *connection).await?;
```

Build `ResultSet` from prepared columns even when `rows` is empty. The generated
SQL is safe here only because every dynamic identifier has passed adapter-owned
qualification and quoting; do not weaken `AssertSqlSafe` handling.

**Step 6: Implement native qualification**

- PostgreSQL: `"schema"."relation"`.
- MySQL: `` `database`.`relation` ``.
- SQLite: `"attached_alias"."relation"`.

Use the structured qualified name from the stable catalog entry, not path offsets.

**Step 7: Implement Structure from typed catalog metadata**

Return relation summary, columns, grouped indexes/keys/constraints/triggers, and
available DDL with provenance. Do not rebuild structure as display strings.

**Step 8: Run adapter tests**

Run:

```bash
cargo test --test sqlite_adapter relation_ -- --nocapture
LAZYDB_TEST_POSTGRES_URL='postgresql://user:password@localhost:5432/database' \
  cargo test --test postgres_adapter relation_ -- --nocapture
LAZYDB_TEST_MYSQL_URL='mysql://user:password@localhost:3306/database' \
  cargo test --test mysql_adapter relation_ -- --nocapture
```

Expected: PASS; PostgreSQL/MySQL may skip without env vars but run in CI.

**Step 9: Logical commit checkpoint**

```bash
git add src/db/query.rs src/db/catalog.rs src/db/mod.rs src/db/postgres.rs \
  src/db/mysql.rs src/db/sqlite.rs tests/sqlite_adapter.rs \
  tests/postgres_adapter.rs tests/mysql_adapter.rs
git commit -m "feat(relations): add adapter-owned data and structure loading"
```

### Task 17: Transport Relation Requests Safely

**Files:**
- Modify: `src/action.rs`
- Modify: `src/runtime.rs`
- Modify: `src/app.rs`
- Modify: `src/model/relation.rs`
- Create: `tests/relation_runtime.rs`
- Expand: `tests/relation_tabs.rs`
- Modify: `tests/connection_switch.rs`
- Modify: `tests/profile_reducer.rs`
- Modify: `tests/profile_lifecycle.rs`

**Step 1: Write the complete stale-response matrix**

Start from one valid pending request and mutate one field at a time: profile ID,
connection generation, tab ID, tab generation, request ID, relation object ID,
and request kind. Every changed result must leave pending and previous snapshot
state untouched.

**Step 2: Write cancellation and switch tests**

Assert:

- Cancel request N cannot cancel N+1.
- Closing a relation tab cancels all its pending requests.
- Successful switch cancels old requests and preserves completed snapshots.
- Failed switch keeps old relation tabs live.
- Reconnecting the owner keeps old-generation data labeled snapshot until refresh.
- Hidden/deleted-profile tabs cannot refresh.
- A loading Data preview participates in running-query switch/delete guards.

**Step 3: Run tests and observe failure**

Run:

```bash
cargo test --test relation_runtime -- --nocapture
cargo test --test relation_tabs stale -- --nocapture
```

Expected: FAIL because old preview completions lack complete identity.

**Step 4: Define complete relation requests**

```rust
pub struct RelationRequest {
    pub tab_id: Uuid,
    pub tab_generation: u64,
    pub request_id: u64,
    pub connection: ConnectionIdentity,
    pub relation: RelationKey,
    pub kind: RelationRequestKind,
}
```

Add `LoadRelationPreview`, `LoadRelationStructure`, and exact
`CancelRelationRequest` commands. Success/failure actions echo the unchanged
request.

**Step 5: Track runtime tasks by exact request**

Use `HashMap<RelationRequest, JoinHandle<()>>`. Runtime verifies active identity
and relation ownership before dispatch. Shutdown aborts all relation tasks.

**Step 6: Apply results only through one acceptance predicate**

A result applies only if tab ID/generation, pending request ID/kind, relation key,
and current active connection all match. Clear pending ownership before emitting a
cancel command so a racing completion is ignored.

**Step 7: Implement snapshot provenance states**

Derive `LIVE`, `OFFLINE SNAPSHOT`, `PROFILE DELETED SNAPSHOT`, and
`OUT OF SCOPE SNAPSHOT` from immutable snapshot identity and current profile/
scope state. Never toggle a mutable "offline" flag that can relabel old data.

**Step 8: Remove old preview/DDL transport**

Delete `PreviewTable`, `PreviewFinished`, and relation use of schema/name-based DDL
commands after all tests use the relation request path.

**Step 9: Run focused safety tests**

Run:

```bash
cargo test --test relation_runtime --test relation_tabs \
  --test connection_switch --test profile_reducer \
  --test profile_lifecycle -- --nocapture
```

Expected: PASS.

**Step 10: Logical commit checkpoint**

```bash
git add src/action.rs src/runtime.rs src/app.rs src/model/relation.rs \
  tests/relation_runtime.rs tests/relation_tabs.rs \
  tests/connection_switch.rs tests/profile_reducer.rs \
  tests/profile_lifecycle.rs
git commit -m "feat(relations): isolate asynchronous relation requests"
```

### Task 18: Render and Control Relation Data/Structure Pages

**Files:**
- Create: `src/ui/relation.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/ui/layout.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/input/mouse.rs`
- Modify: `tests/ui_render.rs`
- Modify: `tests/keymap.rs`
- Modify: `tests/mouse.rs`
- Modify: `tests/app_flow.rs`

**Step 1: Write failing relation UI tests**

At 80x24, 120x36, and 180x50 cover:

- Mixed SQL/relation tab titles.
- Data selected by default.
- Structure selected through `D`.
- Zero-row column headers and `0 rows`, not affected-row text.
- Generated SQL and explicit 500-row limit.
- Loading, retryable failure, cancelled, and previous-data-during-refresh states.
- All offline snapshot labels.
- Hostile comments/defaults/DDL render as inert text.
- No editor cursor appears on relation pages.

**Step 2: Write failing relation input tests**

Assert:

- `o` toggles Data/Structure.
- `p` selects Data; `D` selects Structure.
- `r` refreshes only the active view.
- `Ctrl-c` cancels the exact active request.
- Grid keys operate in Data and structure keys scroll Structure.
- Pane and tab cycling never call `active_console()` on a relation tab.
- Mouse view selectors and retry targets emit semantic actions.

**Step 3: Run focused tests and observe failure**

Run:

```bash
cargo test --test ui_render relation -- --nocapture
cargo test --test keymap relation -- --nocapture
cargo test --test mouse relation -- --nocapture
```

Expected: FAIL because rendering assumes editor/results console layout.

**Step 4: Add relation-specific layout**

SQL tabs keep the existing editor/result split. Relation tabs render Explorer on
the left and a full relation page on the right. Under 100 columns, Explorer focus
shows Explorer and relation-content focus shows the relation page.

**Step 5: Extract a shared result-grid renderer**

Refactor the current grid to accept `&ResultSet` and `&GridState` instead of
reading an active console internally. Preserve all SQL console behavior and render
prepared zero-row columns for relation Data.

**Step 6: Render typed Structure sections**

Render relation summary/comment, columns in ordinal order, indexes, primary/
unique/foreign keys, constraints, triggers, and DDL with provenance. Sanitize at
render time and bound long metadata.

**Step 7: Add relation hit targets and keymap context**

Add `RelationView` and retry hit targets. Pending key-sequence ownership uses
common workspace-tab ID, not console ID. Normalize invalid Editor focus when a
relation tab is active.

**Step 8: Rewrite end-to-end preview assertions**

In `tests/app_flow.rs`, assert one `WorkspaceTab::Relation`, inspect
`RelationPreview.sql`, verify 500-row behavior, verify Structure data, and verify
the original SQL console document is unchanged.

**Step 9: Run UI/input/app tests**

Run:

```bash
cargo test --test ui_render --test keymap --test mouse \
  --test app_flow -- --nocapture
```

Expected: PASS.

**Step 10: Logical commit checkpoint**

```bash
git add src/ui/relation.rs src/ui/mod.rs src/ui/layout.rs \
  src/input/keymap.rs src/input/mouse.rs tests/ui_render.rs \
  tests/keymap.rs tests/mouse.rs tests/app_flow.rs
git commit -m "feat(ui): add relation data and structure pages"
```

### Task 19: Update Contracts, Documentation, and Full Verification

**Files:**
- Modify: `README.md`
- Modify: `docs/configuration.md`
- Modify: `docs/keybindings.md`
- Modify: `docs/architecture.md`
- Modify: `CONTRIBUTING.md`
- Create: `docs/database-capabilities.md`
- Modify: `src/cli.rs` only after deciding the stable capability-token change
- Modify: `lazydb.nvim/README.md`
- Modify: `lazydb.nvim/doc/lazydb.txt`
- Modify: `lazydb.nvim/tests/lazydb_spec.lua` if the CLI token changes
- Modify: `.github/workflows/ci.yml` only if test target setup changes

**Step 1: Update behavior documentation**

Document:

- Explorer connection roots and statuses.
- Removal of the connection-list popup.
- Profile form, Test Connection discovery, and hierarchical scope format.
- Driver capability matrix and unsupported metadata.
- Directional Explorer keys and connection CRUD keys.
- Relation Data/Structure pages, hard preview limit, cancellation, and snapshots.
- Version-2 configuration with no version-1 migration.
- Single active connection and SQLite single-connection namespace tradeoff.

**Step 2: Update in-app help and capability contract tests**

Decide explicitly whether the existing `profile-manager` capability token remains
as a compatibility label or is replaced by `connection-explorer`,
`catalog-scope`, and `relation-pages`. Because CLI capabilities are documented as
stable, change them only with synchronized Rust and Neovim contract tests.

**Step 3: Run formatting**

Run: `cargo fmt --check`

Expected: PASS. If it fails, run `cargo fmt`, inspect the diff, and rerun the
check.

**Step 4: Run Clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: PASS with no warnings.

**Step 5: Run the full Rust suite**

Run: `cargo test --all-targets --all-features`

Expected: PASS.

**Step 6: Run real PostgreSQL and MySQL adapter suites**

Run:

```bash
LAZYDB_TEST_POSTGRES_URL='postgresql://user:password@localhost:5432/database' \
LAZYDB_TEST_MYSQL_URL='mysql://user:password@localhost:3306/database' \
  cargo test --test postgres_adapter --test mysql_adapter -- --nocapture
```

Expected: PASS with both variables present. A local early-return without variables
is not evidence that live catalog metadata works.

**Step 7: Run Neovim integration tests**

Run:

```bash
nvim --headless -u lazydb.nvim/tests/minimal_init.lua \
  -c "lua require('lazydb_spec').run()" -c qa
```

Expected: PASS.

**Step 8: Run the structural performance gate explicitly**

Run: `cargo test --test explorer_performance -- --nocapture`

Expected: PASS with bounded visit counts for collapsed trees and correct output
for all-expanded trees.

**Step 9: Inspect the final diff for scope and secrets**

Run:

```bash
git status --short
git diff --stat
git diff --check
```

Expected: only intended source, tests, and documentation; no credentials,
generated database files, or unrelated worktree changes.

**Step 10: Logical commit checkpoint**

```bash
git add README.md CONTRIBUTING.md docs src/cli.rs lazydb.nvim .github/workflows/ci.yml
git commit -m "docs(explorer): document integrated database explorer"
```

## Final Acceptance Checklist

- Explorer is the only connection-list surface and shows every saved/session
  profile with textual status.
- Runtime still owns one active connection; safe switching retains the old active
  connection until the target succeeds.
- Tree hierarchy is connection, database with schema count, schema, supported
  group with object count, object, then relation children.
- New profiles select current database and default schema, or all schemas when no
  default schema is configured.
- Custom scope survives save/restart and hidden objects are absent from Explorer,
  completion, and tool-generated object actions.
- Catalog requests are lazy, paged, subtree-refreshable, and reject every stale
  identity dimension.
- PostgreSQL, MySQL, and SQLite capabilities match tested native behavior.
- Column rows and Structure pages show truthful type/default/comment support.
- Table/view and all descendants open one deduplicated relation page; Data opens
  first and returns no more than 500 rows.
- Empty relation previews retain column headers.
- Connection switches, scope changes, cancellation, close, and profile deletion
  cannot relabel or overwrite relation snapshots.
- 80x24, 120x36, and 180x50 remain usable.
- A synthetic 10,000-object catalog has structurally bounded collapsed-tree work.
- `cargo fmt --check`, Clippy, full Rust tests, live database tests, and Neovim
  tests pass.
