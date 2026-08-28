# Compact Data Grid Header Design

## Problem

The shared data grid reserves one row below its header with `Row::bottom_margin(1)` and then paints a horizontal rule into that row. The rule consumes vertical space, interrupts the header background, and makes the column intersections look like stray short vertical lines.

## Design

- Remove the header bottom margin and the custom horizontal rule.
- Keep the one-row header and its vertical column separators.
- Place the first data row immediately below the header.
- Apply the layout consistently to relation previews and SQL query results through the shared data-grid renderer.
- Move empty-state content and mouse hit regions up with the data rows.
- Increase the reported visible-row capacity by one because the separator row no longer consumes space.
- Leave horizontal scrollbar placement unchanged.

## Verification

- Verify the header is immediately followed by the first data row.
- Verify the grid no longer renders the `┼` header-rule intersections.
- Verify vertical column separators remain in the header and body.
- Verify `No rows` appears immediately below the header without internal separators or a selection marker.
- Verify cell hit regions and row scrolling still target the correct absolute rows.
