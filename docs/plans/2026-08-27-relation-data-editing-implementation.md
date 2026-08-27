# Relation Data Editing Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add immediate, transactional editing for live primary-key table previews, including cell updates, row deletion, row insertion and duplication, undo/redo, explicit commit, and explicit rollback.

**Architecture:** Keep relation editing state on `RelationTab`, send immutable typed mutations through App and Runtime, and execute them with bound SQLx values on a relation-owned pinned transaction connection. Use optimistic full-row comparisons, per-action savepoints, compensating undo/redo, and existing connection identity and generation checks.

**Tech Stack:** Rust 2024, Tokio, SQLx 0.9, Ratatui 0.30, Crossterm, existing LazyDB App/Runtime/adapter architecture.

---

## Preconditions

- Preserve the pre-existing uncommitted changes in `src/app.rs` and `tests/app_flow.rs`; inspect their current diff before touching either file.
- Do not edit arbitrary SQL result behavior while adding relation-only editing.
- Do not add a dependency unless dynamic SQLx binding proves impossible with existing SQLx APIs.
- Keep every request immutable and validate tab generation, edit generation, connection identity, target, scope, relation ID, and metadata fingerprint before I/O.
- Run the focused test named in each step before moving to broader test commands.

### Task 1: Add Vertical Grid Viewport

**Files:**
- Modify: `src/model/tab.rs:21-29,170-210`
- Modify: `src/ui/data_grid.rs:17-153`
- Modify: `src/app.rs:4733-4777`
- Modify: `tests/mouse.rs`
- Modify: `tests/relation_tabs.rs`

**Step 1: Write failing model tests**

Add tests proving that `DataGridState`:

- stores `row_offset`;
- keeps the selected row inside a supplied visible-row count;
- scrolls down when the selected row passes the lower viewport edge;
- scrolls up when moving above the upper edge;
- clamps both selection and offset when row count shrinks.

Prefer a focused method with this contract:

```rust
grid.ensure_row_visible(row_count, visible_rows);
```

**Step 2: Run the model test and verify failure**

Run:

```bash
cargo test model::tab::tests --lib
```

Expected: FAIL because `row_offset` and viewport adjustment do not exist.

**Step 3: Implement the minimal viewport model**

Add `row_offset: usize` to `DataGridState`. Extend `clamp` and add `ensure_row_visible`. Treat zero visible rows safely and never produce an offset beyond the last valid row.

**Step 4: Write failing renderer and mouse tests**

Add tests proving that a selected row beyond the first screen is rendered and that `HitTarget::ResultCell.row` contains the underlying result row, not the screen-relative row.

**Step 5: Run focused UI tests and verify failure**

Run:

```bash
cargo test --test mouse result_cell
cargo test --test relation_tabs grid
```

Expected: FAIL because rendering still starts at row zero.

**Step 6: Render from `row_offset`**

In `src/ui/data_grid.rs`:

- compute the number of visible body rows from the render area;
- iterate with `.skip(grid.row_offset).take(visible_rows)`;
- emit absolute result row indexes in hit regions;
- pass the selected row relative to `row_offset` into `TableState`;
- ensure all result rows supplied to `Table` use the same viewport slice.

Update App grid movement to call `ensure_row_visible` with the last known grid viewport height. If viewport height is currently only known during rendering, store it as transient grid layout data through the established App/UI boundary rather than guessing from terminal height.

**Step 7: Verify viewport behavior**

Run:

```bash
cargo test model::tab::tests --lib
cargo test --test mouse
cargo test --test relation_tabs
cargo test --test ui_render
```

Expected: PASS.

### Task 2: Add Pure Relation Edit State

**Files:**
- Create: `src/model/relation_edit.rs`
- Modify: `src/model/mod.rs`
- Modify: `src/model/relation.rs:135-146,225-239`
- Test: `src/model/relation_edit.rs`

**Step 1: Write failing edit-state tests**

Define tests for:

- initializing editable rows from a result snapshot;
- stable `EditableRowId` values after inserting a display row;
- Browse to EditCell and cancel transitions;
- Visual Line anchor and inclusive range while moving both directions;
- row status transitions for changed cells, inserted rows, deleted rows, and conflicts;
- rejecting edits to an already deleted row;
- storing a structured yank row independent from editor registers.

Use domain types shaped around:

```rust
pub enum RelationGridMode {
    Browse,
    EditCell(CellEditorState),
    VisualLine { anchor: usize },
    Busy,
}

pub enum EditableRowState {
    Clean,
    Updated { changed_columns: BTreeSet<usize> },
    InsertDraft,
    Inserted,
    Deleted,
    Conflict { message: String },
}
```

**Step 2: Run tests and verify failure**

Run:

```bash
cargo test model::relation_edit --lib
```

Expected: FAIL because the module does not exist.

**Step 3: Implement the pure state model**

Keep database transport types out of UI mode methods. Store full untruncated `CellValue` vectors and a stable ID. Keep original and current values distinct. Ensure changing a value back to its original value removes that column from the changed set.

Add `edit: Option<RelationEditSession>` to `RelationTab`; initialize it as `None` and do not implicitly make a relation editable.

**Step 4: Add history model tests**

Test that one history entry can contain several row mutations, undo moves the entry to redo only after success, redo moves it back only after success, and a new successful operation clears redo.

**Step 5: Implement history bookkeeping**

Store immutable forward and inverse typed operations plus before/after row snapshots. Do not execute mutation logic from this module; expose success-transition methods for App responses.

**Step 6: Verify pure state tests**

Run:

```bash
cargo test model::relation_edit --lib
cargo test model::relation --lib
```

Expected: PASS.

### Task 3: Build Editable Capability And Metadata Fingerprint

**Files:**
- Create: `src/db/mutation.rs`
- Modify: `src/db/mod.rs`
- Modify: `src/db/catalog.rs:119-212,236-258,361-372`
- Modify: `src/model/relation.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/db/mysql.rs`
- Modify: `src/db/sqlite.rs`
- Test: `tests/catalog_contract.rs`
- Test: `tests/postgres_adapter.rs`
- Test: `tests/mysql_adapter.rs`
- Test: `tests/sqlite_adapter.rs`

**Step 1: Write failing capability tests**

Add table-driven tests for an `EditableRelationCapability` result. It must reject:

- read-only profile;
- non-Table catalog kind;
- non-live provenance;
- missing structure;
- no primary key;
- incomplete composite primary key;
- missing preview column for a key component;
- unsupported identity or comparison values;
- stale connection attribution.

It must accept a live non-read-only table with a complete primary key and bindable rows.

**Step 2: Run tests and verify failure**

Run:

```bash
cargo test editable_relation --lib
```

Expected: FAIL because the capability API does not exist.

**Step 3: Define mutation metadata primitives**

In `src/db/mutation.rs`, define:

```rust
pub struct MutationColumn { /* ordinal, name, type, nullability, generation */ }
pub struct PrimaryKeySpec { /* ordered column indexes */ }
pub struct MetadataFingerprint(/* deterministic digest data */);
pub enum EditableRelationCapability { Editable(EditMetadata), ReadOnly(EditDisabledReason) }
```

Use deterministic structural data for the fingerprint. Do not use Rust's randomized `HashMap` hash output or add a cryptographic dependency. A canonical ordered value type that implements `Eq` is sufficient if it travels in-process.

**Step 4: Implement the capability gate**

Derive edit metadata only from a live `RelationTab`, active profile, active connection, preview columns, and loaded `RelationStructure`. Return a user-facing disable reason for status/help rendering.

**Step 5: Write failing generated-column metadata tests**

Add adapter tests for:

- PostgreSQL identity and sequence-backed serial/default distinction;
- MySQL auto-increment preservation;
- SQLite `INTEGER PRIMARY KEY` rowid alias;
- SQLite `WITHOUT ROWID` primary key;
- SQLite explicit `AUTOINCREMENT` when applicable.

**Step 6: Extend catalog metadata minimally**

Add a database-neutral generated-value classification only if current `identity`, `auto_increment`, `generated_expression`, `hidden`, and default fields cannot express the required insert omission rules. Preserve `OptionalMetadata::Unsupported` versus supported absence.

**Step 7: Verify catalog and capability tests**

Run:

```bash
cargo test --test catalog_contract
cargo test --test sqlite_adapter
cargo test --test mysql_adapter
cargo test --test postgres_adapter
```

Expected: PASS, with live database tests skipped only according to their existing environment gates.

### Task 4: Define Typed Mutation Planning And Value Binding Contracts

**Files:**
- Modify: `src/db/mutation.rs`
- Modify: `src/db/value.rs`
- Create: `src/db/mutation/tests.rs` if the module becomes too large
- Test: `src/db/mutation.rs` or `src/db/mutation/tests.rs`

**Step 1: Write failing planner tests**

Cover:

- ordered composite primary-key extraction;
- `InputValue::Value`, `Null`, and `Default` remaining distinct;
- literal strings `NULL` and `DEFAULT` staying strings;
- rejection of NULL for non-nullable columns;
- rejection of generated or hidden column edits;
- insert classification into Bound, Null, Default, Omitted, and Required;
- copied rows omitting automatic values and requiring non-generated primary-key values;
- unsupported values making optimistic comparison unavailable;
- forward and inverse operation construction.

**Step 2: Run tests and verify failure**

Run:

```bash
cargo test db::mutation --lib
```

Expected: FAIL because typed operations and planner functions are incomplete.

**Step 3: Implement immutable request and result types**

Define:

```rust
pub struct RelationMutationRequest { /* immutable context and operation */ }
pub enum RelationMutation { UpdateCell(..), DeleteRows(..), InsertRow(..) }
pub enum MutationResult { Updated(..), Deleted(..), Inserted(..) }
pub enum MutationErrorKind { Validation, Constraint, Conflict, Stale, Aborted, Connection, OutcomeUnknown }
```

Store column indexes and typed `CellValue` objects, not UI text or prebuilt SQL. Include original values required for optimistic comparison.

**Step 4: Implement input parsing by mutation column type**

Reuse existing decode/preview conventions where appropriate, but never parse from truncated preview text. Return precise validation errors including column name and expected native type. Explicitly reject types that cannot be bound safely in the first release.

**Step 5: Implement insert and inverse planning**

Ensure inverse DELETE uses the final inserted row returned by the database, inverse UPDATE compares against the after-row, and inverse INSERT for a delete preserves explicit original values only when all required columns can be restored.

**Step 6: Verify planner tests**

Run:

```bash
cargo test db::mutation --lib
```

Expected: PASS.

### Task 5: Extend Transaction Worker For Typed Mutations

**Files:**
- Modify: `src/db/transaction.rs:7-56`
- Modify: `src/runtime/transaction.rs:115-216`
- Modify: `src/runtime.rs:174-289,948-1305`
- Modify: `src/action.rs:231-354,374-461`
- Test: `src/runtime/transaction.rs`
- Test: `tests/transaction_reducer.rs`

**Step 1: Write failing worker tests**

Using the existing fake backend pattern, test:

- relation transaction begin on first mutation;
- typed mutation serialization on one worker;
- action savepoint rollback on mutation failure;
- worker remains usable after a recoverable constraint/conflict error;
- transaction aborted permits only rollback;
- commit and rollback terminal dispositions;
- lost acknowledgement quarantines rather than retries;
- owner tab/generation mismatch is rejected before backend execution.

**Step 2: Run tests and verify failure**

Run:

```bash
cargo test runtime::transaction --lib
cargo test --test transaction_reducer
```

Expected: FAIL because the worker only accepts raw SQL execution.

**Step 3: Extend backend and worker requests**

Add typed mutation execution and short-lived savepoint operations to `TransactionBackend`. Prefer one backend method that atomically creates a savepoint, applies a typed mutation, verifies it, and releases or rolls back the savepoint; this keeps dialect transaction details inside adapters.

Do not change SQL console `TransactionRequest::Execute` semantics. Introduce an explicit relation-owner/request variant or a generic worker owner that cannot confuse SQL and relation generations.

**Step 4: Add App/Runtime actions and commands**

Add relation mutation started/succeeded/failed, commit, rollback, and outcome-unknown actions. Commands carry exact immutable context. Runtime owns relation workers keyed by relation tab ID and validates connection and target before dispatch.

**Step 5: Verify worker tests**

Run:

```bash
cargo test runtime::transaction --lib
cargo test --test transaction_reducer
```

Expected: PASS and existing manual SQL transaction tests remain unchanged.

### Task 6: Implement SQLite Cell Update Vertical Slice

**Files:**
- Modify: `src/db/sqlite.rs`
- Modify: `src/db/mod.rs`
- Modify: `src/runtime.rs`
- Modify: `src/app.rs`
- Modify: `src/action.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/ui/relation.rs`
- Modify: `src/ui/theme.rs`
- Modify: `src/ui/data_grid.rs`
- Test: `tests/sqlite_transactions.rs`
- Test: `tests/sqlite_adapter.rs`
- Test: `tests/keymap.rs`
- Test: `tests/relation_tabs.rs`
- Test: `tests/ui_render.rs`

**Step 1: Write failing SQLite mutation integration tests**

Create a table with a primary key and exercise through the adapter/transaction boundary:

- UPDATE executes inside an uncommitted transaction;
- another connection cannot observe it before commit;
- commit makes it visible;
- rollback restores the original value;
- an externally modified original row produces Conflict;
- SQL NULL and literal `"NULL"` remain distinct;
- returned row contains trigger/default/generated effects where SQLite supports them.

**Step 2: Run focused SQLite tests and verify failure**

Run:

```bash
cargo test --test sqlite_transactions relation_mutation -- --nocapture
```

Expected: FAIL because SQLite typed UPDATE is not implemented.

**Step 3: Implement SQLite bound UPDATE**

Resolve the relation by catalog ID and validated qualified name. Quote identifiers using existing SQLite adapter helpers. Build a bound UPDATE with complete primary key and NULL-safe original comparison. Use `RETURNING *` when supported by the project's minimum SQLite runtime; otherwise execute and SELECT by the resulting primary key in the same transaction.

Never interpolate a `CellValue` into SQL text. Reject Unsupported values before constructing the query.

**Step 4: Write failing keymap and reducer tests for cell editing**

Test:

- `i` only acts in an editable relation Data grid;
- read-only profiles, SQL results, views, and missing-key tables do not enter edit mode;
- editor starts with untruncated content;
- Enter emits one immutable update command;
- Esc emits no command;
- Ctrl-n/Ctrl-d preserve explicit semantics;
- busy mode blocks another mutation;
- stale success response does not mutate current tab state.

**Step 5: Run focused UI tests and verify failure**

Run:

```bash
cargo test --test keymap relation_cell_edit
cargo test --test relation_tabs relation_cell_edit
cargo test --test ui_render relation_cell_edit
```

Expected: FAIL because the UI path is not wired.

**Step 6: Wire the App vertical slice**

Implement the smallest complete flow:

```text
i -> CellEditor -> Enter -> typed Command -> Runtime worker
  -> SQLite adapter -> success Action -> row state update
Ctrl-s -> commit -> refresh
Ctrl-x -> rollback -> initial snapshot restoration
```

Add semantic theme colors and render the changed cell background. Add a compact cell-editor overlay and relation transaction status without changing SQL result rendering behavior.

**Step 7: Verify the SQLite vertical slice**

Run:

```bash
cargo test --test sqlite_transactions
cargo test --test sqlite_adapter
cargo test --test keymap
cargo test --test relation_tabs
cargo test --test ui_render
```

Expected: PASS.

### Task 7: Add PostgreSQL And MySQL Update Backends

**Files:**
- Modify: `src/db/postgres.rs`
- Modify: `src/db/mysql.rs`
- Test: `tests/postgres_adapter.rs`
- Test: `tests/mysql_adapter.rs`

**Step 1: Write failing PostgreSQL update tests**

Under existing live-test environment gates, test composite primary keys, `IS NOT DISTINCT FROM`, bound text/bytes/NULL, `RETURNING *`, conflict detection, commit, and rollback.

**Step 2: Implement PostgreSQL mutation UPDATE**

Use SQLx bound parameters and adapter identifier quoting. Return the complete row from `RETURNING *`. Require exactly one returned row.

**Step 3: Write failing MySQL update tests**

Test composite primary keys, `<=>`, bound values, unchanged assignments, post-update SELECT, conflict detection, commit, and rollback.

**Step 4: Implement MySQL mutation UPDATE**

Use bound parameters and `<=>` for optimistic predicates. Do not infer success only from affected rows. Select the resulting full row in the same transaction and validate identity.

**Step 5: Verify driver tests**

Run:

```bash
cargo test --test postgres_adapter relation_mutation -- --nocapture
cargo test --test mysql_adapter relation_mutation -- --nocapture
```

Expected: PASS when live service variables are configured; otherwise tests are explicitly reported as environment-skipped according to repository conventions.

### Task 8: Add Single And Visual-Line Deletes

**Files:**
- Modify: `src/model/relation_edit.rs`
- Modify: `src/action.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/app.rs`
- Modify: `src/ui/data_grid.rs`
- Modify: `src/db/sqlite.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/db/mysql.rs`
- Test: `tests/keymap.rs`
- Test: `tests/relation_tabs.rs`
- Test: `tests/ui_render.rs`
- Test: `tests/sqlite_transactions.rs`
- Test: `tests/postgres_adapter.rs`
- Test: `tests/mysql_adapter.rs`

**Step 1: Write failing sequence and Visual Line tests**

Test:

- first `d` starts a context-bound pending sequence;
- second `d` within timeout deletes the current row;
- timeout, tab change, focus change, generation change, or mode change cancels the sequence;
- `V` anchors the current row;
- `j/k` produce an inclusive contiguous range in either direction and scroll the viewport;
- `d` emits one batch delete;
- Esc clears Visual Line mode.

**Step 2: Run keymap/model tests and verify failure**

Run:

```bash
cargo test --test keymap relation_delete
cargo test model::relation_edit --lib visual
```

Expected: FAIL.

**Step 3: Implement keymap and state behavior**

Extend pending sequences without affecting editor or global sequences. Block delete on InsertDraft, Deleted, Conflict, and Busy rows as defined by capability rules.

**Step 4: Write failing adapter batch-delete tests**

For each driver, test successful single delete, successful composite-key delete, successful multi-row delete, and complete savepoint rollback when the middle row conflicts.

**Step 5: Implement atomic typed DELETE**

Execute deterministic per-row bound DELETE statements under one action savepoint. Use complete key and original-row comparisons. Return the set of deleted stable row IDs only after every row succeeds.

**Step 6: Render and reduce successful deletes**

Keep rows in place with `Deleted` state and gray style. Do not let deleted rows be edited, copied, or deleted again. Keep cursor and viewport valid.

**Step 7: Verify delete behavior**

Run:

```bash
cargo test --test keymap relation_delete
cargo test --test relation_tabs relation_delete
cargo test --test ui_render relation_delete
cargo test --test sqlite_transactions relation_delete
cargo test --test postgres_adapter relation_delete
cargo test --test mysql_adapter relation_delete
```

Expected: PASS subject to existing live-driver gates.

### Task 9: Add Insert Drafts, Yank, And Paste

**Files:**
- Modify: `src/model/relation_edit.rs`
- Modify: `src/db/mutation.rs`
- Modify: `src/action.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/app.rs`
- Modify: `src/ui/data_grid.rs`
- Modify: `src/ui/relation.rs`
- Modify: `src/db/sqlite.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/db/mysql.rs`
- Test: `tests/keymap.rs`
- Test: `tests/relation_tabs.rs`
- Test: `tests/ui_render.rs`
- Test: `tests/sqlite_transactions.rs`
- Test: `tests/postgres_adapter.rs`
- Test: `tests/mysql_adapter.rs`

**Step 1: Write failing draft-state tests**

Test:

- `a` inserts a green draft below the cursor;
- required columns are identified in ordinal order;
- the first required field opens for editing;
- no command is emitted while required values are missing;
- `yy` copies current structured values without touching SQL editor registers;
- `p` inserts below the cursor and omits generated values;
- non-generated primary-key columns become Required;
- canceling an untouched draft removes it;
- constraint failure preserves the draft and message.

**Step 2: Run state/keymap tests and verify failure**

Run:

```bash
cargo test model::relation_edit --lib insert
cargo test --test keymap relation_insert
```

Expected: FAIL.

**Step 3: Implement draft, yank, and paste state**

Keep drafts local until executable. A local draft must not start a transaction. Preserve stable display IDs while replacing a successful draft with the database-returned row.

**Step 4: Write failing adapter INSERT tests**

Cover:

- default-only insert;
- explicit NULL versus DEFAULT;
- automatic key omission;
- explicit composite key;
- generated/default value retrieval;
- unique and foreign-key errors leaving the transaction usable;
- inability to recover a complete final primary key causing savepoint rollback.

**Step 5: Implement INSERT per driver**

- PostgreSQL: bound INSERT with `RETURNING *`.
- MySQL: bound INSERT, generated ID retrieval where applicable, then complete primary-key SELECT.
- SQLite: bound INSERT with `RETURNING *` or rowid/primary-key SELECT fallback based on supported metadata.

If every insert column is omitted, use the dialect's default-values syntax. Never put identity, auto-increment, generated, or hidden columns in the INSERT list unless metadata explicitly marks an allowed override, which is out of scope for the first release.

**Step 6: Wire UI success and errors**

Use green styling for drafts and inserted rows. On success, show returned values. On validation or constraint error, retain the draft and focus the relevant or last-edited field.

**Step 7: Verify insertion behavior**

Run:

```bash
cargo test model::relation_edit --lib insert
cargo test --test keymap relation_insert
cargo test --test relation_tabs relation_insert
cargo test --test ui_render relation_insert
cargo test --test sqlite_transactions relation_insert
cargo test --test postgres_adapter relation_insert
cargo test --test mysql_adapter relation_insert
```

Expected: PASS subject to existing live-driver gates.

### Task 10: Add Undo And Redo Compensation

**Files:**
- Modify: `src/model/relation_edit.rs`
- Modify: `src/db/mutation.rs`
- Modify: `src/action.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/app.rs`
- Modify: `src/runtime.rs`
- Test: `tests/keymap.rs`
- Test: `tests/relation_tabs.rs`
- Test: `tests/sqlite_transactions.rs`

**Step 1: Write failing history reducer tests**

Test:

- update undo and redo;
- insert undo by deleting the returned final row;
- delete undo by inserting complete original values;
- multi-delete undo as one atomic operation;
- several consecutive undo and redo operations;
- a new operation after undo clears redo;
- failed compensation leaves both database-visible row state and history cursor unchanged;
- SQL editor `u` and Ctrl-r remain unchanged when editor focus is active.

**Step 2: Run focused tests and verify failure**

Run:

```bash
cargo test --test relation_tabs relation_undo
cargo test --test keymap relation_undo
```

Expected: FAIL.

**Step 3: Implement compensation dispatch**

`u` dispatches the stored inverse mutation and changes history only after a current-generation success response. Ctrl-r dispatches the stored forward mutation. Both use normal action savepoint handling and optimistic comparison against the last returned row state.

Disable undo for an operation before it executes if the planner cannot construct a complete inverse. Keep `Ctrl-x` available regardless of history support.

**Step 4: Verify transaction-level compensation**

Run:

```bash
cargo test --test sqlite_transactions relation_undo
cargo test --test relation_tabs relation_undo
cargo test --test keymap relation_undo
cargo test --test app_flow
```

Expected: PASS.

### Task 11: Add Lifecycle Guards And Outcome Recovery

**Files:**
- Modify: `src/model/relation_edit.rs`
- Modify: `src/model/transaction.rs`
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/runtime.rs`
- Modify: `src/runtime/transaction.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/ui/relation.rs`
- Modify: `src/ui/mod.rs`
- Test: `tests/transaction_reducer.rs`
- Test: `tests/app_flow.rs`
- Test: `tests/ui_render.rs`

**Step 1: Write failing lifecycle tests**

Cover attempts to:

- refresh a relation;
- change WHERE or ORDER BY;
- switch or close a tab;
- disconnect or delete the profile;
- quit the application;

while a relation transaction or executable draft exists. Require a Commit, Rollback, or Cancel decision. Incomplete untouched drafts may be discarded without database rollback only when no transaction exists.

**Step 2: Write failing failure-injection tests**

Test transaction aborted, mutation response loss, connection loss, commit acknowledgement loss, rollback acknowledgement loss, stale success responses, and metadata change after request creation.

**Step 3: Run focused tests and verify failure**

Run:

```bash
cargo test --test transaction_reducer relation
cargo test --test app_flow relation_transaction
cargo test --test ui_render relation_transaction
```

Expected: FAIL.

**Step 4: Implement one relation-exit confirmation flow**

Reuse the project's existing confirmation patterns, but keep relation and SQL transaction choices distinguishable. Store the deferred navigation/quit intent and resume it only after confirmed commit or rollback. Cancel leaves the user on the relation tab.

**Step 5: Implement failure state transitions**

Reject all mutations while Aborted or OutcomeUnknown. Permit rollback in Aborted. Quarantine/close the pinned connection when outcome is unknown. Never retry commit, rollback, or mutation automatically.

**Step 6: Verify lifecycle and recovery**

Run:

```bash
cargo test --test transaction_reducer
cargo test --test app_flow
cargo test --test ui_render
cargo test runtime::transaction --lib
```

Expected: PASS.

### Task 12: Documentation And Full Verification

**Files:**
- Modify: `docs/keybindings.md`
- Modify: `docs/architecture.md`
- Modify: `README.md` if it documents relation preview capabilities
- Modify: `docs/plans/2026-08-27-relation-data-editing-design.md` only if implementation constraints required an approved design correction

**Step 1: Document user-visible behavior**

Document:

- edit eligibility and disable reasons;
- all Browse, EditCell, and Visual Line shortcuts;
- `Ctrl-s` and `Ctrl-x` transaction lifecycle;
- row colors and conflict behavior;
- NULL and DEFAULT controls;
- undo compensation versus whole-transaction rollback;
- the 500-row, Table-only, primary-key-only first-release boundaries.

**Step 2: Run formatting**

Run:

```bash
cargo fmt --all
cargo fmt --check
```

Expected: PASS.

**Step 3: Run all tests**

Run:

```bash
cargo test --all-targets
```

Expected: PASS. Report PostgreSQL/MySQL environment-gated coverage separately rather than claiming it ran when services were unavailable.

**Step 4: Run Clippy**

Run:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS without warnings.

**Step 5: Inspect the final diff**

Run:

```bash
git status --short
git diff --check
git diff --stat
```

Expected: only intended relation editing, documentation, and test changes; no whitespace errors; pre-existing user changes preserved.

## Final Acceptance Checklist

- Editing is available only for live, non-read-only, primary-key Tables.
- Every executed mutation belongs to one relation-owned transaction.
- Uncommitted changes are invisible to another database connection.
- `Ctrl-s` commits and refreshes; `Ctrl-x` rolls back and restores.
- `i`, `dd`, `V/j/k/d`, `yy`, `p`, `a`, `u`, and Ctrl-r obey their confirmed mode semantics.
- Changed cells, inserted rows, deleted rows, selections, and conflicts are visually distinct.
- INSERT omits generated columns and displays the complete returned row.
- UPDATE and DELETE detect concurrent changes rather than overwriting them.
- Multi-row delete and history compensation are atomic per user operation.
- Values and identifiers are never interpolated unsafely.
- SQL editor and arbitrary SQL result behavior are unchanged.
- Outcome-unknown operations are never retried automatically.
