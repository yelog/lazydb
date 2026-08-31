# Data Grid Column Jump Design

## Goal

Add Vim-style horizontal shortcuts to the shared data grid used by SQL Result
set and Relation DATA:

- `^` selects the first data column in the current row;
- `$` selects the last data column in the current row.

## Behavior

The shortcuts preserve the selected row, vertical scroll position, column-width
overrides, and underlying data. They are no-ops for a grid with no columns and
are enabled only for Result set and Relation DATA grid focus. OUTPUT, Relation
DDL, and SQL Editor behavior remain unchanged.

## Implementation

Map the keys to semantic grid actions in the keymap. Route both actions through
the existing active-grid helper and implement the bounded column selection in
`DataGridState`, allowing the existing viewport synchronization to reveal the
selected column.

Add model and keymap coverage for first/last column selection and update the
Results and Relation Data keybinding documentation.
