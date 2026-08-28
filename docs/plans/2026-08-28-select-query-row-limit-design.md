# SELECT Query Row Limit Design

## Goal

Prevent large read queries from taking a long time to fetch and from freezing
the terminal UI while rendering results. Single-statement, read-only `SELECT`
queries should use the same 500-row limit as relation previews.

## Behavior

- Reuse `RELATION_PREVIEW_LIMIT` (`500`) as the single source of truth.
- Apply the limit only to a single, read-only `SELECT` query.
- Preserve a user-supplied `LIMIT`; do not add a second limit.
- Leave `INSERT`, `UPDATE`, `DELETE`, DDL, multi-statement SQL, and other
  non-query statements unchanged.
- Keep the existing relation preview limit unchanged.
- Keep result navigation limited to rows actually loaded in the result set.

## Implementation Direction

Use the existing SQL parser and read-only query classification to determine
whether a query can be limited. Prefer transforming eligible SQL into a
derived query so the original query's ordering, filtering, CTEs, and existing
limit remain intact. The transformation must be dialect-compatible for
SQLite, MySQL, and PostgreSQL and must handle trailing semicolons safely.

If a query cannot be safely transformed into a derived query, it should not be
silently changed. The implementation should either leave it unchanged or
reject it explicitly according to the existing SQL validation behavior.

## User Feedback

The output/result UI should make the 500-row cap discoverable, especially when
the returned result reaches the cap, while continuing to report the number of
rows actually loaded.

## Testing

- Verify eligible `SELECT` statements receive the 500-row cap.
- Verify existing `LIMIT` values are preserved and not duplicated.
- Verify non-query and multi-statement SQL are not modified.
- Verify semicolon and dialect-specific read query cases.
- Verify result collection and UI row navigation operate on the bounded set.
