# Explorer Catalog Object Editor Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add capability-aware Explorer `a` and `e` workflows for creating and editing PostgreSQL catalog objects while preserving the existing connection-profile editor and the reducer/runtime/adapter ownership boundaries.

**Architecture:** Introduce a typed catalog-mutation protocol beside the existing catalog-drop protocol. Explorer resolves the directly selected node into a mutation anchor, adapters expose valid operations and load authoritative object definitions, typed drafts are converted into immutable adapter-generated plans, and Runtime executes only validated plans before App performs targeted catalog refreshes. PostgreSQL is delivered as vertical slices; MySQL and SQLite initially advertise no mutation operations until equivalent adapter implementations are added.

**Tech Stack:** Rust 2024, Tokio, SQLx PostgreSQL/MySQL/SQLite adapters, Crossterm input, Ratatui UI, existing `Action -> App::update -> Command -> Runtime -> DatabaseConnection` architecture.

---

## Scope And Delivery Policy

### Required PostgreSQL behavior

| Selected Explorer node | `a` create options | `e` behavior |
| --- | --- | --- |
| Connection profile | Database, Login Role (User), Role | Existing Profile Manager |
| Database | Schema | Database object editor |
| Schema | Table, View, Materialized View, Sequence | Schema editor |
| Tables/Views/Materialized Views/Sequences group | The group's object kind | No edit action |
| Table | Column, Index, Primary Key, Unique, Foreign Key, Check | Table editor |
| View | Trigger only when implemented | View editor |
| Materialized View | Index | Materialized-view editor |
| Sequence | No child creation | Sequence editor |
| Column/Index/Constraint | No child creation | Focus matching section/row in owning relation editor |
| Status/Empty/LoadMore/Others | No action | No action |

PostgreSQL `User` is represented as `RoleDraft { login: true }`; do not create a second database-domain model for users.

### Explicitly deferred behavior

- Materialized-view definition replacement. PostgreSQL has no safe in-place equivalent to `CREATE OR REPLACE MATERIALIZED VIEW`; future support must be presented as destructive recreation.
- Function, procedure, type, and trigger editors. Existing Explorer kinds stay browse-only until their typed drafts and planners are implemented.
- Automatic profile, catalog-scope, and SQL-console target migration after a database rename.
- MySQL and SQLite object mutation. Their adapters must return empty/unsupported capabilities rather than inheriting PostgreSQL behavior.
- Arbitrary raw DDL entry in catalog forms. Expressions such as defaults and checks are typed SQL fragments, but the UI never supplies a complete executable statement.

### Safety invariants

- Keep SQL generation in concrete adapters. Do not build DDL in `src/input/keymap.rs`, `src/app.rs`, `src/model/catalog_editor.rs`, or UI modules.
- Never execute the mutable form state. Execute only an immutable, adapter-produced `CatalogMutationPlan` that Runtime validates again.
- Bind definition requests, plans, and results to `ConnectionIdentity`, request ID, catalog epoch, target database, object identity, and baseline fingerprint where editing an existing object.
- Reject catalog mutations in both App and Runtime when the profile is read-only.
- Do not route catalog mutations through an arbitrary SQL console or its MANUAL transaction.
- Use a temporary target connection when the selected PostgreSQL object's database differs from the active pool; do not switch the user's workspace or console target.
- Do not optimistically patch renamed/created objects into `CatalogTree`. Invalidate and reload the smallest authoritative `CatalogTarget` returned by the plan.
- Preserve raw SQL sent to PostgreSQL, but sanitize database errors and display-only SQL before rendering terminal content, following the existing security boundary.
- Commit steps are optional checkpoints. Execute them only when the user explicitly requests commits.

## Milestones

1. Foundation and Explorer dispatch.
2. PostgreSQL Schema create/edit vertical slice.
3. Generic targeted refresh and relation-tab invalidation.
4. PostgreSQL Table and Column create/edit.
5. PostgreSQL Index and Constraint create/edit.
6. PostgreSQL View, Materialized View, and Sequence create/edit.
7. PostgreSQL Database and Role create/edit with maintenance connections.
8. Documentation, integration coverage, and full verification.

## Milestone 1: Foundation And Explorer Dispatch

### Task 1: Define Mutation Anchors, Object Types, And Capabilities

**Files:**
- Create: `src/db/catalog_mutation.rs`
- Modify: `src/db/mod.rs:1-32`
- Test: `tests/catalog_mutation.rs`

**Step 1: Write failing model tests**

Create `tests/catalog_mutation.rs` and cover:

- `CatalogMutationAnchor::Profile` accepts only a profile ID.
- `CatalogMutationAnchor::Catalog` retains the exact `CatalogId`.
- `CatalogMutationAnchor::Group` retains schema ID and `ObjectGroup`.
- A mutation request rejects an object whose profile differs from `ConnectionIdentity.profile_id`.
- A group anchor rejects a non-schema parent and a kind outside the group.
- Every mutation operation has a stable display label.
- Empty MySQL/SQLite mutation capabilities produce no create/edit options.

Start with the shared types:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogMutationMode {
    Create,
    Edit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogMutationAnchor {
    Profile { profile_id: Uuid },
    Catalog(CatalogId),
    Group { schema: CatalogId, group: ObjectGroup },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CatalogObjectType {
    Catalog(CatalogKind),
    LoginRole,
    Role,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CatalogMutationCapabilities {
    pub profile_create: Vec<CatalogObjectType>,
    pub creatable_kinds: Vec<CatalogKind>,
    pub editable_kinds: Vec<CatalogKind>,
}
```

Keep capability option resolution as pure methods over the anchor and optional `CatalogEntry`. Do not put `DatabaseKind` conditionals in App.

**Step 2: Run the focused test and verify failure**

Run:

```bash
cargo test --test catalog_mutation mutation_model
```

Expected: FAIL because `db::catalog_mutation` does not exist.

**Step 3: Add the module and validation**

Implement:

- `CatalogMutationRequest::new(...) -> Result<Self, CatalogMutationError>`.
- `CatalogMutationCapabilities::create_options(...)`.
- `CatalogMutationCapabilities::can_edit(...)`.
- Explicit `CatalogMutationError` variants for unsupported operation, profile mismatch, invalid anchor, empty selection, stale state, invalid draft, and invalid plan.

Do not add SQL or UI state in this task.

**Step 4: Run focused tests**

Run:

```bash
cargo test --test catalog_mutation mutation_model
```

Expected: PASS.

**Step 5: Checkpoint commit, only if requested**

```bash
git add src/db/catalog_mutation.rs src/db/mod.rs tests/catalog_mutation.rs
git commit -m "feat(db): define catalog mutation capabilities"
```

### Task 2: Resolve Direct Explorer Selections Into Mutation Intents

**Files:**
- Modify: `src/action.rs:29-108`
- Modify: `src/input/keymap.rs:1554-1589`
- Modify: `src/app.rs:1140-1225`
- Test: `tests/keymap.rs`
- Test: `tests/catalog_editor_reducer.rs`

**Step 1: Write failing keymap tests**

Add fixtures for each `ExplorerNodeId` variant and assert:

```rust
assert_eq!(
    keymap.map(key(KeyCode::Char('a')), &table_app),
    Some(Action::OpenCatalogCreate),
);
assert_eq!(
    keymap.map(key(KeyCode::Char('e')), &table_app),
    Some(Action::OpenCatalogEdit),
);
```

Also assert:

- `e` on `Profile` still results in `ProfileStartEdit { profile_id }` after reducer resolution.
- `a` and `e` on status, empty, load-more, `EmptyProfiles`, and `Others` are no-ops.
- `e` on a catalog object never emits `ProfileStartEdit`.
- `n` remains `ProfileStartNew`.

**Step 2: Run the tests and verify failure**

Run:

```bash
cargo test --test keymap explorer_catalog_mutation
cargo test --test catalog_editor_reducer explorer_selection
```

Expected: FAIL because the semantic actions and reducer target resolver do not exist.

**Step 3: Add semantic actions**

Add to `Action`:

```rust
OpenCatalogCreate,
OpenCatalogEdit,
```

Change only the Explorer branches:

```rust
KeyCode::Char('a') => Some(Action::OpenCatalogCreate),
KeyCode::Char('e') => Some(Action::OpenCatalogEdit),
```

Keep `n`, `d`, `c`, `x`, and `s` behavior unchanged.

**Step 4: Add a pure target resolver in App/model code**

Implement a pure helper that maps the direct selection to one of:

```rust
enum ExplorerMutationIntent {
    EditProfile(Uuid),
    Create(CatalogMutationAnchor),
    Edit(CatalogMutationAnchor),
}
```

Rules:

- Profile + create becomes a profile anchor.
- Profile + edit becomes `EditProfile`.
- Catalog + create/edit retains that exact catalog ID.
- Group + create retains schema and group.
- Group + edit is unsupported.
- Synthetic rows never inherit actions from their owner.

**Step 5: Run focused tests**

Run:

```bash
cargo test --test keymap explorer_catalog_mutation
cargo test --test catalog_editor_reducer explorer_selection
```

Expected: PASS.

**Step 6: Checkpoint commit, only if requested**

```bash
git add src/action.rs src/input/keymap.rs src/app.rs tests/keymap.rs tests/catalog_editor_reducer.rs
git commit -m "feat(explorer): dispatch catalog create and edit actions"
```

### Task 3: Add Typed Catalog Editor State

**Files:**
- Create: `src/model/catalog_editor.rs`
- Modify: `src/model/mod.rs`
- Modify: `src/model/workspace.rs:102-159`
- Modify: `src/app.rs:185-212`
- Test: `tests/catalog_editor_state.rs`

**Step 1: Write failing state-machine tests**

Cover these transitions:

```text
ObjectPicker -> Form
Loading -> Form
Form -> Previewing -> SqlPreview
SqlPreview -> Applying -> closed on matching success
Any non-busy page -> cancelled
Busy operation -> cancel is ignored
Mismatched request/connection/epoch response -> ignored
```

Also test field validation errors stay in the form and preserve entered values.

**Step 2: Run the focused test and verify failure**

```bash
cargo test --test catalog_editor_state
```

Expected: FAIL because the editor state does not exist.

**Step 3: Implement typed state**

Create:

```rust
pub enum CatalogEditorPage {
    ObjectPicker,
    Loading,
    Form,
    SqlPreview,
}

pub enum CatalogEditorOperation {
    LoadingDefinition { request_id: u64 },
    Planning { request_id: u64 },
    Applying { request_id: u64 },
}

pub struct CatalogEditorState {
    pub mode: CatalogMutationMode,
    pub anchor: CatalogMutationAnchor,
    pub object_type: Option<CatalogObjectType>,
    pub page: CatalogEditorPage,
    pub operation: Option<CatalogEditorOperation>,
    pub catalog_epoch: u64,
    pub options: Vec<CatalogMutationOption>,
    pub selected_option: usize,
    pub draft: Option<CatalogDraft>,
    pub baseline: Option<CatalogObjectDefinition>,
    pub plan: Option<CatalogMutationPlan>,
    pub error: Option<String>,
}
```

Initially define only the draft variants needed by the Schema vertical slice; later tasks extend the enum. Reuse `TextInput` and keep cursor/focus behavior in the state model rather than UI.

Add `Overlay::CatalogEditor` and `App::catalog_editor: Option<CatalogEditorState>` following the existing Profile Manager ownership pattern.

**Step 4: Run tests**

```bash
cargo test --test catalog_editor_state
```

Expected: PASS.

**Step 5: Checkpoint commit, only if requested**

```bash
git add src/model/catalog_editor.rs src/model/mod.rs src/model/workspace.rs src/app.rs tests/catalog_editor_state.rs
git commit -m "feat(model): add catalog editor state machine"
```

### Task 4: Add Catalog Editor Keyboard And Overlay Shell

**Files:**
- Create: `src/ui/catalog_editor.rs`
- Modify: `src/ui/mod.rs:1-13`
- Modify: `src/ui/mod.rs:2410-2750`
- Modify: `src/input/keymap.rs`
- Modify: `src/input/mouse.rs`
- Modify: `src/help.rs`
- Test: `tests/keymap.rs`
- Test: `tests/ui_render.rs`

**Step 1: Write failing input and rendering tests**

Cover:

- Object picker: `j/k`, arrows, Enter, Esc.
- Form: Tab/BackTab, arrows/select cycles, printable input, Backspace, Ctrl-W, Ctrl-U, Enter preview, Esc cancel.
- SQL preview: Enter apply, Esc back to form.
- Busy pages ignore mutation input.
- Render includes mode, object type, qualified target, validation error, SQL statements, warnings, and transaction mode.
- Narrow terminal renders a bounded scrollable form instead of overflowing.

**Step 2: Run and verify failure**

```bash
cargo test --test keymap catalog_editor
cargo test --test ui_render catalog_editor
```

Expected: FAIL because overlay mapping and renderer do not exist.

**Step 3: Implement overlay-first key mapping**

Handle `Overlay::CatalogEditor` before normal Explorer/editor mappings, matching the Profile Manager pattern. Add typed actions for picker movement, field movement/editing, preview request, apply, back, and cancel. Paste must be accepted only by editable text fields.

**Step 4: Implement the overlay shell**

Render four pages:

- `ObjectPicker`: available adapter-provided operations.
- `Loading`: object name and loading indicator.
- `Form`: target header, tabs/sections, fields, inline error.
- `SqlPreview`: immutable planned SQL, warnings, refresh targets, Apply/Cancel.

The UI reads `CatalogEditorState`; it does not derive capabilities or SQL.

**Step 5: Run tests**

```bash
cargo test --test keymap catalog_editor
cargo test --test ui_render catalog_editor
```

Expected: PASS.

**Step 6: Checkpoint commit, only if requested**

```bash
git add src/ui/catalog_editor.rs src/ui/mod.rs src/input/keymap.rs src/input/mouse.rs src/help.rs tests/keymap.rs tests/ui_render.rs
git commit -m "feat(ui): add catalog object editor overlay"
```

## Milestone 2: PostgreSQL Schema Vertical Slice

### Task 5: Define Object Definitions, Drafts, Plans, And Refresh Targets

**Files:**
- Modify: `src/db/catalog_mutation.rs`
- Modify: `src/model/catalog_editor.rs`
- Test: `tests/catalog_mutation.rs`

**Step 1: Write failing protocol tests**

Cover:

- Definition request profile/object/epoch validation.
- Schema definition contains database, name, owner, and supported comment.
- Schema draft validation rejects blank name/owner.
- Plan contains immutable statements, execution mode, refresh targets, warnings, and baseline fingerprint.
- A plan rejects empty statements, profile mismatch, object-type mismatch, and a missing refresh target.
- Plan accessors expose SQL read-only.

**Step 2: Run and verify failure**

```bash
cargo test --test catalog_mutation mutation_protocol
```

Expected: FAIL because definitions and plans are incomplete.

**Step 3: Add minimal shared protocol**

Define:

```rust
pub struct CatalogObjectDefinitionRequest {
    pub connection: ConnectionIdentity,
    pub request_id: u64,
    pub catalog_epoch: u64,
    pub object: CatalogId,
    pub target: ExecutionTarget,
}

pub enum CatalogObjectDefinition {
    Schema(SchemaDefinition),
}

pub enum CatalogDraft {
    Schema(SchemaDraft),
}

pub enum CatalogMutationExecutionMode {
    Transactional,
    Autocommit,
}

pub struct CatalogMutationPlan {
    pub request: CatalogMutationRequest,
    pub object_type: CatalogObjectType,
    pub execution_mode: CatalogMutationExecutionMode,
    pub refresh: Vec<CatalogTarget>,
    pub selection: CatalogSelectionHint,
    pub baseline_fingerprint: Option<String>,
    statements: Vec<String>,
}
```

Use `CatalogTarget` directly for refresh targets instead of introducing a duplicate refresh enum.

**Step 4: Run tests**

```bash
cargo test --test catalog_mutation mutation_protocol
```

Expected: PASS.

**Step 5: Checkpoint commit, only if requested**

```bash
git add src/db/catalog_mutation.rs src/model/catalog_editor.rs tests/catalog_mutation.rs
git commit -m "feat(db): define catalog mutation protocol"
```

### Task 6: Advertise Truthful Per-Adapter Mutation Capabilities

**Files:**
- Modify: `src/db/postgres.rs:342-367`
- Modify: `src/db/mysql.rs:250-274`
- Modify: `src/db/sqlite.rs:123-138`
- Modify: `src/db/mod.rs:181-207`
- Test: `tests/postgres_adapter.rs`
- Test: `tests/mysql_adapter.rs`
- Test: `tests/sqlite_adapter.rs`

**Step 1: Write failing capability tests**

For the first vertical slice assert PostgreSQL supports:

- Profile create options: Database, Login Role, Role are declared but disabled until Milestone 7 with an explanatory availability state.
- Database create child: Schema.
- Schema edit.
- Other create/edit operations remain unavailable until their milestone.

Assert MySQL and SQLite return no mutation options.

Represent availability explicitly:

```rust
pub enum CatalogMutationAvailability {
    Available,
    Unavailable { reason: &'static str },
}
```

This prevents future operations from appearing executable merely because their object type exists.

**Step 2: Run and verify failure**

```bash
cargo test --test postgres_adapter mutation_capabilities
cargo test --test mysql_adapter mutation_capabilities
cargo test --test sqlite_adapter mutation_capabilities
```

Expected: FAIL because adapters do not expose mutation capabilities.

**Step 3: Add `DatabaseConnection::catalog_mutation_capabilities()`**

Dispatch to each concrete adapter. PostgreSQL returns the milestone's truthful matrix; MySQL and SQLite return default empty capabilities.

**Step 4: Run tests**

```bash
cargo test --test postgres_adapter mutation_capabilities
cargo test --test mysql_adapter mutation_capabilities
cargo test --test sqlite_adapter mutation_capabilities
```

Expected: PASS.

**Step 5: Checkpoint commit, only if requested**

```bash
git add src/db/mod.rs src/db/postgres.rs src/db/mysql.rs src/db/sqlite.rs tests/postgres_adapter.rs tests/mysql_adapter.rs tests/sqlite_adapter.rs
git commit -m "feat(db): expose catalog mutation capabilities"
```

### Task 7: Load Authoritative PostgreSQL Schema Definitions

**Files:**
- Modify: `src/db/postgres.rs`
- Modify: `src/db/mod.rs`
- Modify: `src/action.rs`
- Modify: `src/runtime.rs`
- Modify: `src/app.rs`
- Test: `tests/catalog_editor_reducer.rs`
- Test: `tests/postgres_adapter.rs`

**Step 1: Write failing reducer and adapter tests**

Reducer tests:

- `e` on schema emits `Command::LoadCatalogObjectDefinition` with matching connection, target database, object ID, request ID, and epoch.
- Offline, wrong-profile, read-only, or missing entries produce a warning and no command.
- Mismatched load success/failure is ignored.
- Matching success initializes a `SchemaDraft` and opens Form.

PostgreSQL integration test:

- Create a temporary schema with owner and comment.
- Load its definition.
- Assert exact schema, owner, comment, and a deterministic baseline fingerprint.

**Step 2: Run and verify failure**

```bash
cargo test --test catalog_editor_reducer schema_definition
cargo test --test postgres_adapter schema_definition -- --nocapture --test-threads=1
```

Expected: reducer test FAIL; integration test is skipped without `LAZYDB_TEST_POSTGRES_URL`, otherwise FAIL due to missing API.

**Step 3: Add actions and command**

Add:

```rust
Command::LoadCatalogObjectDefinition(CatalogObjectDefinitionRequest)
Action::CatalogObjectDefinitionLoaded { request, definition }
Action::CatalogObjectDefinitionLoadFailed { request, message }
```

**Step 4: Implement PostgreSQL definition loading**

Query `pg_namespace`, `pg_roles`, and `obj_description`/`shobj_description` as appropriate. Use bound values for names/OIDs. Load inside a read-only repeatable-read transaction when multiple catalog queries are required.

Build the fingerprint from stable, normalized definition fields, not from rendered terminal text.

**Step 5: Implement Runtime and reducer stale checks**

Runtime obtains the exact target connection, loads the definition, sanitizes errors, and returns the original request. App accepts only the currently loading request with unchanged active identity and catalog epoch.

**Step 6: Run tests**

```bash
cargo test --test catalog_editor_reducer schema_definition
cargo test --test postgres_adapter schema_definition -- --nocapture --test-threads=1
```

Expected: PASS or documented environment skip for PostgreSQL integration.

**Step 7: Checkpoint commit, only if requested**

```bash
git add src/db/postgres.rs src/db/mod.rs src/action.rs src/runtime.rs src/app.rs tests/catalog_editor_reducer.rs tests/postgres_adapter.rs
git commit -m "feat(postgres): load schema editor definitions"
```

### Task 8: Plan And Execute PostgreSQL Schema Create/Edit

**Files:**
- Modify: `src/db/postgres.rs`
- Modify: `src/db/mod.rs`
- Modify: `src/action.rs`
- Modify: `src/runtime.rs`
- Modify: `src/app.rs`
- Test: `tests/catalog_mutation.rs`
- Test: `tests/catalog_editor_reducer.rs`
- Test: `tests/postgres_adapter.rs`

**Step 1: Write failing planner tests**

Cover exact quoted SQL for:

- `CREATE SCHEMA name AUTHORIZATION owner`.
- Optional `COMMENT ON SCHEMA`.
- Rename with `ALTER SCHEMA old RENAME TO new`.
- Owner change with `ALTER SCHEMA name OWNER TO owner`.
- Add/change/remove comment with `COMMENT ON SCHEMA ... IS .../NULL`.
- Embedded quotes and hostile identifier/comment values.
- No-op edit rejected with a typed `NoChanges` error.
- Refresh target is `CatalogTarget::Schemas { database }`.
- Create selection hint points to the new qualified schema name.

Do not interpolate comments as identifiers. Add a PostgreSQL literal quoting helper with tests or use bound execution where possible; generated DDL still needs safe literal rendering for SQL preview.

**Step 2: Run and verify failure**

```bash
cargo test --test catalog_mutation postgres_schema
cargo test --test catalog_editor_reducer schema_plan
```

Expected: FAIL because planning/execution actions do not exist.

**Step 3: Add planning and execution protocol**

Add:

```rust
Command::PlanCatalogMutation { request, draft, baseline }
Command::ExecuteCatalogMutation(CatalogMutationPlan)
Action::CatalogMutationPlanReady(CatalogMutationPlan)
Action::CatalogMutationPlanFailed { request, message }
Action::CatalogMutationSucceeded { plan, outcome }
Action::CatalogMutationFailed { plan, message }
```

**Step 4: Implement the PostgreSQL schema planner**

The planner validates object type, anchor, draft, baseline object ID, target database, and fingerprint. It returns ordered statements and `Transactional` execution mode.

**Step 5: Implement Runtime execution**

Before execution:

- Revalidate plan structure.
- Re-resolve profile and read-only state.
- Recheck active connection identity.
- Re-load the current definition for edits and compare its fingerprint.
- Run all statements on one physical target connection in one transaction.

Do not call `DatabaseConnection::execute()` separately for each statement because a pool may choose different physical connections and partial application would be possible.

**Step 6: Connect the reducer to preview/apply**

- Form Enter emits the planning command.
- Matching plan opens SQL Preview.
- Preview Enter marks busy and emits execute.
- Failure keeps the form/preview open and displays the sanitized error.
- Matching success closes the overlay only after refresh has been scheduled.

**Step 7: Run tests**

```bash
cargo test --test catalog_mutation postgres_schema
cargo test --test catalog_editor_reducer schema_plan
cargo test --test postgres_adapter schema_mutation -- --nocapture --test-threads=1
```

Expected: PASS or documented PostgreSQL environment skip.

**Step 8: Checkpoint commit, only if requested**

```bash
git add src/db/postgres.rs src/db/mod.rs src/action.rs src/runtime.rs src/app.rs tests/catalog_mutation.rs tests/catalog_editor_reducer.rs tests/postgres_adapter.rs
git commit -m "feat(postgres): create and edit schemas"
```

## Milestone 3: Targeted Refresh And Snapshot Invalidation

### Task 9: Add Generic Catalog Target Invalidation And Reload

**Files:**
- Modify: `src/model/explorer.rs`
- Modify: `src/model/workspace.rs`
- Modify: `src/app.rs`
- Test: `tests/catalog_reducer.rs`
- Test: `tests/catalog_editor_reducer.rs`

**Step 1: Write failing refresh tests**

Cover:

- Invalidating a target removes all loaded pages/cursors for only that owner.
- The previous rows remain visible as stale while the replacement request is pending, matching existing refresh behavior.
- A successful mutation schedules each unique plan refresh target once.
- Stale mutation success does not refresh anything.
- Reload acceptance updates CompletionIndex and increments catalog generation.
- `CatalogSelectionHint` selects the new/renamed object when found and falls back to its parent when absent.

**Step 2: Run and verify failure**

```bash
cargo test --test catalog_reducer targeted_refresh
cargo test --test catalog_editor_reducer mutation_refresh
```

Expected: FAIL because only whole-node refresh/drop removal exists.

**Step 3: Add reusable target refresh methods**

Implement methods such as:

```rust
ExplorerState::invalidate_catalog_target(profile_id, &CatalogTarget)
App::commands_for_catalog_targets(profile_id, targets)
```

Reuse `owner_for_target()` and existing catalog request allocation. Do not duplicate request validation or page replacement logic.

**Step 4: Apply mutation success atomically in App**

On matching success:

- Record the pending selection hint.
- Invalidate all unique refresh targets.
- Increment catalog generation once.
- Refresh frontend search.
- Emit load commands.
- Close the editor overlay.
- Notify success.

Do not alter the normalized tree until normal catalog-page success actions arrive.

**Step 5: Run tests**

```bash
cargo test --test catalog_reducer targeted_refresh
cargo test --test catalog_editor_reducer mutation_refresh
```

Expected: PASS.

**Step 6: Checkpoint commit, only if requested**

```bash
git add src/model/explorer.rs src/model/workspace.rs src/app.rs tests/catalog_reducer.rs tests/catalog_editor_reducer.rs
git commit -m "feat(explorer): refresh mutated catalog targets"
```

### Task 10: Invalidate Affected Relation Tabs

**Files:**
- Modify: `src/model/relation.rs`
- Modify: `src/app.rs`
- Test: `tests/relation_tabs.rs`
- Test: `tests/catalog_editor_reducer.rs`

**Step 1: Write failing lifecycle tests**

Cover:

- Table/column/index/constraint mutation marks matching relation Data and DDL snapshots stale.
- Rename changes native identity and prevents the old tab from running preview/edit mutations.
- Unrelated relation tabs remain live.
- Schema rename invalidates all open relations under that schema.
- Database rename invalidates all open relations under that database.

**Step 2: Run and verify failure**

```bash
cargo test --test relation_tabs catalog_mutation
```

Expected: FAIL because mutation invalidation is not modeled.

**Step 3: Add impact metadata to plans**

Add a typed `CatalogMutationImpact` containing the old object ID, optional owning relation ID, namespace, and whether native identity changes. Avoid parsing SQL to infer impact.

**Step 4: Reconcile tabs on success**

Use existing snapshot provenance/stale behavior. If no suitable stale provenance exists, add a mutation-specific stale reason without discarding the owned snapshot. Disable new database activity from tabs whose native identity changed until they are reopened from refreshed Explorer data.

**Step 5: Run tests**

```bash
cargo test --test relation_tabs catalog_mutation
cargo test --test catalog_editor_reducer relation_invalidation
```

Expected: PASS.

**Step 6: Checkpoint commit, only if requested**

```bash
git add src/db/catalog_mutation.rs src/model/relation.rs src/app.rs tests/relation_tabs.rs tests/catalog_editor_reducer.rs
git commit -m "feat(relation): invalidate snapshots after catalog edits"
```

## Milestone 4: PostgreSQL Table And Column Editing

### Task 11: Add Table Definitions And Multi-Section Relation Drafts

**Files:**
- Modify: `src/db/catalog_mutation.rs`
- Modify: `src/model/catalog_editor.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/ui/catalog_editor.rs`
- Test: `tests/catalog_editor_state.rs`
- Test: `tests/postgres_adapter.rs`
- Test: `tests/ui_render.rs`

**Step 1: Write failing tests**

Definition tests require one consistent snapshot containing:

- Table name, schema, owner, comment.
- Ordered columns with native type, nullable, default, identity, generated expression, collation, and comment.
- Indexes and constraints sufficient for later sections.
- Baseline fingerprint over all editable fields.

State/UI tests require:

- Tabs `General | Columns | Indexes | Constraints`.
- Row add/delete/select for columns.
- Opening `e` from a Column focuses that column row.
- Field values survive tab navigation and validation failures.

**Step 2: Run and verify failure**

```bash
cargo test --test catalog_editor_state relation_draft
cargo test --test postgres_adapter table_definition -- --nocapture --test-threads=1
cargo test --test ui_render table_editor
```

Expected: FAIL.

**Step 3: Extend definition and draft enums**

Add `TableDefinition`, `TableDraft`, `ColumnDefinition`, and `ColumnDraft`. Use stable draft row IDs independent of catalog IDs so newly added rows can be edited before persistence.

Represent a column type as PostgreSQL native type text plus structured optional attributes already available from catalog metadata. Do not force PostgreSQL types into a cross-database enum.

**Step 4: Load PostgreSQL table details consistently**

Reuse relation DDL catalog techniques where practical, but return typed fields rather than parsing generated DDL. Execute all detail queries inside one read-only repeatable-read transaction.

**Step 5: Render General and Columns sections**

Use an internal scroll offset and visible-field calculation. Keep UI rendering passive; all add/delete/focus transitions belong to `CatalogEditorState`.

**Step 6: Run tests**

```bash
cargo test --test catalog_editor_state relation_draft
cargo test --test postgres_adapter table_definition -- --nocapture --test-threads=1
cargo test --test ui_render table_editor
```

Expected: PASS or integration skip.

**Step 7: Checkpoint commit, only if requested**

```bash
git add src/db/catalog_mutation.rs src/model/catalog_editor.rs src/db/postgres.rs src/ui/catalog_editor.rs tests/catalog_editor_state.rs tests/postgres_adapter.rs tests/ui_render.rs
git commit -m "feat(postgres): load table and column editor state"
```

### Task 12: Plan And Execute Table General/Column Changes

**Files:**
- Modify: `src/db/postgres.rs`
- Modify: `src/db/catalog_mutation.rs`
- Modify: `src/model/catalog_editor.rs`
- Test: `tests/catalog_mutation.rs`
- Test: `tests/postgres_adapter.rs`

**Step 1: Write failing planner tests**

Cover:

- Create table with zero columns rejected.
- Create table with quoted names and ordered columns.
- Rename table, move schema, owner, and comment.
- Add/drop/rename column.
- Type change with optional `USING` expression.
- Set/drop default.
- Set/drop `NOT NULL`.
- Add/drop identity and generated expression only when PostgreSQL supports the exact transition.
- Column comment.
- Multiple changes produce a deterministic statement order.
- Drop/type-conversion risks populate plan warnings and `destructive = true` where data loss is possible.
- Refresh includes old/new schema table groups and relation children as applicable.

**Step 2: Run and verify failure**

```bash
cargo test --test catalog_mutation postgres_table_column
```

Expected: FAIL.

**Step 3: Implement baseline-to-draft diffing**

Diff by stable existing `CatalogId` for persisted rows and draft row ID for new rows. Never infer rename solely from matching ordinal position. Explicitly track row intent:

```rust
pub enum DraftRowState {
    Existing { id: CatalogId },
    Added,
    Removed { id: CatalogId },
}
```

**Step 4: Implement PostgreSQL planning**

Order operations to maintain validity:

1. Relation rename/schema move when required for following qualified names.
2. Add columns.
3. Existing column type/default/nullability/identity changes.
4. Column renames.
5. Comments.
6. Drops last.

If an operation cannot be represented safely, return typed unsupported instead of generating approximate SQL.

**Step 5: Add destructive confirmation**

Extend SQL Preview so destructive plans require exact lowercase `y` plus Enter, reusing the interaction rule from catalog drop. Non-destructive plans use normal Apply confirmation.

**Step 6: Run tests**

```bash
cargo test --test catalog_mutation postgres_table_column
cargo test --test postgres_adapter table_column_mutation -- --nocapture --test-threads=1
cargo test --test keymap catalog_editor_destructive
cargo test --test ui_render catalog_editor_destructive
```

Expected: PASS or integration skip.

**Step 7: Checkpoint commit, only if requested**

```bash
git add src/db/postgres.rs src/db/catalog_mutation.rs src/model/catalog_editor.rs src/input/keymap.rs src/ui/catalog_editor.rs tests/catalog_mutation.rs tests/postgres_adapter.rs tests/keymap.rs tests/ui_render.rs
git commit -m "feat(postgres): create and edit tables and columns"
```

## Milestone 5: PostgreSQL Indexes And Constraints

### Task 13: Add Index Drafts And PostgreSQL Planner

**Files:**
- Modify: `src/db/catalog_mutation.rs`
- Modify: `src/model/catalog_editor.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/ui/catalog_editor.rs`
- Test: `tests/catalog_mutation.rs`
- Test: `tests/catalog_editor_state.rs`
- Test: `tests/postgres_adapter.rs`

**Step 1: Write failing tests**

Cover index fields:

- Name, unique, access method.
- Ordered columns or expressions.
- Sort direction and null ordering.
- `INCLUDE` columns.
- Partial-index predicate.
- Tablespace where discoverable.

Cover behavior:

- `a` on Table and MaterializedView offers Index.
- `e` on Index opens owning relation editor focused on that index.
- Editing fields that PostgreSQL cannot alter in place produces a destructive drop/recreate plan with an explicit warning.
- Rename-only uses `ALTER INDEX ... RENAME TO` and is non-destructive.

**Step 2: Run and verify failure**

```bash
cargo test --test catalog_mutation postgres_index
cargo test --test catalog_editor_state index_section
```

Expected: FAIL.

**Step 3: Add typed index model and definition loading**

Do not parse `pg_indexes.indexdef`. Query `pg_index`, `pg_class`, `pg_am`, `pg_attribute`, and `pg_get_expr` to build typed definitions. Preserve expression text as a SQL fragment.

**Step 4: Implement planning and UI**

Use `CREATE [UNIQUE] INDEX`, safe identifier quoting, and explicit expression handling. Drop/recreate plans are destructive and transactional unless an eventual separate `CONCURRENTLY` workflow is designed; do not add `CONCURRENTLY` in this task.

**Step 5: Run tests**

```bash
cargo test --test catalog_mutation postgres_index
cargo test --test catalog_editor_state index_section
cargo test --test postgres_adapter index_mutation -- --nocapture --test-threads=1
```

Expected: PASS or integration skip.

**Step 6: Checkpoint commit, only if requested**

```bash
git add src/db/catalog_mutation.rs src/model/catalog_editor.rs src/db/postgres.rs src/ui/catalog_editor.rs tests/catalog_mutation.rs tests/catalog_editor_state.rs tests/postgres_adapter.rs
git commit -m "feat(postgres): create and edit indexes"
```

### Task 14: Add Primary, Unique, Foreign-Key, And Check Constraints

**Files:**
- Modify: `src/db/catalog_mutation.rs`
- Modify: `src/model/catalog_editor.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/ui/catalog_editor.rs`
- Test: `tests/catalog_mutation.rs`
- Test: `tests/catalog_editor_state.rs`
- Test: `tests/postgres_adapter.rs`

**Step 1: Write failing typed-model tests**

Cover:

- Primary/Unique: ordered columns, deferrable, initially deferred.
- Foreign key: source columns, referenced schema/table/columns, match type, ON UPDATE, ON DELETE, deferrable, initially deferred, validation state.
- Check: expression, `NO INHERIT`, validation state.
- FK source/referenced column count mismatch is rejected.
- Duplicate source columns are rejected.
- Constraint names are optional on create and required/stable on edit.
- `e` from each constraint kind focuses the matching row.

**Step 2: Run and verify failure**

```bash
cargo test --test catalog_mutation postgres_constraint
cargo test --test catalog_editor_state constraint_section
```

Expected: FAIL.

**Step 3: Extend authoritative definition queries**

Use `pg_constraint`, `pg_attribute`, and `pg_get_expr`; resolve FK referenced relation and columns by OID/attribute number. Do not derive behavior from Explorer's abbreviated `ConstraintMetadata`.

**Step 4: Implement planner**

- Rename-only uses `ALTER TABLE ... RENAME CONSTRAINT`.
- Unsupported structural edits use explicit drop/add and are marked destructive.
- New unvalidated FK/check may use `NOT VALID` only when selected by the user.
- Validation state changes may emit `VALIDATE CONSTRAINT`.
- Never add `CASCADE`.

**Step 5: Implement Constraints UI**

Use a kind picker followed by kind-specific fields. Referenced relation and column choices come from loaded catalog data where available; retain a validated text path fallback for unloaded but in-scope relations only if product requirements demand it. The initial implementation should prefer loaded choices and report when more catalog data must be loaded.

**Step 6: Run tests**

```bash
cargo test --test catalog_mutation postgres_constraint
cargo test --test catalog_editor_state constraint_section
cargo test --test postgres_adapter constraint_mutation -- --nocapture --test-threads=1
```

Expected: PASS or integration skip.

**Step 7: Checkpoint commit, only if requested**

```bash
git add src/db/catalog_mutation.rs src/model/catalog_editor.rs src/db/postgres.rs src/ui/catalog_editor.rs tests/catalog_mutation.rs tests/catalog_editor_state.rs tests/postgres_adapter.rs
git commit -m "feat(postgres): create and edit relation constraints"
```

## Milestone 6: PostgreSQL View, Materialized View, And Sequence

### Task 15: Create And Edit Views

**Files:**
- Modify: `src/db/catalog_mutation.rs`
- Modify: `src/model/catalog_editor.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/ui/catalog_editor.rs`
- Test: `tests/catalog_mutation.rs`
- Test: `tests/postgres_adapter.rs`
- Test: `tests/ui_render.rs`

**Step 1: Write failing tests**

Cover create/edit fields:

- Name, schema, owner, comment.
- Query definition.
- Optional explicit output column names.
- Security barrier/invoker and check option only when supported by the target PostgreSQL version.
- `CREATE OR REPLACE VIEW` plans warn when output columns are incompatible.
- View does not offer Column, Index, FK, PK, Unique, or Check create options.

**Step 2: Run and verify failure**

```bash
cargo test --test catalog_mutation postgres_view
cargo test --test postgres_adapter view_mutation -- --nocapture --test-threads=1
```

Expected: FAIL.

**Step 3: Add typed view definition/draft and planner**

Load query text with PostgreSQL catalog functions. Keep the query as a SQL fragment and validate that it is one query expression suitable for `CREATE VIEW`; do not accept trailing statements.

**Step 4: Add version-gated fields**

Use probed server version/capabilities, not string assumptions in UI. Unsupported fields are omitted or disabled with a reason.

**Step 5: Run tests**

```bash
cargo test --test catalog_mutation postgres_view
cargo test --test postgres_adapter view_mutation -- --nocapture --test-threads=1
cargo test --test ui_render view_editor
```

Expected: PASS or integration skip.

**Step 6: Checkpoint commit, only if requested**

```bash
git add src/db/catalog_mutation.rs src/model/catalog_editor.rs src/db/postgres.rs src/ui/catalog_editor.rs tests/catalog_mutation.rs tests/postgres_adapter.rs tests/ui_render.rs
git commit -m "feat(postgres): create and edit views"
```

### Task 16: Create And Edit Materialized Views Safely

**Files:**
- Modify: `src/db/catalog_mutation.rs`
- Modify: `src/model/catalog_editor.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/ui/catalog_editor.rs`
- Test: `tests/catalog_mutation.rs`
- Test: `tests/postgres_adapter.rs`

**Step 1: Write failing tests**

Cover:

- Create with name, schema, owner, comment, query, tablespace, and `WITH [NO] DATA`.
- Edit supports rename, schema, owner, comment, tablespace where native ALTER supports it.
- Edit does not expose query-definition replacement.
- `a` offers Index but not Column/constraints.
- SQL preview clearly identifies `WITH NO DATA` state.

**Step 2: Run and verify failure**

```bash
cargo test --test catalog_mutation postgres_materialized_view
```

Expected: FAIL.

**Step 3: Implement typed draft and planner**

Use separate create/edit form projections so definition text is display-only during edit. Do not silently drop/recreate the materialized view.

**Step 4: Run tests**

```bash
cargo test --test catalog_mutation postgres_materialized_view
cargo test --test postgres_adapter materialized_view_mutation -- --nocapture --test-threads=1
```

Expected: PASS or integration skip.

**Step 5: Checkpoint commit, only if requested**

```bash
git add src/db/catalog_mutation.rs src/model/catalog_editor.rs src/db/postgres.rs src/ui/catalog_editor.rs tests/catalog_mutation.rs tests/postgres_adapter.rs
git commit -m "feat(postgres): manage materialized views"
```

### Task 17: Create And Edit Sequences

**Files:**
- Modify: `src/db/catalog_mutation.rs`
- Modify: `src/model/catalog_editor.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/ui/catalog_editor.rs`
- Test: `tests/catalog_mutation.rs`
- Test: `tests/postgres_adapter.rs`

**Step 1: Write failing tests**

Cover:

- Name, schema, owner, comment.
- Data type, increment, min/max/no-min/no-max, start/restart, cache, cycle/no-cycle.
- Optional `OWNED BY table.column` and `OWNED BY NONE`.
- Numeric validation and PostgreSQL range errors caught before planning where deterministic.
- Sequence selection has no create-child options.

**Step 2: Run and verify failure**

```bash
cargo test --test catalog_mutation postgres_sequence
```

Expected: FAIL.

**Step 3: Implement definition, draft, planner, and UI**

Load from `pg_sequence`, `pg_class`, dependency catalogs, roles, and comments. Preserve numeric values without lossy conversion. Use explicit optional-value states so empty input is not confused with `NO MINVALUE`/`NO MAXVALUE`.

**Step 4: Run tests**

```bash
cargo test --test catalog_mutation postgres_sequence
cargo test --test postgres_adapter sequence_mutation -- --nocapture --test-threads=1
```

Expected: PASS or integration skip.

**Step 5: Checkpoint commit, only if requested**

```bash
git add src/db/catalog_mutation.rs src/model/catalog_editor.rs src/db/postgres.rs src/ui/catalog_editor.rs tests/catalog_mutation.rs tests/postgres_adapter.rs
git commit -m "feat(postgres): create and edit sequences"
```

## Milestone 7: PostgreSQL Database And Role Management

### Task 18: Add Maintenance-Target Connection Execution

**Files:**
- Modify: `src/db/catalog_mutation.rs`
- Modify: `src/db/mod.rs:155-179`
- Modify: `src/runtime.rs`
- Test: `tests/catalog_mutation.rs`
- Test: `tests/connection_switch.rs`
- Test: `tests/postgres_adapter.rs`

**Step 1: Write failing target-routing tests**

Cover:

- Same active database uses the active database clone.
- Different database creates and closes a temporary target connection.
- Temporary connection does not change `App.connection.target`, active workspace, or console execution targets.
- Profile-level operation resolves a maintenance database deterministically: configured maintenance target if later added, otherwise active database when legal, otherwise `postgres`.
- Database rename refuses to run while connected to the database being renamed.
- Credential resolution failure returns a sanitized mutation failure.

**Step 2: Run and verify failure**

```bash
cargo test --test connection_switch catalog_mutation_target
```

Expected: FAIL because Runtime supports target switching for consoles but not isolated catalog operations.

**Step 3: Add execution target policy**

Define:

```rust
pub enum CatalogMutationTarget {
    Database(ExecutionTarget),
    Maintenance { database: String },
}
```

Runtime reuses existing credential resolution and `DatabaseConnection::connect_target()` where possible. Add a narrow helper that returns an owned temporary connection guard or active clone and always closes owned connections.

**Step 4: Run tests**

```bash
cargo test --test connection_switch catalog_mutation_target
```

Expected: PASS.

**Step 5: Checkpoint commit, only if requested**

```bash
git add src/db/catalog_mutation.rs src/db/mod.rs src/runtime.rs tests/catalog_mutation.rs tests/connection_switch.rs tests/postgres_adapter.rs
git commit -m "feat(runtime): route catalog mutations to target databases"
```

### Task 19: Create And Edit PostgreSQL Databases

**Files:**
- Modify: `src/db/catalog_mutation.rs`
- Modify: `src/model/catalog_editor.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/ui/catalog_editor.rs`
- Test: `tests/catalog_mutation.rs`
- Test: `tests/postgres_adapter.rs`

**Step 1: Write failing planner tests**

Create fields:

- Name, owner, template, encoding, locale provider/options, tablespace, connection limit, allow connections, is template, comment where supported.

Edit fields for initial release:

- Owner, connection limit, allow connections, is template, comment.
- Rename only when maintenance-target preconditions are met.

Tests must assert:

- `CREATE DATABASE` plan uses `Autocommit` and exactly one create statement plus separately executed comments/options when PostgreSQL requires it.
- Runtime never wraps `CREATE DATABASE` in a transaction.
- Rename current database is rejected before execution.
- Refresh target is `CatalogTarget::Databases`.
- No automatic profile rewrite occurs.

**Step 2: Run and verify failure**

```bash
cargo test --test catalog_mutation postgres_database
```

Expected: FAIL.

**Step 3: Implement database definitions and planner**

Load from `pg_database`, roles, tablespaces, and shared comments. Keep immutable create-only fields display-only during edit.

For multi-statement autocommit plans, execute sequentially on one maintenance connection and report partial completion explicitly if a later statement fails. Do not claim transactional rollback.

**Step 4: Add post-rename warning**

After successful rename, show a message explaining that connection profiles and saved SQL execution targets were not rewritten. A separate confirmed profile migration can be designed later.

**Step 5: Run tests**

```bash
cargo test --test catalog_mutation postgres_database
cargo test --test postgres_adapter database_mutation -- --nocapture --test-threads=1
```

Expected: unit tests PASS; privileged integration test skips with an explicit reason unless the test role has `CREATEDB`.

**Step 6: Checkpoint commit, only if requested**

```bash
git add src/db/catalog_mutation.rs src/model/catalog_editor.rs src/db/postgres.rs src/ui/catalog_editor.rs tests/catalog_mutation.rs tests/postgres_adapter.rs
git commit -m "feat(postgres): create and edit databases"
```

### Task 20: Create And Edit PostgreSQL Roles

**Files:**
- Modify: `src/db/catalog_mutation.rs`
- Modify: `src/model/catalog_editor.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/ui/catalog_editor.rs`
- Modify: `src/security.rs`
- Test: `tests/catalog_mutation.rs`
- Test: `tests/postgres_adapter.rs`
- Test: `tests/ui_render.rs`

**Step 1: Write failing security and planner tests**

Cover:

- Login Role defaults `LOGIN`; Role defaults `NOLOGIN`.
- Name, superuser, createdb, createrole, inherit, replication, bypass RLS, connection limit, password, valid until, memberships.
- Password is stored in `SecretString`, omitted from Debug/equality snapshots, never rendered in SQL preview, notifications, or errors.
- Preview substitutes a fixed redaction token while execution retains the secret through a separate sealed parameter channel.
- Existing password is never loaded from PostgreSQL and blank edit means unchanged.
- Rename, attributes, membership grant/revoke, and comment plans.

**Step 2: Run and verify failure**

```bash
cargo test --test catalog_mutation postgres_role
cargo test --test ui_render role_editor_secret
```

Expected: FAIL.

**Step 3: Separate preview SQL from executable secret material**

Do not place plaintext passwords in `CatalogMutationPlan.statements`. Add a sealed adapter-owned statement representation capable of carrying secret bind/input material while exposing only redacted preview text. If PostgreSQL DDL cannot bind the password in the chosen SQLx API, construct executable SQL only inside Runtime immediately before sending it, keep it in `SecretString`-backed scope, and never clone it into actions or display state.

**Step 4: Implement role definition/draft/planner**

Use `pg_roles` and membership catalogs. Do not expose password hashes. Gate privileged toggles by server response rather than optimistic client-side privilege checks.

**Step 5: Run tests**

```bash
cargo test --test catalog_mutation postgres_role
cargo test --test ui_render role_editor_secret
cargo test --test postgres_adapter role_mutation -- --nocapture --test-threads=1
```

Expected: unit/render tests PASS; privileged integration test skips unless the role has `CREATEROLE`.

**Step 6: Checkpoint commit, only if requested**

```bash
git add src/db/catalog_mutation.rs src/model/catalog_editor.rs src/db/postgres.rs src/ui/catalog_editor.rs src/security.rs tests/catalog_mutation.rs tests/postgres_adapter.rs tests/ui_render.rs
git commit -m "feat(postgres): create and edit roles"
```

## Milestone 8: Documentation And Release Verification

### Task 21: Make Help And Documentation Capability-Aware

**Files:**
- Modify: `src/help.rs`
- Modify: `docs/keybindings.md:71-102`
- Modify: `docs/architecture.md:126-172`
- Modify: `README.md`
- Test: `tests/keymap.rs`
- Test: `tests/ui_render.rs`

**Step 1: Write failing help tests**

Cover:

- Explorer help shows `a` only when the selected node has an available create option.
- Explorer help shows `e` for profiles and editable catalog objects, not groups/synthetic rows.
- The labels are `add object` and `edit selected object` rather than PostgreSQL-specific text.
- Catalog Editor help changes by picker/form/preview/busy page.

**Step 2: Run and verify failure**

```bash
cargo test --test keymap catalog_editor_help
cargo test --test ui_render catalog_editor_help
```

Expected: FAIL.

**Step 3: Extend shortcut capabilities**

Add `catalog_create_available` and `catalog_edit_available` to `ShortcutCapabilities`, deriving them from current editor/selection state. Keep shortcut catalog entries static and filter via requirements, matching existing help architecture.

**Step 4: Update docs**

Document:

- `n` creates a connection profile.
- `a` creates a supported child object.
- `e` edits the directly selected profile/catalog object.
- Adapter-owned capabilities and DDL planning.
- Definition/plan stale checks.
- Temporary target connections.
- Targeted refresh and relation snapshot invalidation.
- PostgreSQL object support matrix and known limitations.

**Step 5: Run tests**

```bash
cargo test --test keymap catalog_editor_help
cargo test --test ui_render catalog_editor_help
```

Expected: PASS.

**Step 6: Checkpoint commit, only if requested**

```bash
git add src/help.rs docs/keybindings.md docs/architecture.md README.md tests/keymap.rs tests/ui_render.rs
git commit -m "docs: describe explorer catalog object editing"
```

### Task 22: Run End-To-End PostgreSQL Scenarios

**Files:**
- Modify: `tests/postgres_adapter.rs`
- Modify: `CONTRIBUTING.md`

**Step 1: Add one serialized integration scenario**

Under a uniquely named temporary schema:

1. Create schema.
2. Edit owner/comment where privileges permit.
3. Create table with columns.
4. Add/edit a column.
5. Add index, unique, check, and foreign key.
6. Rename supported child objects.
7. Create/edit view.
8. Create/edit materialized view attributes and index.
9. Create/edit sequence.
10. Reload relevant catalog pages after every mutation and assert Explorer-facing metadata.
11. Drop all temporary objects in reverse dependency order.

Use a cleanup guard so a failed assertion still attempts cleanup. Do not run these tests concurrently against one database.

**Step 2: Run the integration suite**

```bash
LAZYDB_TEST_POSTGRES_URL='postgresql://user:password@localhost:5432/database' \
  cargo test --test postgres_adapter catalog_mutation_e2e -- --nocapture --test-threads=1
```

Expected: PASS. Without the environment variable, the test must report the established skip behavior rather than fail.

**Step 3: Document privileged tests**

Add separate instructions for optional Database/Role scenarios and their required `CREATEDB`/`CREATEROLE` privileges. Never require those privileges for the standard CI adapter job.

**Step 4: Checkpoint commit, only if requested**

```bash
git add tests/postgres_adapter.rs CONTRIBUTING.md
git commit -m "test(postgres): cover catalog mutation workflows"
```

### Task 23: Full Verification And Regression Audit

**Files:**
- Modify only files required by failures attributable to this feature.

**Step 1: Format**

```bash
cargo fmt --all -- --check
```

Expected: PASS. If it fails, run `cargo fmt --all`, inspect the diff, and rerun the check.

**Step 2: Run focused non-database tests**

```bash
cargo test --test catalog_mutation
cargo test --test catalog_editor_state
cargo test --test catalog_editor_reducer
cargo test --test catalog_reducer
cargo test --test keymap
cargo test --test relation_tabs
cargo test --test ui_render
```

Expected: PASS.

**Step 3: Run static checks**

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS.

**Step 4: Run the full test suite**

```bash
cargo test --all-targets --all-features
```

Expected: PASS; external-database tests follow their existing environment-gated behavior.

**Step 5: Run configured adapter tests**

```bash
LAZYDB_TEST_POSTGRES_URL='postgresql://user:password@localhost:5432/database' \
  cargo test --test postgres_adapter -- --nocapture --test-threads=1
```

Expected: PASS against PostgreSQL 12 or newer.

**Step 6: Audit invariants manually**

Inspect the final diff and verify:

- No DDL construction entered App, keymap, model, or UI modules.
- No plaintext role password appears in Debug output, actions, plan preview, notifications, or tests.
- MySQL/SQLite do not advertise unsupported PostgreSQL operations.
- All async response reducers reject stale connection/request/epoch/object combinations.
- Read-only checks exist in App and Runtime.
- Cross-database operations do not mutate active workspace/console targets.
- Mutation success refreshes authoritative targets rather than inserting guessed IDs.
- Existing `e` on a profile opens the unchanged Profile Manager.
- Existing `n`, `d`, `c`, `x`, search, relation preview, and DDL shortcuts remain unchanged.

**Step 7: Review worktree**

```bash
git status --short
git diff --check
git diff --stat
```

Expected: only intended implementation, tests, and documentation are changed; `git diff --check` emits no output.

**Step 8: Final checkpoint commit, only if requested**

```bash
git add src tests docs README.md CONTRIBUTING.md
git commit -m "feat(explorer): add PostgreSQL catalog object editor"
```

## Acceptance Criteria

- Explorer `a` opens only valid create operations for the directly selected node.
- Explorer `e` edits the selected profile or catalog object; catalog descendants no longer fall back to profile editing.
- PostgreSQL capabilities match native object semantics, especially View, Materialized View, and Sequence restrictions.
- Schema, Table, Column, Index, Primary/Unique/Foreign-Key/Check, View, Materialized View, Sequence, Database, Login Role, and Role workflows have typed drafts and adapter-owned planners within the milestone scope.
- Existing objects are loaded authoritatively on demand; Explorer summary metadata is not treated as a complete editable definition.
- Every plan has a redacted SQL preview, execution mode, warnings, impact, refresh targets, and selection hint.
- Destructive plans require explicit lowercase `y` confirmation.
- Runtime revalidates profile identity, read-only policy, target, epoch/fingerprint, and plan integrity before execution.
- Transactional plans run on one physical connection and rollback as a unit on failure.
- Autocommit-only PostgreSQL operations are never placed in a transaction and report partial completion honestly.
- Cross-database operations use temporary connections and do not switch the active workspace or console target.
- Successful mutations trigger minimal authoritative reloads and invalidate affected relation snapshots.
- Role passwords never enter displayable or cloneable application state in plaintext.
- MySQL and SQLite expose no catalog mutations until their concrete implementations exist.
- Focused tests, Clippy, formatting, full tests, and configured PostgreSQL integration tests pass.

## Recommended Execution Slices

Do not implement all milestones as one pull request. Use these reviewable slices:

1. Tasks 1-4: protocol, dispatch, state, empty overlay shell.
2. Tasks 5-8: complete Schema create/edit vertical slice.
3. Tasks 9-10: targeted refresh and relation invalidation.
4. Tasks 11-12: Table and Column workflows.
5. Tasks 13-14: Index and Constraint workflows.
6. Tasks 15-17: View, Materialized View, Sequence workflows.
7. Tasks 18-20: target routing, Database, Role workflows.
8. Tasks 21-23: docs, end-to-end tests, regression verification.

Each slice must leave capability declarations truthful. Do not advertise an operation until its definition loader, planner, execution path, refresh path, UI, and tests are all present.
