# Explorer Find Match Centering Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Center the currently selected `/` find match in the Explorer viewport whenever find selection changes.

**Architecture:** Keep `ExplorerTreeState` as the source of truth and reuse its existing `align_selected(ExplorerNodeAlignment::Middle)` operation. Change only the shared find-match selection path, so initial query matches, confirmation, and `n`/`N` all get identical behavior without affecting regular navigation.

**Tech Stack:** Rust 2024, existing Explorer model and integration tests.

---

### Task 1: Center Find Selection

**Files:**
- Modify: `src/model/workspace.rs:353-367`
- Test: `tests/explorer_state.rs`

**Step 1: Add a regression test**

Create a state with multiple profile rows, set a viewport height of five, open
find, and query a row near the bottom. Assert that selecting the match places
the selected row at the middle screen position, subject to end-of-list clamping.
Then navigate with `n` and `N` and assert the same invariant for each result.

**Step 2: Implement the minimal change**

In `select_find_match`, replace the visibility-only scroll operation with
`align_selected(ExplorerNodeAlignment::Middle)` after assigning the normalized
selection. Keep `sync_selected_index()` afterward so the legacy `selected` and
`scroll` fields stay synchronized.

**Step 3: Run focused verification**

Run:

```bash
cargo test --test explorer_state visible_find -- --nocapture
```

Expected: all visible-find tests pass, including the new centering regression.

**Step 4: Run the complete suite**

Run:

```bash
cargo test
```

Expected: all tests pass.
