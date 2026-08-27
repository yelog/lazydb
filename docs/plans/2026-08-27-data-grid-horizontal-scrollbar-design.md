# Data Grid Horizontal Scrollbar Design

## Goal

Keep every result column available on narrow terminals while rendering only the columns that fit in the current viewport. Make hidden columns discoverable and reachable through an interactive horizontal scrollbar shared by relation previews and SQL query results.

## Behavior

- Result sets retain every column in database or SQL projection order.
- A grid starts at the first column.
- The viewport displays complete columns starting at a stored column offset.
- Moving the selected cell left or right adjusts the viewport only when the selected column crosses a viewport edge.
- A horizontal scrollbar appears inside the panel bottom edge only when the total column width exceeds the available width.
- The scrollbar thumb represents the visible column range. Its position represents the first visible column.
- Clicking before or after the thumb moves by one visible page.
- Dragging the thumb maps the pointer position to a column offset and snaps to a complete column.
- Scrollbar interaction keeps the selected column inside the new viewport.
- When all columns fit, the scrollbar is omitted and no table height is consumed.
- Relation previews and SQL results use the same state and renderer behavior.

## State And Rendering

`DataGridState` gains `column_offset`, the index of the first visible column. Clamping a grid also clamps this offset to the result column count.

The shared data-grid renderer computes widths, determines whether overflow exists, reserves one bottom row when needed, and derives the visible complete-column range from `column_offset`. Rendering returns enough viewport geometry through hit regions for mouse interaction. It no longer infers the first visible column from the selected column on every frame.

Keyboard grid movement updates selection and then normalizes the offset so the selected column remains visible. Column resizing and terminal resizing use the same normalization during rendering and subsequent movement.

## Mouse Interaction

The UI state tracks an active horizontal scrollbar drag. New hit targets distinguish the scrollbar thumb and track-page regions. Mouse down on the thumb begins dragging, mouse movement dispatches an offset update, and mouse up ends dragging. Clicking either side of the thumb dispatches a page-sized offset change.

All scrollbar movement is column-based. Partial columns are not rendered.

## Testing

- Narrow grids initially show the first columns.
- Moving selection across either viewport edge updates the offset.
- Page clicks and thumb dragging reach the beginning, middle, and end.
- Offsets remain valid after column-count, column-width, and terminal-width changes.
- No scrollbar appears when all columns fit.
- Both relation and SQL grids share the behavior.
- The final result column is reachable without removing any columns from the result model.
