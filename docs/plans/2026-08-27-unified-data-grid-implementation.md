# Unified Data Grid Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Give SQL console results and relation previews one shared Data Grid and Query Bar, including safe server-side WHERE/ORDER BY replay for eligible SQL results.

**Architecture:** Extract presentation and viewport state from relation-specific code into reusable grid/query primitives. Keep SQL console and relation controllers separate, then add generation-safe derived SQL execution over immutable successful read-only query snapshots.

**Tech Stack:** Rust 2024, Ratatui, Crossterm, SQLParser, SQLx, Tokio

---

### Task 1: Add Shared Data Grid State and Geometry

**Files:**
- Create: `src/model/data_grid.rs`
- Modify: `src/model/mod.rs`
- Modify: `src/model/tab.rs`
- Modify: `src/model/relation.rs`
- Test: `src/model/data_grid.rs`

**Steps:**

1. Add failing tests for automatic Unicode-aware widths, explicit overrides, selected-column viewport movement, vertical offsets, and selection clamping.
2. Define `DataGridState` with selected row/column, row/column offsets, and width overrides.
3. Move `automatic_relation_column_widths` and visible-column calculations into pure generic helpers.
4. Replace `ConsoleTab.selected_row/selected_column` and `RelationTab.grid/column_widths` with `DataGridState`.
5. Add temporary accessors only where needed to keep reducer changes focused; remove them by the end of Task 3.
6. Run `cargo test data_grid --all-features` and expect all tests to pass.

### Task 2: Extract the Shared Data Grid Renderer

**Files:**
- Create: `src/ui/data_grid.rs`
- Modify: `src/ui/mod.rs:887-1025`
- Modify: `src/ui/relation.rs:178-325`
- Test: `tests/ui_render.rs`
- Test: `tests/mouse.rs`

**Steps:**

1. Add fixtures rendering the same `ResultSet` through SQL and relation surfaces; assert matching headers, values, widths, and selected styles.
2. Add tests for horizontal overflow, zero rows with columns, no-column affected-row results, NULL, Unsupported, hostile headers, and wide Unicode.
3. Add a vertical-scroll mouse test proving cell hit regions refer to displayed absolute rows.
4. Implement a shared `DataGridView` renderer using relation DATA's content-aware behavior as the baseline.
5. Delegate both SQL and relation DATA to it; delete duplicate table construction and hit-region loops.
6. Keep relation footer/status and SQL result tabs outside the grid renderer.
7. Run `cargo test --test ui_render --test mouse --all-features` and expect all tests to pass.

### Task 3: Generalize Grid Actions and Reducer Routing

**Files:**
- Modify: `src/action.rs`
- Modify: `src/app.rs:1325-1397,2061-2103,4337-4379`
- Modify: `src/input/keymap.rs:549-598`
- Modify: `src/input/mouse.rs`
- Modify: `src/ui/mod.rs`
- Test: `tests/keymap.rs`
- Test: `tests/mouse.rs`
- Test: `tests/relation_tabs.rs`
- Test: `tests/workspace_tabs.rs`

**Steps:**

1. Add failing tests proving `[`/`]`/`=` and mouse resize work identically on SQL and relation DATA.
2. Rename relation column actions and hit targets to generic grid actions.
3. Add one active-grid resolver in App that returns the active `DataGridState` and displayed `ResultSet` dimensions.
4. Route move/select/resize/reset through the common resolver and clamp after every mutation.
5. Ensure non-DATA views and tabs with no result safely no-op.
6. Delete duplicated SQL/relation grid branches.
7. Run focused input, mouse, relation, and workspace tab tests.

### Task 4: Add Shared Query Bar State and Rendering

**Files:**
- Create: `src/model/data_query.rs`
- Create: `src/ui/query_bar.rs`
- Modify: `src/model/mod.rs`
- Modify: `src/model/relation.rs`
- Modify: `src/model/tab.rs`
- Modify: `src/ui/relation.rs`
- Modify: `src/ui/mod.rs`
- Test: `tests/ui_render.rs`
- Test: `tests/profile_draft.rs` only if generic text-input behavior requires it

**Steps:**

1. Add tests for identical WHERE/ORDER BY rendering, focus, cursor, disabled reason, and error display.
2. Generalize `RelationQueryState/Input/Options` into `DataQueryState/Input/Options`.
3. Add `DataQueryCapability::{Relation, DerivedSql, Unavailable}` without implementing SQL replay yet.
4. Move query-bar rendering out of `ui/relation.rs` and render it above both DATA grids.
5. Preserve current relation submitted options and request behavior.
6. Give SQL result state an initial Unavailable capability with a stable reason.
7. Run UI and relation tests.

### Task 5: Generalize Query Bar Input Actions

**Files:**
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/input/keymap.rs:601-648`
- Modify: `src/input/mouse.rs`
- Modify: `src/ui/mod.rs`
- Test: `tests/keymap.rs`
- Test: `tests/mouse.rs`
- Test: `tests/relation_tabs.rs`

**Steps:**

1. Add failing tests for `/`, `s`, Enter, Esc, Tab, editing keys, and disabled SQL query bars.
2. Rename relation query actions/hit targets to generic data-query actions.
3. Route editing through the active tab's `DataQueryState`.
4. Keep submission controller-specific: relation reload now; SQL derived execution in Task 8.
5. Ensure disabled capability rejects focus and editing without losing grid navigation.
6. Run focused reducer/keymap/mouse tests.

### Task 6: Generalize Fragment Validation

**Files:**
- Rename/refactor: `src/sql/relation_filter.rs` to `src/sql/data_query.rs`
- Modify: `src/sql/mod.rs`
- Test: `src/sql/data_query.rs`

**Steps:**

1. Preserve all existing relation fragment tests under generic names.
2. Add dialect tests for comments, multiple statements, LIMIT/FETCH, locks, ORDER BY ALL, and malformed expressions.
3. Return `DataQueryOptions` instead of relation-owned options.
4. Keep adapter SQL concatenation behavior unchanged for this task.
5. Run `cargo test sql::data_query --all-features`.

### Task 7: Analyze Safe SQL Derived-Query Capability

**Files:**
- Create: `src/sql/derived_result.rs`
- Modify: `src/sql/mod.rs`
- Modify: `src/model/tab.rs`
- Modify: `src/app.rs` query-finish paths
- Test: `tests/sql_execution.rs`
- Test: `src/sql/derived_result.rs`

**Steps:**

1. Add table-driven tests for eligible SELECT, JOIN, CTE, and aggregate queries.
2. Add ineligible tests for multi-statement SQL, EXPLAIN, DML RETURNING, transaction controls, procedure/function statements, parse failure, and lock-bearing queries.
3. Define immutable `SqlResultSource` containing exact SQL, connection, target, dialect, source execution generation, and document revision.
4. Derive capability only after a successful source execution and only while connection/target/transaction state remains compatible.
5. Store a bounded stable disabled reason for UI.
6. Run derived-result and SQL execution tests.

### Task 8: Build and Execute Derived Result Queries

**Files:**
- Modify: `src/sql/derived_result.rs`
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/runtime.rs`
- Modify: `src/model/tab.rs`
- Test: `tests/sql_execution.rs`
- Test: `tests/connection_switch.rs`
- Test: adapter tests for SQLite/PostgreSQL/MySQL

**Steps:**

1. Add builder tests for each dialect, trailing semicolons, no clauses, WHERE only, ORDER only, and both clauses.
2. Parse the final wrapper and enforce the outer 500-row limit.
3. Add `DataResultState { base, derived, source, query, generation }` to consoles.
4. Add a dedicated derived-result command/action identity carrying console, source generation, derived generation, connection, target, and SQL.
5. Execute only through the exact active target; do not mutate editor text or original execution draft.
6. Accept only exact current identities. Keep base visible during loading/failure and expose a sanitized error.
7. Clearing both clauses removes `derived` immediately and restores base without a command.
8. Running a new source query clears derived/query state and invalidates pending responses.
9. Test stale response, target switch, disconnect, retry, and base restoration.

### Task 9: Add Multiple Result-Set Selection

**Files:**
- Modify: `src/model/tab.rs`
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/ui/mod.rs`
- Test: `tests/ui_render.rs`
- Test: `tests/keymap.rs`
- Test: adapter multi-statement tests

**Steps:**

1. Add `active_result_set` and selection helpers to SQL result state.
2. Default to the last set containing columns/rows, otherwise the final set.
3. Display `RESULT n/m` and selected-set row count rather than aggregate rows beside DATA.
4. Add navigation actions and documented keys.
5. Clamp/reset grid state when switching sets.
6. Disable derived replay for multi-statement outcomes with a clear reason.
7. Run UI, keymap, and adapter tests.

### Task 10: Documentation and Complete Verification

**Files:**
- Modify: `docs/keybindings.md`
- Modify: `docs/architecture.md`
- Modify: `README.md`
- Verify all modified source/tests

**Steps:**

1. Document shared DATA behavior, resize/reset keys, filters, derived-query restrictions, 500-row limit, and result-set switching.
2. Update Rendering and Database boundaries to distinguish shared grid from source controllers.
3. Run `cargo fmt --all -- --check`.
4. Run `cargo test --all-features --all-targets`.
5. Run `cargo clippy --all-features --all-targets -- -D warnings`.
6. Run `git diff --check` and inspect the complete diff.
7. Manually verify SQL SELECT/JOIN/CTE filters, disabled multi-statement/DML cases, relation preview provenance, wide tables, resized columns, and mouse behavior after vertical scrolling.

Do not commit implementation changes until explicitly requested. Preserve unrelated untracked SQL semantic-assistance plan files.
