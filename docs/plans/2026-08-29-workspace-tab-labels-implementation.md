# Workspace Tab Labels Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Render workspace tabs as `{content icon} {tab name}` without positional sequence numbers.

**Architecture:** Keep `WorkspaceTab::title` and all persisted data unchanged. Resolve icons inside the Ratatui tab renderer using the existing `IconSet`, relation metadata, SQL execution targets, and the active connection as fallback.

**Tech Stack:** Rust 2024, Ratatui 0.30, Cargo test, rustfmt, Clippy

---

### Task 1: Add Tab Label Regression Tests

**Files:**
- Modify: `tests/ui_render.rs`

**Step 1:** Add an ASCII-mode render test containing one SQL tab and one table-preview tab.

**Step 2:** Assert the output contains `SQ console_1` and `TB users`, and does not contain `01 console_1` or `02 users`.

**Step 3:** Add a test where the active connection is SQLite but the SQL tab is bound to a PostgreSQL profile, and assert the label uses `PG`.

**Step 4:** Run `cargo test --test ui_render workspace_tabs_use_content_icons_instead_of_sequence_numbers --all-features` and confirm the old renderer fails.

### Task 2: Render Content-Aware Tab Icons

**Files:**
- Modify: `src/ui/mod.rs:210-241`
- Modify: `src/ui/mod.rs:629-661`

**Step 1:** Pass the existing `IconSet` into `render_tabs`.

**Step 2:** For relation tabs, resolve the icon with `icons.catalog(tab.descriptor.kind)`.

**Step 3:** For SQL tabs, resolve the database kind from the bound profile, then the active profile, then the connected server; use the generic database icon only if none is available.

**Step 4:** Replace the numbered label format with `format!(" {icon} {title} ")` while retaining sanitization, truncation, styling, and hit-region width calculation.

### Task 3: Verify the Change

**Files:**
- Verify: `src/ui/mod.rs`
- Verify: `tests/ui_render.rs`

**Step 1:** Run the focused UI render tests.

**Step 2:** Run `cargo fmt --all -- --check`.

**Step 3:** Run `cargo clippy --all-targets --all-features -- -D warnings`.

**Step 4:** Run `cargo test --all-targets --all-features`.
