# Empty Data Grid Design

## Problem

When a result set has columns but no rows, the data grid still selects cell `(0, selected_column)`. That cell does not exist, but Ratatui's table renderer still applies selection layout to it. The result is a short vertical marker below the header rule, which looks like a broken border.

## Design

- Keep rendering the column headers and header rule so the result schema remains visible.
- Do not assign a selected cell when the result has no rows.
- Render a muted, centered `No rows` message in the first line of the data area.
- Preserve all existing behavior for non-empty result sets, including selection, scrolling, resizing, and highlighting.

## Verification

Add a UI rendering regression test for a relation preview with columns and zero rows. Verify that:

- `No rows` is visible.
- The header and header rule remain visible.
- The line below the header rule contains no vertical selection or column separator marker.
