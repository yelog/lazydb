# Explorer Catalog Object Drop Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make Explorer `d` delete only the directly selected profile or database object, with adapter-generated SQL and an exact lowercase `y` plus Enter confirmation for catalog drops.

**Architecture:** Add a two-phase catalog-drop protocol: the app requests an adapter-owned plan from the runtime, then presents the returned immutable plan in a dedicated overlay and sends it back for execution only after strict confirmation. Bind every plan and response to the active `ConnectionIdentity`, catalog generation, object identity, and request ID; reconcile Explorer, completion, execution-target, and relation snapshot state only after a matching success response.

**Tech Stack:** Rust 2024, Tokio, SQLx PostgreSQL/MySQL/SQLite adapters, Crossterm input, Ratatui UI, existing reducer/command runtime architecture.

---

## Implementation Constraints

- Do not construct destructive SQL in `src/input/keymap.rs`, `src/app.rs`, or UI modules.
- Do not route catalog drops through a SQL console or its manual transaction.
- Do not add `CASCADE` or `IF EXISTS` to generated statements.
- Do not permit an arbitrary SQL string supplied by UI state to become executable.
- Do not make a catalog descendant fall back to its profile or owning relation.
- Treat every `CatalogKind` explicitly. Return a typed unsupported result rather than guessing.
- Keep profile deletion behavior unchanged when the directly selected node is `ExplorerNodeId::Profile`.
- Run focused tests after each task and the complete verification suite at the end.
- The commit steps below are checkpoints for an implementation session. Execute them only when the user has explicitly requested commits.

### Task 1: Correct Explorer `d` Dispatch

**Files:**
- Modify: `src/action.rs`
- Modify: `src/input/keymap.rs:1187-1204`
- Test: `tests/keymap.rs`

**Step 1: Write failing direct-node keymap tests**

Extend the Explorer keymap fixtures to select each direct node shape and assert:

```rust
assert_eq!(
    keymap.map(key(KeyCode::Char('d')), &app),
    Some(Action::RequestDropCatalogObject { id: table_id.clone() })
);
```

Add separate assertions that:

```rust
// Only a directly selected profile root requests profile deletion.
assert_eq!(
    keymap.map(key(KeyCode::Char('d')), &profile_app),
    Some(Action::ProfileRequestDelete { profile_id })
);

// Synthetic/presentation rows do not inherit a destructive owner action.
assert_eq!(keymap.map(key(KeyCode::Char('d')), &group_app), None);
assert_eq!(keymap.map(key(KeyCode::Char('d')), &status_app), None);
```

Cover `Catalog`, `Group`, `Status`, `LoadMore`, `Empty`, `EmptyProfiles`, and
`Others`. The regression assertion is that a selected table never emits
`ProfileRequestDelete`.

**Step 2: Run the focused test and verify failure**

Run:

```bash
cargo test --test keymap explorer_d
```

Expected: FAIL because `RequestDropCatalogObject` does not exist and catalog
nodes still map to `ProfileRequestDelete`.

**Step 3: Add the catalog drop request action**

Add to `Action`:

```rust
RequestDropCatalogObject {
    id: CatalogId,
},
```

Import `CatalogId` in `src/action.rs` alongside the existing catalog types.

**Step 4: Match the directly selected node**

Replace the `selected_profile`-based `d` branch with a direct match:

```rust
KeyCode::Char('d') => {
    return match app.explorer.normalized.selected.as_ref() {
        Some(ExplorerNodeId::Profile(profile_id)) => {
            Some(Action::ProfileRequestDelete {
                profile_id: *profile_id,
            })
        }
        Some(ExplorerNodeId::Catalog(id)) => {
            Some(Action::RequestDropCatalogObject { id: id.clone() })
        }
        _ => None,
    };
}
```

Keep `selected_profile` for `e`, `c`, and `x` unless their existing inherited
behavior is independently found to be wrong; this task changes only `d`.

**Step 5: Run focused tests**

Run:

```bash
cargo test --test keymap explorer_d
```

Expected: PASS.

**Step 6: Checkpoint commit, only if requested**

```bash
git add src/action.rs src/input/keymap.rs tests/keymap.rs
git commit -m "fix(explorer): scope delete shortcut to selected node"
```

### Task 2: Define Trusted Catalog Drop Types

**Files:**
- Create: `src/db/catalog_drop.rs`
- Modify: `src/db/mod.rs`
- Modify: `src/action.rs`
- Test: `tests/catalog_drop.rs`

**Step 1: Write failing model validation tests**

Create `tests/catalog_drop.rs` with tests for:

- Plan connection profile matches `CatalogId.connection_id`.
- SQL must contain exactly one non-empty statement.
- SQL containing `CASCADE` is rejected.
- SQL containing `IF EXISTS` is rejected.
- Empty display names and SQL are rejected.
- Every `CatalogKind` has a stable display label.

Use the existing SQL parser/risk classifier where it can reliably enforce one
DDL statement. Do not use substring checks as the sole multi-statement defense;
quoted strings and comments must not create false positives.

**Step 2: Run the new test and verify failure**

Run:

```bash
cargo test --test catalog_drop
```

Expected: FAIL because `db::catalog_drop` does not exist.

**Step 3: Add immutable plan and typed errors**

Define the minimal shared protocol:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogDropPlan {
    pub request_id: u64,
    pub connection: ConnectionIdentity,
    pub catalog_generation: u64,
    pub object_id: CatalogId,
    pub object_kind: CatalogKind,
    pub display_name: String,
    sql: String,
}

impl CatalogDropPlan {
    pub fn sql(&self) -> &str {
        &self.sql
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CatalogDropError {
    #[error("cannot drop {kind}: {reason}")]
    Unsupported { kind: &'static str, reason: String },
    #[error("catalog drop target is stale")]
    Stale,
    #[error("catalog drop target does not belong to the active connection")]
    WrongConnection,
    #[error("catalog drop plan is invalid: {0}")]
    InvalidPlan(String),
    #[error("{0}")]
    Database(String),
}
```

Keep construction private to the database module, for example with
`pub(crate) fn validated(...)`. Expose SQL read-only. Add a stable
`CatalogKind::display_name()` helper if the label is useful outside this module.

**Step 4: Add two-phase actions and commands**

Add actions:

```rust
CatalogDropPlanSucceeded { plan: CatalogDropPlan },
CatalogDropPlanFailed {
    request_id: u64,
    connection: ConnectionIdentity,
    object_id: CatalogId,
    message: String,
},
CatalogDropInsert(char),
CatalogDropBackspace,
CatalogDropClear,
CatalogDropConfirm,
CatalogDropCancel,
CatalogDropSucceeded {
    request_id: u64,
    connection: ConnectionIdentity,
    object_id: CatalogId,
},
CatalogDropFailed {
    request_id: u64,
    connection: ConnectionIdentity,
    object_id: CatalogId,
    message: String,
},
```

Add commands:

```rust
PlanCatalogDrop {
    request_id: u64,
    connection: ConnectionIdentity,
    catalog_generation: u64,
    object_id: CatalogId,
},
ExecuteCatalogDrop {
    plan: CatalogDropPlan,
},
```

The execute command carries the sealed plan, not a separate editable SQL string.

**Step 5: Run model tests**

Run:

```bash
cargo test --test catalog_drop
```

Expected: PASS.

**Step 6: Checkpoint commit, only if requested**

```bash
git add src/db/catalog_drop.rs src/db/mod.rs src/action.rs tests/catalog_drop.rs
git commit -m "feat(db): define trusted catalog drop plans"
```

### Task 3: Implement PostgreSQL Drop Planning

**Files:**
- Modify: `src/db/postgres.rs`
- Modify: `src/db/mod.rs`
- Test: `tests/postgres_adapter.rs`
- Test: `tests/catalog_drop.rs`

**Step 1: Write failing PostgreSQL planning tests**

Use pure planner fixtures rather than requiring a live PostgreSQL server. Cover:

- `DROP DATABASE`, `DROP SCHEMA`, `DROP TABLE`, `DROP VIEW`,
  `DROP MATERIALIZED VIEW`, `DROP SEQUENCE`, and `DROP TYPE`.
- `ALTER TABLE ... DROP COLUMN`.
- `ALTER TABLE ... DROP CONSTRAINT` for primary, unique, foreign, and check
  constraints.
- `DROP INDEX` using its schema-qualified identity.
- `DROP TRIGGER ... ON schema.table`.
- Functions and procedures include a complete argument type identity.
- A function/procedure without signature metadata returns `Unsupported`.
- Names such as `order`, `Mixed Case`, and `odd"name` are quoted with
  `quote_identifier`.
- No result includes `CASCADE` or `IF EXISTS`.

**Step 2: Run tests and verify failure**

Run:

```bash
cargo test --test postgres_adapter catalog_drop
```

Expected: FAIL because PostgreSQL has no drop planner.

**Step 3: Add a PostgreSQL planner input**

Pass the target `CatalogEntry`, owning relation when required, and adapter-owned
native metadata into a pure helper. Do not infer a routine signature from a
display label. If existing `CatalogMetadata` cannot retain the required routine
identity, add the smallest explicit metadata variant/field and populate it in
PostgreSQL catalog discovery before enabling routine drops.

The planner must exhaustively match `CatalogKind`. Return `Unsupported` for any
case lacking a native identity or safe statement.

**Step 4: Implement dialect-correct statements**

Use existing `postgres::quote_identifier`. Fully qualify schema objects. For
database-level operations, reject dropping the database used by the active
connection unless a safe maintenance connection is explicitly implemented;
YAGNI means rejecting it in this feature is acceptable.

**Step 5: Run PostgreSQL tests**

Run:

```bash
cargo test --test postgres_adapter catalog_drop
cargo test --test catalog_drop postgres
```

Expected: PASS.

**Step 6: Checkpoint commit, only if requested**

```bash
git add src/db/postgres.rs src/db/mod.rs src/db/catalog.rs tests/postgres_adapter.rs tests/catalog_drop.rs
git commit -m "feat(postgres): plan catalog object drops"
```

### Task 4: Implement MySQL Drop Planning

**Files:**
- Modify: `src/db/mysql.rs`
- Modify: `src/db/mod.rs`
- Test: `tests/mysql_adapter.rs`
- Test: `tests/catalog_drop.rs`

**Step 1: Write failing MySQL planning tests**

Cover:

- Database/Schema aliases map to one valid MySQL namespace operation and do not
  produce duplicate semantic choices.
- `DROP TABLE` and `DROP VIEW` use database-qualified names.
- Materialized views, sequences, and standalone types return `Unsupported`.
- `ALTER TABLE ... DROP COLUMN`.
- `DROP INDEX ... ON database.table`.
- Primary key uses `ALTER TABLE ... DROP PRIMARY KEY`.
- Foreign keys use `DROP FOREIGN KEY`.
- Unique/check constraints use syntax supported by the configured MySQL version
  and their native metadata; otherwise return `Unsupported`.
- Triggers, functions, and procedures use valid database qualification.
- Reserved words, spaces, and backticks use `mysql::quote_identifier`.
- No statement contains `CASCADE` or `IF EXISTS`.

**Step 2: Run tests and verify failure**

Run:

```bash
cargo test --test mysql_adapter catalog_drop
```

Expected: FAIL because MySQL has no drop planner.

**Step 3: Implement exhaustive MySQL planning**

Reuse `qualified_name`, `parent_id`, `owning_relation_id`, and existing native
constraint/index metadata. Add only metadata that catalog discovery can source
reliably. Reject operations whose syntax depends on unavailable server metadata.

Reject dropping the active database from its own active connection unless the
runtime gains a separately targeted safe connection. Do not reconnect silently.

**Step 4: Run MySQL tests**

Run:

```bash
cargo test --test mysql_adapter catalog_drop
cargo test --test catalog_drop mysql
```

Expected: PASS.

**Step 5: Checkpoint commit, only if requested**

```bash
git add src/db/mysql.rs src/db/mod.rs src/db/catalog.rs tests/mysql_adapter.rs tests/catalog_drop.rs
git commit -m "feat(mysql): plan catalog object drops"
```

### Task 5: Implement SQLite Drop Planning

**Files:**
- Modify: `src/db/sqlite.rs`
- Modify: `src/db/mod.rs`
- Test: `tests/sqlite_adapter.rs`
- Test: `tests/catalog_drop.rs`

**Step 1: Write failing SQLite planning tests**

Cover:

- `DROP TABLE`, `DROP VIEW`, `DROP INDEX`, and `DROP TRIGGER` with attached-schema
  qualification accepted by SQLite.
- `Database`, `Schema`, `MaterializedView`, routines, sequences, and types return
  explicit `Unsupported` results.
- Column and constraint operations that require table reconstruction return
  `Unsupported`; do not perform implicit migrations.
- `main`, `temp`, and attached aliases never generate `DROP DATABASE` or
  `DROP SCHEMA`.
- Embedded double quotes are escaped with the adapter's existing quote helper.
- No statement contains `CASCADE` or `IF EXISTS`.

**Step 2: Run tests and verify failure**

Run:

```bash
cargo test --test sqlite_adapter catalog_drop
```

Expected: FAIL because SQLite has no drop planner.

**Step 3: Implement exhaustive SQLite planning**

Use SQLite's existing identifier quoting and exact schema-object rules. Keep the
single-connection model. Do not introduce table-copy/rebuild behavior as a side
effect of pressing `d`.

**Step 4: Run SQLite tests**

Run:

```bash
cargo test --test sqlite_adapter catalog_drop
cargo test --test catalog_drop sqlite
```

Expected: PASS.

**Step 5: Checkpoint commit, only if requested**

```bash
git add src/db/sqlite.rs src/db/mod.rs tests/sqlite_adapter.rs tests/catalog_drop.rs
git commit -m "feat(sqlite): plan supported catalog drops"
```

### Task 6: Add Runtime Plan Generation

**Files:**
- Modify: `src/runtime.rs:181-258`
- Modify: `src/db/mod.rs`
- Modify: `src/action.rs`
- Test: `tests/relation_runtime.rs` or create `tests/catalog_drop_runtime.rs`

**Step 1: Write failing runtime plan tests**

Use the runtime test harness/fake adapter boundary to verify:

- A matching active connection returns `CatalogDropPlanSucceeded`.
- Missing or changed active connection returns `CatalogDropPlanFailed`.
- Wrong profile IDs are rejected before adapter planning.
- Read-only profiles are rejected.
- Duplicate/stale request completion cannot replace a newer request.
- Unsupported adapter results preserve their specific reason.

**Step 2: Run and verify failure**

Run:

```bash
cargo test --test catalog_drop_runtime plan
```

Expected: FAIL because `PlanCatalogDrop` is not dispatched.

**Step 3: Add database planning dispatch**

Add to `DatabaseConnection`:

```rust
pub async fn plan_catalog_drop(
    &self,
    request: CatalogDropPlanRequest,
) -> Result<CatalogDropPlan, CatalogDropError>
```

Dispatch to the active adapter. The request must include the connection,
catalog generation, object ID, and request ID. If planning needs fresh owner or
signature metadata, query it through the adapter under the same identity rather
than trusting UI display text.

**Step 4: Dispatch `Command::PlanCatalogDrop`**

Use `active_database(...)` and a tracked background task. Send exactly one
success/failure action. Sanitize database text at the existing error boundary.

Track the latest plan request so stale task completion can be ignored or
aborted, following catalog search/relation request generation patterns.

**Step 5: Run runtime tests**

Run:

```bash
cargo test --test catalog_drop_runtime plan
```

Expected: PASS.

**Step 6: Checkpoint commit, only if requested**

```bash
git add src/runtime.rs src/db/mod.rs src/action.rs tests/catalog_drop_runtime.rs
git commit -m "feat(runtime): generate catalog drop plans"
```

### Task 7: Add App Planning State And Safety Checks

**Files:**
- Modify: `src/model/workspace.rs:102-144`
- Modify: `src/model/text_input.rs`
- Modify: `src/app.rs`
- Test: `tests/catalog_reducer.rs`

**Step 1: Write failing reducer tests**

Cover:

- `RequestDropCatalogObject` sends one `PlanCatalogDrop` command containing the
  active identity, generation, and exact direct object ID.
- Offline/wrong-profile/read-only selections do not send a command and set a
  specific status.
- A running query, relation edit transaction, or manual console transaction is
  blocked without cancelling or resolving that work.
- A matching plan success opens the drop overlay.
- A stale request ID, connection, generation, or object result is ignored.
- Plan failure leaves Explorer unchanged and shows its reason.

**Step 2: Run and verify failure**

Run:

```bash
cargo test --test catalog_reducer catalog_drop_plan
```

Expected: FAIL because the reducer does not handle catalog drop actions.

**Step 3: Add dedicated overlay state**

Prefer a struct to an oversized enum payload:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogDropConfirmState {
    pub plan: CatalogDropPlan,
    pub input: TextInput,
    pub busy: bool,
    pub error: Option<String>,
}
```

Add `Overlay::CatalogDropConfirm(CatalogDropConfirmState)`.

Add only the `TextInput` accessors needed for exact comparison and clear; reuse
existing `insert`, `backspace`, and `clear` behavior.

**Step 4: Implement request checks and response matching**

Resolve the current catalog entry from the active profile tree before emitting
the command. Reuse existing query/transaction checks where semantics match.
Store the latest request identity in app state or in a planning overlay/state so
only the exact response can open confirmation.

**Step 5: Run reducer tests**

Run:

```bash
cargo test --test catalog_reducer catalog_drop_plan
```

Expected: PASS.

**Step 6: Checkpoint commit, only if requested**

```bash
git add src/model/workspace.rs src/model/text_input.rs src/app.rs tests/catalog_reducer.rs
git commit -m "feat(app): request validated catalog drop plans"
```

### Task 8: Implement Strict Confirmation Input

**Files:**
- Modify: `src/input/keymap.rs`
- Modify: `src/app.rs`
- Test: `tests/keymap.rs`
- Test: `tests/catalog_reducer.rs`

**Step 1: Write failing keymap and reducer tests**

Assert all required sequences:

```rust
// Enter alone: no execute command.
// 'y' alone: edits input only.
// 'y', Enter: exactly one ExecuteCatalogDrop command.
// 'Y', Enter: no command.
// 'yes', Enter: no command.
// busy + Enter: no duplicate command.
// Backspace edits, Ctrl-U clears, Esc closes.
```

Also verify invalid Enter stores `Type y before executing` without closing the
overlay.

**Step 2: Run and verify failure**

Run:

```bash
cargo test --test keymap catalog_drop_confirm
cargo test --test catalog_reducer catalog_drop_confirm
```

Expected: FAIL because the overlay input mapping is absent.

**Step 3: Map overlay keys before Explorer/global actions**

When `Overlay::CatalogDropConfirm` is active:

```rust
KeyCode::Char(character) => Action::CatalogDropInsert(character),
KeyCode::Backspace => Action::CatalogDropBackspace,
KeyCode::Char('u') with CONTROL => Action::CatalogDropClear,
KeyCode::Enter => Action::CatalogDropConfirm,
KeyCode::Esc => Action::CatalogDropCancel,
```

Do not map `y` directly to execution. Do not reuse
`map_profile_delete_confirmation`.

**Step 4: Implement reducer confirmation**

On Enter, compare the complete input with exactly `"y"`. If valid and not busy,
set `busy = true`, clear the previous error, and emit one
`ExecuteCatalogDrop { plan: plan.clone() }`. Otherwise store the prompt error.

**Step 5: Run focused tests**

Run:

```bash
cargo test --test keymap catalog_drop_confirm
cargo test --test catalog_reducer catalog_drop_confirm
```

Expected: PASS.

**Step 6: Checkpoint commit, only if requested**

```bash
git add src/input/keymap.rs src/app.rs tests/keymap.rs tests/catalog_reducer.rs
git commit -m "feat(ui): require typed catalog drop confirmation"
```

### Task 9: Render The Catalog Drop Overlay

**Files:**
- Create: `src/ui/catalog_drop.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/ui/state.rs` if hit regions or cursor state require it
- Test: `tests/ui_render.rs`

**Step 1: Write failing render tests**

Add narrow and wide terminal cases. Assert the buffer contains:

- Object-specific title such as `DROP TABLE`.
- Exact adapter-generated SQL, including quotes and qualification.
- `Type y and press Enter to execute`.
- The current input.
- `Esc cancel`.
- Busy text and disabled input during execution.
- Database error text after a failed execution.

Ensure long SQL wraps without removing its final identifier or hiding the input
line.

**Step 2: Run and verify failure**

Run:

```bash
cargo test --test ui_render catalog_drop
```

Expected: FAIL because no catalog drop renderer exists.

**Step 3: Add the dedicated renderer**

Use existing `render_panel`, safe terminal text handling, theme primitives, and
responsive sizing. Render SQL from `plan.sql()` without editing or reconstructing
it. Do not render an Enter-activated permanent-delete button.

Place the terminal cursor after the confirmation input while idle. Suppress it
while busy.

**Step 4: Run render tests**

Run:

```bash
cargo test --test ui_render catalog_drop
```

Expected: PASS.

**Step 5: Checkpoint commit, only if requested**

```bash
git add src/ui/catalog_drop.rs src/ui/mod.rs src/ui/state.rs tests/ui_render.rs
git commit -m "feat(ui): render catalog drop confirmation"
```

### Task 10: Execute Sealed Plans In Runtime

**Files:**
- Modify: `src/runtime.rs`
- Modify: `src/db/mod.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/db/mysql.rs`
- Modify: `src/db/sqlite.rs`
- Test: `tests/catalog_drop_runtime.rs`

**Step 1: Write failing execution tests**

Cover:

- A matching sealed plan executes exactly its stored statement once.
- Changed/missing active connection fails without calling execute.
- The runtime rejects a tampered/invalid plan.
- A read-only profile fails before database execution.
- Concurrent duplicate execution for the same request is rejected.
- Success and failure actions preserve request, connection, and object identity.
- MySQL execution does not use an active console transaction worker.

**Step 2: Run and verify failure**

Run:

```bash
cargo test --test catalog_drop_runtime execute
```

Expected: FAIL because `ExecuteCatalogDrop` is not dispatched.

**Step 3: Add adapter execution method**

Add a database-owned operation such as:

```rust
pub async fn execute_catalog_drop(
    &self,
    plan: &CatalogDropPlan,
) -> Result<(), CatalogDropError>
```

Revalidate the plan invariant and adapter kind before executing `plan.sql()`.
Do not expose a public constructor that allows the app to replace SQL.

**Step 4: Add runtime task tracking and dispatch**

Track in-flight catalog drop request IDs separately from query tasks. Resolve the
profile from the runtime registry and enforce `read_only == false`. Validate the
active connection before execution and again before reporting success if needed
for stale response behavior.

Send `CatalogDropSucceeded` or `CatalogDropFailed`; remove the task on completion.

**Step 5: Run execution tests**

Run:

```bash
cargo test --test catalog_drop_runtime execute
```

Expected: PASS.

**Step 6: Checkpoint commit, only if requested**

```bash
git add src/runtime.rs src/db/mod.rs src/db/postgres.rs src/db/mysql.rs src/db/sqlite.rs tests/catalog_drop_runtime.rs
git commit -m "feat(runtime): execute sealed catalog drop plans"
```

### Task 11: Reconcile Explorer And Completion State

**Files:**
- Modify: `src/model/explorer.rs:505-514`
- Modify: `src/sql/completion.rs:64-133`
- Modify: `src/app.rs`
- Test: `tests/explorer_state.rs`
- Test: `tests/sql_completion.rs`
- Test: `tests/catalog_reducer.rs`

**Step 1: Write failing state tests**

Cover:

- Successful table drop removes the table and all loaded descendants.
- Successful schema/database drop removes the complete loaded subtree.
- Selection moves to the nearest surviving parent/row.
- Expanded IDs and group children no longer reference removed nodes.
- Completion entries for the removed IDs disappear.
- Failed/stale drop changes no catalog or completion state.
- Catalog generation advances so late pages cannot restore removed objects.
- The nearest valid parent target is refreshed once.

**Step 2: Run and verify failure**

Run:

```bash
cargo test --test explorer_state catalog_drop
cargo test --test sql_completion remove_subtree
cargo test --test catalog_reducer catalog_drop_success
```

Expected: FAIL because completion has no subtree removal and success is not
handled.

**Step 3: Add completion removal**

Add a small method that removes exact catalog IDs and rebuilds indexes:

```rust
pub fn remove_ids(&mut self, removed: &HashSet<CatalogId>) {
    self.entries.retain(|entry| !removed.contains(&entry.id));
    self.rebuild();
}
```

Do not attempt incremental maintenance of every secondary map unless profiling
shows rebuilding is too expensive.

**Step 4: Apply matching success atomically in the reducer**

Verify the active overlay plan matches the response. Capture the parent refresh
target, call `remove_subtree`, remove completion entries, advance generation,
close the overlay, and schedule one parent refresh. Show a qualified success
message.

For a matching failure, set `busy = false`, clear confirmation input, retain the
plan, and store the sanitized error.

**Step 5: Run state tests**

Run:

```bash
cargo test --test explorer_state catalog_drop
cargo test --test sql_completion remove_subtree
cargo test --test catalog_reducer catalog_drop
```

Expected: PASS.

**Step 6: Checkpoint commit, only if requested**

```bash
git add src/model/explorer.rs src/sql/completion.rs src/app.rs tests/explorer_state.rs tests/sql_completion.rs tests/catalog_reducer.rs
git commit -m "feat(explorer): reconcile dropped catalog objects"
```

### Task 12: Reconcile Relation Snapshots And Execution Targets

**Files:**
- Modify: `src/model/relation.rs:71-77`
- Modify: `src/model/execution_target.rs`
- Modify: `src/app.rs`
- Modify: `src/ui/relation.rs`
- Test: `tests/relation_tabs.rs`
- Test: `tests/execution_target.rs`
- Test: `tests/ui_render.rs`

**Step 1: Write failing lifecycle tests**

Cover:

- Dropping a relation marks matching Data and DDL snapshots as dropped-object
  snapshots and prevents refresh as a live object.
- Dropping a schema/database applies the same state to descendant relation tabs.
- Unrelated relation tabs remain live.
- Dropping a namespace that owns the active execution target is blocked before
  planning unless a safe target transition is known.
- The UI renders `OBJECT DROPPED SNAPSHOT` distinctly from profile-deleted and
  out-of-scope snapshots.

**Step 2: Run and verify failure**

Run:

```bash
cargo test --test relation_tabs dropped_snapshot
cargo test --test execution_target catalog_drop
cargo test --test ui_render dropped_snapshot
```

Expected: FAIL because no dropped-object provenance exists.

**Step 3: Add dropped snapshot provenance**

Add:

```rust
RelationSnapshotProvenance::ObjectDroppedSnapshot
```

Update all exhaustive matches and UI labels. Determine descendant membership by
catalog identity/native path rules already validated by the tree, not by string
prefixes on display names.

**Step 4: Add namespace target protection**

Before planning Database or Schema drops, compare the object identity to the
active `ExecutionTarget`. Block with a concrete status if the operation would
invalidate it. Do not silently choose another database/schema in this feature.

**Step 5: Run lifecycle tests**

Run:

```bash
cargo test --test relation_tabs dropped_snapshot
cargo test --test execution_target catalog_drop
cargo test --test ui_render dropped_snapshot
```

Expected: PASS.

**Step 6: Checkpoint commit, only if requested**

```bash
git add src/model/relation.rs src/model/execution_target.rs src/app.rs src/ui/relation.rs tests/relation_tabs.rs tests/execution_target.rs tests/ui_render.rs
git commit -m "feat(relation): retain dropped object snapshots"
```

### Task 13: Add Contextual Hints And Documentation

**Files:**
- Modify: `src/help.rs`
- Modify: relevant Explorer footer/hint module under `src/ui/`
- Modify: `docs/keybindings.md:70-103`
- Modify: `docs/database-capabilities.md`
- Test: `tests/ui_render.rs`
- Test: `tests/keymap.rs`

**Step 1: Write failing hint/help tests**

Assert:

- Profile root shows `d delete connection`.
- Supported table/view selection shows `d drop table/view`.
- Unsupported and synthetic nodes do not advertise an executable destructive
  hint.
- Contextual help has distinct entries for profile deletion and catalog drop.

**Step 2: Run and verify failure**

Run:

```bash
cargo test --test ui_render explorer_drop_hint
cargo test --test keymap catalog_drop_help
```

Expected: FAIL because `d` is documented only as profile deletion.

**Step 3: Implement contextual labels**

Derive labels from the direct selected node and known adapter capability. If
capability is not cached, use a conservative generic `d drop object`; do not
claim support before planning.

Update `docs/keybindings.md` to state that `d` deletes a connection only on its
profile root and otherwise requests a drop for the directly selected catalog
object. Document exact lowercase `y` plus Enter confirmation.

Update `docs/database-capabilities.md` with the adapter-owned drop-plan contract,
default dependency restriction, and explicit unsupported behavior.

**Step 4: Run documentation-facing tests**

Run:

```bash
cargo test --test ui_render explorer_drop_hint
cargo test --test keymap catalog_drop_help
```

Expected: PASS.

**Step 5: Checkpoint commit, only if requested**

```bash
git add src/help.rs src/ui docs/keybindings.md docs/database-capabilities.md tests/ui_render.rs tests/keymap.rs
git commit -m "docs(explorer): document catalog drop workflow"
```

### Task 14: Full Verification And Regression Review

**Files:**
- Modify only files required by verification failures

**Step 1: Format the workspace**

Run:

```bash
cargo fmt --all --check
```

Expected: PASS. If it fails, run `cargo fmt --all`, inspect the formatting diff,
then rerun the check.

**Step 2: Run focused feature tests together**

Run:

```bash
cargo test --test catalog_drop
cargo test --test catalog_drop_runtime
cargo test --test keymap explorer_d
cargo test --test keymap catalog_drop
cargo test --test catalog_reducer catalog_drop
cargo test --test ui_render catalog_drop
```

Expected: PASS.

**Step 3: Run all adapter and lifecycle tests**

Run:

```bash
cargo test --test postgres_adapter
cargo test --test mysql_adapter
cargo test --test sqlite_adapter
cargo test --test explorer_state
cargo test --test sql_completion
cargo test --test relation_tabs
cargo test --test execution_target
```

Expected: PASS.

**Step 4: Run the complete test suite**

Run:

```bash
cargo test --all-targets
```

Expected: PASS.

**Step 5: Run Clippy with warnings denied**

Run:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS.

**Step 6: Inspect the final diff**

Run:

```bash
git status --short
git diff --check
git diff --stat
```

Expected: only catalog-drop implementation, tests, and documentation are
changed; `git diff --check` reports no whitespace errors. Do not revert unrelated
user changes in a dirty worktree.

**Step 7: Final checkpoint commit, only if requested**

```bash
git add src tests docs/keybindings.md docs/database-capabilities.md docs/plans/2026-09-01-explorer-catalog-drop-design.md docs/plans/2026-09-01-explorer-catalog-drop-implementation.md
git commit -m "feat(explorer): safely drop catalog objects"
```
