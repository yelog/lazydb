# Record View Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a read-only, keyboard-first Record View overlay for the selected row in SQL Results and relation Data grids.

**Architecture:** Add lightweight overlay state that stores only field scrolling, while the active shared Grid remains the source of the selected row and current displayed data. Route semantic Record View actions through App, render current values without cloning rows, and give the overlay input priority over normal Results bindings.

**Tech Stack:** Rust 2024, Ratatui 0.30, Crossterm, existing LazyDB App/Keymap/Overlay/DataGrid architecture

---

## Preconditions And Invariants

- Follow `docs/plans/2026-08-28-record-view-design.md`.
- Preserve identical behavior for SQL Data and relation Data through the shared
  Grid path.
- Keep Record View read-only and free of database commands.
- Do not clone `ResultSet`, rows, or `CellValue` into overlay state.
- Preserve unrelated working-tree changes, including current adapter/runtime
  edits.
- Do not commit unless the user explicitly requests it.

### Task 1: Add Record View State And Pure Field Scrolling

**Files:**
- Create: `src/model/record_view.rs`
- Modify: `src/model/mod.rs`
- Test: `src/model/record_view.rs`

**Step 1: Write failing model tests**

Add tests for a default offset of zero, bounded one-line movement, first/last
jumps, changing from a long result to a shorter one, zero columns, and visible
capacities of zero and one.

Use behavior equivalent to:

```rust
#[test]
fn field_navigation_clamps_to_the_visible_range() {
    let mut state = RecordViewState::default();
    state.move_fields(4, 10, 3);
    assert_eq!(state.field_offset, 4);

    state.move_fields(20, 10, 3);
    assert_eq!(state.field_offset, 7);

    state.move_fields(-20, 10, 3);
    assert_eq!(state.field_offset, 0);
}
```

Add `jump_first()` and `jump_last(field_count, viewport_rows)` expectations.

**Step 2: Run the focused test and verify failure**

Run:

```bash
cargo test model::record_view::tests --lib
```

Expected: FAIL because the module and `RecordViewState` do not exist.

**Step 3: Implement minimal pure state**

Create:

```rust
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecordViewState {
    pub field_offset: usize,
}
```

Implement bounded `move_fields`, `jump_first`, `jump_last`, and `clamp` methods.
Calculate the maximum offset as
`field_count.saturating_sub(viewport_rows.min(field_count))`; use saturating
arithmetic and make unknown/zero viewport capacity a safe no-op except that zero
fields resets the offset.

**Step 4: Export the module and run the focused test**

Run: `cargo test model::record_view::tests --lib`

Expected: PASS.

### Task 2: Add Overlay And Semantic Actions

**Files:**
- Modify: `src/model/workspace.rs:51-87`
- Modify: `src/action.rs:460-502`
- Modify: `src/app.rs:5504-5568` and the `App::update` action match near `2851`
- Test: `src/app.rs` test module near existing Grid tests around `7395`

**Step 1: Write failing App tests for opening boundaries**

Build SQL-result fixtures and assert:

- `OpenRecordView` installs `Overlay::RecordView(Default::default())` for a Data
  result with at least one row and one column;
- it is a no-op for zero rows, zero columns, Output/Plan, and non-Data relation
  views;
- it preserves `Focus::Results` and emits no `Command`.

Add an equivalent ready relation Data fixture.

**Step 2: Run focused tests and verify failure**

Run:

```bash
cargo test app::tests::record_view --lib
```

Expected: FAIL because the overlay and actions do not exist.

**Step 3: Define model and actions**

Add:

```rust
Overlay::RecordView(crate::model::record_view::RecordViewState)
```

Add semantic actions:

```rust
OpenRecordView,
RecordViewMoveFields(isize),
RecordViewJumpFirstField,
RecordViewJumpLastField,
RecordViewMoveRow(isize),
CloseRecordView,
```

Keep these separate from generic `DismissOverlay` so movement and cleanup remain
specific and testable.

**Step 4: Add a zero-copy active-record accessor**

Add a crate-visible view type containing references and indexes, equivalent to:

```rust
pub(crate) struct ActiveRecord<'a> {
    pub columns: &'a [ColumnMeta],
    pub values: &'a [CellValue],
    pub row_index: usize,
    pub row_count: usize,
}
```

Implement `App::active_record()` using the same source-selection rules as the
Grid renderer and `active_grid_dimensions`:

- SQL Data chooses displayed derived outcome before base outcome and then the
  last result set;
- relation Data chooses current retained snapshot metadata;
- if a relation edit session exists, values come from the current editable
  row's `current` values;
- otherwise values come from the current `ResultSet` row;
- selection is bounded and missing/empty data returns `None`.

If necessary, extract a borrowed `relation_result_ref` helper instead of using
the existing cloning `relation_result`; do not broaden unrelated query logic.

**Step 5: Implement opening and closing reducers**

`OpenRecordView` validates `active_record().is_some()` before installing the
overlay. `CloseRecordView` removes only a Record View overlay. Both return no
runtime commands.

**Step 6: Run focused tests**

Run: `cargo test app::tests::record_view --lib`

Expected: opening tests PASS.

**Step 7: Write failing row-movement tests**

Open a multi-row record and assert `RecordViewMoveRow(1/-1)`:

- clamps rather than wraps;
- updates the active Grid selected row;
- calls existing row visibility behavior;
- leaves selected column, column offset, and widths unchanged;
- resets `field_offset` to zero;
- works for SQL Data and relation Data;
- is a no-op when data disappears while the overlay remains open.

**Step 8: Implement Record View movement reducers**

Route row movement through `with_active_grid` and existing bounded Grid movement
logic. Route field actions only when `Overlay::RecordView` is active. Use the
current column count and a render-synchronized field viewport capacity added in
Task 4; until that capacity is known, movement is a safe no-op.

Avoid borrowing `self.overlay` and the active tab at the same time by calculating
dimensions/capacity before taking mutable references.

**Step 9: Run focused App tests**

Run: `cargo test app::tests::record_view --lib`

Expected: PASS.

### Task 3: Add Record View Key Routing

**Files:**
- Modify: `src/input/keymap.rs:50-285,576-625,1063-1127`
- Test: `tests/keymap.rs` near existing Results tests around `604`

**Step 1: Write failing keymap tests**

Cover:

- `v` maps to `OpenRecordView` from SQL Data and relation Data;
- `v` does not open on SQL Output/Plan, relation DDL, empty rows, or zero columns;
- while Record View is open, `j/k`, arrows, `h/l`, arrows, `Home`, `End`, `G`,
  `Esc`, `q`, and `v` map to Record View actions;
- ordinary Grid/relation actions do not leak through the overlay;
- `gg` maps to first field;
- invalid/expired `g` prefixes produce no action and clear pending state.

**Step 2: Run focused tests and verify failure**

Run:

```bash
cargo test --test keymap record_view
```

Expected: FAIL because Record View mappings do not exist.

**Step 3: Add the opening binding**

In `map_results`, map `v` to `OpenRecordView` only when App reports an active
record. Reuse one App capability method rather than duplicating SQL/relation
result checks in the keymap.

**Step 4: Add overlay-priority routing**

Add a Record View branch alongside the specialized Help/Profile/confirmation
branches and before generic `app.overlay.is_some()` handling. Clear unrelated
pending state and map:

```text
j/Down       RecordViewMoveFields(1)
k/Up         RecordViewMoveFields(-1)
h/Left       RecordViewMoveRow(-1)
l/Right      RecordViewMoveRow(1)
Home         RecordViewJumpFirstField
G/End        RecordViewJumpLastField
Esc/q/v      CloseRecordView
```

Extend `Pending` with a Record View goto prefix or otherwise reuse the timed
pending mechanism without allowing normal Grid `gg` to execute beneath the
overlay. A second `g` within 750 ms maps to `RecordViewJumpFirstField`; invalid,
expired, focus-changed, tab-changed, or overlay-closed sequences do nothing.

**Step 5: Run keymap tests**

Run: `cargo test --test keymap record_view`

Expected: PASS.

### Task 4: Render The Responsive Record View Overlay

**Files:**
- Create: `src/ui/record_view.rs`
- Modify: `src/ui/mod.rs:1-40,105-117,203-298,1634-1650`
- Test: `tests/ui_render.rs`

**Step 1: Write failing SQL rendering tests**

Extend or create a fixture with multiple rows and columns including `NULL`, an
empty string, unsupported data, Unicode/control characters, and a long text
value. Open Record View and assert the rendered buffer contains:

- `RECORD VIEW` and `ROW 1 / N`;
- `FIELD`, `TYPE`, and `VALUE`;
- fields in result-column order;
- sanitized previews;
- visible `NULL` and an empty value distinct from `NULL`;
- the fixed navigation footer.

Render after `RecordViewMoveRow(1)` and verify row position and values change.

**Step 2: Run the focused UI tests and verify failure**

Run:

```bash
cargo test --test ui_render record_view
```

Expected: FAIL because no Record View renderer exists.

**Step 3: Add render-time viewport synchronization**

Extend `UiState` with a small Record View viewport snapshot containing tab ID
and visible field-row capacity. Reset it in `render_with_state_using_icons`, set
it from the Record View renderer, and synchronize it through the existing
runtime render-state update pattern to a semantic action such as
`RecordViewViewportChanged { tab_id, field_rows }`.

Reject stale snapshots whose tab ID is no longer active or whose overlay is no
longer Record View. This keeps terminal geometry out of the keymap/model and
lets field movement use the last rendered capacity.

**Step 4: Implement responsive popup geometry**

Create `ui::record_view::render` and dispatch
`Overlay::RecordView(state)` from `render_overlay`. Reuse `centered`, `Clear`,
`panel_block`, Theme, and terminal sanitization. Target 88 columns and about 70
percent of available height, but use saturating calculations and retain a safe
minimal rendering path for short/narrow terminals.

Divide the inner area into:

```text
row position
header/rule
scrolling field rows
footer
```

**Step 5: Render ordered field/type/value rows**

Resolve `app.active_record()` on every frame. Allocate bounded FIELD and TYPE
widths and the remaining cells to VALUE. Sanitize names, type names, and preview
text. Use `CellValue::preview(visible_value_width)` and match Data Grid styles:

```rust
CellValue::Null => muted italic,
CellValue::Unsupported { .. } => warning,
_ => normal text,
```

If a row is shorter than its metadata, render the missing cell as `NULL`. If
`active_record()` is `None`, render `Record no longer available` and the close
footer without stale content.

**Step 6: Add scrolling and compact-layout render tests**

Verify first/last field jumps, a middle offset, a terminal near 80x24, and a very
small terminal. Assert no panic and that the visible field slice changes while
the header/footer remain stable.

Add an equivalent relation Data fixture and assert geometry/content matches SQL
for the same metadata and values. Add a relation edit-session case proving the
overlay displays current staged values but remains read-only.

**Step 7: Run focused UI and App tests**

Run:

```bash
cargo test --test ui_render record_view
cargo test app::tests::record_view --lib
```

Expected: PASS.

### Task 5: Register Contextual Help And Documentation

**Files:**
- Modify: `src/help.rs:3-76` and Results entries in `shortcuts`
- Modify: `src/app.rs:423-650`
- Modify: `docs/keybindings.md:162-181`
- Test: `src/help.rs`
- Test: `tests/ui_render.rs` near Results help test around `594`

**Step 1: Write failing help tests**

Add `HelpShortcutId::ResultsOpenRecordView` and assert Results help contains one
entry with key `v` and description `open Record View`. Assert it is searchable
and executable through the help palette.

**Step 2: Run focused tests and verify failure**

Run:

```bash
cargo test help --lib
cargo test --test ui_render results_help
```

Expected: FAIL because the shortcut is absent.

**Step 3: Register and resolve the shortcut**

Add the Results catalog entry and resolve it to `Action::OpenRecordView` in
`App::execute_help_shortcut`. Keep capability validation in the normal reducer
so invoking help on an unavailable/empty Data Grid is a safe no-op.

**Step 4: Update keybinding documentation**

Add `v | Open Record View for the selected row` to Results and document the
overlay controls and read-only/preview limitations immediately below the table.

**Step 5: Run focused help tests**

Run:

```bash
cargo test help --lib
cargo test --test ui_render results_help
```

Expected: PASS.

### Task 6: Regression Verification

**Files:**
- Verify only; fix only failures caused by this feature

**Step 1: Format and inspect the diff**

Run:

```bash
cargo fmt --all -- --check
git diff --check
git diff -- src/model/record_view.rs src/model/mod.rs src/model/workspace.rs src/action.rs src/app.rs src/input/keymap.rs src/ui/record_view.rs src/ui/mod.rs src/help.rs tests/keymap.rs tests/ui_render.rs docs/keybindings.md docs/plans/2026-08-28-record-view-design.md docs/plans/2026-08-28-record-view-implementation.md
```

Expected: formatting and whitespace checks PASS; diff contains only intended
Record View changes plus the two plan documents.

**Step 2: Run focused regression suites**

Run:

```bash
cargo test model::record_view::tests --lib
cargo test app::tests::record_view --lib
cargo test --test keymap
cargo test --test ui_render
cargo test --test relation_tabs
```

Expected: all PASS.

**Step 3: Run static checks**

Run:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS with no warnings.

**Step 4: Run the complete test suite**

Run:

```bash
cargo test --all-features
```

Expected: PASS.

**Step 5: Manual keyboard acceptance test**

Run LazyDB against SQLite, execute a query with multiple rows and enough columns
to scroll, then verify:

1. Focus Results and press `v`.
2. Confirm current row fields/types/previews are correct.
3. Use `j/k`, arrows, `gg`, and `G` to navigate fields.
4. Use `h/l` and Left/Right to inspect adjacent rows without closing.
5. Confirm first/last rows clamp without wrapping.
6. Close with `Esc`, `q`, and `v` in separate attempts.
7. Confirm Results retains focus and the Grid selects the last inspected row.
8. Repeat from a relation Data preview.

Expected: all acceptance points pass; no editing or database write occurs.
