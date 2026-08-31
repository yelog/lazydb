# Result Set Query Bar Layout

## Goal

Place the console result `WHERE` and `ORDER BY` controls inside the `RESULT SET` panel, directly above the table header, matching the established `RELATION DATA` hierarchy.

## Design

`render_data` owns and renders the `RESULT SET` panel border. It splits the panel's inner area into the query bar, an optional running-status row, and the result body. The result table and loading skeleton receive a borderless surface block so they do not create a second panel.

All states use the same hierarchy: ready results, a running query with previous results, initial loading, and the empty state. Completion popup anchoring remains tied to the result panel viewport.

## Verification

Add a render regression assertion that the query controls appear below the `RESULT SET` top border and above the result table header. Run the focused UI render tests and formatting checks.
