# Completion Popup Interaction Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make SQL completion cursor-anchored, keyword-first in general contexts, and non-modal for continued typing.

**Architecture:** Reuse `EditorRenderSnapshot::cursor_screen_cell` as the only anchor source. Keep completion candidates in App state, but let normal editor keys dismiss stale candidates before editing and schedule a refreshed popup.

**Tech Stack:** Rust 1.94, Ratatui 0.30.2, Crossterm 0.29.

---

### Task 1: Preserve Typing While Completion Is Open

**Files:**
- Modify: `src/input/keymap.rs`
- Modify: `src/app.rs`
- Modify: `tests/keymap.rs`
- Modify: `tests/app_flow.rs`

**Steps:**
1. Add a failing test proving a printable Insert-mode key maps to `EditorKey` while completion is open.
2. Change popup key handling to return early only for `Ctrl-N/P`, Enter, and Escape.
3. Clear the stale popup before applying `EditorKey` or `EditorPaste`; allow editor effects to schedule fresh completion.
4. Run `cargo test --test keymap --test app_flow -- --nocapture`.

### Task 2: Prioritize SQL Keywords

**Files:**
- Modify: `src/sql/completion.rs`
- Modify: `tests/sql_completion.rs`

**Steps:**
1. Add a failing test proving `SELECT` ranks first for prefix `s` even when catalog names also match.
2. Always add matching keywords for unqualified input.
3. Give keywords the highest context score only in general SQL context; retain semantic catalog priority after FROM/JOIN, qualifiers, and routine calls.
4. Run `cargo test --test sql_completion -- --nocapture`.

### Task 3: Anchor Popup to the Cursor

**Files:**
- Modify: `src/ui/mod.rs`
- Modify: `tests/ui_render.rs`

**Steps:**
1. Add tests for placement below the cursor, upward fallback, and editor-bound clamping.
2. Return an absolute cursor anchor and text viewport from `render_editor`.
3. Place the popup below the cursor when it fits, otherwise above it; clamp width, height, and x-coordinate to the text viewport.
4. Include candidate detail width in popup sizing while preserving the ten-row cap.
5. Run `cargo test --test ui_render -- --nocapture`.

### Task 4: Verify the Complete Change

**Steps:**
1. Run `cargo fmt --check`.
2. Run `cargo clippy --all-targets --all-features -- -D warnings`.
3. Run `cargo test --all-targets --all-features`.
4. Run `git diff --check`.
