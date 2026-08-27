# Data Grid Horizontal Scrollbar Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Preserve and expose every result column on narrow terminals through a column-snapped horizontal viewport and draggable scrollbar.

**Architecture:** Store the first visible column in the shared grid state, centralize viewport calculations in `ui::data_grid`, and use actions plus UI hit regions to connect keyboard and mouse interactions. Both relation previews and SQL query results continue to use the shared renderer.

**Tech Stack:** Rust 2024, Ratatui 0.30, Crossterm mouse events, existing reducer/action architecture.

---

### Task 1: Add Persistent Grid Offset

**Files:**
- Modify: `src/model/tab.rs`

1. Add tests proving `DataGridState::clamp` clamps `column_offset` for empty and non-empty results.
2. Run `cargo test model::tab::tests --lib` and verify the new tests fail.
3. Add `column_offset: usize` and clamp it to the last valid column.
4. Run `cargo test model::tab::tests --lib` and verify it passes.

### Task 2: Build Correct Column Viewport Calculations

**Files:**
- Modify: `src/ui/data_grid.rs`

1. Add unit tests for first-page rendering, selected-column visibility, last-page reachability, and no-overflow behavior.
2. Run the focused data-grid tests and verify the current suffix-fitting algorithm fails.
3. Replace `visible_column_start` with helpers that calculate the complete-column range from an explicit offset and normalize an offset around a selected column.
4. Account for column spacing and the scrollbar row in available geometry.
5. Run the focused tests and verify they pass.

### Task 3: Render The Horizontal Scrollbar

**Files:**
- Modify: `src/ui/data_grid.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/ui/relation.rs`

1. Add rendering tests that assert overflow shows a track/thumb and fitting columns do not reserve a scrollbar row.
2. Add shared scrollbar geometry with thumb size and position based on visible and total columns.
3. Render the table in the area above the scrollbar and render the track inside the panel.
4. Add hit regions for the thumb and the track before/after it.
5. Ensure relation footer SQL text does not overwrite the scrollbar.
6. Run UI tests.

### Task 4: Keep Keyboard Selection And Viewport Synchronized

**Files:**
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/model/tab.rs`

1. Add reducer tests covering right-edge and left-edge selection movement.
2. Add actions for setting and paging the horizontal grid offset.
3. Update grid movement to normalize the offset around the selected column using the latest rendered viewport capacity or a deterministic column range.
4. Clamp offsets on new result dimensions and select a visible column after explicit scrollbar movement.
5. Run focused app/model tests.

### Task 5: Add Mouse Track Click And Thumb Dragging

**Files:**
- Modify: `src/ui/mod.rs`
- Modify: `src/input/mouse.rs`
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Test: `tests/mouse.rs`

1. Add mouse tests for clicking before/after the thumb, beginning a drag, dragging to both ends, and releasing it.
2. Add scrollbar hit targets carrying the geometry required to map pointer positions to column offsets.
3. Track an active grid-scrollbar drag in `UiState`.
4. Map mouse down/move/up to page, set-offset, and end-drag actions.
5. Keep offsets column-snapped and clamp all pointer-derived values.
6. Run `cargo test --test mouse`.

### Task 6: Regression Verification

**Files:**
- Modify tests only if a discovered behavior lacks coverage.

1. Run `cargo fmt --check` and format if required.
2. Run focused grid, app, UI, and mouse tests.
3. Run `cargo test`.
4. Run `cargo clippy --all-targets --all-features -- -D warnings`.
5. Inspect `git diff` to ensure only the scrollbar design, plan, implementation, and tests changed.
