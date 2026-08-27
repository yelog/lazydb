# Visible Objects Discovery Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Automatically discover all visible databases and schemas in the Visible Objects picker, including bounded cross-database PostgreSQL discovery with partial-failure reporting.

**Architecture:** Add a discovery-specific action/command lifecycle keyed by request id and draft fingerprint. Keep saved scope separate from discovered candidates, perform PostgreSQL fan-out in runtime with resolved credentials, and render loading/partial/global warnings without changing the active connection.

**Tech Stack:** Rust 2024, Tokio, Futures, SQLx 0.9, Ratatui 0.30, Cargo test, Clippy

---

### Task 1: Model Discovery Requests And Warnings

**Files:**
- Modify: `src/db/catalog.rs:176-185`
- Modify: `src/model/profile_manager.rs:265-313,1091-1123`
- Modify: `src/action.rs:69-107,300-360`
- Test: `tests/profile_reducer.rs`

Add catalog warning output, a pending scope-discovery request, explicit refresh/success/failure actions, and a `DiscoverProfileCatalog` command. Write reducer tests first for request identity and stale fingerprints.

### Task 2: Start Discovery On Open And Refresh

**Files:**
- Modify: `src/app.rs:720-790,2402-2433`
- Modify: `src/model/profile_manager.rs:1144-1342`
- Test: `tests/profile_reducer.rs`

Extract draft validation/submission creation shared by Test and Scope discovery. Open the picker immediately, preserve saved rows, dispatch discovery unless a matching fresh snapshot exists, and force dispatch on refresh. Keep late responses inert.

### Task 3: Execute Discovery With Existing Credentials

**Files:**
- Modify: `src/runtime.rs:165-342,1441-1494`
- Test: `tests/profile_runtime.rs`

Dispatch `DiscoverProfileCatalog` through the existing registry/keyring/session credential resolver. Connect, discover, close, and emit the dedicated response without probing or changing active connection state.

### Task 4: Discover PostgreSQL Schemas Across Databases

**Files:**
- Modify: `src/db/postgres.rs:47-53,225-253`
- Modify: `src/db/mod.rs:192-206`
- Modify: `src/runtime.rs`
- Test: `tests/postgres_adapter.rs`

List `pg_database` rows that are non-template, allow connections, and pass `has_database_privilege(..., 'CONNECT')`. Fan out temporary profile connections with concurrency four, query schemas, sort deterministically, and convert individual failures into discovery warnings.

### Task 5: Render Loading, Warnings, And Refresh

**Files:**
- Modify: `src/input/keymap.rs:372-383`
- Modify: `src/ui/profiles.rs:228-292`
- Modify: `src/model/profile_manager.rs:1215-1342`
- Test: `tests/keymap.rs`
- Test: `tests/ui_render.rs`

Map `r` to refresh, show loading and warning status, retain unavailable saved rows, and update the footer hint. Add focused UI and keymap tests.

### Task 6: Verify

Run:

```bash
cargo fmt --check
cargo test --test profile_reducer --all-features
cargo test --test profile_runtime --all-features
cargo test --test postgres_adapter --all-features
cargo test --test keymap --all-features
cargo test --test ui_render --all-features
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: all commands pass; manual PostgreSQL verification shows all connectable databases, per-database schemas, preserved saved selections, and actionable partial warnings.

### Task 7: Add Stable Expansion And Tri-State Selection

**Files:**
- Modify: `src/model/profile_manager.rs:315-323,1198-1322,1632-1722`
- Modify: `src/ui/profiles.rs:241-278`
- Test: `tests/profile_draft.rs`
- Test: `tests/ui_render.rs`

Replace `ScopeRow.selected` with an explicit `ScopeSelectionState`. Persist expansion for selected databases, remove the `All schemas` row, compute database state from scope plus discovered schemas, and pass discovered schema context into toggles. Lock down navigation stability, partial rendering, database select-all/deselect-all, `All` exclusion conversion, last-schema removal, saved unavailable schemas, and MySQL mirroring with focused tests.
