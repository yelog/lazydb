# Task 18 Relation UI Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close the confirmed Task 18 relation-tab UI gaps while preserving SQL-console behavior and Task 17 request identity.

**Architecture:** Keep relation state in `RelationTab`, but branch grid reducers and focus traversal by active workspace tab. Make relation rendering select a ready snapshot from either `Ready` or `previous`, with bounded status controls layered around that body. Keep sanitization at display boundaries and use existing catalog metadata types.

**Tech Stack:** Rust, Ratatui, Crossterm, Tokio, Cargo integration tests.

---

### Task 1: Add failing reducer and input tests

**Files:**
- Modify: `tests/relation_tabs.rs`
- Modify: `tests/mouse.rs`
- Modify: `tests/keymap.rs`

Write tests proving relation grid movement/selection uses preview dimensions, mouse cell selection updates the relation grid, relation focus alternates Explorer/Results, and SQL grid/focus behavior remains unchanged.

Run: `cargo test --test relation_tabs --test mouse --test keymap`
Expected: new relation assertions fail against current reducers.

### Task 2: Add failing request lifecycle and render tests

**Files:**
- Modify: `tests/relation_runtime.rs`
- Modify: `tests/ui_render.rs`

Cover exact cancellation before refresh, retained Data and Structure snapshots during Loading/Failed/Cancelled, sanitized bounded failure/column/title output, trigger and typed metadata sections, Data provenance, and bounded status controls.

Run: `cargo test --test relation_runtime --test ui_render`
Expected: new assertions fail against current rendering/lifecycle behavior.

### Task 3: Implement reducer and request lifecycle fixes

**Files:**
- Modify: `src/app.rs`

Branch `GridMove` and `GridSelect` for relation tabs and active preview dimensions. Implement relation-only two-focus traversal. In `load_active_relation`, capture the prior exact pending request and return `CancelRelationRequest` before the new load command on refresh.

Run: `cargo test --test relation_tabs --test relation_runtime --test mouse --test keymap`
Expected: focused reducer/input/runtime tests pass.

### Task 4: Implement safe relation rendering

**Files:**
- Modify: `src/ui/relation.rs`
- Modify: `src/ui/mod.rs`

Render retained snapshots as the main body with bounded status banners and retry/cancel hit regions. Add Data provenance, trigger section, prioritized typed column metadata, and render-time bounds/sanitization for failure messages, result columns, and relation workspace titles.

Run: `cargo test --test ui_render --test mouse`
Expected: focused render/input tests pass without SQL regressions.

### Task 5: Run complete validation

Run the relation UI/input/app tests, full all-features tests, all-target check, strict clippy, fmt check, and diff check. Inspect `git diff` and `git status`; do not commit.
