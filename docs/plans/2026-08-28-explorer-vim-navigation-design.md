# Explorer Vim Navigation Design

**Status:** Approved

**Date:** 2026-08-28

## Summary

Explorer will use its actual rendered viewport height for selection and scrolling,
and will support the same Vim-style vertical navigation contract as the Results
data grid. The normal tree gains `gg`, `G`, `H`, `M`, `L`, page movement, and
selected-row alignment. Inline catalog search remains an input-oriented mode and
does not receive these new commands.

Expanded relation columns will be ordered by database ordinal position so that
their order matches relation preview headers across PostgreSQL, MySQL, and SQLite.

## Problems

### Column order

The adapters load `ColumnMetadata.ordinal_position`, but catalog child pages use a
generic kind-and-name sort key. Columns are therefore displayed alphabetically in
Explorer while relation previews retain the result-set order supplied by the
database.

### Premature scrolling

Explorer rendering displays `inner.height` rows, but selection operations call
`ensure_selected_visible(8)`. When the rendered pane is taller or shorter than
eight rows, the model and UI disagree about the visible range. This causes `j` to
scroll before selection reaches the actual bottom of the pane.

### Missing tree navigation

Explorer currently exposes only relative movement and maps Home/End to extreme
deltas. It has no explicit model operations for document targets, viewport
targets, page movement, or selected-row alignment.

## Goals

- Keep the selected Explorer node visible after every navigation or tree change.
- Begin scrolling only when one-step movement would leave the rendered viewport.
- Add Vim-compatible document, viewport, page, and alignment commands.
- Preserve the existing inline search input contract.
- Make expanded column order match relation preview column order.
- Reuse the Results grid's established navigation semantics where applicable.
- Keep viewport calculations in the model and rendering measurements in the UI.

## Non-goals

- Adding a command mode to Explorer inline search.
- Changing search result ranking or search result scrolling.
- Changing ordering for non-column catalog children.
- Sharing one concrete state type between Explorer and the Results grid.
- Changing horizontal Explorer behavior.

## Confirmed Interaction

The new commands apply only while Explorer has focus and the normal tree is
visible.

| Key | Behavior |
| --- | --- |
| `j`, Down | Select the next node; scroll one row only when moving below the viewport |
| `k`, Up | Select the previous node; scroll one row only when moving above the viewport |
| `gg`, Home | Select the first node in the visible tree projection |
| `G`, End | Select the last node in the visible tree projection |
| `H` | Select the top node currently displayed |
| `M` | Select the middle node currently displayed |
| `L` | Select the bottom node currently displayed |
| `Ctrl-f` | Move selection and viewport down one page |
| `Ctrl-b` | Move selection and viewport up one page |
| `Ctrl-d` | Move selection and viewport down half a page |
| `Ctrl-u` | Move selection and viewport up half a page |
| `zz` | Align the selected node to the viewport middle |
| `zt` | Align the selected node to the viewport top |
| `zb` | Align the selected node to the viewport bottom |

One page is the actual number of rendered Explorer rows. Half a page is
`max(viewport_rows / 2, 1)`. Movement clamps at tree boundaries. Alignment does
not change selection and may be limited by the beginning or end of the tree.

For `H`, `M`, and `L`, targets are calculated from the rows actually displayed.
The final partial viewport therefore uses its real top, middle, and bottom rows,
not an imagined full-height range.

Inline search keeps its existing behavior. Printable `g`, `G`, `H`, `M`, `L`,
and `z` characters edit the query. Search result movement remains on `j/k`,
arrows, Home, and End.

## Architecture

### Viewport measurement

`render_explorer` already derives the tree's display area as the panel block's
`inner` rectangle. It records `inner.height` in `UiState`, parallel to the editor
and data-grid viewport measurements. After rendering, runtime dispatches an
`ExplorerViewportChanged` action when the measured row count changes.

The action updates the Explorer model's viewport height, clamps its scroll range,
and ensures the current selection remains visible. A zero-height viewport is
valid during compact or transitional layouts and must not panic or invent a
visible range.

```text
render_explorer(inner.height)
    -> UiState.explorer_viewport_rows
    -> Action::ExplorerViewportChanged(rows)
    -> ExplorerTreeState viewport update
    -> clamp scroll and selection visibility
```

### State ownership

`ExplorerTreeState` remains authoritative for the selected node and vertical
scroll offset. It gains the current viewport height and explicit operations for:

- selecting a document or viewport target;
- moving by a full or half page;
- aligning the selected node within the viewport;
- applying a changed viewport height.

`ExplorerState.selected` and `ExplorerState.scroll` remain compatibility
projections used by existing application and mouse paths. After each normalized
tree operation, they are synchronized from `ExplorerTreeState`; they do not run
independent viewport calculations.

All existing hard-coded `ensure_selected_visible(8)` calls are removed. Search
location, projection rebuild, mouse selection, expansion/collapse reconciliation,
and normal movement use the current model viewport.

### Navigation types

Explorer uses small semantic enums equivalent to the Results grid contract:

```rust
enum ExplorerNodeTarget {
    First,
    Last,
    ViewTop,
    ViewMiddle,
    ViewBottom,
}

enum ExplorerScrollAmount {
    HalfPage,
    Page,
}

enum ExplorerNodeAlignment {
    Top,
    Middle,
    Bottom,
}
```

Keeping these types in the Explorer model makes actions descriptive and avoids
encoding unrelated operations as magic `isize` deltas. Names may be shortened if
the implementation remains unambiguous within the module.

### Scrolling rules

For one-node movement, selection changes first and scroll changes only if the new
index lies outside `[scroll, scroll + viewport_rows)`. Moving onto the current
bottom row does not scroll; moving once more does.

For page movement, both selection and scroll move by the same bounded delta, then
the normal visibility invariant is applied. This preserves the selected node's
screen-relative position where tree boundaries permit it.

For alignment, selection is unchanged. The desired screen row is zero, the
middle row `(viewport_rows - 1) / 2`, or the bottom row
`viewport_rows - 1`. The resulting scroll offset is clamped to the maximum valid
offset.

Every operation handles empty trees, one-row trees, viewport height zero, partial
final pages, and stale selections without underflow.

### Key sequences

The existing keymap pending-state mechanism is extended for Explorer `g` and `z`
sequences. It retains the current 750 ms timeout and validates focus, editor mode,
and active tab identity before resolving a sequence.

`gg` maps to the first-node target. `zz`, `zt`, and `zb` map to alignment actions.
`G`, `H`, `M`, and `L` map directly. Control-page keys are intercepted only when
Explorer's normal tree has focus; they must not override Ctrl-U search clearing.
Invalid or expired Explorer sequences produce no navigation action.

## Column Ordering

Column ordering must be fixed before catalog pagination is finalized, not in the
UI projection. Sorting only at render time could produce incorrect order across
pages and would make cursor order disagree with display order.

Each adapter's catalog child sort key follows this rule:

1. Preserve the existing catalog-kind rank.
2. For `CatalogKind::Column`, sort by `ColumnMetadata.ordinal_position`.
3. Use column name and native path as deterministic tie-breakers.
4. For every non-column kind, preserve the existing name and native-path order.

The adapters already retrieve ordinal positions from native metadata:

- PostgreSQL uses `pg_attribute.attnum`.
- MySQL uses `information_schema.columns.ordinal_position`.
- SQLite derives the position from PRAGMA column metadata.

Relation previews continue to use database result metadata order. With catalog
columns ordered by the same ordinal source, Explorer and preview headers agree.

## Error And Boundary Handling

- Empty trees reset selection and scroll to their existing empty-state behavior.
- A viewport height of zero records the measurement but defers visual alignment.
- Tree refresh, collapse, deletion, or search location reconciles stale selection
  before enforcing visibility.
- Resize clamps scroll to the new maximum and keeps selection visible.
- Short trees always use scroll zero.
- Database ordinal values are validated by existing adapter conversion paths;
  malformed values continue to return adapter errors rather than silently
  reordering columns.
- No database work or search lifecycle behavior changes.

## UI And Help

Explorer rendering continues slicing the visible projection from normalized
`scroll`, but the slice and model now use the same measured row count. Existing
hit regions are generated from that slice, so mouse selection remains aligned.

The contextual help list and `docs/keybindings.md` document the complete command
set. The footer remains concise and shows only a discoverable subset rather than
all shortcuts.

## Testing

### Model tests

- One-step movement does not scroll before reaching the actual viewport edge.
- Moving beyond the bottom or top scrolls by the minimum required amount.
- First, last, view-top, view-middle, and view-bottom targets work on full and
  partial viewports.
- Full-page and half-page movement preserve visibility and clamp at both ends.
- Top, middle, and bottom alignment preserve selection and clamp scroll.
- Empty, one-row, short, odd-height, even-height, and zero-height viewports are
  covered.
- Resize, expansion, collapse, refresh, and selection reconciliation retain the
  visibility invariant.

### Keymap tests

- `gg`, `G`, `H`, `M`, `L`, `Ctrl-f`, `Ctrl-b`, `Ctrl-d`, and `Ctrl-u` map to the
  intended Explorer actions.
- `zz`, `zt`, and `zb` resolve through pending state.
- Invalid, expired, focus-changed, and tab-changed sequences do not execute.
- The commands do not activate in Editor, Results, overlays, or Explorer search.
- Search continues accepting command letters as query text and Ctrl-U still
  clears the query.

### Adapter and contract tests

- PostgreSQL, MySQL, and SQLite return relation columns in ordinal order.
- Fixtures use deliberately non-alphabetical definitions such as
  `z_col, a_col, m_col`.
- Explorer column order is compared with relation preview headers.
- Non-column catalog ordering and pagination cursor behavior remain unchanged.

### UI tests

- Standard and compact terminal sizes report the actual Explorer inner height.
- The selected row remains inside the rendered slice after navigation and resize.
- The last rendered hit region corresponds to the model's viewport bottom.

## Acceptance Criteria

- Expanded columns appear in the same order as table or view preview headers.
- `j` reaches the bottom rendered row without changing scroll; the next `j`
  scrolls and keeps selection visible.
- `gg` and `G` select the first and last visible tree nodes.
- `H`, `M`, and `L` select the actual top, middle, and bottom displayed nodes.
- Ctrl page commands move in measured full- or half-page increments.
- `zz`, `zt`, and `zb` align the current selection without changing it.
- All commands clamp safely at tree and viewport boundaries.
- Explorer inline search retains its current input and navigation behavior.
- No Explorer path uses a hard-coded viewport height.
