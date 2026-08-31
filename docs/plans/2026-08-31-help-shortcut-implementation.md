# Help Shortcut Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the footer's displayed `F1 help` hint with `? help`, document both `?` and `F1` in contextual help, and make its Enter action a no-op inside the help panel.

**Architecture:** Reuse the existing `HelpShortcutId` and contextual shortcut registry. Add one shared help entry showing `? (also F1)` before context-specific entries, update the footer label, and short-circuit the existing help shortcut executor for that entry before it closes the overlay.

**Tech Stack:** Rust 2024, Crossterm, Ratatui, Cargo test.

---

### Task 1: Add and verify the contextual help entry

**Files:**
- Modify: `src/help.rs:4-88, 161-219`
- Test: `src/help.rs` unit tests

**Step 1:** Add `Help` to `HelpShortcutId` and prepend a `?` entry to `shortcuts`.

**Step 2:** Add a test asserting the entry exists for Explorer, Editor, and Results contexts.

**Step 3:** Run `cargo test help` and confirm it passes.

### Task 2: Update footer and prevent recursive help execution

**Files:**
- Modify: `src/ui/mod.rs:2035`
- Modify: `src/app.rs:1105-1113`
- Test: `src/app.rs` unit tests

**Step 1:** Change the footer text to `? help`.

**Step 2:** Return an empty command list for `HelpShortcutId::Help` while retaining the help overlay.

**Step 3:** Add a regression test that opens help, executes the help entry, and verifies the overlay remains open.

**Step 4:** Run targeted tests, then `cargo fmt --check` and `cargo test`.
