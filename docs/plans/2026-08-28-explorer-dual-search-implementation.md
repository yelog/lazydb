# Explorer Dual Search Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Split Explorer search into `/` local visible-tree find and `f` server-backed, hierarchy-preserving catalog search with match highlighting and `n`/`N` navigation.

**Architecture:** Add a synchronous snapshot-based find state beside the existing asynchronous catalog-search state. Keep the normal Explorer projection authoritative for `/`; derive a separate ancestor-preserving projection from `CatalogSearchHit` values for `f`, without changing adapter search contracts or normal lazy-tree expansion.

**Tech Stack:** Rust, Tokio, Crossterm, Ratatui, existing Explorer model/reducer/runtime architecture and PostgreSQL/MySQL/SQLite catalog adapters.

---

### Task 1: Local Find Domain State

**Files:**
- Modify: `src/model/workspace.rs:164-321`
- Test: `tests/explorer_state.rs`

**Step 1: Write failing state tests**

Add focused tests that construct a normalized Explorer containing an expanded and
a collapsed branch, then assert:

```rust
#[test]
fn visible_find_snapshots_only_projected_primary_labels() {
    let mut explorer = explorer_with_loaded_visible_and_hidden_objects();

    explorer.open_find();
    explorer.edit_find(|query| query.push_str("user"));

    let find = explorer.find.as_ref().unwrap();
    assert!(find.matches.contains(&visible_user_id()));
    assert!(!find.matches.contains(&hidden_user_audit_id()));
    assert_eq!(find.match_position(), Some((1, 1)));
}
```

Also cover case-insensitive matching, primary-label-only matching, an empty query,
and multiple substring occurrences in one label counting as one result. Reuse the
existing Explorer fixtures in `tests/explorer_state.rs`; do not create an adapter
or runtime fixture for this synchronous feature.

**Step 2: Run tests and verify failure**

Run:

```bash
cargo test --test explorer_state visible_find -- --nocapture
```

Expected: compilation fails because the find state and methods do not exist.

**Step 3: Add minimal local-find types**

In `src/model/workspace.rs`, add:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorerSearchPhase {
    Editing,
    Confirmed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExplorerFindRow {
    pub id: ExplorerNodeId,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExplorerFindState {
    pub phase: ExplorerSearchPhase,
    pub query: String,
    pub rows: Vec<ExplorerFindRow>,
    pub matches: Vec<ExplorerNodeId>,
    pub current: usize,
    pub original_selected: Option<ExplorerNodeId>,
}
```

Add `find: Option<ExplorerFindState>` to `ExplorerState`. Implement:

- `open_find()` by snapshotting `self.visible()` into `(id, label)` rows;
- `edit_find()` by changing the query and recomputing matches;
- ASCII/Unicode-safe case-insensitive comparison using the same normalization
  strategy already used by catalog search where possible;
- `find_match_position()` returning one-based `(current, total)`, or `(0, 0)` for
  no matches.

An empty query must produce no matches. Do not inspect `CatalogTree::entries()` or
emit commands.

**Step 4: Run focused tests**

Run:

```bash
cargo test --test explorer_state visible_find -- --nocapture
```

Expected: all visible-find snapshot and matching tests pass.

### Task 2: Local Find Confirmation And Navigation

**Files:**
- Modify: `src/model/workspace.rs:218-321`
- Test: `tests/explorer_state.rs`

**Step 1: Write failing navigation tests**

Add tests for:

- Enter confirmation selecting the first match;
- forward and backward navigation wrapping at both ends;
- selection and `normalized.scroll` following the current match;
- cancellation during editing restoring `original_selected`;
- clearing confirmed find retaining the current selected node;
- a disappeared node being skipped rather than selected.

Express the central behavior directly:

```rust
explorer.confirm_find();
assert_eq!(explorer.selected_id(), Some(&first_match));
explorer.move_find_match(-1);
assert_eq!(explorer.selected_id(), Some(&last_match));
```

**Step 2: Run tests and verify failure**

Run:

```bash
cargo test --test explorer_state find_navigation -- --nocapture
```

Expected: compilation fails for missing confirmation/navigation methods.

**Step 3: Implement confirmation and cyclic movement**

Add cohesive methods to `ExplorerState`:

```rust
pub fn confirm_find(&mut self) -> bool;
pub fn move_find_match(&mut self, delta: isize) -> bool;
pub fn close_find(&mut self, restore_original: bool);
```

Use modular index movement rather than `saturating_add_signed`, so `n` on the last
match reaches the first and `N` on the first reaches the last. Resolve IDs against
the current normal `visible()` projection, update `normalized.selected`, and call
the existing visibility/scroll helper. Keep all operations no-op safe for empty
queries and empty match lists.

**Step 4: Run focused tests**

Run:

```bash
cargo test --test explorer_state find_navigation -- --nocapture
```

Expected: all local-find lifecycle and wraparound tests pass.

### Task 3: Catalog Search Tree Projection

**Files:**
- Modify: `src/model/workspace.rs:177-321`
- Modify: `src/model/explorer.rs` only if a small existing-order query is needed
- Test: `tests/explorer_state.rs`
- Test: `tests/catalog_reducer.rs`

**Step 1: Write failing projection tests**

Create `CatalogSearchPage` fixtures with:

- two hits sharing profile/database/schema/group ancestors;
- one relation-child hit requiring its relation ancestor;
- an unrelated sibling absent from all hit paths;
- response hits intentionally out of order.

Assert that the resulting projection:

```rust
assert_eq!(rows.iter().filter(|row| row.is_match).count(), 2);
assert_eq!(rows.iter().filter(|row| row.id == shared_schema).count(), 1);
assert!(!rows.iter().any(|row| row.id == unrelated_table));
assert!(rows.windows(2).all(stable_tree_order));
```

Also assert that every matching row retains its original `CatalogSearchHit` index
or stable ID so locating and `n`/`N` navigation can resolve the actual hit.

**Step 2: Run tests and verify failure**

Run:

```bash
cargo test --test explorer_state catalog_search_tree -- --nocapture
cargo test --test catalog_reducer explorer_search -- --nocapture
```

Expected: projection assertions fail because accepted pages still expose only flat
hits.

**Step 3: Add the temporary result-tree model**

Add a presentation model resembling:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExplorerCatalogSearchRow {
    pub id: ExplorerNodeId,
    pub depth: usize,
    pub label: String,
    pub kind: Option<CatalogKind>,
    pub profile_kind: Option<DatabaseKind>,
    pub is_match: bool,
    pub hit_index: Option<usize>,
}
```

Store `rows` and an ordered list of matching row/hit IDs in the catalog search
state. Build them in `accept_search_page()` from each hit's `ancestors` and
`entry`:

1. Insert the profile root.
2. Insert each real ancestor in parent-before-child order.
3. Insert an `ExplorerNodeId::Group` for schema-owned kinds using the existing
   `search_object_group()` mapping.
4. Insert the hit entry and mark it as a match.
5. Deduplicate every node by stable ID.
6. Sort siblings by normal catalog/native path order, with stable qualified path
   fallback.
7. Flatten only included paths with every branch treated as expanded.

Do not write these rows into `ExplorerTreeState` and do not modify
`normalized.expanded` while building them.

**Step 4: Keep asynchronous acceptance semantics intact**

Preserve checks for connection, session, and generation before replacing hits or
rows. A rejected page must change neither the previous flat hit data nor the new
tree projection.

**Step 5: Run focused tests**

Run:

```bash
cargo test --test explorer_state catalog_search_tree -- --nocapture
cargo test --test catalog_reducer explorer_search -- --nocapture
```

Expected: tree projection and existing stale-result tests pass.

### Task 4: Catalog Search Phase, Navigation, And Restoration

**Files:**
- Modify: `src/model/workspace.rs:177-321`
- Modify: `src/app.rs:2621-2680`
- Test: `tests/explorer_state.rs`
- Test: `tests/catalog_reducer.rs`

**Step 1: Write failing lifecycle tests**

Cover:

- catalog search starts in editing phase;
- `n` and `N` remain query input during editing;
- confirmation selects the first actual match, not its ancestor;
- match navigation wraps and skips ancestor-only rows;
- ordinary result movement can select any projected row;
- closing without location restores original normal selection, scroll, and
  expansion;
- closing after `locate_search_hit()` retains the located normal-tree path.

**Step 2: Run tests and verify failure**

Run:

```bash
cargo test --test explorer_state catalog_search_navigation -- --nocapture
cargo test --test catalog_reducer catalog_search_restore -- --nocapture
```

Expected: lifecycle/restoration assertions fail.

**Step 3: Extend catalog-search state minimally**

Add:

- `phase: ExplorerSearchPhase`;
- separate selected result-tree row and current hit indexes if one index cannot
  represent both concepts clearly;
- a snapshot containing normal `selected`, `scroll`, and `expanded`;
- `located` as the switch that suppresses snapshot restoration after a successful
  locate.

Rename methods/actions from generic `ExplorerSearch*` to
`ExplorerCatalogSearch*` where doing so removes ambiguity. Perform the rename in
one mechanical change across `src/action.rs`, `src/app.rs`, `src/input/keymap.rs`,
tests, and `src/ui/mod.rs`; do not leave compatibility aliases because there are no
external consumers of internal actions.

**Step 4: Implement result-tree and hit movement**

Keep two explicit operations:

- result-row movement for arrows/`j`/`k`/Home/End;
- cyclic actual-hit movement for confirmed `n`/`N`.

Update `locate_search_hit()` to resolve the selected row's `hit_index`; return
false when an ancestor-only row is selected. On successful location, mark the
search as located and retain the existing merge, group expansion, selection, and
pagination-safe behavior.

**Step 5: Implement restoration in the reducer/model boundary**

Closing catalog search must restore its snapshot only when `located` is absent.
Always return `Command::CancelCatalogSearch`. Ensure `connection_changed()` clears
both local find and catalog search without applying a stale snapshot to the new
connection.

**Step 6: Run focused tests**

Run:

```bash
cargo test --test explorer_state catalog_search_navigation -- --nocapture
cargo test --test catalog_reducer -- --nocapture
```

Expected: all catalog search navigation, location, stale-response, and restoration
tests pass.

### Task 5: Actions And Keyboard Routing

**Files:**
- Modify: `src/action.rs:430-450`
- Modify: `src/input/keymap.rs:143-165`
- Modify: `src/input/keymap.rs:796-838`
- Modify: `src/app.rs:650-670`
- Modify: `src/app.rs:2621-2680`
- Test: `src/input/keymap.rs:1003-1066`
- Test: `tests/keymap.rs` if Explorer integration fixtures live there

**Step 1: Write failing keymap tests**

Add assertions for:

```rust
assert_eq!(map_explorer_key('/'), Some(Action::ExplorerFindOpen));
assert_eq!(map_explorer_key('f'), Some(Action::ExplorerCatalogSearchOpen));
```

Then cover these states:

- local-find editing maps printable `n`/`N` to insertion;
- Enter maps to find confirmation;
- confirmed find maps `n`/`N` to next/previous match;
- Esc cancels editing or clears confirmed find;
- after find closes, lowercase `n` maps to `ProfileStartNew`;
- catalog-search editing accepts `n`/`N` as query characters;
- confirmed catalog search maps `n`/`N` to hit navigation;
- Relation Data `/` and Editor search keys retain their existing actions.

**Step 2: Run tests and verify failure**

Run:

```bash
cargo test input::keymap::tests::explorer -- --nocapture
cargo test --test keymap explorer -- --nocapture
```

Expected: `/` still opens catalog search and `f` has no Explorer mapping.

**Step 3: Add semantic local-find actions**

Add actions for open, insert, backspace, clear, confirm, next/previous, and close.
Keep next/previous semantic rather than passing magic `isize` values from the
keymap. Route them in `App::update` to the synchronous model methods and return no
commands.

**Step 4: Route by active phase before normal Explorer bindings**

Refactor the current single `app.explorer.search.is_some()` preemption block into
small catalog-search and local-find branches. Editing branches consume printable
text. Confirmed branches reserve `n`, `N`, and Esc but allow normal Explorer
bindings only where the approved interaction requires them.

In normal `map_explorer()`:

```rust
KeyCode::Char('/') => Some(Action::ExplorerFindOpen),
KeyCode::Char('f') => Some(Action::ExplorerCatalogSearchOpen),
```

Opening either mode clears the other; opening local find cancels an active catalog
request.

**Step 5: Run focused tests**

Run:

```bash
cargo test input::keymap::tests::explorer -- --nocapture
cargo test --test keymap explorer -- --nocapture
cargo test --test catalog_reducer explorer_search -- --nocapture
```

Expected: all routing tests pass and local find emits no search command.

### Task 6: Reusable Match Highlighting

**Files:**
- Modify: `src/ui/mod.rs:519-669`
- Test: `src/ui/mod.rs:1798-1804`
- Test: `tests/ui_render.rs`

**Step 1: Write failing rendering tests**

At 80x24, render a normal Explorer with three visible labels matching the local
query. Assert the buffer contains `/ user`, `(1/3)`, and the normal tree labels.
Inspect cell styles across a matching label to prove only matching character cells
receive the match style while indentation/icon/non-matching text retain their base
styles.

Add a Unicode label case so highlighting does not split a UTF-8 character or use
byte offsets as terminal columns.

**Step 2: Run tests and verify failure**

Run:

```bash
cargo test --test ui_render explorer_visible_find -- --nocapture
cargo test ui::tests::explorer_match_spans -- --nocapture
```

Expected: local find state is not rendered in the normal tree and match spans do
not exist.

**Step 3: Add a pure label-span helper**

Create a small helper in `src/ui/mod.rs` that takes sanitized label text, query,
base style, match style, and returns `Vec<Span<'static>>`. It must:

- find all case-insensitive non-overlapping occurrences;
- operate on valid character boundaries;
- preserve the selected row background in every returned style;
- return one base span for empty/no-match input.

Do not introduce a regex dependency.

**Step 4: Render local find without replacing the tree**

Remove the unconditional search replacement for local find. Reserve one line for
the `/ query` and `(current/total)` status while find is active, then render the
normal tree in the remaining area using existing hit regions and scroll behavior.
Set the terminal cursor only in editing phase.

Apply the stronger style to the current matching node and a weaker style to other
matching nodes. Keep selected-row background precedence.

**Step 5: Run focused tests**

Run:

```bash
cargo test --test ui_render explorer_visible_find -- --nocapture
cargo test ui::tests::explorer_match_spans -- --nocapture
```

Expected: counter, tree retention, styles, and Unicode tests pass.

### Task 7: Tree-Shaped Catalog Search Rendering

**Files:**
- Modify: `src/ui/mod.rs:671-829`
- Test: `tests/ui_render.rs:110-170`

**Step 1: Replace flat-result expectations with tree expectations**

Update existing Explorer search UI fixtures to provide a result page with shared
ancestors. Assert the rendered buffer contains correctly indented profile,
database/schema/group, and matching object rows in that order. Assert unrelated
sibling text is absent.

Retain separate tests for idle, loading, empty, truncated, failed, and located
states at compact size.

**Step 2: Run tests and verify failure**

Run:

```bash
cargo test --test ui_render explorer_catalog_search -- --nocapture
```

Expected: current UI renders only flat object/path rows.

**Step 3: Render the model-provided tree projection**

Change catalog-search rendering to iterate over projected rows, not raw hits. Reuse
the normal Explorer conventions for:

- two-space depth indentation;
- expanded branch marker for ancestor rows;
- profile/database, group, and catalog icons;
- catalog kind colors;
- selected-row background.

Use the Task 6 span helper for matching labels. Count and navigate actual matches,
not ancestor rows. Continue sanitizing all adapter-provided text before rendering.

Show the editing cursor only during catalog-search editing. In confirmed mode,
render compact `n/N` and Esc guidance. Preserve `Searching...`, no-result,
truncation, total-count, retry, and located indicators.

**Step 4: Run focused UI tests**

Run:

```bash
cargo test --test ui_render explorer_catalog_search -- --nocapture
cargo test --test ui_render explorer_search -- --nocapture
```

Expected: hierarchical and lifecycle rendering tests pass at 80x24 and 120x36.

### Task 8: Documentation And Full Regression Verification

**Files:**
- Modify: `docs/keybindings.md:67-98`
- Modify: `docs/architecture.md`
- Modify: `README.md` only if it describes Explorer search keys
- Verify: `docs/plans/2026-08-28-explorer-dual-search-design.md`

**Step 1: Update the keyboard contract**

Document:

- `/` searches only the visible expanded-tree snapshot;
- Enter confirms and `n`/`N` wrap through highlighted matches;
- `f` searches the full active catalog scope and retains result hierarchy;
- lowercase `n` creates a profile only when no confirmed find owns the key;
- input editing, clearing, cancellation, location, retry, and result limits.

Remove statements claiming `/` queries every catalog object.

**Step 2: Update architecture documentation**

Describe the separation between synchronous visible find and asynchronous catalog
search, the temporary result-tree projection, and the rule that neither projection
alters normal lazy loading unless a catalog hit is explicitly located.

**Step 3: Format and run focused suites**

Run:

```bash
cargo fmt --check
cargo test --test explorer_state
cargo test --test catalog_reducer
cargo test --test keymap
cargo test --test ui_render
```

Expected: formatting and all focused suites pass.

**Step 4: Run static analysis**

Run:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: no warnings.

**Step 5: Run the full test suite**

Run:

```bash
cargo test --all-targets --all-features
```

Expected: all tests pass. If adapter integration tests require unavailable database
services, record exactly which tests were skipped or failed for environmental
reasons and run every remaining unit/contract suite.

**Step 6: Inspect the final diff**

Run:

```bash
git diff --check
```

Expected: no whitespace errors; changes are limited to Explorer search model,
actions, reducer, keymap, UI, tests, and documentation. Do not modify adapter SQL
unless a failing regression test proves the existing contract is insufficient.
