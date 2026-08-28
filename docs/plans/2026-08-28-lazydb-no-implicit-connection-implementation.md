# LazyDB No Implicit Connection Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Ensure normal startup displays profiles without connecting, while explicit CLI connection targets still connect automatically.

**Architecture:** Keep the existing `StartupProfiles.selected` field as the optional automatic startup target. Remove only the fallback that assigns the first persisted profile; the existing runtime startup action and manual explorer connection path remain unchanged.

**Tech Stack:** Rust, Tokio runtime, Clap CLI, integration tests, `cargo fmt`, `cargo test`.

---

### Task 1: Make startup selection explicit-only

**Files:**
- Modify: `src/runtime.rs:2570-2585`

**Step 1: Remove the implicit first-profile fallback**

Keep the value returned from explicit `--url` and `--profile` handling unchanged. Do not replace `None` with `profiles.first()` for ordinary startup.

**Step 2: Preserve password behavior**

Continue binding `LAZYDB_PASSWORD` only to the explicit selected profile. With no selected profile, no startup password should be assigned.

### Task 2: Add a non-empty-store regression test

**Files:**
- Test: `tests/startup_profiles.rs`

**Step 1: Add a test with persisted profiles and no connection flags**

Load a temporary store containing at least two SQLite profiles and assert that profile order is preserved while `startup.selected` is `None`.

**Step 2: Keep explicit startup tests as the contract**

Retain the existing `--profile` and `--url` assertions proving explicit startup selection remains populated.

### Task 3: Verify behavior

**Step 1: Format changed Rust files**

Run: `cargo fmt --all -- --check`

Expected: PASS.

**Step 2: Run focused startup tests**

Run: `cargo test --test startup_profiles`

Expected: PASS, including normal-startup and explicit-startup cases.

**Step 3: Run the full test suite**

Run: `cargo test`

Expected: PASS.
