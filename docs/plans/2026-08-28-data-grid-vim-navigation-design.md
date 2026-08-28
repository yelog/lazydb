# Data Grid Vim Navigation Design

## Goal

Extend the shared Data Grid used by SQL results and relation DATA previews with
a fixed row-number column and Vim-style vertical navigation:

- `gg` and `G` select the first and last data rows;
- `H`, `M`, and `L` select the top, middle, and bottom visible rows;
- `Ctrl-d`/`Ctrl-u` move down/up by half a page;
- `Ctrl-f`/`Ctrl-b` move down/up by a full page;
- `zz`, `zt`, and `zb` align the selected row to the middle, top, and bottom of
  the viewport.

The behavior must remain identical on both Data Grid surfaces and preserve the
existing invariant that the selected row is visible.

## Current Architecture

`DataGridState` stores the selected cell, row and column offsets, rendered row
capacity, and column-width overrides. The shared renderer computes the actual
viewport from terminal geometry, publishes a `DataGridViewport`, and the runtime
synchronizes that snapshot back into the active tab. Ordinary `j`/`k` movement
uses minimal scrolling through `ensure_row_visible`.

The keymap already supports timed multi-key sequences. Pending sequences are
scoped to the current focus, editor mode, and tab UUID, which makes it suitable
for `gg` and the `z` commands.

## Architecture

Keep key handling semantic and geometry-free:

1. The keymap translates keys and sequences into Grid navigation actions.
2. App routes each action through `with_active_grid`, supplying the current row
   count.
3. Data Grid state logic calculates bounded selection and row offsets from the
   last synchronized `viewport_rows`.
4. The renderer remains the final authority on actual terminal geometry and
   synchronizes any clamped viewport back to App.

Do not calculate page sizes in the keymap and do not rely on render-time repair
as the primary navigation mechanism.

## Semantic Actions

Use compact semantic types instead of one Action variant per key:

```rust
pub enum GridRowTarget {
    First,
    Last,
    ViewTop,
    ViewMiddle,
    ViewBottom,
}

pub enum GridScrollAmount {
    HalfPage,
    Page,
}

pub enum GridRowAlignment {
    Top,
    Middle,
    Bottom,
}
```

Expose them through actions equivalent to:

```rust
GridSelectRow(GridRowTarget)
GridScrollRows {
    direction: isize,
    amount: GridScrollAmount,
}
GridAlignSelectedRow(GridRowAlignment)
```

`GridMove` remains responsible for cell-wise `h`/`j`/`k`/`l` movement.

## Navigation Semantics

For a non-empty grid:

```text
visible       = viewport_rows
first_visible = row_offset
last_visible  = min(row_offset + visible - 1, row_count - 1)
middle        = first_visible + (last_visible - first_visible) / 2
max_offset    = row_count - min(visible, row_count)
```

An even-height viewport uses the upper middle row. All arithmetic is saturating
and clamped to actual row bounds.

### Absolute And Viewport Selection

- `gg` sets selection and offset to zero.
- `G` selects `row_count - 1` and reveals it at the last legal viewport.
- `H`, `M`, and `L` select the first, middle, and last actual visible rows
  without changing `row_offset`.
- A partial final page uses its actual last row, not a virtual viewport edge.

### Page Movement

Page movement changes both selection and viewport in the same direction:

- half page: `max(1, viewport_rows / 2)` rows;
- full page: `max(1, viewport_rows)` rows.

Selection and offset are each bounded by their legal ranges. In the middle of a
result this preserves the selected row's screen-relative position. Near either
end, the viewport stops at its boundary while selection can continue toward the
first or last row. Repeating a command at the final boundary is a no-op.

This selected-row movement is intentional: offset-only scrolling conflicts with
the existing selected-row-visible invariant and would be undone by the renderer.

### Selected-Row Alignment

Alignment keeps `selected_row` unchanged and calculates a new offset:

```text
zt = selected_row
zz = selected_row - floor((visible - 1) / 2)
zb = selected_row - (visible - 1)
```

The result is clamped to `0..=max_offset`. Near the beginning or end, exact
placement is impossible without blank overscroll, so the viewport uses the
nearest legal position.

### Empty Or Unknown Viewports

- With zero rows, selection and offset remain zero.
- With `viewport_rows == 0`, viewport-relative and page commands are safe
  no-ops. Absolute `gg` and `G` may still select valid absolute rows; the first
  subsequent render reveals the selection.

## Key Sequences And Modes

Extend the existing pending-key state with Grid goto and alignment prefixes:

- `g` starts a goto sequence; a second `g` selects the first row.
- `z` starts an alignment sequence; `z`, `t`, or `b` performs the requested
  alignment.
- Sequences retain the existing 750 ms timeout and focus/editor-mode/tab-ID
  checks.
- An invalid or expired second key clears the pending sequence without action.

Map `G`, `H`, `M`, and `L` directly while Results has focus. Map
`Ctrl-d/u/f/b` in the control-key branch only when Results has focus so SQL
Editor control behavior remains unchanged.

For relation tabs, navigation is enabled wherever existing Grid browse movement
is valid. Cell-edit mode must continue routing printable keys to the editor, and
busy state must not mutate Grid navigation state. Visual-line navigation should
preserve its existing anchor semantics.

## Fixed Row-Number Column

The row number is a presentation gutter, not a synthetic database column. It
must not be added to `ResultSet.columns` or row values and must not affect:

- `selected_column`;
- relation editing or mutation values;
- result metadata;
- column-width overrides;
- data-column resize and mouse targets.

The renderer prepends a fixed row-number cell and separator to every header/body
row. Its header is `#`; body values are the absolute one-based indexes
`row_index + 1`, so the first row on a scrolled page is `row_offset + 1`.

Calculate a stable width from the complete result size:

```text
row_number_width = max(1, decimal_digits(row_count)) + 2
```

The extra cells provide horizontal padding. The gutter remains present while
data columns scroll horizontally.

## Renderer Geometry

Account for the fixed gutter before calculating horizontally visible data
columns:

- subtract the row-number width and its separator from `available`;
- include the fixed constraint and separator in header and body cells;
- adjust the Ratatui selected-cell index so original data column zero remains
  the first selectable data cell;
- begin data-cell and resize hit regions after the gutter;
- give the gutter no hit region and no resize boundary;
- include its segment in the header rule;
- constrain the horizontal scrollbar track to the scrollable data-column area.

Use saturating width arithmetic. Extremely narrow areas retain the gutter and at
least one clipped data column without panicking.

## Help And Documentation

Register each new Results command in contextual help so it is searchable and
executable through the existing help palette. Update `docs/keybindings.md` with
the complete Results navigation contract.

## Error Handling And Invariants

- Grid state never indexes outside current rows or columns.
- The selected row remains visible after every completed navigation and render
  synchronization.
- Stale viewport snapshots remain rejected by tab UUID.
- All commands are no-ops when no active DATA grid exists.
- Navigation does not alter horizontal selection, offsets, or width overrides.
- The row-number gutter is never selectable, editable, or resizable.

## Testing

### Model And App

Cover zero rows, one row, fewer than one page, exactly one page, and multiple
pages. Verify:

- absolute first/last selection;
- top/middle/bottom selection on full and partial pages;
- half/full-page movement in the middle and at both boundaries;
- screen-relative position preservation during normal page movement;
- top/middle/bottom alignment and boundary clamping;
- safe behavior before viewport capacity is known;
- no horizontal Grid state changes;
- equivalent SQL DATA and relation DATA behavior.

### Keymap

Cover direct uppercase keys, control keys, valid `gg` and `z` sequences,
timeouts, invalid second keys, focus changes, tab changes, and relation Grid
modes.

### Rendering And Mouse

Verify:

- the `#` header and one-based absolute row numbers;
- correct numbers after vertical scrolling;
- a fixed gutter after horizontal scrolling;
- correct selected-cell highlighting;
- original data column zero remains column zero in hit targets;
- resize handles remain aligned with data columns;
- the scrollbar excludes the fixed gutter;
- SQL and relation render identical geometry for equivalent results.

### Regression Verification

Run focused model, keymap, UI render, relation/workspace, and mouse suites, then
format, Clippy with warnings denied, and the complete all-features test suite.
