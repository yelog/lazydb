# Data Grid Column Jump Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `^` and `$` navigation to the first and last column of the current row in SQL Result set and Relation DATA grids.

**Architecture:** Add a semantic `GridSelectColumn` action and `GridColumnTarget` model enum. The keymap maps the keys only through the shared grid-navigation path, while `App` routes the action through `with_active_grid`. `DataGridState` performs bounded selection without changing row or viewport state.

**Tech Stack:** Rust, crossterm key events, existing LazyDB Action/keymap/DataGridState architecture, Rust unit and integration tests.

---

### Task 1: Add model and action semantics

**Files:**
- Modify: `src/model/tab.rs`
- Modify: `src/action.rs`
- Test: `src/model/tab.rs`

**Step 1: Write the failing model test**

Add coverage for first and last column selection, asserting that the selected row, row offset, column offset, viewport rows, and widths are unchanged. Add an empty-column case that remains safe.

**Step 2: Run the focused model test**

Run: `cargo test model::tab::tests --lib`
Expected: FAIL because the column target type and selection method do not exist.

**Step 3: Implement the minimal model/action API**

Add `GridColumnTarget::{First, Last}`, add `Action::GridSelectColumn(GridColumnTarget)`, and implement `DataGridState::select_column_target(target, column_count)` with bounded first/last selection and an empty-column no-op/reset-safe behavior.

**Step 4: Run the focused model test**

Run: `cargo test model::tab::tests --lib`
Expected: PASS.

### Task 2: Route keybindings through the shared grid path

**Files:**
- Modify: `src/input/keymap.rs`
- Modify: `src/app.rs`
- Test: `tests/keymap.rs`

**Step 1: Write the failing keymap tests**

Cover `^` and `$` in a SQL Result DATA context and Relation DATA browse context. Assert that the keys map to `GridSelectColumn(First/Last)`. Cover Relation cell-edit mode to ensure the existing text-input path remains authoritative.

**Step 2: Run the focused keymap tests**

Run: `cargo test --test keymap`
Expected: FAIL because the actions are not mapped or handled yet.

**Step 3: Implement keymap and App routing**

Map `^` and `$` in the shared grid navigation mapping used by Results and Relation DATA browse mode. Add the corresponding `App::update` branch using `with_active_grid` and the active column count.

**Step 4: Run the focused keymap tests**

Run: `cargo test --test keymap`
Expected: PASS.

### Task 3: Update help and keybinding documentation

**Files:**
- Modify: `src/help.rs`
- Modify: `src/app.rs`
- Modify: `docs/keybindings.md`

Add contextual help entries and executable help actions for the two shortcuts. Document them in the Results section and the Relation Data section without changing the DDL/OUTPUT contracts.

### Task 4: Format and verify regressions

**Files:**
- No additional files.

Run:

```bash
cargo fmt --check
cargo test --lib
cargo test --test keymap
cargo test --test ui_render
cargo test --test relation_runtime
```

Expected: all commands pass. If formatting differs, run `cargo fmt` and repeat the checks.
