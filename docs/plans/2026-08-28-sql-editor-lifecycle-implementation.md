# SQL Editor Lifecycle Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement the plan task-by-task.

**Goal:** Add close, reopen, and confirmed permanent deletion workflows for persistent SQL Editors while keeping relation tabs ephemeral.

**Architecture:** Introduce an ordered persistent SQL Editor registry in `App`, while retaining `tabs` as the open mixed-tab projection and `EditorWorkspace` as the sole SQL text owner. Extend workspace persistence with open state and exact UUID-based deletion, then expose lifecycle actions through `Space q`, `Space x`, `Space e`, and a searchable overlay.

**Tech Stack:** Rust, Ratatui, Crossterm, Tokio, Serde/TOML, integration tests.

---

### Task 1: Version the workspace format and persist open state

**Files:** `src/persistence/workspace.rs`, `tests/workspace_persistence.rs`

Write migration and round-trip tests, add `open` to persisted consoles, advance the format, and decode the prior version with `open: true`. Run `cargo test --test workspace_persistence` before and after implementation.

### Task 2: Add the persistent SQL Editor registry

**Files:** `src/model/tab.rs`, `src/app.rs`, `tests/workspace_tabs.rs`

Add an ordered registry for all SQL Editors, initialize and update it on creation, derive snapshots from it, and restore only open editors. Keep text in `EditorWorkspace` without duplicating it.

### Task 3: Separate close from delete reducers

**Files:** `src/action.rs`, `src/model/transaction.rs`, `src/model/workspace.rs`, `src/app.rs`, tests

Make `CloseActiveTab` hide SQL editors while retaining their records. Add confirmed delete actions with UUID-targeted overlay state and reuse transaction exit deferral.

### Task 4: Delete the exact persisted SQL file

**Files:** `src/action.rs`, `src/persistence/workspace.rs`, `src/runtime.rs`, tests

Add serialized UUID-specific SQL file deletion under workspace mutation locking. Tolerate missing files and preserve unrelated files.

### Task 5: Add the searchable SQL Editor overlay

**Files:** `src/model/sql_editor_list.rs`, `src/model/mod.rs`, `src/model/workspace.rs`, `src/action.rs`, `src/app.rs`, `src/input/keymap.rs`, `src/input/mouse.rs`, `src/ui/mod.rs`, tests

Add stable searchable listing, open markers, activation of open editors, and reconstruction of hidden editors.

### Task 6: Add lifecycle shortcuts and help

**Files:** `src/editor/mod.rs`, `src/input/keymap.rs`, `src/help.rs`, `docs/keybindings.md`, `README.md`, tests

Map `Space q` to close, `Space x` to delete, and `Space e` to the editor list consistently across editor and relation/result contexts. Update help and documentation.

### Task 7: Verify

Run `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`, and inspect the final diff for scope and regressions.
