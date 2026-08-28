# Explorer Frontend Catalog Search Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace server-backed `f` search with an immediate, normal-style frontend filter that excludes relation children and updates as catalog loading continues.

**Architecture:** Keep `CatalogTree` authoritative and derive an ancestor-preserving filtered `VisibleExplorerNode` projection from its cached entries. Reuse normal Explorer row presentation, keep normal expansion state isolated, and broaden automatic loading only for searchable non-relation object groups.

**Tech Stack:** Rust 2024, Ratatui, Crossterm, Tokio, existing Explorer model/reducer/runtime architecture.

---

### Task 1: Filtered Catalog Projection

**Files:**
- Modify: `src/model/explorer.rs:894-942,1336-1460`
- Test: `tests/explorer_state.rs`

**Step 1: Add failing projection tests**

Build a loaded catalog with shared namespace ancestors, unrelated siblings, two
matching relations, and loaded relation children. Assert that a filtered query:

- returns normal `ExplorerNodeId` values;
- retains Profile/Database/Schema/Group ancestors;
- omits unrelated branches;
- excludes Column, Index, Constraint, and relation-owned Trigger entries;
- never emits children below a relation;
- preserves normal catalog/group order.

**Step 2: Run the focused test**

```bash
cargo test --test explorer_state frontend_catalog_search -- --nocapture
```

Expected: compilation fails because the filtered projection API is absent.

**Step 3: Add the minimal projection API**

Add a filtered projection result containing normal visible rows and matching node
IDs. Match allowed catalog entries case-insensitively by primary name and qualified
path. Build an included-node set from matches and their catalog/group/profile
ancestors, then flatten it in normal tree order. Do not inspect or mutate normal
expansion for inclusion, and terminate at relations.

**Step 4: Run projection and performance tests**

```bash
cargo test --test explorer_state frontend_catalog_search -- --nocapture
cargo test --test explorer_performance
```

Expected: all pass.

### Task 2: Frontend Search State And Lifecycle

**Files:**
- Modify: `src/model/workspace.rs:201-515`
- Modify: `src/action.rs:450-480,590-610`
- Modify: `src/app.rs:2747-2835,4690-4761`
- Test: `tests/explorer_state.rs`
- Test: `tests/catalog_reducer.rs`

**Step 1: Add failing lifecycle tests**

Assert that opening and editing `f` synchronously produces filtered rows and
matches, preserves a selected stable ID while pages arrive, and emits no
`Command::SearchCatalog`. Assert Esc restores original selection/scroll and Enter
selects the normal object, expands only its namespace/group ancestors, centers it,
and closes search.

**Step 2: Run focused tests**

```bash
cargo test --test explorer_state frontend_search_lifecycle -- --nocapture
cargo test --test catalog_reducer frontend_search -- --nocapture
```

Expected: current server-backed lifecycle assertions fail.

**Step 3: Replace hit-backed state**

Remove `hits`, server generations, temporary catalog rows, truncation, and located
state from active `f` behavior. Store filtered normal rows, matching IDs, current
selection, scroll, and original normal selection/scroll. Recompute synchronously
after query edits and accepted catalog pages.

Keep the server search command and adapter contracts compiled but unused. Remove
only action/command paths that become unreachable if doing so does not broaden the
change into adapter deletion.

**Step 4: Implement locate-and-close**

Resolve the selected matching ID in `CatalogTree`, expand Profile/Database/Schema
and its presentation Group, never expand the matched relation, select and center
the node, then remove search state. Esc restores the original normal state.

**Step 5: Run lifecycle tests**

Run the commands from Step 2 and expect all tests to pass.

### Task 3: Shared Explorer Row Rendering

**Files:**
- Modify: `src/ui/mod.rs:533-987`
- Test: `tests/ui_render.rs`

**Step 1: Add failing render tests**

Render the same Profile, Group, and catalog object normally and through `f` search.
Assert equal labels, indentation, markers, icons, kind colors, metadata, and selected
background. Assert the search-only difference is query highlighting and input/status
lines. Verify a loaded expanded table has no child rows or expansion marker.

**Step 2: Extract shared row presentation**

Move normal row span construction into one helper accepting a normal visible row,
selection state, projection expansion state, and optional highlight query. Use it
from normal, `/`, and `f` paths where practical. Remove hard-coded `Profile` and
`format!("{group:?}")` search labels.

**Step 3: Render indexing status**

Show `Indexing catalog...` while relevant catalog object-group requests remain
pending. Keep existing results visible and preserve the query cursor behavior.

**Step 4: Run UI tests**

```bash
cargo test --test ui_render explorer_search -- --nocapture
```

Expected: all search presentation tests pass.

### Task 4: Search Keyboard Routing

**Files:**
- Modify: `src/input/keymap.rs:165-195`
- Modify: `src/app.rs:2791-2835`
- Test: `tests/keymap.rs`

**Step 1: Add failing keymap tests**

Cover editing input, Backspace, Ctrl-U, arrows and `j`/`k`, Enter locate-and-close,
Esc restore, and confirmed `n`/`N` match navigation. Ensure no server retry binding
is advertised or emitted.

**Step 2: Simplify routing**

Route search editing and result navigation to synchronous model actions. Enter
locates and closes. Keep focus isolation and pending-key reset semantics consistent
with the existing search input mode.

**Step 3: Run tests**

```bash
cargo test --test keymap explorer_search -- --nocapture
```

Expected: all pass.

### Task 5: Background Searchable Object Loading

**Files:**
- Modify: `src/app.rs:4903-4935,6691-6700`
- Test: `tests/catalog_reducer.rs`
- Test: `tests/sqlite_adapter.rs`
- Test: `tests/postgres_adapter.rs`
- Test: `tests/mysql_adapter.rs`

**Step 1: Add failing scheduling tests**

Given adapter-supported group summaries, assert automatic requests include all
searchable non-relation groups and exclude relation children. Verify unsupported
groups are not requested for each database kind.

**Step 2: Separate preload intent from completion intent**

Keep `completion_group` scoped to completion semantics. Add a capability-aware
search preload predicate for Tables, Views, Materialized Views, Functions,
Procedures, Sequences, Types, and Triggers. Use existing group summaries and
adapter capability truth rather than issuing requests known to be unsupported.

**Step 3: Refresh active search after pages**

After applying a catalog page and rebuilding the normal projection, recompute an
active frontend search while retaining its selected stable ID where possible.

**Step 4: Run reducer and adapter tests**

```bash
cargo test --test catalog_reducer
cargo test --test sqlite_adapter
cargo test --test postgres_adapter
cargo test --test mysql_adapter
```

Expected: local tests pass; externally configured adapter tests remain truthful
about skips.

### Task 6: Full Regression Verification

**Files:**
- Verify all modified files

**Step 1: Format and lint**

```bash
cargo fmt -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: both pass.

**Step 2: Run the complete suite**

```bash
cargo test
```

Expected: all tests pass.

**Step 3: Inspect scope**

```bash
git diff --check
git status --short
```

Confirm unrelated worktree files remain untouched and server adapter search removal
has not expanded the change beyond the approved frontend migration.
