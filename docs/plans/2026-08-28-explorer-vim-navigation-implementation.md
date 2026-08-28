# Explorer Vim Navigation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make Explorer column ordering match relation previews and add viewport-correct Vim vertical navigation to the normal Explorer tree.

**Architecture:** Measure Explorer's rendered inner height in `UiState` and synchronize it into `ExplorerTreeState`, which remains authoritative for selection and scrolling. Add semantic target, page, and alignment operations modeled after `DataGridState`, route them through explicit actions and the existing pending-key state machine, and make adapter catalog pagination sort columns by `ColumnMetadata.ordinal_position`.

**Tech Stack:** Rust 2024, Ratatui, Crossterm, SQLx, Tokio, existing reducer/runtime architecture, Rust unit and integration tests.

---

Implementation must follow the approved design in
`docs/plans/2026-08-28-explorer-vim-navigation-design.md`. Do not create commits
unless the user explicitly requests them.

### Task 1: Order Catalog Columns By Ordinal Position

**Files:**
- Modify: `src/db/sqlite.rs:1723-1777`
- Modify: `src/db/postgres.rs:1961-2065`
- Modify: `src/db/mysql.rs:2383-2482`
- Test: `tests/sqlite_adapter.rs`
- Test: `tests/postgres_adapter.rs`
- Test: `tests/mysql_adapter.rs`

**Step 1: Add failing adapter assertions**

Extend each adapter's existing relation-child catalog test with a table whose
declaration order is deliberately different from alphabetical order:

```sql
CREATE TABLE column_order (
    z_col INTEGER,
    a_col TEXT,
    m_col BOOLEAN
)
```

Load the relation-child catalog page and assert that filtered Column entries are:

```rust
assert_eq!(column_names, vec!["z_col", "a_col", "m_col"]);
assert_eq!(column_ordinals, vec![1, 2, 3]);
```

Use each test file's current database fixture and catalog request helpers. Do not
introduce a second adapter setup path.

**Step 2: Run the focused tests and verify failure**

Run:

```bash
cargo test --test sqlite_adapter column_order -- --nocapture
cargo test --test postgres_adapter column_order -- --nocapture
cargo test --test mysql_adapter column_order -- --nocapture
```

Expected: the SQLite test fails with alphabetical order; PostgreSQL and MySQL
tests either fail similarly or require their existing opt-in database environment.
Record skipped external-adapter tests accurately rather than claiming they ran.

**Step 3: Add an ordinal-aware child sort component**

In each adapter, keep the existing catalog-kind rank as the first component. For
columns, encode ordinal position before the display name; for other kinds retain
the current name ordering. Use a fixed-width numeric component so lexicographic
cursor comparison preserves numeric order:

```rust
fn child_sort_key(entry: &CatalogEntry) -> String {
    let order = match &entry.metadata {
        CatalogMetadata::Column(column) => format!("{:010}", column.ordinal_position),
        _ => entry.qualified_name.object.clone(),
    };
    format!("{:02}\0{}", catalog_kind_rank(entry.kind), order)
}
```

Keep `child_tie_breaker` deterministic. For columns it must include the column
name/native-path suffix so malformed duplicate ordinals do not destabilize
pagination. Ensure `CatalogMetadata` is already imported before adding imports.

**Step 4: Run adapter tests**

Run the focused commands from Step 2, followed by:

```bash
cargo test --test catalog_contract
cargo test --test sqlite_adapter
```

Expected: all locally runnable tests pass; PostgreSQL/MySQL tests pass when their
documented test databases are available.

### Task 2: Add Explorer Viewport And Navigation Model Semantics

**Files:**
- Modify: `src/model/explorer.rs:772-1061`
- Modify: `src/model/workspace.rs:164-175`
- Modify: `src/model/workspace.rs:323-360`
- Modify: `src/model/workspace.rs:533-593`
- Test: `tests/explorer_state.rs`

**Step 1: Add failing one-step viewport tests**

Add a helper that creates an expanded Explorer projection with at least twelve
visible nodes. Write tests equivalent to:

```rust
explorer.set_viewport_height(5);
explorer.move_selection(1);
// Move selection onto screen row 4.
assert_eq!(explorer.scroll, 0);
explorer.move_selection(1);
assert_eq!(explorer.scroll, 1);
```

Also test upward movement: selecting the current top row does not scroll, and one
more `k` scrolls upward.

**Step 2: Add failing semantic-navigation tests**

Cover these cases in `tests/explorer_state.rs`:

```rust
select_target(First)
select_target(Last)
select_target(ViewTop)
select_target(ViewMiddle)
select_target(ViewBottom)
scroll_nodes(1, HalfPage)
scroll_nodes(-1, Page)
align_selected(Top)
align_selected(Middle)
align_selected(Bottom)
```

Assert both selected visible index and `scroll`. Include a final partial viewport,
one-row viewport, short tree, zero-height viewport, and movement at both tree
boundaries. Mirror the proven expectations in `src/model/tab.rs:408-568` rather
than inventing different Explorer semantics.

**Step 3: Run tests to verify failure**

Run:

```bash
cargo test --test explorer_state viewport -- --nocapture
cargo test --test explorer_state navigation -- --nocapture
```

Expected: compilation fails because the semantic types and methods do not exist.

**Step 4: Add semantic Explorer types and viewport state**

Near `VisibleExplorerNode`, add public model enums:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorerNodeTarget {
    First,
    Last,
    ViewTop,
    ViewMiddle,
    ViewBottom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorerScrollAmount {
    HalfPage,
    Page,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorerNodeAlignment {
    Top,
    Middle,
    Bottom,
}
```

Add `viewport_height: usize` to `ExplorerTreeState`, initialized to zero. Prefer a
private field with a getter if no external tests need struct literals; preserve
the project's current construction patterns.

**Step 5: Implement viewport-correct operations**

Change `ExplorerTreeState::move_selection` to use stored viewport height rather
than accepting a height argument. Add:

```rust
pub fn set_viewport_height(&mut self, height: usize)
pub fn select_target(&mut self, target: ExplorerNodeTarget)
pub fn scroll_nodes(&mut self, direction: isize, amount: ExplorerScrollAmount)
pub fn align_selected(&mut self, alignment: ExplorerNodeAlignment)
```

Use the Results grid formulas from `src/model/tab.rs:245-330`, adapted from row
indexes to `VisibleExplorerNode.id`. Keep one local bounded-index helper if it is
used by multiple operations. Do not share `DataGridState` itself.

For zero viewport height, store the height and avoid alignment/page movement.
`ensure_selected_visible` should no longer accept a caller-supplied height.

**Step 6: Make ExplorerState a thin synchronization layer**

Update `ExplorerState::move_selection`, `select_id`, `rebuild_projection`, and
`locate_search_hit` to call normalized operations without `8`. Add wrapper methods
for viewport changes, semantic targets, page movement, and alignment; each wrapper
must call `sync_selected_index()` after normalized mutation.

Remove every Explorer call to `ensure_selected_visible(8)`. Search with:

```bash
rg 'ensure_selected_visible\(8\)|move_selection\([^)]*,\s*8\)' src tests
```

Expected: no matches.

**Step 7: Run model tests**

Run:

```bash
cargo test --test explorer_state
cargo test --test catalog_reducer
cargo test explorer --lib
```

Expected: all pass.

### Task 3: Synchronize The Rendered Explorer Viewport

**Files:**
- Modify: `src/action.rs:434-462`
- Modify: `src/ui/mod.rs:103-112`
- Modify: `src/ui/mod.rs:188-210`
- Modify: `src/ui/mod.rs:519-669`
- Modify: `src/runtime.rs:2450-2565`
- Modify: `src/app.rs:2621-2685`
- Test: `tests/ui_render.rs`

**Step 1: Write failing UI viewport tests**

Extend existing Explorer rendering tests to render at two terminal heights. Use
`render_with_state` and assert:

```rust
assert_eq!(state.explorer_viewport_rows, Some(expected_inner_height));
```

Derive expected values from the known test layout, accounting for the Explorer
panel border. Include one standard and one compact non-`TooSmall` layout.

**Step 2: Run the UI test and verify failure**

Run:

```bash
cargo test --test ui_render explorer_viewport -- --nocapture
```

Expected: compilation fails because `UiState::explorer_viewport_rows` does not
exist.

**Step 3: Record viewport rows during rendering**

Add:

```rust
pub explorer_viewport_rows: Option<usize>,
```

to `UiState`, initialize/reset it with the existing viewport fields in
`render_with_state_using_icons`, and set it in `render_explorer` from
`inner.height as usize` before either the search or normal-tree early return.

Recording it before the search return ensures a pane resized while search is open
will be correct when search closes.

**Step 4: Add viewport synchronization action**

Add `Action::ExplorerViewportChanged(usize)`. Handle it in `App::update` by calling
the ExplorerState viewport wrapper and returning no commands.

In `runtime.rs`, add `sync_explorer_viewport` next to
`sync_editor_viewport`/`sync_grid_viewport`. Dispatch only when the measured value
differs from the model value, preventing redraw action loops. Call it at both
existing viewport synchronization sites after drawing.

**Step 5: Run UI and reducer tests**

Run:

```bash
cargo test --test ui_render explorer
cargo test --test explorer_state
cargo test --test app_flow
```

Expected: all pass.

### Task 4: Route Semantic Explorer Actions Through App

**Files:**
- Modify: `src/action.rs:434-462`
- Modify: `src/app.rs:2621-2685`
- Test: `tests/explorer_state.rs`
- Test: existing reducer tests in `src/app.rs` if appropriate

**Step 1: Add reducer-level failing tests**

Construct an App with an expanded Explorer fixture, set its viewport through
`ExplorerViewportChanged(5)`, then dispatch each semantic action. Assert the
normalized selected node, compatibility `selected`, and both scroll fields stay
synchronized.

**Step 2: Add explicit actions**

Add:

```rust
ExplorerSelectTarget(ExplorerNodeTarget),
ExplorerScrollNodes {
    direction: isize,
    amount: ExplorerScrollAmount,
},
ExplorerAlignSelected(ExplorerNodeAlignment),
```

Keep `ExplorerMove(isize)` for `j/k`, arrows, mouse wheel, and existing help
execution. Import the semantic types from `model::explorer` rather than duplicating
them in `action.rs`.

**Step 3: Implement reducer arms**

Each arm delegates to one `ExplorerState` wrapper and returns `Vec::new()`. Do not
put index arithmetic in `App::update`.

**Step 4: Run focused tests**

Run:

```bash
cargo test explorer_navigation --lib
cargo test --test explorer_state
cargo test --test mouse
```

Expected: all pass, including existing mouse-wheel relative movement.

### Task 5: Add Explorer Vim Key Sequences And Control Keys

**Files:**
- Modify: `src/input/keymap.rs:18-28`
- Modify: `src/input/keymap.rs:236-265`
- Modify: `src/input/keymap.rs:304-363`
- Modify: `src/input/keymap.rs:445-490`
- Modify: `src/input/keymap.rs:796-839`
- Test: `src/input/keymap.rs:972-1279`
- Test: `tests/keymap.rs`

**Step 1: Write failing direct-key tests**

With `app.focus = Focus::Explorer` and no search, assert:

```rust
G => ExplorerSelectTarget(Last)
H => ExplorerSelectTarget(ViewTop)
M => ExplorerSelectTarget(ViewMiddle)
L => ExplorerSelectTarget(ViewBottom)
Ctrl-f => ExplorerScrollNodes { direction: 1, amount: Page }
Ctrl-b => ExplorerScrollNodes { direction: -1, amount: Page }
Ctrl-d => ExplorerScrollNodes { direction: 1, amount: HalfPage }
Ctrl-u => ExplorerScrollNodes { direction: -1, amount: HalfPage }
```

Update Home/End expectations to semantic First/Last actions.

**Step 2: Write failing sequence tests**

Test two-event mappings for `gg`, `zz`, `zt`, and `zb`. Also cover an invalid
second key, focus change, and sequence timeout using the existing pending-state
test style. Assert that `g` and `z` alone emit no action.

**Step 3: Write search-isolation tests**

Open Explorer search and assert that `g`, `G`, `H`, `M`, `L`, and `z` map to
`ExplorerSearchInsert(character)`, while Ctrl-U remains
`ExplorerSearchClear`. Assert no semantic tree action is produced.

**Step 4: Add Explorer pending states**

Add `ExplorerGoto` and `ExplorerAlign` pending variants, or generalize the current
grid variants only if doing so reduces code without weakening focus checks. Start
these pending states only when Explorer has focus and search is absent.

Resolve:

```rust
(ExplorerGoto, 'g') => ExplorerSelectTarget(First)
(ExplorerAlign, 'z') => ExplorerAlignSelected(Middle)
(ExplorerAlign, 't') => ExplorerAlignSelected(Top)
(ExplorerAlign, 'b') => ExplorerAlignSelected(Bottom)
```

Treat an invalid second key like the Grid sequence path: consume it and produce no
unrelated Explorer command.

**Step 5: Add direct and control mappings**

Map direct keys in `map_explorer`. In the control-modifier branch, check
`Focus::Explorer` with `search.is_none()` before the general control handling.
Do not broaden matching to extra modifiers.

**Step 6: Run keymap tests**

Run:

```bash
cargo test input::keymap --lib
cargo test --test keymap
```

Expected: all pass, including Results grid Vim navigation and Explorer search key
routing regressions.

### Task 6: Update Help, Footer, And Keyboard Documentation

**Files:**
- Modify: `src/help.rs:1-40`
- Modify: `src/help.rs:196-240`
- Modify: `src/app.rs:430-475`
- Modify: `src/ui/mod.rs:1211-1217`
- Modify: `docs/keybindings.md:67-97`
- Test: `src/help.rs:460-510`

**Step 1: Add failing help-contract tests**

Extend Explorer help tests to assert that searchable rows exist for document
movement, viewport targets, page movement, and alignment. Give commands with
different actions separate IDs, following the current help contract.

**Step 2: Add help shortcut definitions and execution mapping**

Add concise labels and key displays for:

```text
gg / G
H / M / L
Ctrl-d / Ctrl-u
Ctrl-f / Ctrl-b
zz / zt / zb
```

Map each help shortcut ID in `App::execute_help_shortcut` to the same semantic
actions as the keymap. Avoid simulating raw key sequences from help.

**Step 3: Update visible documentation**

Update the Explorer table in `docs/keybindings.md` with the approved semantics and
state explicitly that these commands apply to the normal tree, not search input.
Keep the footer compact, for example:

```text
j/k move   gg/G ends   Ctrl-d/u page   Enter open
```

Use the final wording that fits existing footer width tests; do not list every
binding there.

**Step 4: Run help and UI tests**

Run:

```bash
cargo test help --lib
cargo test --test ui_render footer -- --nocapture
```

Expected: all pass.

### Task 7: Verify Cross-Path Visibility And Full Regression Suite

**Files:**
- Modify if needed: `tests/explorer_state.rs`
- Modify if needed: `tests/catalog_reducer.rs`
- Modify if needed: `tests/ui_render.rs`
- Verify: all changed production and documentation files

**Step 1: Add missing cross-path regression tests**

Confirm tests cover these non-keyboard paths with a measured viewport:

- mouse selection outside the current slice;
- search locate followed by search close;
- expanding or collapsing before the selected node;
- catalog replacement/removal of the selected node;
- resizing from a tall viewport to a short viewport and back;
- a final partial page.

Add only tests not already covered by Tasks 2-4.

**Step 2: Run formatting**

Run:

```bash
cargo fmt --all -- --check
```

If it fails, run `cargo fmt --all`, then rerun the check.

**Step 3: Run static checks**

Run:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: no warnings.

**Step 4: Run the complete local suite**

Run:

```bash
cargo test --all-targets --all-features
```

Expected: all locally runnable tests pass. Clearly report any PostgreSQL/MySQL
environment-dependent tests that were not runnable.

**Step 5: Inspect the final diff**

Run:

```bash
git status --short
git diff --check
git diff -- src/action.rs src/app.rs src/input/keymap.rs src/model/explorer.rs src/model/workspace.rs src/runtime.rs src/ui/mod.rs src/help.rs src/db/sqlite.rs src/db/postgres.rs src/db/mysql.rs tests docs
```

Verify:

- no hard-coded Explorer viewport height remains;
- search input routing precedes normal Explorer sequence handling;
- adapter cursor sort and display order are identical;
- non-column ordering is unchanged;
- no unrelated user changes were reverted;
- no commit is created unless explicitly requested.
