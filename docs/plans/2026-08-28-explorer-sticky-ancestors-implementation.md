# Explorer Sticky Ancestors Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Keep every offscreen ancestor of the selected Explorer node pinned at the top while preserving selection visibility, tree navigation, find behavior, and mouse interaction.

**Architecture:** Add one pure, UI-neutral viewport projection to `ExplorerTreeState` that derives pinned ancestors, compact-height omission, body rows, and effective body height from the visible tree, selection, scroll, and measured tree height. Make all selection and scroll operations use that same projection, then have the Ratatui renderer consume it for both normal browsing and in-tree find while flat catalog search keeps its existing path.

**Tech Stack:** Rust 2024, Ratatui 0.30, Crossterm 0.29, existing normalized Explorer model and Rust integration tests.

---

## Implementation Constraints

- Follow the approved design in `docs/plans/2026-08-28-explorer-sticky-ancestors-design.md`.
- Do not add a dependency or replace the existing Explorer projection.
- Keep Ratatui types out of `src/model/explorer.rs` and `src/model/workspace.rs`.
- Keep `ExplorerTreeState::scroll` as an absolute index into `visible()`.
- Derive pinned rows; do not persist a second pinned selection or pinned ID list.
- Resolve parents through `visible_parent()`, never by subtracting depth.
- Format only rows present in the final viewport.
- Do not change flat catalog-search rendering.
- Preserve unrelated worktree changes and stage only files changed by each task.

### Task 1: Add the Pure Sticky Viewport Projection

**Files:**
- Modify: `src/model/explorer.rs:778-816`
- Modify: `src/model/explorer.rs:1023-1071`
- Modify: `src/model/explorer.rs:1290-1345`
- Test: `tests/explorer_state.rs`

**Step 1: Write failing projection tests**

Add a fixture that creates one expanded path with enough sibling table rows to
scroll the selected table below the top. Reuse the existing `fixture()`,
`explorer_with_fixture()`, and catalog helpers where possible instead of creating
a second tree builder.

Add these focused tests:

```rust
#[test]
fn viewport_pins_only_selected_ancestors_above_scroll() {
    let (explorer, ids) = sticky_fixture();
    let viewport = explorer.viewport(6);

    assert_eq!(
        viewport.pinned.iter().map(|row| &row.id).collect::<Vec<_>>(),
        vec![&ids.profile, &ids.database, &ids.schema, &ids.tables]
    );
    assert_eq!(viewport.rows.first().map(|row| &row.id), Some(&ids.first_body));
    assert!(!viewport.rows.iter().any(|row| row.id == ids.tables));
}

#[test]
fn viewport_does_not_duplicate_an_ancestor_still_in_the_body() {
    let (mut explorer, ids) = sticky_fixture();
    explorer.scroll = ids.tables_index;

    let viewport = explorer.viewport(6);

    assert!(!viewport.pinned.iter().any(|row| row.id == ids.tables));
    assert_eq!(viewport.rows.first().map(|row| &row.id), Some(&ids.tables));
}

#[test]
fn compact_viewport_keeps_nearest_ancestor_and_body_row() {
    let (explorer, ids) = sticky_fixture();
    let viewport = explorer.viewport(2);

    assert_eq!(viewport.pinned.len(), 1);
    assert_eq!(viewport.pinned[0].id, ids.tables);
    assert_eq!(viewport.rows.len(), 1);
    assert_eq!(viewport.hidden_ancestor_count, 3);
}

#[test]
fn one_row_viewport_reserves_the_row_for_the_selected_body() {
    let (explorer, ids) = sticky_fixture();
    let viewport = explorer.viewport(1);

    assert!(viewport.pinned.is_empty());
    assert_eq!(viewport.rows, vec![ids.selected_row]);
    assert_eq!(viewport.body_height, 1);
}

#[test]
fn zero_height_viewport_is_empty() {
    let (explorer, _) = sticky_fixture();
    let viewport = explorer.viewport(0);

    assert!(viewport.pinned.is_empty());
    assert!(viewport.rows.is_empty());
    assert_eq!(viewport.body_height, 0);
}
```

The helper IDs should use `ExplorerNodeId`, not copied display labels. If the
selected row is not initially the first body row, assert that it is contained in
`viewport.rows` instead of weakening the production behavior.

**Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --test explorer_state viewport_pins_only_selected_ancestors_above_scroll
cargo test --test explorer_state compact_viewport_keeps_nearest_ancestor_and_body_row
```

Expected: compilation fails because `ExplorerTreeState::viewport` and its result
type do not exist.

**Step 3: Add projection result types**

Near `VisibleExplorerNode`, add UI-neutral public result types:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExplorerViewport {
    pub pinned: Vec<VisibleExplorerNode>,
    pub rows: Vec<VisibleExplorerNode>,
    pub hidden_ancestor_count: usize,
    pub body_height: usize,
}
```

If equality is already unavailable on `VisibleExplorerNode`, derive
`Eq, PartialEq` there too. Do not add display strings or Ratatui styles.

**Step 4: Add selected ancestor resolution**

Add one private method that walks `visible_parent()` from `selected` to the root,
reverses the collected IDs, and maps them through a one-pass visible-row index.
The selected node itself must not be included.

The implementation shape should be:

```rust
fn selected_ancestors(
    &self,
    indexes: &HashMap<ExplorerNodeId, usize>,
) -> Vec<(usize, ExplorerNodeId)> {
    let mut ancestors = Vec::new();
    let mut current = self.selected.as_ref().and_then(|id| self.visible_parent(id));
    while let Some(id) = current {
        current = self.visible_parent(&id);
        if let Some(index) = indexes.get(&id).copied() {
            ancestors.push((index, id));
        }
    }
    ancestors.reverse();
    ancestors
}
```

Use references where convenient, but keep one parent traversal and one visible
index construction per projection.

**Step 5: Implement compact pinned-row selection**

Implement `pub fn viewport(&self, height: usize) -> ExplorerViewport`:

1. Call `visible()` once.
2. Return an empty result for `height == 0` or no rows.
3. Build `HashMap<ExplorerNodeId, usize>` in one pass.
4. Find ancestors whose visible index is strictly less than `self.scroll`.
5. Reserve at least one row for the normal body.
6. Retain nearest ancestors by removing root-side ancestors first.
7. Set `hidden_ancestor_count` to the omitted count.
8. Do not reserve an omission-indicator row at heights one or two.
9. At height three or greater, reserve one indicator row only when doing so still
   leaves one body row and at least one nearest pinned ancestor; otherwise omit
   the indicator from the screen projection while retaining the count.
10. Slice normal body rows from `scroll` for `body_height` rows.

Do not clone the complete visible vector twice. Clone only final pinned and body
rows into the result.

**Step 6: Run projection tests**

Run:

```bash
cargo test --test explorer_state viewport_
cargo test --test explorer_state compact_viewport_
cargo test --test explorer_state one_row_viewport_
cargo test --test explorer_state zero_height_viewport_
```

Expected: all new projection tests pass.

**Step 7: Commit the projection**

```bash
git add src/model/explorer.rs tests/explorer_state.rs
git commit -m "feat(explorer): project sticky ancestor rows"
```

### Task 2: Make Navigation Use Effective Body Height

**Files:**
- Modify: `src/model/explorer.rs:1134-1242`
- Modify: `src/model/explorer.rs:1290-1302`
- Test: `tests/explorer_state.rs:389-453`

**Step 1: Write failing navigation tests**

Use the sticky fixture from Task 1 and add:

```rust
#[test]
fn movement_keeps_selection_in_the_body_below_pinned_rows() {
    let (mut explorer, ids) = sticky_fixture();
    explorer.set_viewport_height(6);
    explorer.select(ids.last_table.clone());
    explorer.ensure_selected_visible();

    let viewport = explorer.viewport(6);
    assert!(viewport.rows.iter().any(|row| row.id == ids.last_table));
    assert!(!viewport.pinned.iter().any(|row| row.id == ids.last_table));
}

#[test]
fn page_and_half_page_use_the_sticky_body_height() {
    let (mut explorer, _) = sticky_fixture();
    explorer.set_viewport_height(7);
    let body_height = explorer.viewport(7).body_height;
    let before = explorer.selected_visible_index().unwrap();

    explorer.scroll_nodes(1, ExplorerScrollAmount::Page);
    assert_eq!(explorer.selected_visible_index().unwrap(), before + body_height);
}

#[test]
fn sticky_alignment_places_selection_within_the_body() {
    let (mut explorer, ids) = sticky_fixture();
    explorer.set_viewport_height(7);
    explorer.select(ids.selected.clone());

    explorer.align_selected(ExplorerNodeAlignment::Top);
    let top = explorer.viewport(7);
    assert_eq!(top.rows.first().map(|row| &row.id), Some(&ids.selected));

    explorer.align_selected(ExplorerNodeAlignment::Bottom);
    let bottom = explorer.viewport(7);
    assert_eq!(bottom.rows.last().map(|row| &row.id), Some(&ids.selected));
}
```

Also extend the existing measured-viewport test so flat profile roots retain the
same expected page and alignment behavior when there are no pinned ancestors.

**Step 2: Run tests to verify behavioral failure**

Run:

```bash
cargo test --test explorer_state movement_keeps_selection_in_the_body_below_pinned_rows
cargo test --test explorer_state page_and_half_page_use_the_sticky_body_height
cargo test --test explorer_state sticky_alignment_places_selection_within_the_body
```

Expected: at least page size and alignment fail because methods still use total
`viewport_height`.

**Step 3: Centralize stabilized scroll calculation**

Replace the old fixed-height `update_scroll` calculation with helpers that use the
same viewport projection as rendering. Keep the logic in `ExplorerTreeState`, not
in `ExplorerState` or UI code.

Implement a bounded stabilization helper with this behavior:

```rust
fn body_height_at_scroll(&self, scroll: usize) -> usize;

fn stabilize_scroll_for_selection(
    &self,
    candidate: usize,
    selected_index: usize,
    alignment: Option<ExplorerNodeAlignment>,
) -> usize;
```

The helper may temporarily evaluate a supplied scroll without mutating `self`, or
delegate to a private projection function that accepts explicit `rows`,
`selected`, `scroll`, and `height`. Prefer the latter to avoid cloning the whole
state.

Bound iterations to `ancestor_count + 2`. Stop when candidate scroll and body
height no longer change. Always clamp to valid row bounds. If body height is zero,
use `selected_index` and return without subtraction.

**Step 4: Update every viewport-dependent operation**

Modify:

- `move_selection()` to update selection and call the stabilized visibility
  helper.
- `select_target(ViewTop/ViewMiddle/ViewBottom)` to select from the current normal
  body rows returned by the viewport projection.
- `scroll_nodes()` to use current `body_height` for page and half-page step.
- `align_selected()` to solve top, middle, or bottom position inside body rows.
- `ensure_selected_visible()` to reject any position covered by sticky rows.
- `set_viewport_height()` to continue storing total tree height and then stabilize
  selection visibility.

Do not change `First` and `Last` semantics.

**Step 5: Run focused model tests**

Run:

```bash
cargo test --test explorer_state viewport_scroll_keeps_selection_visible
cargo test --test explorer_state vim_targets_page_moves_and_alignment_use_the_measured_viewport
cargo test --test explorer_state movement_keeps_selection_in_the_body_below_pinned_rows
cargo test --test explorer_state page_and_half_page_use_the_sticky_body_height
cargo test --test explorer_state sticky_alignment_places_selection_within_the_body
```

Expected: all pass; existing root-only scroll expectations remain unchanged.

**Step 6: Run all Explorer state tests**

Run:

```bash
cargo test --test explorer_state
```

Expected: all tests pass.

**Step 7: Commit navigation semantics**

```bash
git add src/model/explorer.rs tests/explorer_state.rs
git commit -m "fix(explorer): account for sticky rows in navigation"
```

### Task 3: Expose a Presentation Viewport Without Formatting Offscreen Rows

**Files:**
- Modify: `src/model/workspace.rs:150-177`
- Modify: `src/model/workspace.rs:654-820`
- Test: `tests/explorer_state.rs`

**Step 1: Write a failing presentation projection test**

Add a test that calls a new `ExplorerState::viewport(height)` and verifies that
both pinned and body rows contain full presentation metadata:

```rust
#[test]
fn workspace_viewport_presents_pinned_and_body_rows() {
    let (state, ids) = sticky_workspace_fixture();
    let viewport = state.viewport(6);

    assert_eq!(viewport.pinned[0].id, ids.profile);
    assert_eq!(viewport.pinned[0].label, "primary");
    assert_eq!(viewport.pinned.last().unwrap().label, "Tables");
    assert!(viewport.rows.iter().any(|row| row.id == ids.selected));
}
```

**Step 2: Run the test to verify it fails**

Run:

```bash
cargo test --test explorer_state workspace_viewport_presents_pinned_and_body_rows
```

Expected: compilation fails because the workspace presentation viewport does not
exist.

**Step 3: Add the workspace result type**

Near `VisibleCatalogNode`, add:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisibleExplorerViewport {
    pub pinned: Vec<VisibleCatalogNode>,
    pub rows: Vec<VisibleCatalogNode>,
    pub hidden_ancestor_count: usize,
    pub body_height: usize,
}
```

**Step 4: Refactor row presentation for subsets**

Change `visible_rows()` so it can present an iterator or vector of
`VisibleExplorerNode` without requiring callers to project the entire tree.
Then implement:

```rust
pub fn viewport(&self, height: usize) -> VisibleExplorerViewport {
    let viewport = self.normalized.viewport(height);
    VisibleExplorerViewport {
        pinned: self.visible_rows(viewport.pinned),
        rows: self.visible_rows(viewport.rows),
        hidden_ancestor_count: viewport.hidden_ancestor_count,
        body_height: viewport.body_height,
    }
}
```

Retain `visible()` for search snapshots, tests, selection synchronization, and
other existing callers. Do not change `visible_search()`.

**Step 5: Run focused tests**

Run:

```bash
cargo test --test explorer_state workspace_viewport_presents_pinned_and_body_rows
cargo test --test explorer_state visible_find_
```

Expected: all pass.

**Step 6: Commit the presentation boundary**

```bash
git add src/model/workspace.rs tests/explorer_state.rs
git commit -m "refactor(explorer): expose sticky presentation viewport"
```

### Task 4: Render Sticky Rows and Register Stable Hit Regions

**Files:**
- Modify: `src/ui/mod.rs:583-738`
- Test: `tests/ui_render.rs:1147-1183`

**Step 1: Add a reusable Explorer app fixture for UI tests**

Add a local helper in `tests/ui_render.rs` that builds an `App` with one expanded
profile/database/schema/Tables path and enough table rows to force scroll. Return
the important stable IDs with the app. Reuse catalog entry constructors already
imported in the file.

Set:

```rust
app.focus = Focus::Explorer;
app.explorer.normalized.scroll = table_body_start;
app.explorer.normalized.selected = Some(selected_table.clone());
```

Use an Explorer height large enough for four pinned rows and at least two body
rows.

**Step 2: Write failing UI and hit-region tests**

Add:

```rust
#[test]
fn explorer_renders_offscreen_ancestors_as_sticky_rows() {
    let (app, ids) = sticky_explorer_app();
    let (output, state) = render_with_state(&app, 80, 24);

    for label in ["primary", "app", "public", "Tables"] {
        assert!(output.contains(label), "missing sticky {label}: {output}");
    }
    for id in [&ids.profile, &ids.database, &ids.schema, &ids.tables] {
        assert!(state.hit_regions.iter().any(|region| {
            region.target == HitTarget::ExplorerRow(id.clone())
        }));
    }
}

#[test]
fn explorer_does_not_duplicate_sticky_nodes_in_body_hit_regions() {
    let (app, ids) = sticky_explorer_app();
    let (_, state) = render_with_state(&app, 80, 24);

    let table_group_targets = state.hit_regions.iter().filter(|region| {
        region.target == HitTarget::ExplorerRow(ids.tables.clone())
    }).count();
    assert_eq!(table_group_targets, 1);
}

#[test]
fn explorer_keeps_selected_row_below_sticky_rows() {
    let (app, ids) = sticky_explorer_app();
    let (_, state) = render_with_state(&app, 80, 24);
    let pinned_bottom = explorer_row_y(&state, &ids.tables);
    let selected_y = explorer_row_y(&state, &ids.selected);

    assert!(selected_y > pinned_bottom);
}
```

**Step 3: Run tests to verify they fail**

Run:

```bash
cargo test --test ui_render explorer_renders_offscreen_ancestors_as_sticky_rows
cargo test --test ui_render explorer_does_not_duplicate_sticky_nodes_in_body_hit_regions
cargo test --test ui_render explorer_keeps_selected_row_below_sticky_rows
```

Expected: sticky labels and hit targets are absent or the selected row is placed
at the old top coordinate.

**Step 4: Extract one tree-row renderer**

Extract the body of the current `displayed.into_iter().map(...)` into a helper:

```rust
fn explorer_list_item(
    visible: &VisibleCatalogNode,
    app: &App,
    theme: Theme,
    icons: icons::IconSet,
) -> ListItem<'static>;
```

The helper must preserve all existing behavior from `src/ui/mod.rs:646-731`:

- Expansion marker.
- Group and catalog icon selection.
- Sanitized label, metadata, endpoint, and comment.
- Profile provenance and connection status spans.
- Selected background and emphasis.

Use owned strings so the returned `ListItem` has a safe lifetime.

**Step 5: Render the final viewport projection**

In normal `render_explorer()`:

1. Call `app.explorer.viewport(inner.height as usize)`.
2. Render pinned rows at `inner.y`.
3. If `hidden_ancestor_count > 0` and the projection reserved an indicator row,
   render `⋮ N ancestors` in `theme.muted` with `theme.surface` background.
4. Render normal body rows immediately below pinned rows and optional indicator.
5. Register `ExplorerRow(id)` hit regions for pinned and normal rows using their
   actual screen Y coordinate.
6. Do not register a hit region for the indicator.

If the model result needs an explicit `show_hidden_indicator: bool` to avoid
re-deriving whether a row was reserved, add that boolean to both model viewport
types and cover it in Task 1 tests.

**Step 6: Run focused UI tests**

Run:

```bash
cargo test --test ui_render explorer_
```

Expected: all Explorer UI tests pass.

**Step 7: Commit sticky rendering**

```bash
git add src/ui/mod.rs tests/ui_render.rs
git commit -m "feat(ui): render Explorer sticky ancestors"
```

### Task 5: Integrate In-tree Find and Keep Flat Search Unchanged

**Files:**
- Modify: `src/ui/mod.rs:583-601`
- Modify: `src/ui/mod.rs:740-850`
- Modify: `src/runtime.rs:2565-2572` only if viewport synchronization needs a clarified field name
- Test: `tests/explorer_state.rs:595-635`
- Test: `tests/ui_render.rs`

**Step 1: Write failing find-mode UI tests**

Add:

```rust
#[test]
fn explorer_find_renders_prompt_then_sticky_ancestors() {
    let (mut app, ids) = sticky_explorer_app();
    app.explorer.open_find();
    app.explorer.edit_find(|query| query.push_str("order"));

    let (_, state) = render_with_state(&app, 80, 24);
    let prompt_y = explorer_inner_top(&state);
    let profile_y = explorer_row_y(&state, &ids.profile);
    let selected_y = explorer_row_y(&state, &ids.selected);

    assert!(profile_y > prompt_y);
    assert!(selected_y > explorer_row_y(&state, &ids.tables));
}

#[test]
fn explorer_flat_search_does_not_render_sticky_ancestor_targets() {
    let (mut app, ids) = sticky_explorer_app();
    open_frontend_catalog_search(&mut app, "order");

    let (_, state) = render_with_state(&app, 80, 24);
    assert!(!state.hit_regions.iter().any(|region| {
        region.target == HitTarget::ExplorerRow(ids.tables.clone())
    }));
}
```

Adapt the search setup to existing helpers and assert a known result row remains
present so the test cannot pass because search rendered nothing.

**Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --test ui_render explorer_find_renders_prompt_then_sticky_ancestors
cargo test --test ui_render explorer_flat_search_does_not_render_sticky_ancestor_targets
```

Expected: find lacks sticky rows. The flat-search assertion may already pass and
serves as a regression lock.

**Step 3: Measure tree height below the find prompt**

Change `render_explorer()` viewport synchronization:

```rust
let tree_height = inner.height.saturating_sub(u16::from(app.explorer.find.is_some()));
state.explorer_viewport_rows = Some(tree_height as usize);
```

The stored model height now consistently means rows available to the tree
projection, excluding the find prompt. Normal mode still reports all inner rows.
Flat search may continue reporting its existing list height if its movement model
depends on it; keep that path explicit rather than sharing sticky projection.

**Step 4: Reuse sticky rendering below the find prompt**

Refactor normal and find tree rendering into a helper accepting a `Rect` that
contains only tree rows:

```rust
fn render_explorer_tree(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    theme: Theme,
    state: &mut UiState,
    icons: icons::IconSet,
    find: Option<&ExplorerFindState>,
);
```

Find-specific label highlighting can remain a callback or optional lookup inside
the shared row helper. Do not duplicate sticky placement or hit-region code.

`render_explorer_find()` should:

1. Render the prompt in its first row.
2. Set the editing cursor as before.
3. Pass `Rect::new(area.x, area.y + 1, area.width, area.height - 1)` to the shared
   sticky tree renderer.

**Step 5: Update find-centering expectations**

The model now receives one fewer tree row while find is open. Update the existing
`visible_find_centers_each_current_match_in_the_viewport` setup to call
`set_viewport_height()` with the actual tree height and assert matches remain
visible in `viewport.rows`, rather than asserting scroll offsets that ignore
sticky rows.

Keep exact scroll assertions for the root-only case when they remain meaningful.

**Step 6: Run find and search tests**

Run:

```bash
cargo test --test explorer_state visible_find_
cargo test --test ui_render explorer_find_
cargo test --test ui_render explorer_flat_search_
```

Expected: all pass.

**Step 7: Commit mode integration**

```bash
git add src/ui/mod.rs src/runtime.rs tests/explorer_state.rs tests/ui_render.rs
git commit -m "feat(explorer): keep sticky context during find"
```

Omit `src/runtime.rs` from staging if no source change was necessary.

### Task 6: Verify Full Mouse Interaction and State Changes

**Files:**
- Modify: `tests/mouse.rs`
- Modify: `tests/ui_render.rs`
- Modify: `src/input/mouse.rs` only if tests reveal a real target-mapping defect

**Step 1: Write sticky-row mouse tests**

Render the sticky Explorer fixture to a `UiState`, retrieve the sticky schema and
group hit regions, and feed their coordinates to `map_mouse()`.

Add:

```rust
#[test]
fn clicking_a_sticky_ancestor_selects_its_stable_id() {
    let (app, state, ids) = rendered_sticky_explorer();
    let area = explorer_hit_area(&state, &ids.schema);
    let action = map_mouse(left_down(area.x, area.y), &state, &app);

    assert_eq!(action, Some(Action::ExplorerSelect(ids.schema)));
}

#[test]
fn double_clicking_a_sticky_ancestor_uses_primary_action() {
    let (app, state, ids) = rendered_sticky_explorer();
    let area = explorer_hit_area(&state, &ids.tables);

    assert_eq!(
        map_mouse(left_down(area.x, area.y), &state, &app),
        Some(Action::ExplorerSelect(ids.tables.clone()))
    );
    assert_eq!(
        map_mouse(left_down(area.x, area.y), &state, &app),
        Some(Action::ExplorerPrimary)
    );
}
```

Follow the existing mouse test clock/click tracker conventions if direct repeated
events are timing-sensitive.

**Step 2: Run tests to verify behavior**

Run:

```bash
cargo test --test mouse clicking_a_sticky_ancestor_selects_its_stable_id
cargo test --test mouse double_clicking_a_sticky_ancestor_uses_primary_action
```

Expected: tests should pass once UI hit regions are correct. If they fail, fix the
smallest hit-target issue; do not add a sticky-specific action.

**Step 3: Add a collapse/reprojection UI test**

Add a test that selects the sticky group, executes the existing collapse action
through `App::update`, rerenders, and verifies descendant hit targets are gone and
selection follows existing fallback behavior.

```rust
#[test]
fn collapsing_a_sticky_ancestor_removes_descendant_targets() {
    let (mut app, ids) = sticky_explorer_app();
    app.update(Action::ExplorerSelect(ids.tables.clone()));
    app.update(Action::ExplorerCollapse);

    let (_, state) = render_with_state(&app, 80, 24);
    assert!(!state.hit_regions.iter().any(|region| {
        region.target == HitTarget::ExplorerRow(ids.selected.clone())
    }));
}
```

Use the actual collapse action name from `src/action.rs` if it differs.

**Step 4: Run mouse and UI tests**

Run:

```bash
cargo test --test mouse
cargo test --test ui_render explorer_
```

Expected: all pass.

**Step 5: Commit interaction coverage**

```bash
git add tests/mouse.rs tests/ui_render.rs src/input/mouse.rs
git commit -m "test(explorer): cover sticky ancestor interactions"
```

Omit `src/input/mouse.rs` if it was unchanged.

### Task 7: Performance and Full Regression Verification

**Files:**
- Modify: `tests/explorer_state.rs`
- Modify: `docs/plans/2026-08-28-explorer-sticky-ancestors-design.md` only if implementation details required an approved clarification

**Step 1: Add a projection performance regression test**

Extend the existing large Explorer fixture or performance test with 10,000 table
rows under one group. Select a late table, set a realistic viewport, and call the
sticky viewport projection.

Prefer a structural assertion over a fragile timing threshold:

```rust
#[test]
fn sticky_projection_visits_only_one_selected_ancestor_chain() {
    let mut explorer = large_explorer(10_000);
    explorer.select(last_table_id(&explorer));
    explorer.set_viewport_height(30);

    let (viewport, metrics) = explorer.viewport_with_visit_count(30);

    assert_eq!(viewport.pinned.len(), 4);
    assert!(viewport.rows.len() <= 30);
    assert!(metrics.ancestor_visits <= 8);
}
```

If adding a public metrics method would pollute production API, put a
`#[doc(hidden)]` test helper beside existing `visible_with_visit_count()` or assert
against that existing visit count plus final viewport sizes. Do not use a
machine-specific millisecond limit.

**Step 2: Run the performance-focused test**

Run:

```bash
cargo test --test explorer_state sticky_projection_visits_only_one_selected_ancestor_chain
```

Expected: pass with ancestor work bounded by tree depth.

**Step 3: Format and lint**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: both commands exit successfully. If formatting fails, run `cargo fmt`,
then rerun `cargo fmt --check`.

**Step 4: Run focused suites**

Run:

```bash
cargo test --test explorer_state
cargo test --test ui_render
cargo test --test mouse
```

Expected: all tests pass.

**Step 5: Run the full test suite**

Run:

```bash
cargo test --all-targets --all-features
```

Expected: all tests pass.

**Step 6: Inspect final changes**

Run:

```bash
git status --short
git diff --check
git diff -- src/model/explorer.rs src/model/workspace.rs src/ui/mod.rs src/input/mouse.rs tests/explorer_state.rs tests/ui_render.rs tests/mouse.rs docs/plans/2026-08-28-explorer-sticky-ancestors-design.md docs/plans/2026-08-28-explorer-sticky-ancestors-implementation.md
```

Expected: no whitespace errors; only intended sticky-ancestor changes are in the
reviewed diff. Do not revert unrelated worktree changes.

**Step 7: Commit final hardening**

```bash
git add src/model/explorer.rs src/model/workspace.rs src/ui/mod.rs tests/explorer_state.rs tests/ui_render.rs tests/mouse.rs docs/plans/2026-08-28-explorer-sticky-ancestors-design.md docs/plans/2026-08-28-explorer-sticky-ancestors-implementation.md
git commit -m "test(explorer): verify sticky ancestor projection"
```

Include `src/input/mouse.rs` or `src/runtime.rs` only if they contain intentional
changes. Before committing, ensure earlier task commits did not already include
the two plan files; never stage unrelated files.
