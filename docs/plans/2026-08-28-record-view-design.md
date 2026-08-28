# Record View Design

## Goal

Add a read-only Record View overlay to the shared Data Grid used by SQL Results
and relation Data previews. With Results focused, `v` opens the selected row as
an ordered vertical list of fields and values. Users can inspect fields and move
between records without closing the overlay.

## Scope

The first version includes:

- SQL Results and relation Data previews;
- one centered, read-only overlay;
- field/value rows in result-column order;
- bounded value previews using the existing `CellValue::preview` contract;
- field scrolling and first/last-field navigation;
- previous/next-record navigation that updates the underlying Grid selection;
- contextual help and keybinding documentation.

The first version excludes editing, full-value viewers, JSON formatting, field
search, multi-record comparison, column reordering, and eager LOB loading.

## Chosen Architecture

Represent the feature as `Overlay::RecordView(RecordViewState)`. The overlay
state stores only the field offset needed for vertical scrolling. The active
Grid's `selected_row` remains the single source of truth for the current record,
and the current displayed result remains the source of columns and values.

This is preferred over copying a row into the overlay because copied values can
become stale after query or relation refreshes and can duplicate large values.
It is preferred over a new `ResultView` page because Record View is temporary
inspection, not a peer of SQL Data/Output/Plan or relation Data/DDL.

App exposes a read-only active-record accessor for rendering and routes row
movement through the existing bounded Grid path. For relation previews with an
edit session, the accessor uses the current editable row values because those
are what the shared Grid displays; Record View does not expose mutation actions.

## State And Data Flow

`RecordViewState` contains:

```text
field_offset: usize
```

Opening follows this flow:

1. Results has focus and the active tab is showing a Data Grid.
2. The Grid has at least one row and one result column.
3. `v` maps to `OpenRecordView`.
4. App installs `Overlay::RecordView` with offset zero.
5. Rendering resolves the active columns, current displayed row, selected row
   index, and total row count without copying the result.

Changing records routes through an explicit Record View action, updates the
underlying Grid `selected_row` with bounded movement, calls the existing
visibility/clamp logic, and resets `field_offset` to zero. Closing the overlay
leaves Results focused and preserves the last inspected row as the Grid
selection.

Async query or relation updates may replace the displayed result while the
overlay is open. Rendering always resolves current data. If no current record is
available, the overlay shows a bounded `Record no longer available` state; it
does not retain or display a stale row snapshot.

## Interaction

Outside the overlay:

| Key | Action |
| --- | --- |
| `v` | Open Record View for the current Grid row |

Inside the overlay:

| Key | Action |
| --- | --- |
| `j/k`, Down/Up | Scroll fields down/up |
| `gg`, Home | Jump to the first field |
| `G`, End | Jump to the last field |
| `h/l`, Left/Right | Move to the previous/next record |
| `Esc`, `q`, `v` | Close Record View |

Record movement clamps at the first and last rows. It does not wrap. Field
movement clamps to the available field range. Opening is a no-op outside a Data
Grid or when the active result has no rows or no columns.

The overlay has input priority over global Grid navigation. Its key handler
clears unrelated pending sequences. `gg` uses the existing timed pending-key
mechanism but is scoped to the Record View overlay.

## Presentation

Use a centered popup sized from the terminal, with a target width of 88 columns
and a target height of 70 percent of the available area, bounded by the existing
minimum-size behavior. Clear the popup area before rendering.

The border title is ` RECORD VIEW `. The first content line shows
`ROW n / total`, followed by a header and ordered field rows:

```text
FIELD                  TYPE             VALUE
id                     INTEGER          42
name                   TEXT             Ada
active                 BOOLEAN          true
```

Type is included because it is already present in `ColumnMeta` and is useful for
distinguishing formatted values without adding another interaction. Allocate
bounded widths for FIELD and TYPE and give the remaining width to VALUE. Clip
sanitized field/type text and call `CellValue::preview` with the visible value
width. Style `NULL` as muted italic and unsupported values as warning, matching
the shared Grid.

The footer is fixed:

```text
j/k fields   h/l records   gg/G first/last field   Esc close
```

The field viewport is derived from popup geometry. Rendering clamps the stored
offset against the current column count and visible capacity before slicing.
Very narrow or short terminals render a safe reduced layout without panicking.

## Error Handling And Invariants

- Record View never executes database I/O.
- Values are terminal-sanitized before rendering.
- The overlay never clones a `ResultSet`, row, or `CellValue`.
- Grid `selected_row` is the only current-record index.
- Record movement never changes the selected column or horizontal Grid state.
- The selected Grid row remains visible after record movement.
- Missing row cells render as `NULL`, matching the Grid's defensive behavior.
- Empty results and non-Data views cannot open Record View.
- A replaced or removed result cannot leave stale record content visible.
- Dismissing the overlay preserves Results focus and the last inspected row.

## Help And Documentation

Add `v: open Record View` to Results contextual help and
`docs/keybindings.md`. Record View controls appear in its fixed footer rather
than the normal Results help catalog because the overlay owns input while open.

## Testing

### Model And App

- Record View state clamps field offsets for zero, short, and long column lists.
- Opening succeeds for SQL Data and relation Data with rows and columns.
- Opening is a no-op for empty rows, zero columns, SQL Output/Plan, and relation
  DDL.
- Previous/next movement clamps at row bounds, updates Grid selection, keeps the
  row visible, preserves horizontal state, and resets field offset.
- The active-record accessor returns derived SQL data when displayed and current
  relation edit-session values when present.
- Replaced or removed results yield no current record rather than stale data.

### Keymap And Help

- `v` opens only from an active Results Data Grid.
- Record View input preempts ordinary Grid and relation shortcuts.
- Field, record, first/last, and close keys map to the expected semantic actions.
- Invalid and expired `g` prefixes do nothing and clear pending state.
- Results contextual help exposes an executable Record View entry.

### Rendering

- The popup renders row position, field names, types, and values in column order.
- `NULL`, unsupported, empty-string, truncated, Unicode, and terminal-control
  values render safely and distinctly.
- Field scrolling changes the visible slice and preserves the fixed footer.
- Record movement updates title and values.
- Small terminal dimensions do not panic.
- SQL and relation sources render the same geometry for equivalent records.

### Regression Verification

Run focused model/App, keymap, help, relation-tab, and UI-render tests, then
format, Clippy with warnings denied, and the full all-features test suite.
