# SQL Result Data Query Completion Design

## Goal

Make column completion work in the `WHERE` and `ORDER BY` inputs above SQL search result tables, matching the existing relation preview behavior.

## Design

- Keep `DataQueryCompletion` and the existing keymap/acceptance flow shared by both tab types.
- For relation previews, retain the current candidate sources: catalog columns and the current relation result columns.
- For SQL result tabs, use the last result set from the original query outcome as the only candidate source. This reflects the columns visible to the outer derived query, including aliases, expressions, and aggregates.
- Share matching, case-insensitive deduplication, ordering, and the ten-item limit between both sources.
- Capture the query bar cursor in the SQL result renderer and draw the existing completion popup after the result table.

## Behavior

- Completion starts after at least one identifier character is typed.
- Candidate acceptance continues to quote identifiers according to the active SQL dialect.
- Completion remains available while a derived filter query is loading because candidates come from the original query result.
- Empty result sets with column metadata still provide candidates.

## Verification

- Existing relation completion tests remain passing.
- SQL result completion tests cover matching an output alias, accepting candidates in both inputs, and using result columns without rows.
- SQL result rendering tests verify the popup is anchored and stays inside the terminal viewport.
