# Data Grid Viewport Navigation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `h/j/k/l` Data Grid navigation scroll only when the selected cell crosses the current viewport boundary, with symmetric horizontal and vertical behavior in SQL results and relation DATA previews.

**Architecture:** Keep keymap actions semantic and geometry-free. Let the shared Data Grid renderer compute the actual viewport produced by terminal size and column widths, publish that snapshot through `UiState`, and synchronize it back to the active tab after drawing. Store the synchronized viewport capacity in `DataGridState`, then apply one minimal-scroll visibility rule after keyboard and mouse selection changes.

**Tech Stack:** Rust 2024, Ratatui 0.30, Crossterm, existing LazyDB App/Runtime/UiState architecture

---

## Preconditions And Invariants

- Work only in `/Users/yelog/workspace/tui/lazydb-table-preview` on branch `task/table-preview`.
- Preserve commit `09cd959` table visuals, temporal decoding, typed mutation values, column resizing, and horizontal scrollbar mouse behavior.
- Do not modify `src/input/keymap.rs`; `h/j/k/l` already map correctly to `Action::GridMove`.
- Treat scrolling as a model concern after layout synchronization, not as a keymap concern.
- Use minimal scrolling: moving onto the current first/last visible row or column does not scroll; moving beyond that boundary scrolls only enough to reveal the new selection.
- When the selection is already at the first/last result row or column, repeated movement remains a no-op and must not scroll an otherwise valid viewport.
- Keep SQL DATA and relation DATA behavior identical by changing the shared grid path.
- Do not add a visible vertical scrollbar in this change. The requirement is viewport following through `row_offset`; a new scrollbar widget is separate UI scope.
- Do not commit until explicitly requested.

### Task 1: Separate Grid Selection Clamping From Row Visibility

**Files:**
- Modify: `src/model/tab.rs:21-28,172-199`
- Test: `src/model/tab.rs:201-end`

**Step 1: Write failing row-visibility boundary tests**

Add tests with `visible_rows = 3` proving the selected row may reach the viewport edge without scrolling and scrolls only after crossing it:

```rust
#[test]
fn row_viewport_scrolls_only_after_selection_crosses_an_edge() {
    let mut state = DataGridState {
        selected_row: 1,
        row_offset: 0,
        viewport_rows: 3,
        ..DataGridState::default()
    };

    state.selected_row = 2;
    state.ensure_row_visible(10);
    assert_eq!(state.row_offset, 0);

    state.selected_row = 3;
    state.ensure_row_visible(10);
    assert_eq!(state.row_offset, 1);

    state.selected_row = 1;
    state.ensure_row_visible(10);
    assert_eq!(state.row_offset, 1);

    state.selected_row = 0;
    state.ensure_row_visible(10);
    assert_eq!(state.row_offset, 0);
}
```

Add a second regression test proving row visibility does not mutate column state when `column_widths` is empty:

```rust
#[test]
fn ensuring_row_visibility_does_not_clamp_columns_from_width_overrides() {
    let mut state = DataGridState {
        selected_column: 5,
        column_offset: 4,
        selected_row: 6,
        row_offset: 0,
        viewport_rows: 5,
        column_widths: Vec::new(),
    };

    state.ensure_row_visible(10);

    assert_eq!(state.selected_column, 5);
    assert_eq!(state.column_offset, 4);
    assert_eq!(state.row_offset, 2);
}
```

**Step 2: Run the focused model tests and verify failure**

Run:

```bash
cargo test model::tab::tests --lib
```

Expected: FAIL because `viewport_rows` does not exist and `ensure_row_visible` currently accepts a capacity argument while incorrectly deriving `column_count` from `column_widths.len()`.

**Step 3: Add transient viewport capacity to `DataGridState`**

Add:

```rust
pub viewport_rows: usize,
```

This is runtime workspace state, not persisted database data. Update explicit `DataGridState` constructors in unit/integration tests with `..Default::default()` or a concrete value. Existing `Default` behavior remains `0`, meaning layout has not yet supplied a usable capacity.

**Step 4: Make row visibility row-only**

Replace the current contract with:

```rust
pub fn ensure_row_visible(&mut self, row_count: usize) {
    self.selected_row = self.selected_row.min(row_count.saturating_sub(1));
    self.row_offset = self.row_offset.min(row_count.saturating_sub(1));

    if row_count == 0 {
        self.row_offset = 0;
        return;
    }

    let visible_rows = self.viewport_rows;
    if visible_rows == 0 {
        return;
    }

    if self.selected_row < self.row_offset {
        self.row_offset = self.selected_row;
    } else if self.selected_row >= self.row_offset.saturating_add(visible_rows) {
        self.row_offset = self.selected_row + 1 - visible_rows;
    }

    self.row_offset = self
        .row_offset
        .min(row_count.saturating_sub(visible_rows.min(row_count)));
}
```

Do not call `clamp()` from this method. `clamp()` still receives the actual row and column counts from App and remains responsible for complete dimension clamping.

For `viewport_rows == 0`, preserve the clamped `row_offset` instead of resetting it. This avoids a pre-layout movement unexpectedly jumping to row zero; the first real render snapshot will provide the capacity.

**Step 5: Run model tests**

Run:

```bash
cargo test model::tab::tests --lib
```

Expected: PASS.

### Task 2: Define A Rendered Data Grid Viewport Snapshot

**Files:**
- Modify: `src/ui/mod.rs:101-138,189-210`
- Modify: `src/ui/data_grid.rs:17-67`
- Modify: `src/ui/mod.rs:919-970`
- Modify: `src/ui/relation.rs:101-219`
- Test: `tests/ui_render.rs`

**Step 1: Write failing UI snapshot tests**

Extend `tests/ui_render.rs` to inspect `UiState.grid_viewport` after rendering a SQL result with enough rows and columns to overflow. Assert:

```rust
assert_eq!(viewport.tab_id, app.active_console().id);
assert_eq!(viewport.column_offset, expected_first_column);
assert_eq!(viewport.row_offset, expected_first_row);
assert_eq!(viewport.visible_rows, expected_body_height);
```

Add a relation DATA equivalent and assert its snapshot uses the relation tab ID. Add a no-data/non-DATA assertion proving `grid_viewport` is `None` on SQL OUTPUT and relation STRUCTURE.

Avoid asserting a hand-guessed layout height. Derive the expected `visible_rows` from the first and last rendered `ResultCell` hit-region rows or use a stable terminal fixture whose result area is already covered by layout tests.

**Step 2: Run focused UI tests and verify failure**

Run:

```bash
cargo test --test ui_render grid_viewport -- --nocapture
```

Expected: FAIL because `UiState` has no grid viewport snapshot.

**Step 3: Add a snapshot type to the UI boundary**

In `src/ui/mod.rs`, define:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DataGridViewport {
    pub tab_id: Uuid,
    pub column_offset: usize,
    pub row_offset: usize,
    pub visible_rows: usize,
}
```

Import `uuid::Uuid` if it is not already in scope. Add:

```rust
pub grid_viewport: Option<DataGridViewport>,
```

to `UiState`, initialize it to `None`, and clear it at the beginning of every `render_with_state_using_icons` call next to `editor_viewport` and `completion_popup`.

Clearing is mandatory so a DATA snapshot cannot remain active after switching to OUTPUT, STRUCTURE, an empty result, an overlay-only compact state, or a different tab.

**Step 4: Pass stable tab identity to the shared renderer**

Add `tab_id: Uuid` to `data_grid::render`. Thread it through:

- SQL `render_data` / `render_result_table` using `ConsoleTab.id`;
- relation `render_data` / `render_relation_result_table` using `RelationTab.id`.

Do not infer identity from result contents or active indexes.

**Step 5: Publish the actual rendered viewport**

After computing `first`, `visible`, `visible_rows`, and the clamped render-time `row_offset`, write:

```rust
state.grid_viewport = Some(DataGridViewport {
    tab_id,
    column_offset: first,
    row_offset,
    visible_rows,
});
```

Publish the snapshot only after `result.columns.is_empty()` has been rejected. For a result with columns and zero rows, publish `visible_rows` and `row_offset = 0`; this allows subsequent data refreshes to start with valid layout capacity.

**Step 6: Verify UI snapshots**

Run:

```bash
cargo test --test ui_render grid_viewport -- --nocapture
cargo test --test ui_render
```

Expected: PASS.

### Task 3: Synchronize Rendered Viewport State Through Runtime

**Files:**
- Modify: `src/action.rs:130-145,429-437`
- Modify: `src/runtime.rs:2382-2453,2476-2489`
- Modify: `src/app.rs:1220-1240,2362-2378`
- Modify: `src/app.rs:4900-4946`
- Test: `src/app.rs`
- Test: `tests/workspace_tabs.rs`
- Test: `tests/relation_tabs.rs`

**Step 1: Write failing reducer identity tests**

Add tests for a new viewport action:

```rust
Action::GridViewportChanged(DataGridViewport {
    tab_id,
    column_offset: 4,
    row_offset: 10,
    visible_rows: 8,
})
```

Assert it updates only the matching active DATA tab. Add negative cases for:

- stale/different `tab_id`;
- active SQL OUTPUT view;
- active relation STRUCTURE view;
- a tab with no active grid;
- offsets larger than current result dimensions, which must be clamped.

For accepted snapshots, assert:

```rust
grid.column_offset == 4
grid.row_offset == 10
grid.viewport_rows == 8
```

**Step 2: Run reducer tests and verify failure**

Run:

```bash
cargo test grid_viewport --lib
cargo test --test workspace_tabs grid_viewport
cargo test --test relation_tabs grid_viewport
```

Expected: FAIL because the action and reducer do not exist.

**Step 3: Add the viewport synchronization action**

Add:

```rust
GridViewportChanged(crate::ui::DataGridViewport),
```

to `Action`. This follows the existing `EditorViewportChanged` UI-to-App boundary. Ensure action routing/guards that enumerate harmless navigation actions include this action where necessary, but do not expose it through keymap or help.

**Step 4: Add an identity-safe reducer**

Implement a focused App method:

```rust
fn sync_grid_viewport(&mut self, viewport: DataGridViewport) {
    let dimensions = self.active_grid_dimensions();
    let Some(tab) = self.tabs.get_mut(self.active_tab) else {
        return;
    };
    if tab.id() != viewport.tab_id {
        return;
    }

    let grid = match tab {
        WorkspaceTab::Sql(tab) if tab.result_view == ResultView::Data => &mut tab.grid,
        WorkspaceTab::Relation(tab) if tab.view == RelationView::Data => &mut tab.grid,
        _ => return,
    };

    grid.column_offset = viewport.column_offset;
    grid.row_offset = viewport.row_offset;
    grid.viewport_rows = viewport.visible_rows;
    grid.clamp(dimensions.0, dimensions.1);
    grid.ensure_row_visible(dimensions.0);
}
```

Keep dimension lookup before mutable tab borrowing to satisfy Rust borrowing rules. `ensure_row_visible` protects against a resize snapshot that makes the current selection fall outside the new vertical viewport.

**Step 5: Synchronize after every successful render**

In `runtime.rs`, add:

```rust
fn sync_grid_viewport(app: &mut App, runtime: &mut Runtime, state: &UiState) {
    let Some(viewport) = state.grid_viewport else {
        return;
    };
    apply_action(app, runtime, Action::GridViewportChanged(viewport));
}
```

Call it immediately after `sync_editor_viewport` for:

- the initial draw;
- every subsequent redraw.

Avoid dispatching when the current App grid already equals the snapshot to prevent needless reducer work. Add an App accessor or compare the current grid state in Runtime; do not trigger another draw from the sync action.

**Step 6: Run viewport synchronization tests**

Run:

```bash
cargo test grid_viewport --lib
cargo test --test workspace_tabs grid_viewport
cargo test --test relation_tabs grid_viewport
cargo test --test ui_render grid_viewport
```

Expected: PASS.

### Task 4: Apply Symmetric Minimal Scrolling During Grid Movement

**Files:**
- Modify: `src/app.rs:4940-4946,5630-5645`
- Modify: `src/ui/data_grid.rs:294-328,419-end`
- Test: `src/app.rs`
- Test: `src/ui/data_grid.rs`
- Test: `tests/relation_tabs.rs`

**Step 1: Write a failing horizontal sequence test**

Add an App-level test using a result with at least ten equal-width columns. First synchronize a viewport representing columns `4..=6`, then apply moves:

```rust
GridViewportChanged { column_offset: 4, ... }
GridSelect { row: 0, column: 5 }
GridMove { rows: 0, columns: -1 }
```

Assert:

```text
selected_column = 4
column_offset = 4
```

Apply one more `h`; before rendering, selection becomes `3` while offset remains `4`. Render and synchronize the resulting viewport; then assert:

```text
selected_column = 3
column_offset = 3
```

Add the symmetric right-edge case proving the viewport moves only after selection crosses the last visible column.

**Step 2: Write failing vertical sequence tests**

Use ten rows with `viewport_rows = 3`:

```text
row_offset = 0, selected_row = 1
j -> selected 2, offset 0
j -> selected 3, offset 1
```

Then test upward movement:

```text
row_offset = 3, selected_row = 4
k -> selected 3, offset 3
k -> selected 2, offset 2
```

Add result-boundary assertions:

```text
at final result row, repeated j leaves selection and offset unchanged
at first result row, repeated k leaves selection and offset unchanged
```

**Step 3: Run movement tests and verify failure**

Run:

```bash
cargo test grid_navigation --lib
cargo test --test relation_tabs grid_navigation
```

Expected: vertical tests FAIL because `move_grid` does not call row visibility. The original horizontal regression test fails until render-time offset synchronization from Tasks 2-3 is active.

**Step 4: Ensure vertical selection visibility after movement**

Update `move_grid`:

```rust
fn move_grid(&mut self, rows: isize, columns: isize) {
    self.with_active_grid(|grid, (row_count, column_count)| {
        grid.selected_row = move_bounded(grid.selected_row, rows, row_count);
        grid.selected_column = move_bounded(grid.selected_column, columns, column_count);
        grid.clamp(row_count, column_count);
        grid.ensure_row_visible(row_count);
    });
}
```

Do not directly change `column_offset` in `move_grid`. Horizontal visibility depends on actual column widths and available terminal width, so `viewport_start` remains the single horizontal layout algorithm. Its calculated `first` is synchronized back after render.

**Step 5: Keep mouse and programmatic selection consistent**

After `select_grid` sets and clamps row/column, call:

```rust
grid.ensure_row_visible(row_count);
```

Mouse clicks are normally already visible, so this is mostly a consistency guard. `set_grid_column_offset`, used by scrollbar interactions, intentionally continues to select the offset column and clamp dimensions.

**Step 6: Preserve the current horizontal algorithm**

Do not replace `viewport_start`. Once `column_offset` reflects the prior rendered `first`, its existing rules are correct:

```rust
if selected < start {
    return selected;
}
while !visible_columns(widths, start, available).contains(&selected) {
    start += 1;
}
```

Update/add pure tests that explicitly use the synchronized previous start:

```rust
assert_eq!(viewport_start(&widths, 4, 4, 20), 4);
assert_eq!(viewport_start(&widths, 4, 3, 20), 3);
```

This captures the exact reported `h` regression.

**Step 7: Run movement tests**

Run:

```bash
cargo test grid_navigation --lib
cargo test ui::data_grid::tests --lib
cargo test --test relation_tabs grid_navigation
cargo test --test mouse
```

Expected: PASS.

### Task 5: Cover Resize, Tab Switching, And Both Grid Surfaces

**Files:**
- Modify: `tests/ui_render.rs`
- Modify: `tests/workspace_tabs.rs`
- Modify: `tests/relation_tabs.rs`
- Modify: `tests/mouse.rs` only if existing scrollbar tests need snapshot synchronization
- Test: all listed files

**Step 1: Add terminal resize coverage**

Render the same overflowing SQL grid at two terminal heights and assert the second `DataGridViewport.visible_rows` changes. Feed each snapshot through `GridViewportChanged` and assert the selected row remains visible after shrinking and expanding.

Use actual render snapshots rather than duplicating layout constants in tests.

**Step 2: Add SQL and relation parity coverage**

Create equivalent ten-row/ten-column result fixtures for:

- active SQL DATA;
- active relation DATA.

Apply the same navigation sequence and assert matching selected rows, selected columns, row offsets, and synchronized column offsets.

**Step 3: Add stale snapshot coverage during tab switching**

Render tab A to capture a viewport, activate tab B, then dispatch tab A's snapshot. Assert tab B and tab A are both unchanged. Render tab B and dispatch its snapshot; assert only tab B updates.

This test protects against the Runtime processing a layout snapshot after a fast tab switch or resize.

**Step 4: Add column-width-change coverage**

Resize a visible column so fewer columns fit, render again, and assert:

- the selected column remains visible;
- the new actual `column_offset` is published;
- moving `h` to the new left edge does not scroll;
- moving one more column left does scroll.

**Step 5: Run cross-surface regression tests**

Run:

```bash
cargo test --test ui_render grid_viewport -- --nocapture
cargo test --test workspace_tabs grid_viewport
cargo test --test relation_tabs grid_navigation
cargo test --test mouse
```

Expected: PASS.

### Task 6: Complete Verification And Manual Acceptance

**Files:**
- Verify all modified files
- Do not modify docs unless navigation behavior is explicitly documented elsewhere

**Step 1: Format and lint**

Run:

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-features --all-targets -- -D warnings
```

Expected: PASS.

**Step 2: Run the full suite**

Run:

```bash
cargo test --all-features --all-targets
```

Expected: PASS.

**Step 3: Inspect repository hygiene**

Run:

```bash
git diff --check
git status --short --branch
git diff -- src/action.rs src/model/tab.rs src/ui/mod.rs src/ui/data_grid.rs src/ui/relation.rs src/runtime.rs src/app.rs tests/ui_render.rs tests/workspace_tabs.rs tests/relation_tabs.rs tests/mouse.rs
```

Expected: no whitespace errors and no files unrelated to Data Grid viewport navigation. Preserve commit `09cd959` and do not amend it unless explicitly requested.

**Step 4: Perform manual TUI acceptance**

In both a SQL result and relation DATA preview with horizontal and vertical overflow:

1. Move `l` to the last visible column; verify no scroll.
2. Press `l` once more; verify a minimal one-column/rightward scroll that keeps the new selected column visible.
3. Move `h` to the first visible column; verify no scroll.
4. Press `h` once more; verify a minimal leftward scroll.
5. Move `j` to the last visible row; verify no scroll.
6. Press `j` once more; verify a minimal one-row downward scroll.
7. Move `k` to the first visible row; verify no scroll.
8. Press `k` once more; verify a minimal one-row upward scroll.
9. At the first/last result row and column, press the outward key repeatedly; verify the viewport does not drift.
10. Resize the terminal and a column, then repeat the edge checks.
11. Switch rapidly between SQL and relation tabs; verify neither inherits the other's offsets.
12. Verify horizontal scrollbar dragging and track clicking still select and reveal the expected columns.

**Step 5: Report verification limits**

In the completion summary, state:

- all commands actually run and their results;
- whether manual SQL and relation checks were completed;
- whether terminal resize and column resize were manually exercised;
- any behavior intentionally excluded, especially drawing a visible vertical scrollbar.
