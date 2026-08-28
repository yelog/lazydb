# SQL Editor Input And Completion Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix Insert/Replace Ctrl+U, completion acceptance lifecycle, and relation candidate presentation.

**Architecture:** Keep editing semantics in `EditorWorkspace`, distinguish completion scheduling policy while applying editor effects, and use existing completion label/detail fields for primary and secondary relation text.

**Tech Stack:** Rust 2024, Crossterm, Modalkit, Ratatui, existing SQL completion model.

---

### Task 1: Ctrl+U Editing

**Files:** `src/input/keymap.rs`, `src/editor/mod.rs`, `src/editor/tests.rs`

Add failing single-line, multiline, Unicode, line-start, and undo tests. Route
Insert/Replace Ctrl+U to the editor and implement one revision-producing deletion
from cursor to logical line start. Run editor and keymap tests.

### Task 2: Completion Acceptance Lifecycle

**Files:** `src/app.rs`, `tests/app_flow.rs`, `tests/sql_completion.rs`

Add a failing acceptance test asserting no popup or completion schedule remains
after Enter. Add an explicit apply-effects scheduling policy and suppress only the
accepted edit's completion schedule. Verify the next user edit schedules normally.

### Task 3: Relation Candidate Presentation

**Files:** `src/sql/completion.rs`, `src/ui/mod.rs`, `tests/sql_completion.rs`, `tests/ui_render.rs`

Make relation labels object-only, derive deduplicated parent qualifiers in
parentheses as detail, preserve insertion text, and verify muted detail rendering.

### Task 4: Regression Verification

Run `cargo fmt -- --check`, focused editor/completion/UI tests, and
`cargo test --all-targets`.
