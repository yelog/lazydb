# Explorer Follow Active Tab Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Activating a workspace tab reveals its corresponding Explorer object by default, with an application setting to disable the behavior.

**Architecture:** Observe active tab changes in `App::update` so keyboard, mouse, console management, navigation, and closing tabs share one synchronization path. Resolve stable object IDs from tab metadata, reuse `ExplorerTreeState::reveal_node` and the existing sticky-ancestor viewport scrolling, and preserve keyboard focus. Retry an unresolved reveal only when catalog metadata arrives, stopping when the user navigates Explorer or activates another tab; never establish a connection just to reveal an object.

**Tech Stack:** Rust, serde/TOML application settings, ratatui Explorer projection, existing integration tests.

---

### Task 1: Add the configuration contract

**Files:** `src/config.rs`, `config/default.toml`, `docs/configuration.md`.

1. Add `[explorer] auto_reveal_active_tab = true` to embedded defaults and expose `AppConfig.explorer`.
2. Preserve partial overrides, older settings files, and strict validation; test enabled defaults and a disabled user override.
3. Document selection, ancestor expansion, viewport scrolling, and preserved pane focus.
4. Run `cargo test --lib config::tests`.

### Task 2: Reuse and complete Explorer reveal

**Files:** `src/model/explorer.rs`, `src/model/workspace.rs`, `tests/explorer_state.rs`.

1. Cover revealing nodes inside collapsed connection groups and the `Others` region.
2. Correct ancestor resolution to include actual connection groups.
3. Add an `ExplorerState` wrapper that reveals a node, ensures visibility, and synchronizes cached row/scroll values.
4. Run `cargo test --test explorer_state`.

### Task 3: Synchronize tab activation

**Files:** `src/app.rs`, `src/runtime.rs`, `tests/explorer_tab_sync.rs`, `tests/relation_tabs.rs`.

1. Cover next/previous/mouse tab activation, closing the active tab, disabling the setting, console namespaces, connection boundaries, MongoDB/Redis/principal tabs, scrolling, and pending metadata.
2. Resolve relation tabs by stable object ID, SQL consoles by execution database/schema, and native database tabs by their native Explorer IDs.
3. Reveal on actual tab changes and explicit activation; retain current pane focus and leave unbound consoles alone.
4. When metadata is missing, reveal the nearest loaded ancestor and retry on metadata arrival, without overriding later manual Explorer navigation.
5. Connect the setting to runtime startup and keep default behavior sourced from embedded configuration.
6. Run `cargo test --test explorer_tab_sync --test relation_tabs --test workspace_tabs --test explorer_state`.

### Task 4: Validate the integrated patch

1. Run formatting checks on touched Rust files and `git diff --check`.
2. Run relevant configuration, Explorer, workspace, native database, and input regressions.
3. Review the diff against pre-existing working tree edits; do not commit or alter unrelated work.

## Implementation outcome

All four tasks are complete. Tab synchronization lives in `src/app/explorer.rs`;
the runtime applies the setting before restoring the workspace. Revealing a
loaded object uses one tree projection and no display-row formatting, covered
with a 956-table regression fixture. Pending reveals stop after manual Explorer
navigation, disabling the setting, or switching to another tab. Active search
and find views are dismissed when revealing an object so the target is visible.

Validation: the complete library suite and 17 related integration suites passed
with 1,904 tests passed, zero failed, and seven existing ignored tests. The local
DuckDB 1.5.6 runtime was supplied through `LAZYDB_DUCKDB_LIB_DIR`. Formatting and
diff whitespace checks also passed. Existing unrelated working tree changes were
preserved, and no commit was created.
