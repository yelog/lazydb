# Relation data visual hierarchy design

## Goal

Improve the relation preview hierarchy so users can distinguish workspace tabs, relation view tabs, query inputs, and table headers at a glance. Preserve all existing query, completion, grid navigation, editing, resize, and scrolling behavior.

## Current problems

- The relation page renders `DATA`, `DDL`, and the relation name on one plain text line. `DATA` and `DDL` do not read as interactive tabs, while the relation name duplicates the active workspace tab.
- The relation query bar sits above the `RELATION DATA` panel, so its scope is visually ambiguous.
- `WHERE` and `ORDER BY` resemble passive labels when empty because their inputs have no persistent affordance.
- The grid header relies on a flat gray background. It is visually heavy but does not create a refined separation between column labels and data rows.
- The SQL result view shares the query bar, so query-input affordance should remain consistent across SQL and relation workflows.

## Chosen direction

Use a layered workbench hierarchy:

1. Workspace tabs remain the primary navigation. Their active state keeps the filled accent background.
2. `DATA` and `DDL` become secondary tabs. Their active state uses accent text, bold weight, and a short underline rather than a filled background.
3. The duplicated relation name is removed from the secondary tab row.
4. On relation data pages, the query bar moves inside the `RELATION DATA` panel and above the grid header.
5. Query fields use icons and persistent underline input slots in both relation and SQL result views.
6. Grid headers replace the flat gray fill with emphasized text and a horizontal divider.

This keeps the relation view visibly subordinate to the workspace tab while making its local navigation and controls discoverable.

## Relation page structure

The relation page uses the following vertical hierarchy:

```text
 DATA  DDL
 ----

+- RELATION DATA -----------------------------------------+
| [filter] WHERE     value_______________________________ |
| [sort]   ORDER BY  value_______________________________ |
|                                                         |
| # | id        | name            | created_at            |
| --+-----------+-----------------+----------------------- |
| 1 | 1001      | Alice           | 2026-08-31            |
+---------------------------------------------------------+
```

On sufficiently wide layouts, `WHERE` and `ORDER BY` remain side by side to preserve vertical space. On narrow layouts they stack vertically. Query errors render immediately below the fields inside the same panel.

The DDL view uses the same secondary tab treatment but does not render the query bar.

## Secondary tabs

- Render only `DATA` and `DDL`; remove the relation title from this row.
- Active tab: accent foreground, bold label, and a short underline aligned to the label.
- Inactive tab: muted foreground with no filled background.
- Keep spacing between labels large enough to produce separate click targets.
- Keep the tab rail on the base surface so it cannot be confused with the filled active workspace tab.
- Update hit regions to match the rendered label widths and underline treatment.

## Query inputs

The shared query bar receives the same visual treatment in SQL and relation contexts.

- Prefix `WHERE` with a filter icon.
- Prefix `ORDER BY` with a sort icon.
- Provide Nerd Font, Unicode, and ASCII mappings through `IconSet`.
- Render a persistent underline across the available input width, including when the value is empty.
- Unfocused state: muted icon and label, text foreground for entered values, border color for the underline.
- Focused state: accent icon, label, value, and underline.
- Disabled state: muted treatment remains, with no hit region, preserving the current capability behavior.
- Preserve the existing bar cursor, horizontal input projection, completion anchoring, click targets, validation error placement, and submission behavior.

The query bar should accept the active icon set from `UiState` rather than instantiate a separate icon configuration.

## Relation panel composition

The query bar must be inside the `RELATION DATA` border. The panel should own the query controls, grid, footer metadata, loading state, and errors as one scoped unit.

The implementation should avoid nested full borders. The outer `RELATION DATA` block remains the only container border. Internal regions use spacing and divider lines.

Loading and empty states retain the panel title and border. If there is no snapshot, the query bar remains available according to its current capability, and loading or empty content fills the remaining panel body.

## Grid header

- Remove the `grid_header` background fill from header cells.
- Use the normal panel surface as the header background.
- Render column names in bold text.
- Keep `#` muted so row numbering remains secondary.
- Keep vertical separators in `grid_border`, but reduce their visual dominance relative to the column labels.
- Add a horizontal divider directly below the header using `grid_border` or `border` color.
- Preserve selected-row, selected-cell, resize targets, horizontal scrollbar, edit-state colors, and empty-grid alignment.

Adding the divider consumes one grid row. Viewport calculations and hit-region Y positions must be updated together so keyboard and mouse selection continue to address the visible rows correctly.

## Responsive behavior

- Wide query bars render two equal-width fields on one row.
- Narrow query bars stack fields when each horizontal field would not have enough room for its icon, label, input value, and a meaningful underline slot.
- Error text occupies a row below the fields.
- Decorative underlines and dividers may shorten when constrained, but must not remove the final usable grid row.
- Existing supported terminal sizes in UI render tests remain readable and panic-free.

## Accessibility and terminal compatibility

- Active state is communicated by color, bold weight, and underline, not color alone.
- Input affordance remains visible when fields are empty and unfocused.
- ASCII mode uses fixed-width textual icons and ASCII-safe underline/divider characters.
- Unicode and Nerd Font modes must maintain correct cell widths and hit regions.
- Terminal control sanitization remains unchanged for relation names, query values, errors, SQL, headers, and cell values.

## Testing

Add or update render tests to verify:

- Relation pages render `DATA` and `DDL` without repeating the relation name in the secondary tab row.
- Active relation view tabs have a visible underline and correct hit regions.
- Relation query fields render within the `RELATION DATA` panel before the grid header.
- SQL and relation query bars both render filter and sort icons plus persistent input underlines.
- Nerd Font, Unicode, and ASCII modes render safe query icons without layout corruption.
- Empty, loading, failed, and ready relation states retain the panel hierarchy.
- Grid headers no longer use `grid_header` as a flat fill and include a horizontal divider.
- Grid row hit regions, selection, empty state, column resize targets, and viewport metrics remain correct after the extra divider row.
- Existing 80x24, 120x36, and 180x50 relation render coverage remains valid.

## Non-goals

- No changes to query SQL generation, validation, completion candidates, or submission behavior.
- No changes to relation data editing semantics.
- No changes to workspace tab styling.
- No theme overhaul or new runtime dependency.
- No new relation title inside the content area; the workspace tab remains the object identity source.
