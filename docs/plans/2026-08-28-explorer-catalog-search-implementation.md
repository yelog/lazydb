# Explorer Catalog Search Implementation Plan

> **For Claude:** Implement this plan task-by-task with test-first changes.

**Goal:** Add `/`-activated, server-backed search for every actual catalog object in the active Explorer connection.

**Architecture:** Add an independent catalog-search contract beside paged tree loading. Keep search state as an Explorer projection, route asynchronous work through the existing Action/Command/Runtime boundary, and merge only a selected hit's navigation path into the normal tree.

**Tech Stack:** Rust, Tokio, SQLx, Crossterm, Ratatui, existing PostgreSQL/MySQL/SQLite adapters.

---

### Task 1: Search Domain Contract

**Files:**
- Modify: `src/db/catalog.rs`
- Modify: `src/db/mod.rs`
- Test: `tests/catalog_contract.rs`

1. Add failing contract tests for request validation, result identity, bounded limits, and hit ancestors.
2. Run `cargo test --test catalog_contract catalog_search -- --nocapture` and verify failure.
3. Add `CatalogSearchRequest`, `CatalogSearchHit`, and `CatalogSearchPage` with connection, generation, scope, query, and limit validation.
4. Add the object-safe database connection search method.
5. Run the focused contract tests and verify they pass.

### Task 2: Explorer Search State

**Files:**
- Modify: `src/model/workspace.rs`
- Modify: `src/model/explorer.rs`
- Test: `tests/explorer_state.rs`

1. Add failing tests for opening, editing, clearing, result selection, stale generation rejection, and close behavior.
2. Add search lifecycle and state to `ExplorerState` without changing normal tree projection.
3. Add hit location that inserts required real entries, expands ancestors/groups, and selects the stable hit ID.
4. Verify focused Explorer state tests.

### Task 3: Actions, Reducer, and Runtime

**Files:**
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Modify: `src/runtime.rs`
- Test: `tests/catalog_reducer.rs`
- Test: `tests/connection_switch.rs`

1. Add failing tests for empty-query suppression, latest-generation submission, stale result rejection, retry, and connection invalidation.
2. Add semantic search actions and `SearchCatalog` command.
3. Schedule a 150ms debounce through Runtime and dispatch only the latest generation.
4. Execute adapter search in a cancellable background task and return success/failure actions.
5. Verify reducer and connection-switch tests.

### Task 4: PostgreSQL Search

**Files:**
- Modify: `src/db/postgres.rs`
- Test: `tests/postgres_adapter.rs`

1. Add failing tests for name/path matching, scope, kinds, ancestors, ordering, and limit.
2. Implement scoped native catalog search across supported PostgreSQL objects and relation children.
3. Verify PostgreSQL adapter tests.

### Task 5: MySQL Search

**Files:**
- Modify: `src/db/mysql.rs`
- Test: `tests/mysql_adapter.rs`

1. Add failing tests for case-insensitive matching, scope, supported kinds, ancestors, ordering, and limit.
2. Implement scoped `information_schema` search.
3. Verify MySQL adapter tests.

### Task 6: SQLite Search

**Files:**
- Modify: `src/db/sqlite.rs`
- Test: `tests/sqlite_catalog.rs`

1. Add failing tests for attached schema scope, relation and child matching, ancestors, ordering, and limit.
2. Implement `sqlite_schema` plus bounded PRAGMA metadata search.
3. Verify SQLite adapter tests.

### Task 7: Keyboard Interaction

**Files:**
- Modify: `src/input/keymap.rs`
- Test: `tests/keymap.rs`

1. Add failing tests for `/`, text, Backspace, Ctrl-U, navigation, Enter, Esc, and retry.
2. Route keys to search actions before normal Explorer bindings while search is active.
3. Keep `/` contextual to Explorer and preserve existing Relation Data `/` behavior.
4. Verify keymap tests.

### Task 8: Explorer Rendering

**Files:**
- Modify: `src/ui/mod.rs`
- Test: `tests/ui_render.rs`

1. Add failing snapshots/assertions for input, loading, results, path disambiguation, located marker, empty, truncated, failure, and compact layout.
2. Render the inline query row, flat result projection, and contextual status/footer.
3. Reuse existing kind icons, colors, sanitization, and stable mouse hit regions where applicable.
4. Verify UI tests at 80x24 and 120x36.

### Task 9: Documentation and Full Verification

**Files:**
- Modify: `docs/keybindings.md`
- Modify: `docs/architecture.md`
- Modify: `README.md` if Explorer capabilities are enumerated there.

1. Document the search contract and keybindings.
2. Run `cargo fmt --check`.
3. Run focused catalog, Explorer, keymap, UI, and adapter tests.
4. Run `cargo clippy --all-targets --all-features -- -D warnings`.
5. Run `cargo test --all-targets --all-features`.
6. Record any environment-dependent adapter tests that could not run and their residual risk.
