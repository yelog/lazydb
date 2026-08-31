# Data Grid Header Colors Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Restore a compact filled header for SQL result sets and relation data using colors aligned with the Deep Space theme.

**Architecture:** Add a dedicated header foreground token beside the existing grid-header background token. Update the shared data-grid renderer to use both tokens in a one-row header, so all grid consumers receive identical geometry and styling.

**Tech Stack:** Rust, Ratatui, cargo test

---

### Task 1: Add semantic header colors

**Files:**
- Modify: `src/ui/theme.rs:17-61`

**Steps:**

1. Add `grid_header_text` to `Theme`.
2. Set `grid_header` to `Color::Rgb(24, 48, 58)`.
3. Set `grid_header_text` to `Color::Rgb(184, 235, 229)`.

### Task 2: Restore the compact filled header

**Files:**
- Modify: `src/ui/data_grid.rs:94-107`
- Modify: `src/ui/data_grid.rs:153`
- Modify: `src/ui/data_grid.rs:211-220`
- Modify: `src/ui/data_grid.rs:296-328`

**Steps:**

1. Change the header row height from two rows to one.
2. Remove the generated horizontal divider and intersection glyphs.
3. Style header cells with `grid_header_text` on `grid_header`.
4. Style vertical header separators with `grid_border` on `grid_header`.
5. Update visible-row capacity, hit-region origins, and empty-state placement for a one-row header.

### Task 3: Verify behavior

**Files:**
- Test: `src/ui/data_grid.rs`
- Test: `src/ui/theme.rs`

**Steps:**

1. Run `cargo fmt --check` and expect success.
2. Run `cargo test ui::data_grid` and expect all data-grid tests to pass.
3. Run `cargo test ui::theme` and expect all theme tests to pass.
4. Run `cargo test` and expect the complete suite to pass.
