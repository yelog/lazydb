# Editor Execution Target Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Implement this plan task-by-task with focused regression tests before each production change.

**Goal:** Make each SQL editor select, display, persist, and execute against a truthful database/schema target.

**Architecture:** Derive selector candidates from the active normalized catalog, store the selected target on `ConsoleTab`, and carry the target through App commands into Runtime validation. Reconnect the single active adapter when backend target context changes and fail closed on every mismatch.

**Tech Stack:** Rust 2024, Ratatui, Crossterm, Tokio, SQLx, Serde/TOML.

---

### Task 1: Target Initialization And Candidate Model

**Files:**
- Modify: `src/model/execution_target.rs`
- Modify: `src/model/workspace.rs`
- Modify: `src/app.rs`
- Test: `tests/execution_target.rs`
- Test: `tests/workspace_tabs.rs`

**Steps:**

1. Add failing tests for initial/new console default targets.
2. Add a target selector state containing owned candidates and selection.
3. Build valid candidates from the active profile catalog and configured default.
4. Run `cargo test --test execution_target --test workspace_tabs`.

### Task 2: Functional Selector UI And Reducer

**Files:**
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/input/keymap.rs`
- Modify: `src/ui/mod.rs`
- Test: `tests/keymap.rs`
- Test: `tests/ui_render.rs`
- Test: `tests/sql_execution.rs`

**Steps:**

1. Add failing selector render/navigation/confirmation tests.
2. Render target rows with selected highlighting and current-target context.
3. Confirm a target only when query and transaction guards permit it.
4. Emit workspace persistence after a successful target change.
5. Run focused keymap, render, and reducer tests.

### Task 3: Runtime Target Identity

**Files:**
- Modify: `src/action.rs`
- Modify: `src/runtime.rs`
- Modify: `src/db/mod.rs`
- Modify: `src/db/postgres.rs`
- Modify: `src/db/mysql.rs`
- Modify: `src/db/sqlite.rs`
- Test: `tests/connection_switch.rs`
- Test: `tests/sql_execution.rs`
- Test: `tests/sqlite_transactions.rs`

**Steps:**

1. Add failing tests for target mismatch rejection.
2. Carry `ExecutionTarget` in automatic and manual execution commands.
3. Record the exact target on active runtime connections.
4. Apply database/schema overrides while constructing adapters.
5. Reject mismatched commands before database I/O.
6. Run focused runtime and adapter tests.

### Task 4: Target Switching And Fail-Closed Execution

**Files:**
- Modify: `src/app.rs`
- Modify: `src/runtime.rs`
- Modify: `src/sql/execution.rs`
- Test: `tests/connection_switch.rs`
- Test: `tests/sql_execution.rs`
- Test: `tests/transaction_reducer.rs`

**Steps:**

1. Add tests proving failed switching leaves the old target active.
2. Request target reconnect before mutating the console target.
3. Validate execution draft target against console and runtime target.
4. Remove the empty-target fallback in `run_active_sql`.
5. Run focused switching and execution tests.

### Task 5: Persistence, Completion, And Documentation

**Files:**
- Modify: `src/app.rs`
- Modify: `src/persistence/workspace.rs`
- Modify: `docs/keybindings.md`
- Modify: `docs/architecture.md`
- Test: `tests/workspace_persistence.rs`
- Test: `tests/sql_completion.rs`

**Steps:**

1. Verify target round-trip and invalid-target fallback behavior.
2. Use active console schema for completion ranking.
3. Document selector behavior and transaction guard.
4. Run `cargo fmt --check`.
5. Run `cargo test`.
6. Run `cargo clippy --all-targets --all-features -- -D warnings`.
