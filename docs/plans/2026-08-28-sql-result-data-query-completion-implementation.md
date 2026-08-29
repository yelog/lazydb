# SQL Result Data Query Completion Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enable column completion in `WHERE` and `ORDER BY` inputs for SQL result tables.

**Architecture:** Reuse the existing `DataQueryCompletion` state and keymap. Relation tabs keep catalog plus result columns; SQL tabs use the original query's last `ResultSet.columns`, then share matching, deduplication, sorting, and popup rendering.

**Tech Stack:** Rust, Ratatui, existing SQL identifier matching and integration tests.

---

### Task 1: Add SQL result candidates to the shared refresh path

**Files:**
- Modify: `src/app.rs:5536-5612`
- Test: `tests/relation_tabs.rs` and `tests/ui_render.rs`

1. Collect candidate names and types from either the active relation's catalog/result columns or the active SQL tab's original outcome result columns.
2. Apply the existing identifier match, case-insensitive deduplication, deterministic ordering, and ten-item limit once for both paths.
3. Preserve the existing replacement range and dialect-aware acceptance behavior.
4. Add coverage for a SQL result output alias and empty-row result metadata.

### Task 2: Render SQL result completion popup

**Files:**
- Modify: `src/ui/mod.rs:1511-1557`
- Test: `tests/ui_render.rs`

1. Store the cursor returned by `query_bar::render`.
2. Render the existing data-query completion popup after the SQL result grid, anchored to the SQL result viewport.
3. Verify popup content and bounds in a SQL result rendering test.

### Task 3: Verify the complete behavior

**Files:**
- No additional files.

1. Run focused relation and UI tests.
2. Run `cargo fmt --check` and the full test suite.
3. Review the diff to ensure only the completion path, SQL result renderer, tests, and plan documents changed.
