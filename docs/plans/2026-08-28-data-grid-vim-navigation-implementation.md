# Data Grid Vim Navigation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a fixed one-based row-number gutter and Vim-style vertical navigation to the shared SQL/relation Data Grid.

**Architecture:** Keep key handling semantic and geometry-free. Put bounded vertical-navigation calculations on shared `DataGridState`, route semantic actions through App, and retain the renderer/runtime viewport snapshot as the authority on actual terminal geometry. Render row numbers as a fixed presentation gutter so database column indexes, editing, resizing, and horizontal scrolling remain unchanged.

**Tech Stack:** Rust 2024, Ratatui 0.30, Crossterm, existing LazyDB App/Runtime/UiState architecture

---

## Preconditions And Invariants

- Work in `/Users/yelog/workspace/tui/lazydb` unless the user supplies a separate worktree.
- Follow `docs/plans/2026-08-28-data-grid-vim-navigation-design.md`.
- Preserve the existing `DataGridViewport` render-to-App synchronization and stale-tab UUID guard.
- Preserve minimal scrolling for ordinary `h/j/k/l` movement.
- Keep SQL DATA and relation DATA behavior identical through the shared Grid path.
- Do not add a vertical scrollbar or blank overscroll.
- Do not put row numbers in `ResultSet`, editable rows, or column-width overrides.
- Do not commit unless the user explicitly requests it. Any implementation workflow that normally commits after each task must skip those commit steps.

### Task 1: Add Pure Vertical Navigation To DataGridState

**Files:**
- Modify: `src/model/tab.rs:21-37,181-end`
- Test: `src/model/tab.rs`

**Step 1: Write failing tests for absolute and viewport row selection**

Add table-driven tests covering `First`, `Last`, `ViewTop`, `ViewMiddle`, and
`ViewBottom`. Use a state whose synchronized viewport starts at row 4 and shows
five rows:

```rust
#[test]
fn selecting_semantic_rows_uses_absolute_and_visible_bounds() {
    let base = DataGridState {
        selected_row: 6,
        row_offset: 4,
        viewport_rows: 5,
        ..DataGridState::default()
    };

    let cases = [
        (GridRowTarget::First, 0, 0),
        (GridRowTarget::Last, 11, 7),
        (GridRowTarget::ViewTop, 4, 4),
        (GridRowTarget::ViewMiddle, 6, 4),
        (GridRowTarget::ViewBottom, 8, 4),
    ];

    for (target, selected, offset) in cases {
        let mut state = base.clone();
        state.select_row_target(target, 12);
        assert_eq!((state.selected_row, state.row_offset), (selected, offset));
    }
}
```

Add a partial-final-page case such as `row_count = 10`, `row_offset = 7`, and
`viewport_rows = 5`; `ViewBottom` must select row 9 and `ViewMiddle` must select
row 8. Add zero-row assertions proving all states stay at zero.

**Step 2: Run the focused tests and verify failure**

Run:

```bash
cargo test model::tab::tests --lib
```

Expected: FAIL because `GridRowTarget` and `select_row_target` do not exist.

**Step 3: Define semantic navigation types beside DataGridState**

Add copyable, comparable model enums:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GridRowTarget {
    First,
    Last,
    ViewTop,
    ViewMiddle,
    ViewBottom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GridScrollAmount {
    HalfPage,
    Page,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GridRowAlignment {
    Top,
    Middle,
    Bottom,
}
```

Keep these in `model::tab`; they describe shared Grid state behavior and avoid a
model-to-Action dependency.

**Step 4: Implement absolute and visible target selection**

Add a method equivalent to:

```rust
pub fn select_row_target(&mut self, target: GridRowTarget, row_count: usize) {
    if row_count == 0 {
        self.selected_row = 0;
        self.row_offset = 0;
        return;
    }

    match target {
        GridRowTarget::First => {
            self.selected_row = 0;
            self.row_offset = 0;
        }
        GridRowTarget::Last => {
            self.selected_row = row_count - 1;
            self.ensure_row_visible(row_count);
        }
        GridRowTarget::ViewTop
        | GridRowTarget::ViewMiddle
        | GridRowTarget::ViewBottom => {
            if self.viewport_rows == 0 {
                return;
            }
            let first = self.row_offset.min(row_count - 1);
            let last = first
                .saturating_add(self.viewport_rows.saturating_sub(1))
                .min(row_count - 1);
            self.selected_row = match target {
                GridRowTarget::ViewTop => first,
                GridRowTarget::ViewMiddle => first + (last - first) / 2,
                GridRowTarget::ViewBottom => last,
                GridRowTarget::First | GridRowTarget::Last => unreachable!(),
            };
        }
    }
}
```

Do not mutate horizontal fields.

**Step 5: Run the focused tests**

Run: `cargo test model::tab::tests --lib`

Expected: PASS.

**Step 6: Write failing half-page and full-page movement tests**

Add tests with 20 rows, five visible rows, selection 6, and offset 4:

```rust
#[test]
fn page_scroll_moves_selection_and_viewport_together() {
    let mut state = DataGridState {
        selected_row: 6,
        row_offset: 4,
        viewport_rows: 5,
        ..DataGridState::default()
    };

    state.scroll_rows(1, GridScrollAmount::HalfPage, 20);
    assert_eq!((state.selected_row, state.row_offset), (8, 6));

    state.scroll_rows(-1, GridScrollAmount::Page, 20);
    assert_eq!((state.selected_row, state.row_offset), (3, 1));
}
```

Add boundary cases proving downward movement clamps selection to 19 and offset
to 15, upward movement clamps both to zero, and repeated movement at a final
boundary is a no-op. Add a one-row viewport test proving half-page movement uses
a minimum step of one.

**Step 7: Run the tests and verify failure**

Run: `cargo test model::tab::tests --lib`

Expected: FAIL because `scroll_rows` does not exist.

**Step 8: Implement page movement**

Add a small private helper for the legal maximum row offset and implement:

```rust
fn max_row_offset(&self, row_count: usize) -> usize {
    row_count.saturating_sub(self.viewport_rows.min(row_count))
}

pub fn scroll_rows(
    &mut self,
    direction: isize,
    amount: GridScrollAmount,
    row_count: usize,
) {
    if row_count == 0 {
        self.selected_row = 0;
        self.row_offset = 0;
        return;
    }
    if self.viewport_rows == 0 || direction == 0 {
        return;
    }

    let step = match amount {
        GridScrollAmount::HalfPage => (self.viewport_rows / 2).max(1),
        GridScrollAmount::Page => self.viewport_rows.max(1),
    };
    let delta = if direction.is_negative() {
        -(step.min(isize::MAX as usize) as isize)
    } else {
        step.min(isize::MAX as usize) as isize
    };

    self.selected_row = move_bounded_index(self.selected_row, delta, row_count);
    self.row_offset = move_bounded_index(
        self.row_offset,
        delta,
        self.max_row_offset(row_count).saturating_add(1),
    );
    self.ensure_row_visible(row_count);
}
```

Implement `move_bounded_index` privately in `model/tab.rs`, or reuse an existing
model-safe helper if exploration finds one without creating a dependency on
`app.rs`. The helper must handle `count == 0`, signed underflow, and overflow.

**Step 9: Write failing selected-row alignment tests**

Cover all alignments with selection 10, five visible rows, and 30 rows:

```rust
#[test]
fn selected_row_can_be_aligned_within_the_viewport() {
    let cases = [
        (GridRowAlignment::Top, 10),
        (GridRowAlignment::Middle, 8),
        (GridRowAlignment::Bottom, 6),
    ];

    for (alignment, offset) in cases {
        let mut state = DataGridState {
            selected_row: 10,
            row_offset: 0,
            viewport_rows: 5,
            ..DataGridState::default()
        };
        state.align_selected_row(alignment, 30);
        assert_eq!(state.selected_row, 10);
        assert_eq!(state.row_offset, offset);
    }
}
```

Add first/last-row clamping, zero-row, and unknown-viewport cases.

**Step 10: Implement alignment and run model tests**

Implement the design formula using `saturating_sub`, clamp with
`max_row_offset`, then run:

```bash
cargo test model::tab::tests --lib
```

Expected: PASS.

### Task 2: Route Semantic Navigation Through Action And App

**Files:**
- Modify: `src/action.rs:1-20,440-455`
- Modify: `src/app.rs:530-620,2480-2500,5057-5125`
- Test: `src/app.rs`
- Test: `tests/workspace_tabs.rs`
- Test: `tests/relation_tabs.rs`

**Step 1: Write failing App-level parity tests**

Create equivalent SQL DATA and relation DATA fixtures with at least 20 rows.
Synchronize both to `row_offset = 4`, `visible_rows = 5`, and select row 6. Apply
the planned semantic actions and assert equal `(selected_row, row_offset)` after
each operation.

At minimum cover:

```text
ViewMiddle       -> (6, 4)
HalfPage down    -> (8, 6)
Align Bottom     -> (8, 4)
Last             -> (19, 15)
First            -> (0, 0)
```

Also assert `selected_column`, `column_offset`, and `column_widths` remain
unchanged.

**Step 2: Run focused tests and verify failure**

Run:

```bash
cargo test grid_vim_navigation --lib
cargo test --test workspace_tabs grid_vim_navigation
cargo test --test relation_tabs grid_vim_navigation
```

Expected: FAIL because the Action variants do not exist.

**Step 3: Add semantic Action variants**

Import or fully qualify the new model enums and add:

```rust
GridSelectRow(crate::model::tab::GridRowTarget),
GridScrollRows {
    direction: isize,
    amount: crate::model::tab::GridScrollAmount,
},
GridAlignSelectedRow(crate::model::tab::GridRowAlignment),
```

Keep `GridViewportChanged` and existing mouse/resize actions unchanged.

**Step 4: Permit the actions on active relation tabs**

Extend the active-relation and active-console action guard lists near
`App::update` with all three variants. Without this, relation tabs with no SQL
console would reject the new actions before reaching the reducer.

**Step 5: Route actions through the existing active-grid boundary**

Add reducer arms equivalent to:

```rust
Action::GridSelectRow(target) => {
    self.with_active_grid(|grid, (row_count, _)| {
        grid.select_row_target(target, row_count);
    });
    Vec::new()
}
Action::GridScrollRows { direction, amount } => {
    self.with_active_grid(|grid, (row_count, _)| {
        grid.scroll_rows(direction, amount, row_count);
    });
    Vec::new()
}
Action::GridAlignSelectedRow(alignment) => {
    self.with_active_grid(|grid, (row_count, _)| {
        grid.align_selected_row(alignment, row_count);
    });
    Vec::new()
}
```

Do not duplicate row arithmetic in App. Do not change `move_grid`.

**Step 6: Run App and cross-surface tests**

Run:

```bash
cargo test grid_vim_navigation --lib
cargo test --test workspace_tabs grid_vim_navigation
cargo test --test relation_tabs grid_vim_navigation
```

Expected: PASS.

### Task 3: Add Direct And Multi-Key Bindings

**Files:**
- Modify: `src/input/keymap.rs:16-30,204-235,254-288,315-400,726-762`
- Test: `src/input/keymap.rs:846-end`

**Step 1: Write failing direct-key tests**

Use a SQL DATA app focused on Results and assert exact actions for:

```rust
assert_eq!(keymap.map(key(KeyCode::Char('G')), &app),
    Some(Action::GridSelectRow(GridRowTarget::Last)));
assert_eq!(keymap.map(key(KeyCode::Char('H')), &app),
    Some(Action::GridSelectRow(GridRowTarget::ViewTop)));
assert_eq!(keymap.map(key(KeyCode::Char('M')), &app),
    Some(Action::GridSelectRow(GridRowTarget::ViewMiddle)));
assert_eq!(keymap.map(key(KeyCode::Char('L')), &app),
    Some(Action::GridSelectRow(GridRowTarget::ViewBottom)));
```

Add exact control-key assertions for `Ctrl-d/u/f/b`. Confirm the same keys do
not emit Grid actions while Editor has focus.

**Step 2: Write failing sequence tests**

Assert:

- first `g` returns `None`, second `g` returns `First`;
- first `z` returns `None`, then `z/t/b` returns the three alignments;
- a different tab UUID or focus invalidates the pending sequence;
- an expired sequence does not execute;
- an invalid second Grid-prefix key is consumed and clears pending state;
- a fresh following key is handled normally.

Tests in the same module may replace the pending timestamp with
`Instant::now() - SEQUENCE_TIMEOUT - Duration::from_millis(1)` rather than
sleeping.

**Step 3: Run keymap tests and verify failure**

Run: `cargo test input::keymap::tests --lib`

Expected: FAIL because Grid prefix states and mappings do not exist.

**Step 4: Extend Pending and start prefixes only for navigable Grids**

Add:

```rust
GridGoto,
GridAlign,
```

Introduce a focused predicate such as `is_grid_navigation_focus(app)` that is
true only for:

- SQL tabs with Results focus and `ResultView::Data`;
- relation tabs with Results focus, `RelationView::Data`, and a Grid mode that
  permits browse navigation.

Follow existing relation mode behavior exactly: cell editing and Busy must not
start Grid sequences; preserve VisualLine behavior if existing `j/k` movement is
available there.

Before the general non-editor mapping, start `GridGoto` on unmodified `g` and
`GridAlign` on unmodified `z` when the predicate is true.

**Step 5: Map pending Grid sequences**

Extend `map_pending`:

```rust
(Pending::GridGoto, KeyCode::Char('g')) => {
    Some(Action::GridSelectRow(GridRowTarget::First))
}
(Pending::GridAlign, KeyCode::Char('z')) => {
    Some(Action::GridAlignSelectedRow(GridRowAlignment::Middle))
}
(Pending::GridAlign, KeyCode::Char('t')) => {
    Some(Action::GridAlignSelectedRow(GridRowAlignment::Top))
}
(Pending::GridAlign, KeyCode::Char('b')) => {
    Some(Action::GridAlignSelectedRow(GridRowAlignment::Bottom))
}
```

When a valid-context pending Grid prefix receives an invalid second key, consume
that key and clear the prefix. Preserve existing invalid-key behavior for Leader,
Window, Previous/Next, and relation mutation prefixes unless tests demonstrate a
shared contract that should change.

**Step 6: Map direct uppercase and control keys**

Add `G/H/M/L` to `map_results`. In the control-key branch, before the fallback
`_ => None`, map only when `is_grid_navigation_focus(app)`:

```rust
KeyCode::Char('d') => GridScrollRows { direction: 1, HalfPage }
KeyCode::Char('u') => GridScrollRows { direction: -1, HalfPage }
KeyCode::Char('f') => GridScrollRows { direction: 1, Page }
KeyCode::Char('b') => GridScrollRows { direction: -1, Page }
```

Require `event.modifiers == KeyModifiers::CONTROL` for these commands rather
than accepting unrelated modifier combinations.

**Step 7: Run keymap and relation editing tests**

Run:

```bash
cargo test input::keymap::tests --lib
cargo test relation_data --lib
cargo test --test relation_tabs
```

Expected: PASS, including existing edit-cell, visual-line, delete/yank, undo, and
redo bindings.

### Task 4: Render A Fixed Row-Number Gutter

**Files:**
- Modify: `src/ui/data_grid.rs:19-184,206-310,370-444`
- Test: `src/ui/data_grid.rs`
- Test: `tests/ui_render.rs`
- Test: `tests/mouse.rs`

**Step 1: Add failing pure geometry tests**

Extract small helpers and test them independently:

```rust
#[test]
fn row_number_width_tracks_absolute_result_size() {
    assert_eq!(row_number_width(0), 3);
    assert_eq!(row_number_width(9), 3);
    assert_eq!(row_number_width(10), 4);
    assert_eq!(row_number_width(500), 5);
}

#[test]
fn first_data_cell_follows_the_fixed_gutter() {
    assert_eq!(selected_data_cell(0), 2);
    assert_eq!(selected_data_cell(1), 4);
}
```

The expected width is decimal digits plus two padding cells. A zero-row result
still reserves one digit.

**Step 2: Add failing render tests for header and absolute indexes**

In `tests/ui_render.rs`, render a Grid with at least 12 rows and assert the
buffer contains:

- `#` in the fixed header;
- `1` on the first page;
- after synchronizing `row_offset` to 9, absolute numbers `10`, `11`, and `12`,
  not screen-relative `1`, `2`, and `3`.

Render enough columns to overflow horizontally, change `column_offset`, and
assert `#` and the same row numbers remain visible.

**Step 3: Add failing mouse geometry tests**

Update/add `tests/mouse.rs` cases proving:

- clicking the first rendered data column still emits `GridSelect { column: 0 }`;
- clicking row-number cells emits no cell-selection action;
- the first data-column resize boundary remains attached to data column zero;
- horizontal scrollbar paging/dragging starts after the fixed gutter.

Do not update expected coordinates blindly. Derive them from rendered hit
regions where possible, then assert target identity and non-overlap with the
gutter.

**Step 4: Run focused tests and verify failure**

Run:

```bash
cargo test ui::data_grid::tests --lib
cargo test --test ui_render row_number -- --nocapture
cargo test --test mouse grid -- --nocapture
```

Expected: FAIL because the gutter is not rendered and existing hit regions
start where it must be placed.

**Step 5: Implement row-number width and Grid constraints**

Add helpers equivalent to:

```rust
fn row_number_width(row_count: usize) -> u16 {
    row_count.max(1).to_string().len().saturating_add(2) as u16
}

fn selected_data_cell(visible_position: usize) -> usize {
    2 + visible_position.saturating_mul(2)
}
```

Change `grid_constraints` to prepend:

```rust
Constraint::Length(number_width)
Constraint::Length(1)
```

then append visible data columns and their separators. Keep original database
column indexes in `visible`.

**Step 6: Reserve fixed horizontal geometry before data-column layout**

In `render`, calculate:

```text
number_width = row_number_width(result.rows.len())
fixed_width  = number_width + 1
available    = area.width - 4 - fixed_width
```

Use saturating arithmetic and `.max(1)` for data `available`. Use this reduced
width consistently in `total_width`, `viewport_start`, `visible_columns`, and
`last_page_start` calls.

Do not reduce vertical `visible_rows`; the gutter adds width only.

**Step 7: Prepend header and body gutter cells**

Update header generation to produce:

```text
[# cell, separator cell, visible data cells...]
```

Update body generation to receive the absolute `row_index` and prepend
`row_index + 1`. Right-align the number within its padded width using Ratatui
cell content or explicit spaces, and sanitize only database-provided text.

Give the gutter a muted/header style. Let Ratatui's row highlight apply when the
row is selected, but keep selected-cell highlight on real data columns only.

Replace current selected-cell calculation with:

```rust
let selected_column = visible
    .iter()
    .position(|index| *index == grid.selected_column)
    .map_or(selected_data_cell(0), selected_data_cell);
```

**Step 8: Shift hit regions, header rule, and scrollbar**

Start data hit-region `x` at:

```text
area.x + 2 + number_width + 1
```

Start resize-boundary accumulation at the same point. Do not create hit regions
for the gutter or its separator.

Prepend the gutter segment and a `┼` to `render_header_rule`. Pass fixed gutter
geometry to `render_scrollbar` and set its track to the scrollable data area,
not the complete inner table area. Keep scrollbar offset math based only on
actual database columns.

**Step 9: Run formatter and focused Grid tests**

Run:

```bash
cargo fmt --all
cargo test ui::data_grid::tests --lib
cargo test --test ui_render row_number -- --nocapture
cargo test --test ui_render grid_viewport -- --nocapture
cargo test --test mouse grid -- --nocapture
```

Expected: PASS. Existing viewport capacity must remain unchanged except where a
horizontal scrollbar appears because the fixed gutter leaves less data width.

### Task 5: Expose Commands Through Contextual Help

**Files:**
- Modify: `src/help.rs:3-49,290-358,377-end`
- Modify: `src/app.rs:430-520`
- Test: `src/help.rs`
- Test: `src/app.rs`

**Step 1: Write failing help-contract tests**

Assert Results help contains independent searchable rows for:

```text
gg
G
H / M / L
Ctrl-d / Ctrl-u
Ctrl-f / Ctrl-b
zz / zt / zb
```

It is acceptable to group symmetric keys in one help row only when selecting
that row has one unambiguous executable action. Since opposite directions have
different actions, use independent IDs for down/up and forward/back.

Add App tests that execute each new help ID and assert it dispatches the same
semantic Action as its keyboard equivalent.

**Step 2: Run focused tests and verify failure**

Run:

```bash
cargo test help::tests --lib
cargo test help_shortcut --lib
```

Expected: FAIL because IDs and entries do not exist.

**Step 3: Add Results help IDs and entries**

Add descriptive IDs such as:

```text
ResultsFirstRow
ResultsLastRow
ResultsViewTop
ResultsViewMiddle
ResultsViewBottom
ResultsHalfPageDown
ResultsHalfPageUp
ResultsPageDown
ResultsPageUp
ResultsAlignMiddle
ResultsAlignTop
ResultsAlignBottom
```

Add concise Results entries with exact key labels and searchable descriptions.
Keep existing movement, toggle, and relation-specific entries.

**Step 4: Map help execution to semantic actions**

Extend `App::execute_help_shortcut` with exactly the same Action variants used by
the keymap. Do not emulate multi-key input for `gg` or `z` commands; the help
palette should dispatch the final semantic Action directly.

**Step 5: Run help and App tests**

Run:

```bash
cargo test help::tests --lib
cargo test help_shortcut --lib
```

Expected: PASS.

### Task 6: Document The Keyboard Contract

**Files:**
- Modify: `docs/keybindings.md:146-162`

**Step 1: Update Results keybindings**

Extend the Results table with:

```markdown
| `gg`, `G` | Select first/last row |
| `H`, `M`, `L` | Select top/middle/bottom visible row |
| `Ctrl-d`, `Ctrl-u` | Move down/up half a page |
| `Ctrl-f`, `Ctrl-b` | Move down/up one page |
| `zz`, `zt`, `zb` | Align selected row to middle/top/bottom |
```

Add one sentence explaining that page movement moves selection with the viewport
and row numbers are fixed, one-based, and unaffected by horizontal scrolling.

**Step 2: Check documentation formatting**

Run: `git diff --check -- docs/keybindings.md docs/plans/2026-08-28-data-grid-vim-navigation-design.md docs/plans/2026-08-28-data-grid-vim-navigation-implementation.md`

Expected: no output.

### Task 7: Cross-Surface Regression And Full Verification

**Files:**
- Verify all modified files
- Test: `src/model/tab.rs`
- Test: `src/input/keymap.rs`
- Test: `src/app.rs`
- Test: `tests/ui_render.rs`
- Test: `tests/mouse.rs`
- Test: `tests/workspace_tabs.rs`
- Test: `tests/relation_tabs.rs`

**Step 1: Run focused unit suites**

Run:

```bash
cargo test model::tab::tests --lib
cargo test input::keymap::tests --lib
cargo test help::tests --lib
cargo test ui::data_grid::tests --lib
cargo test grid_vim_navigation --lib
```

Expected: PASS.

**Step 2: Run focused integration suites**

Run:

```bash
cargo test --test ui_render
cargo test --test mouse
cargo test --test workspace_tabs
cargo test --test relation_tabs
```

Expected: PASS.

**Step 3: Format and lint**

Run:

```bash
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --all-features --all-targets -- -D warnings
```

Expected: PASS with no warnings.

**Step 4: Run the complete suite**

Run:

```bash
cargo test --all-features --all-targets
```

Expected: PASS.

**Step 5: Inspect repository hygiene**

Run:

```bash
git diff --check
git status --short --branch
git diff -- src/action.rs src/model/tab.rs src/input/keymap.rs src/app.rs src/ui/data_grid.rs src/help.rs docs/keybindings.md tests/ui_render.rs tests/mouse.rs tests/workspace_tabs.rs tests/relation_tabs.rs
```

Expected: no whitespace errors and no unrelated files modified by this work.
Preserve any pre-existing user changes.

**Step 6: Perform manual TUI acceptance**

In both a SQL result and relation DATA preview with more rows and columns than
fit on screen:

1. Verify the fixed `#` gutter shows absolute one-based row numbers.
2. Scroll horizontally and verify the gutter does not move.
3. Use `gg` and `G` and verify selection reaches the first and last rows.
4. On a middle page, verify `H`, `M`, and `L` select visible top, middle, and
   bottom rows without scrolling.
5. Verify `Ctrl-d/u` move selection and viewport by half a page.
6. Verify `Ctrl-f/b` move selection and viewport by a full page.
7. Verify `zz/zt/zb` align the current row and clamp naturally near result ends.
8. Resize the terminal and repeat viewport-relative commands.
9. Enter relation cell-edit mode and verify printable navigation-prefix keys are
   not stolen from cell editing.
10. Click data cells and resize columns around the fixed gutter; verify targets
    remain correct.

**Step 7: Report verification limits**

In the completion summary, list every command actually run and its result. State
whether SQL and relation manual checks, terminal resizing, horizontal scrolling,
cell editing, mouse selection, and column resizing were exercised. Do not claim
manual checks that were not performed.
