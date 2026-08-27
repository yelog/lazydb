# Relation Data Editing Design

## Goal

Add transactional row editing to live table previews when the active connection profile is not read-only. Users can edit cells, mark one or more rows for deletion, duplicate or add rows, undo and redo operations, and explicitly commit or roll back the table-editing transaction.

## Scope

The first release only enables editing when all of the following are true:

- The active tab is a relation Data view.
- The catalog object is a real table.
- The preview and structure snapshots are live and belong to the active connection identity and catalog scope.
- The connection profile is not read-only.
- Complete structure metadata is available.
- The table has a complete primary key, including all columns of a composite key.
- All values needed for row identity and optimistic comparison can be safely bound by the adapter.
- No relation refresh or mutation request is already in flight.
- The relation mutation transaction is not aborted, committing, rolling back, or outcome-unknown.

SQL query results, views, offline snapshots, out-of-scope snapshots, tables without primary keys, and stale relation metadata remain read-only. Unique constraints and full-row matching are not primary-key fallbacks in the first release.

## Existing Architecture

LazyDB routes input through `Keymap -> Action -> App::update -> Command -> Runtime -> DatabaseConnection/Adapter`. Relation previews already preserve the relation catalog identity, connection identity, scope, request generation, snapshot provenance, preview data, and structure metadata. This makes `RelationTab` the only safe first-release editing surface.

The shared data grid currently provides one selected cell and horizontal visibility, but no vertical viewport, row selection, edit state, mutation history, or row identity. SQL editor undo and redo belong to Modalkit and must not be reused for database mutation history.

The existing manual transaction worker already owns a pinned physical connection and handles explicit commit, rollback, cancellation, quarantine, and outcome-unknown states. Relation editing will reuse and extend this transaction infrastructure while retaining an independent lifecycle from a SQL console manual transaction.

## Chosen Approach

Each editable `RelationTab` owns a relation edit session. The first operation that needs database I/O starts a dedicated transaction on a pinned connection. Every successful user operation executes immediately inside that transaction, so inserted rows can display database-generated values and updated rows can display trigger or generated-column results.

Relation editing uses dedicated controls:

- `Ctrl-s`: commit the relation transaction.
- `Ctrl-x`: roll back the relation transaction.
- `u`: undo the most recent successful relation operation.
- `Ctrl-r`: redo the most recently undone relation operation.

The relation transaction and SQL console manual transaction do not share a physical connection. They reuse common worker and state-machine infrastructure but have independent owners, generations, commands, and status.

The UI never creates SQL literals. It sends typed relation mutation requests to Runtime. Each database adapter validates the relation identity, quotes identifiers for its dialect, binds values through SQLx, checks optimistic predicates, and returns typed rows.

## Edit Session Model

Mutation state belongs to `RelationTab`, not `ResultSet`, `DataGridState`, or transient `UiState`.

```rust
enum GridEditMode {
    Browse,
    EditCell(CellEditor),
    VisualLine { anchor: usize },
    Busy,
}

enum RowState {
    Clean,
    Updated { changed_columns: ColumnMask },
    Inserted,
    Deleted,
    Conflict,
}

struct EditableRow {
    stable_id: EditableRowId,
    original: Vec<CellValue>,
    current: Vec<CellValue>,
    primary_key: Vec<CellValue>,
    state: RowState,
}

struct RelationEditSession {
    transaction: RelationTransactionState,
    mode: GridEditMode,
    rows: Vec<EditableRow>,
    yank_register: Option<YankedRow>,
    undo: Vec<AppliedMutation>,
    redo: Vec<AppliedMutation>,
    base_generation: u64,
    metadata_fingerprint: MetadataFingerprint,
}
```

`EditableRowId` is an in-memory stable identifier. Grid row indexes are display positions and may change after insertion or refresh. Database row identity always uses the ordered original primary-key values.

`DataGridState` gains a vertical `row_offset`. Moving or selecting a row keeps it in the visible window. Rendering and mouse hit regions use the same offset so displayed, keyboard-selected, and mouse-selected row indexes agree.

## Modes And Keybindings

### Browse Mode

| Key | Behavior |
| --- | --- |
| `i` | Open a small editor containing the current cell value. |
| `dd` | Delete the current row as one operation. |
| `V` | Enter Visual Line mode anchored at the current row. |
| `yy` | Copy the current row to a relation-grid register. |
| `p` | Create a copied row below the cursor, omitting generated values. |
| `a` | Create a schema-aware empty row below the cursor. |
| `u` | Undo the last successful relation operation. |
| `Ctrl-r` | Redo the last undone relation operation. |
| `Ctrl-s` | Commit the active relation transaction. |
| `Ctrl-x` | Roll back the active relation transaction. |
| `h/j/k/l` | Move the selected cell. |

`dd` and `yy` use the existing timed key-sequence infrastructure. A pending sequence is bound to the active tab ID, relation generation, focus, and grid mode so it cannot fire after context changes.

The existing relation `p` shortcut for switching to the Data view only applies outside an editable relation Data grid. The existing `o`, `D`, `r`, `/`, and `s` behaviors remain available when their current context allows them.

### Cell Editor Mode

The editor is a small overlay containing the current cell's untruncated value. It is a simple line editor, not a Vim editor.

| Key | Behavior |
| --- | --- |
| Text input | Edit a temporary string. |
| `Enter` | Validate, execute, and save the new value. |
| `Esc` | Cancel without database I/O. |
| `Ctrl-n` | Set SQL NULL explicitly. |
| `Ctrl-d` | Set SQL DEFAULT explicitly. |
| Left/Right/Home/End | Move the input cursor. |
| Backspace/Delete | Delete input characters. |

The input model distinguishes a literal string from SQL semantics:

```rust
enum InputValue {
    Value(String),
    Null,
    Default,
}
```

Typing `NULL` or `DEFAULT` creates those literal strings. `Ctrl-n` and `Ctrl-d` express SQL NULL and DEFAULT. Generated, hidden, identity, and auto-increment columns are not directly editable. NULL is rejected for non-nullable columns.

### Visual Line Mode

`V` selects the current row. `j` and `k` move the cursor and update the contiguous range between the anchor and cursor. `d` deletes the complete selected range as one atomic operation and one history entry. `Esc` leaves Visual Line mode without changing rows. Other mutation keys are not supported in this mode in the first release.

## Visual States

Theme receives semantic colors for updated, inserted, deleted, conflict, and Visual Line states. Data-grid rendering does not hard-code RGB values.

Style precedence is:

1. Current cell cursor.
2. Visual Line selection.
3. Conflict.
4. Deleted row.
5. Inserted row.
6. Updated cell.
7. Normal row.

Updated cells receive a changed-cell background. Deleted rows remain at their display position with a gray row background until commit or rollback. Inserted and copied rows use a green row background. Conflict rows use the error semantic color. Busy mode retains row state colors but rejects additional mutation input.

## Schema-Aware Insertion

An insert draft classifies every column as a bound value, NULL, DEFAULT, omitted, or required.

```rust
enum InsertValue {
    Bound(CellValue),
    Null,
    Default,
    Omitted,
    Required,
}
```

- Generated, hidden, identity, and auto-increment columns are omitted.
- Columns with defaults initially use DEFAULT.
- Nullable columns initially use NULL.
- Non-null columns without defaults or generation are required.

`a` creates a green draft row and enters the first required cell editor when required values exist. No SQL is executed until all required fields are valid. This is the only local-only mutation state; it represents incomplete input rather than a database change.

`yy` copies the visible row into a relation-specific structured register. `p` creates a draft below the cursor. Generated and automatic columns are omitted, and non-generated primary-key fields become required to avoid copying a duplicate key. Other values are copied from the row's current transaction-visible state.

After INSERT succeeds, the adapter returns the complete database row. The draft is replaced with returned values so defaults, automatic keys, generated columns, and trigger effects are shown accurately.

Catalog metadata must be extended where current capabilities are insufficient:

- PostgreSQL sequence-backed `serial` columns must be distinguishable from arbitrary defaults.
- SQLite rowid aliases, `WITHOUT ROWID`, and explicit `AUTOINCREMENT` semantics must be represented.
- MySQL continues to use existing auto-increment metadata.

## Typed Mutation Boundary

The application dispatches immutable requests containing tab and edit generations, connection identity, exact target, relation catalog ID, scope, metadata fingerprint, and a typed operation.

```rust
struct RelationMutationRequest {
    tab_id: Uuid,
    tab_generation: u64,
    edit_generation: u64,
    connection: ConnectionIdentity,
    target: ExecutionTarget,
    relation: CatalogId,
    scope: CatalogScope,
    metadata_fingerprint: MetadataFingerprint,
    operation: RelationMutation,
}

enum RelationMutation {
    UpdateCell(UpdateCellMutation),
    DeleteRows(Vec<DeleteRowMutation>),
    InsertRow(InsertRowMutation),
}
```

Runtime repeats all capability and stale-context checks before database I/O. Adapters resolve the catalog relation again rather than trusting UI names. They use dialect-specific SQLx query builders and bound values; App never interpolates values or identifiers into raw SQL.

## Optimistic Concurrency

UPDATE and DELETE match the complete ordered primary key and safely comparable original row values. This rejects an operation if another connection changed the row after the preview.

- PostgreSQL uses `IS NOT DISTINCT FROM`.
- MySQL uses `<=>`.
- SQLite uses NULL-safe `IS` semantics or an equivalent expanded predicate.

Generated, hidden, and unsupported values cannot silently weaken the predicate. If the adapter cannot safely round-trip a value required for the configured comparison, the row remains read-only.

An operation that matches zero rows becomes a conflict and is rolled back to its action savepoint. More than one affected row is a planner or metadata invariant failure: the action is rolled back and the edit session is disabled pending refresh.

Adapters read the complete resulting row after UPDATE. PostgreSQL can use `RETURNING *`. MySQL performs an in-transaction SELECT because affected-row semantics differ when an assigned value is unchanged. SQLite uses `RETURNING *` when supported or a primary-key SELECT fallback.

## Deletes

Single-row and Visual Line deletes share one typed batch operation. The worker creates one action savepoint, deletes selected rows in deterministic display order, validates each optimistic match, and releases the savepoint only if every row succeeds. Any error or conflict rolls back the entire selected range.

Successfully deleted rows remain in the grid as gray rows. They cannot be edited, copied, or deleted again. They disappear only after a successful commit refresh.

## Transaction Lifecycle

The first operation that requires SQL acquires a relation-tab-owned physical connection and begins a transaction. Incomplete insert drafts do not start a transaction until they become executable.

Every operation uses a short-lived savepoint:

```text
SAVEPOINT lazydb_action_N
execute typed mutation
verify affected rows and returned rows
RELEASE SAVEPOINT
append history entry
clear redo history
```

The savepoint provides atomic failure handling for the current action. It is not retained as the undo mechanism.

`Ctrl-s` blocks new input, commits, clears the edit session after a confirmed acknowledgement, and refreshes relation data and structure. A lost commit acknowledgement enters `OutcomeUnknown`; LazyDB does not retry commit.

`Ctrl-x` blocks new input, rolls back, restores the initial preview snapshot, and clears history after a confirmed acknowledgement. A rollback failure or lost acknowledgement enters `OutcomeUnknown`, closes or quarantines the connection, and requires refresh.

Refreshing, changing WHERE or ORDER BY, closing the tab, switching or deleting the connection profile, and exiting the application are blocked while a relation transaction or executable draft is active. The user chooses Commit, Rollback, or Cancel navigation.

## Undo And Redo

Undo and redo are relation-operation history, separate from SQL editor history.

- Every successful operation stores its forward mutation, inverse mutation, before rows, and returned after rows.
- `u` executes the inverse mutation inside the same relation transaction.
- `Ctrl-r` re-executes the forward mutation.
- A new successful forward operation clears the redo stack.
- A failed inverse or redo leaves the history cursor unchanged.
- A multi-row delete is one history entry and is undone atomically.

Inverse operations are:

- UPDATE: update the returned after-row back to the before-row.
- INSERT: delete the returned row by its final primary key and comparison values.
- DELETE: insert the complete original row or rows.

These are compensating operations inside an uncommitted transaction, not physical database time travel. Triggers, generated columns, and constraints may prevent an exact inverse. Unsupported inverse operations are disabled before execution and reported clearly. `Ctrl-x` remains the lossless way to discard every database change in the active transaction.

## Error Handling

| Error | Behavior |
| --- | --- |
| Input parse failure | Keep editor open and report the target column type. |
| Required/NOT NULL violation | Do not send SQL; focus the invalid cell. |
| Constraint violation | Roll back the action savepoint and preserve the draft or previous row state. |
| Optimistic conflict | Roll back the action and mark the row as conflict. |
| Stale generation or metadata | Reject before I/O and require refresh. |
| Transaction aborted | Enter Aborted and only allow whole-transaction rollback. |
| Connection lost | Enter OutcomeUnknown, quarantine the worker connection, and refresh after reconnect. |
| Commit acknowledgement lost | Enter OutcomeUnknown and never retry automatically. |
| Read-only rejection | Abort the edit session and roll back when the outcome is known. |

Connection closure normally rolls back an open transaction, but the UI never assumes that outcome without acknowledgement.

## Read-Only Defense

Editing is rejected at three layers:

1. Keymap and App capability checks do not expose mutation actions for read-only or ineligible relations.
2. Runtime revalidates profile, active connection identity, target, scope, relation ID, tab generation, edit generation, and metadata fingerprint.
3. Existing database-native read-only connection enforcement remains the final boundary.

The profile's `read_only` setting is the available product capability signal. Server permission discovery is not introduced in this feature; permission failures remain database errors.

## Testing Strategy

Pure model tests cover viewport movement, Visual Line ranges, stable row IDs, edit parsing, insert planning, capability gates, history branching, and mutation/inverse construction.

Keymap tests cover timed `dd` and `yy`, context invalidation, edit controls, Visual Line controls, relation versus SQL result behavior, and preservation of SQL editor undo/redo.

Reducer and rendering tests cover row and cell styles, mouse indexes with `row_offset`, transaction status, stale responses, commit/rollback navigation prompts, and snapshot restoration.

SQLite integration tests provide the default end-to-end mutation suite. Environment-gated PostgreSQL and MySQL tests cover dialect predicates, generated key retrieval, RETURNING or SELECT behavior, and connection-level transaction visibility. Each adapter covers UPDATE, conflict, DELETE, atomic multi-delete rollback, INSERT, generated values, commit, rollback, and relevant value types.

Repository-wide verification is:

```bash
cargo fmt --check
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
```

## Delivery Sequence

1. Add grid viewport and pure relation edit state.
2. Add editable capability and complete generated-value metadata.
3. Define typed mutation planning and adapter binding primitives.
4. Implement a SQLite vertical slice for cell UPDATE, commit, and rollback.
5. Add PostgreSQL and MySQL UPDATE implementations.
6. Add delete and Visual Line behavior.
7. Add insert drafts, row copy, and generated-value retrieval.
8. Add undo/redo compensation and lifecycle prompts.
9. Complete failure injection, live-driver tests, documentation, and full verification.

## Explicit Non-Goals

- Editing arbitrary SQL result sets.
- Editing views or materialized views.
- Editing tables without a primary key.
- Falling back to unique constraints or full-row identity.
- Sharing a transaction with a SQL console manual transaction.
- Cross-page row selection or editing more than the current 500-row preview.
- Vim editing inside the cell editor.
- Automatic retry after an outcome-unknown mutation, commit, or rollback.
- Guaranteed trigger-transparent undo; whole-transaction rollback is the guaranteed recovery path.
