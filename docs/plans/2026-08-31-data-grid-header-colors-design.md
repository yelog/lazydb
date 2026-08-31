# Data Grid Header Colors Design

## Problem

The shared data grid currently renders its header with the same surface color as
the body and uses a second row for a bright horizontal rule. This weakens the
table hierarchy and makes both SQL result sets and relation data look visually
unfinished.

## Design

- Restore a compact, single-row header without the horizontal rule.
- Use a deep teal-gray header background (`RGB(24, 48, 58)`) that is visibly
  raised from the existing Deep Space surface without competing with selection.
- Use soft cyan header text (`RGB(184, 235, 229)`) with bold weight to preserve
  readability and connect the grid to the existing cyan accent.
- Keep the row-number header and every column header on one continuous color
  band.
- Render header separators with the existing grid-border color on the new header
  background.
- Apply the change through the shared data-grid renderer so SQL result sets and
  relation data remain visually identical.
- Leave body rows, selected cells, column widths, scrolling, and editing states
  unchanged.

## Verification

- Verify the header occupies one row and the first data row follows immediately.
- Verify no horizontal rule or cross intersections are rendered under the header.
- Verify the header background covers row numbers, separators, and data columns.
- Verify SQL result sets and relation data use the same header styling.
- Verify empty-state placement, visible-row capacity, mouse targets, and scrolling
  remain aligned after removing the second header row.
