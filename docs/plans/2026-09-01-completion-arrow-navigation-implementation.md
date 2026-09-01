# Completion Arrow Navigation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Allow Up and Down to navigate active completion candidates in the SQL Editor and data-query header inputs without changing inactive-input navigation.

**Architecture:** Extend the existing keymap branches that already translate `Ctrl+P` and `Ctrl+N` into completion actions. Map Up to the previous action and Down to the next action only while completion state is present; leave the downstream selection state and wrapping logic unchanged.

**Tech Stack:** Rust, crossterm key events, existing LazyDB action/keymap/completion state, Cargo tests.

---

### Task 1: Add arrow-key mappings for SQL Editor completion

**Files:**
- Modify: `src/input/keymap.rs:349-366`
- Test: `src/input/keymap.rs` keymap tests or the closest existing keymap integration tests

**Step 1: Add focused keymap coverage**

Extend the existing completion keymap tests to assert that an unmodified `Down` produces `Action::CompletionNext` and an unmodified `Up` produces `Action::CompletionPrevious` when editor completion is active.

**Step 2: Run the focused test**

Run: `cargo test keymap`

Expected: the new assertions fail before the mapping is implemented.

**Step 3: Implement the minimal mapping**

Add `KeyCode::Down` to the `CompletionNext` match arm and `KeyCode::Up` to the `CompletionPrevious` match arm. Keep the branch guarded by active insert-mode completion.

**Step 4: Re-run the focused test**

Run: `cargo test keymap`

Expected: PASS.

### Task 2: Add arrow-key mappings for `where`/`order by` completion

**Files:**
- Modify: `src/input/keymap.rs:1366-1383`
- Test: `src/input/keymap.rs` data-query keymap tests

**Step 1: Add focused keymap coverage**

Extend the data-query completion tests to assert that `Down` maps to `Action::DataQueryCompletionNext` and `Up` maps to `Action::DataQueryCompletionPrevious` while either header input has completion active.

**Step 2: Run the focused test**

Run: `cargo test keymap`

Expected: the new assertions fail before the mapping is implemented.

**Step 3: Implement the minimal mapping**

Add unmodified `KeyCode::Down` and `KeyCode::Up` arms alongside the existing Ctrl+N/Ctrl+P arms. Do not add these mappings to the normal text-input edit mapping, so arrows retain cursor movement when completion is absent.

**Step 4: Re-run the focused test**

Run: `cargo test keymap`

Expected: PASS.

### Task 3: Verify formatting and regression safety

**Files:**
- Verify: `src/input/keymap.rs`
- Verify: `docs/plans/2026-09-01-completion-arrow-navigation-design.md`
- Verify: `docs/plans/2026-09-01-completion-arrow-navigation-implementation.md`

**Step 1: Format**

Run: `cargo fmt --all -- --check`

Expected: PASS.

**Step 2: Run the full test suite**

Run: `cargo test`

Expected: PASS.

**Step 3: Run compilation verification**

Run: `cargo check`

Expected: PASS.
