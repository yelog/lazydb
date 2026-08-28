# Compact Data Grid Header Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove the data-grid header separator row so headers and data rows render contiguously.

**Architecture:** Update the shared `data_grid` vertical geometry once so SQL and relation grids stay consistent. Remove the explicit rule renderer, shift data coordinates upward, and adapt rendering tests to verify visual and interaction geometry.

**Tech Stack:** Rust 2024, Ratatui 0.30, `TestBackend` integration tests.

---

### Task 1: Lock in the compact header layout

**Files:**
- Modify: `tests/ui_render.rs`

**Steps:**

1. Replace the header-rule assertion with assertions that `┼` is absent and vertical separators remain.
2. Assert the first rendered data row immediately follows the header.
3. Update the empty-grid test to assert `No rows` immediately follows the header.
4. Run the focused tests and confirm they fail against the current separator-row layout.

### Task 2: Remove the separator row and update geometry

**Files:**
- Modify: `src/ui/data_grid.rs`

**Steps:**

1. Remove `Row::bottom_margin(1)`.
2. Remove `render_header_rule` and its unused text imports.
3. Change the first data-row coordinate from `area.y + 3` to `area.y + 2`.
4. Change visible-row height subtraction from `4 + overflow` to `3 + overflow`.
5. Move `No rows` from `area.y + 3` to `area.y + 2` and allow it when the area height is at least 3.
6. Run the focused UI tests and confirm they pass.

### Task 3: Verify interaction and rendering regressions

**Files:**
- Verify: `tests/ui_render.rs`
- Verify: `tests/mouse.rs`

**Steps:**

1. Run `cargo fmt --check`.
2. Run `cargo test --test ui_render`.
3. Run `cargo test --test mouse`.
4. Run `git diff --check`.
