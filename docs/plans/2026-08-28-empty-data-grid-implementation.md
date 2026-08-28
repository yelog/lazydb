# Empty Data Grid Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Render a clear empty state without the stray selection marker when a data grid has columns but no rows.

**Architecture:** Keep the shared `Table` rendering path for both empty and populated results. Make selection conditional on row availability, then overlay a centered muted empty-state message in the data region.

**Tech Stack:** Rust 2024, Ratatui 0.30, integration tests with `TestBackend`.

---

### Task 1: Add the empty-grid rendering regression test

**Files:**
- Modify: `tests/ui_render.rs`

**Step 1: Write the failing test**

Create a relation preview whose `ResultSet` has columns and no rows, render it through the existing `TestBackend` helper, and assert that `No rows` is present and that the first data line below the header rule has no vertical marker.

**Step 2: Run the test to verify it fails**

Run: `cargo test --test ui_render empty_relation_preview_renders_clean_empty_state -- --exact`

Expected: FAIL because `No rows` is not currently rendered and the empty table still has a selected cell.

### Task 2: Fix empty-grid selection and render the empty state

**Files:**
- Modify: `src/ui/data_grid.rs:177-186`

**Step 1: Write the minimal implementation**

- Pass `None` to `TableState::with_selected_cell` when `result.rows.is_empty()`.
- After rendering the table and header rule, render `No rows` centered in a one-line rectangle at `area.y + 3`, using the muted foreground and surface background.
- Leave populated-grid selection and scrolling unchanged.

**Step 2: Run the focused test**

Run: `cargo test --test ui_render empty_relation_preview_renders_clean_empty_state -- --exact`

Expected: PASS.

### Task 3: Verify formatting and regressions

**Files:**
- Verify: `src/ui/data_grid.rs`
- Verify: `tests/ui_render.rs`

**Step 1: Format check**

Run: `cargo fmt --check`

Expected: PASS.

**Step 2: Run the UI rendering suite**

Run: `cargo test --test ui_render`

Expected: PASS.
