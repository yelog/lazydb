# Key Sequence Timeout Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make pending key-sequence timeout apply between adjacent valid keys so counted pane resize commands retain their full count.

**Architecture:** Keep the existing `Keymap` pending-state model and refresh its timestamp whenever a valid key transitions to another pending state. Add a small private transition helper so every continuation follows the same timing rule, while initial sequences still use `set_pending` and completed or invalid sequences still clear state.

**Tech Stack:** Rust, Crossterm key events, Cargo tests

---

### Task 1: Cover Pending-State Continuation Timing

**Files:**
- Modify: `src/input/keymap.rs`
- Test: `src/input/keymap.rs`

**Step 1: Write the failing tests**

Add unit tests that seed a pending state with an expired timestamp, perform a valid continuation, and assert the resulting pending timestamp is fresh. Cover both another count digit and the transition from `WindowCount` to `Window`.

**Step 2: Run tests to verify they fail**

Run: `cargo test input::keymap::tests --lib`

Expected: the new timestamp assertions fail because continuation transitions retain the expired timestamp.

**Step 3: Implement the minimal shared transition**

Add a private helper that stores a pending state with `Instant::now()` while preserving focus, editor mode, and tab identity. Use it for accepted pending-to-pending transitions instead of reconstructing tuples with the prior `started` value.

**Step 4: Run focused tests**

Run: `cargo test input::keymap::tests --lib`

Expected: PASS.

### Task 2: Verify Counted Pane Resize Behavior

**Files:**
- Test: `tests/keymap.rs`
- Test: `src/editor/tests.rs`
- Test: `src/model/workspace.rs`

**Step 1: Strengthen behavioral coverage if needed**

Ensure the keymap integration test covers a multi-digit count and direction mappings for both width and height. Keep editor-local count coverage because editor normal mode has its own input path.

**Step 2: Run relevant tests**

Run: `cargo test --test keymap`

Run: `cargo test normal_mode_counted_window_resize_emits_shared_effect --lib`

Run: `cargo test pane_resize --lib`

Expected: PASS.

**Step 3: Run project checks**

Run: `cargo fmt --check`

Run: `cargo test`

Expected: PASS.
