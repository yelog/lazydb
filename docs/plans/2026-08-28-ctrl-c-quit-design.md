# Ctrl+C Quit Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the application-wide `Q` quit shortcut with `Ctrl+C`, including while overlays and editor input modes are active.

**Architecture:** Handle `Ctrl+C` immediately after filtering key-release events in `Keymap::map`, before all overlay and mode-specific routing. Remove the existing uppercase `Q` quit branch while preserving contextual lowercase `q` bindings. Update keymap regression tests to assert global `Ctrl+C` behavior and that uppercase `Q` is no longer a quit key.

**Tech Stack:** Rust, crossterm key events, Cargo test.

---

### Task 1: Update global quit key mapping

**Files:**
- Modify: `src/input/keymap.rs:51-60, 508-512`

**Step 1: Add global Ctrl+C handling**

Return `Some(Action::Quit)` for an exact `KeyModifiers::CONTROL` plus `KeyCode::Char('c')` event before overlay handling.

**Step 2: Remove the Q quit branch**

Delete the uppercase `Q` condition that currently returns `Action::Quit`.

### Task 2: Update regression tests

**Files:**
- Modify: `tests/keymap.rs:802-826`

**Step 1: Assert Ctrl+C exits in all relevant modes**

Cover normal editor mode, insert editor mode, an open completion popup, and a non-editor focus.

**Step 2: Assert Q is not a global exit**

Replace the existing uppercase `Q` quit expectations with assertions that it is routed as editor input or ignored according to the existing mode behavior.

### Task 3: Verify

**Step 1: Run focused tests**

Run `cargo test --test keymap` and expect all tests to pass.

**Step 2: Run formatting check**

Run `cargo fmt --all -- --check` and expect no formatting differences.
