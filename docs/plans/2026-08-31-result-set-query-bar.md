# Result Set Query Bar Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Move the SQL result `WHERE` and `ORDER BY` controls inside the `RESULT SET` border above the table header.

**Architecture:** Make `render_data` render the outer panel and split its inner rectangle. Pass borderless blocks to the existing table and skeleton renderers, mirroring `src/ui/relation.rs::render_data` without changing query behavior or table APIs.

**Tech Stack:** Rust, Ratatui, Cargo integration tests

---

### Task 1: Add The Layout Regression

**Files:**
- Modify: `tests/ui_render.rs:1598-1612`

**Step 1: Extend the existing shared-query-bar render test**

Record the output line containing `RESULT SET`, the line containing `WHERE`, and the result header line. Assert that their order is panel title, query controls, then table header.

**Step 2: Run the focused test and verify it fails**

Run: `cargo test --test ui_render sql_data_renders_shared_query_bar_above_the_grid`

Expected: FAIL because the query controls currently render before the `RESULT SET` border.

### Task 2: Move The Query Bar Into The Panel

**Files:**
- Modify: `src/ui/mod.rs:1854-1987`

**Step 1: Render the panel before splitting content**

Create the `RESULT SET` block, obtain its inner rectangle, render it over the full result area, and split the inner rectangle into query and result areas.

**Step 2: Keep status and table rendering inside the panel**

Use a borderless surface block for result tables and skeletons. Keep the running-status row above the table and render empty-state text in the inner result body.

**Step 3: Run the focused test and verify it passes**

Run: `cargo test --test ui_render sql_data_renders_shared_query_bar_above_the_grid`

Expected: PASS.

### Task 3: Verify UI Rendering

**Files:**
- Verify: `src/ui/mod.rs`
- Verify: `tests/ui_render.rs`

**Step 1: Format changed Rust files**

Run: `cargo fmt --check`

Expected: PASS.

**Step 2: Run SQL query-bar render tests**

Run: `cargo test --test ui_render sql_data_`

Expected: PASS.

**Step 3: Run the complete UI render suite**

Run: `cargo test --test ui_render`

Expected: PASS.
