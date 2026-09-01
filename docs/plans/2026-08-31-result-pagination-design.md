# Result Pagination Design

## Goal

Replace the fixed 500-row relation-preview and eligible SQL-result cap with a
shared, DataGrip-style pagination experience. Users can move to the first,
previous, next, or last page and choose 10, 100, 250, 500, or 1000 rows per
page. The default remains 500 rows.

Initial and sequential page loads must not pay the cost of an exact count.
Until the end is observed, the footer reports a lower bound such as
`1-500 of 501+`. Requesting the last page computes and caches the exact total,
after which the footer reports values such as `2762001-2762388 of 2,762,388`.

## Scope

Pagination applies to both result-producing surfaces:

- relation Data previews opened from the Explorer;
- SQL Console results produced by one safely parseable, read-only `SELECT`.

The SQL Console result-filter query is part of the paginated query. A submitted
`WHERE` or `ORDER BY` expression resets pagination to the first page and
invalidates an exact total derived from the previous query identity.

The following results retain their current execution and rendering behavior
without pagination:

- multi-statement execution;
- DML, DDL, transaction control, and other non-read-only statements;
- lock-bearing queries;
- queries that cannot be parsed and safely wrapped;
- stored-procedure or other multi-result-set output.

## Existing Architecture

`RELATION_PREVIEW_LIMIT` is currently fixed at 500. The SQLite, PostgreSQL, and
MySQL relation adapters append `LIMIT 500` directly. Eligible SQL Console
queries are wrapped by `sql::bounded_query`, while filtered SQL results are
built by `sql::build_derived_query`; both paths also hard-code the same cap.

`RelationRequest` contains relation identity, generation, connection, scope,
and filter options, but no page request. `RelationPreview` contains SQL and a
`QueryOutcome`, but no page metadata. `ConsoleTab` and `DerivedResultState`
similarly store outcomes without pagination state. The relation footer renders
`[500 row limit]` and the loaded row count rather than interactive navigation.

The existing request generations, cancellation commands, previous snapshots,
and stale-response checks are suitable for page loading and should remain the
concurrency boundary.

## Chosen Approach

Use offset pagination with a one-row lookahead for ordinary page requests:

```sql
LIMIT page_size + 1 OFFSET offset
```

Only `page_size` rows enter the displayed `ResultSet`. The additional row is a
probe that establishes whether another row exists. This avoids a full count on
initial load and normal next/previous navigation while preserving direct first
and last-page controls for relations and arbitrary eligible read queries.

When the user requests the last page, execute an exact count first, derive the
last page offset, and then load that page. Cache the exact count only while the
query identity remains unchanged.

Offset pagination is preferred over keyset pagination for this feature because
the same behavior must support views, aggregates, set queries, user limits,
and arbitrary projected columns. Keyset pagination requires a stable unique
key that these results do not necessarily expose and cannot cheaply implement
direct last-page navigation.

`COUNT(*) OVER()` is rejected because it makes every ordinary page load pay
the full count cost and would remove the intended lower-bound behavior.

## Shared Pagination Model

Pagination is domain state, not transient renderer state. Add a shared model
used by relation and SQL result owners:

```rust
const DEFAULT_PAGE_SIZE: PageSize = PageSize::FiveHundred;

enum PageSize {
    Ten,
    OneHundred,
    TwoHundredFifty,
    FiveHundred,
    OneThousand,
}

enum TotalRows {
    LowerBound(u64),
    Exact(u64),
}

struct ResultPagination {
    page_size: PageSize,
    offset: u64,
    visible_rows: usize,
    has_next: bool,
    total: TotalRows,
}
```

`PageSize` is a closed enum so invalid or excessive values cannot cross the
App-to-Runtime boundary. It provides checked conversions for the SQL limit,
lookahead limit, offsets, and display ranges. The maximum requested row count
is 1001 including the probe.

The model derives its display range and navigation capabilities:

- first page is enabled when `offset > 0`;
- previous page is enabled when `offset > 0`;
- next page is enabled when `has_next` is true;
- last page is enabled when the result is not already known to be on the last
  page;
- an empty result displays `0-0 of 0`;
- a non-empty page displays one-based inclusive indexes;
- all totals and indexes use grouped decimal formatting in the UI.

`LowerBound` is the minimum total established by loaded rows. A full page with
a probe establishes `offset + page_size + 1`. A short page establishes the
exact total `offset + visible_rows`, so naturally reaching the end changes the
state to `Exact` without a count query.

An exact total is not part of the database result model globally. It belongs to
the current paginated result identity and is invalidated whenever that identity
changes.

## Query Identity And Reset Rules

Pagination begins at offset zero with page size 500 for a newly created tab.
The selected page size remains tab-local after subsequent queries, but every
new query starts at the first page.

The query identity includes all inputs that affect the result rows:

- connection identity and execution target;
- relation catalog identity or original SQL execution draft;
- submitted relation/SQL `WHERE` and `ORDER BY` options;
- tab and query generation;
- transaction generation where applicable.

The current page resets to zero and the exact total is discarded after:

- executing new base SQL;
- submitting or clearing result filters;
- refreshing a relation or SQL result;
- changing the relation object, execution target, connection, or scope;
- a successful relation mutation commit followed by refresh.

Changing page size also returns to the first page. If no query-identity input
changed, an already exact total may be retained because page size does not
change the underlying result set.

## Relation Preview Queries

Extend `RelationRequest` with a validated page request. The adapters continue
to verify catalog identity and scope, quote identifiers, and validate filter
fragments before constructing SQL.

An ordinary page request is:

```sql
SELECT *
FROM <verified relation>
WHERE <submitted condition>
ORDER BY <submitted ordering>
LIMIT <page_size + 1> OFFSET <offset>
```

The exact count excludes ordering because ordering cannot affect cardinality:

```sql
SELECT COUNT(*)
FROM <verified relation>
WHERE <submitted condition>
```

The count and page query execute in one Runtime task and use the same acquired
connection. They are not required to open a transaction in auto-commit mode.
The returned preview includes page metadata and only visible rows; the probe is
never exposed to editing, record view, clipboard operations, or row statistics.

## SQL Console Queries

Only a single read-only query accepted by the existing derived-result parser is
paginated. Normalize the source by removing a trailing semicolon, then preserve
the complete user query as an inner semantic boundary:

```sql
SELECT *
FROM (<user SELECT>) AS __lazydb_page
LIMIT <page_size + 1> OFFSET <offset>
```

The count query is:

```sql
SELECT COUNT(*)
FROM (<user SELECT>) AS __lazydb_count
```

An existing user `LIMIT`, `OFFSET`, or `FETCH` remains inside the derived table.
The exact total therefore describes the result of the user's query, including
their own cap, rather than the underlying base tables.

For SQL result filtering, first build the semantic filtered result and then
wrap it for pagination:

```sql
SELECT *
FROM (
    SELECT *
    FROM (<user SELECT>) AS __lazydb_result
    WHERE <result condition>
    ORDER BY <result ordering>
) AS __lazydb_page
LIMIT <page_size + 1> OFFSET <offset>
```

The implementation may omit a redundant wrapper where the AST builder can
prove equivalent semantics, but correctness and one shared builder take
priority over shorter generated SQL.

SQL execution keeps `LastExecution.draft.sql` as the original user source.
Generated page SQL is request data, not a replacement for provenance or the
editor source. Manual-transaction pagination continues through the transaction
worker so all pages observe that console's pinned connection and transaction.

## Last-Page Flow

Last-page navigation is a distinct request intent, not a UI-side guessed
offset:

1. App validates that the current result is pageable and not blocked by edits.
2. App dispatches a last-page request bound to the current query generation.
3. Runtime obtains an exact count using the relation or SQL count builder.
4. Runtime computes `((total - 1) / page_size) * page_size`, or zero when total
   is zero, using checked arithmetic.
5. Runtime loads that offset with the ordinary one-row lookahead query.
6. App accepts the response only if its complete request identity is current.
7. App stores `Exact(total)` and renders the corrected page range.

Concurrent inserts or deletes can occur between count and page execution in
auto-commit mode. If the computed last page returns no rows for a non-zero
total, Runtime recounts and retries once. A second mismatch returns the newest
available page and marks the count as a lower bound rather than looping.

Ordinary next-page navigation that returns a short page establishes an exact
total without a count. An unexpected empty next page moves back to the last
non-empty page and records that boundary as exact for the accepted generation.

## Interaction Design

Render one shared pagination bar as the final row of each pageable data panel.
Its logical layout is:

```text
|<  <  1-500 v  of 501+  >  >|  ...
```

The controls are:

- first page;
- previous page;
- current inclusive range;
- page-size selector attached to the range;
- total or lower bound;
- next page;
- last page;
- existing result actions or provenance information when space permits.

The page-size selector opens a small single-choice overlay with 10, 100, 250,
500, and 1000. The active value is highlighted. Mouse hit regions dispatch the
same actions as keyboard activation.

Pagination actions are available from the result grid without taking over
existing Vim row-navigation keys. The first implementation adds explicit
actions and mouse controls; contextual keyboard hints may expose non-conflicting
bindings separately. When the selector is open, Up/Down moves, Enter applies,
and Esc closes it.

On a narrow terminal, preserve the range, total, previous, and next controls.
Hide, in order, provenance/action details, last/first controls, and the visible
page-size marker. Hidden controls remain available through the selector/action
system; text must be clipped rather than allowed to overwrite the grid.

While a page request runs, retain the previous snapshot and show the existing
loading treatment. Disable repeated actions that target the same page. A newer
page action cancels the prior request where the current runtime path supports
cancellation, and stale responses cannot overwrite the accepted page.

After a successful page change:

- select the first visible row and current valid column;
- reset vertical `row_offset` to zero;
- preserve valid manual column widths and horizontal position;
- close record view because its page-local row identity is no longer valid.

## Editing And Transaction Rules

Relation editing remains page-local. Cross-page edit accumulation is not part
of this feature.

First/previous/next/last navigation and page-size changes are blocked while the
relation has an active mutation transaction, an executable insert draft, or any
uncommitted edit state. The UI reports: `Commit or roll back relation changes
before changing pages`.

No pagination action silently rolls back changes. After commit, the existing
refresh flow returns to the first page and invalidates the previous exact total.
After rollback, the current page remains available and pagination can resume.

SQL Console manual transactions do not block pagination. Their generated page
and count requests execute on the existing pinned transaction worker and obey
its running, cancellation, aborted, and outcome-unknown states.

## Ordering And Consistency

Pagination does not invent an ordering. SQL databases do not guarantee stable
cross-page row membership without a deterministic `ORDER BY`, and automatically
adding a primary key would change relation query performance and semantics while
remaining impossible for views and arbitrary SQL.

The result UI and help text should state that stable paging requires a unique
ordering when no `ORDER BY` is submitted or present. This is guidance, not a
blocking error.

Exact counts are observations tied to a query generation, not durable database
facts. Auto-commit pages can change under concurrent writes. Refresh invalidates
the count. Manual transaction pages use the transaction's visibility rules.

## Error Handling

| Condition | Behavior |
| --- | --- |
| Page query fails | Retain the previous page, show the sanitized error, and allow retry. |
| Count query fails | Retain the current page and lower-bound total; report that last page could not be resolved. |
| Stale response | Ignore it using tab, query, request, connection, and transaction generations. |
| Offset/limit overflow | Reject before database I/O as an internal pagination error. |
| Empty result | Display `0-0 of 0` and disable all navigation. |
| Concurrently emptied last page | Recount and retry once, then return the newest safe state. |
| Unsupported SQL result | Render the result normally with disabled pagination and a concise reason. |
| Active relation edits | Do not dispatch; prompt for commit or rollback. |

## Performance Characteristics

Initial and sequential requests fetch at most 1001 rows and do not count the
complete result. Exact counting occurs only when the user asks for the last
page. Count SQL excludes an outer `ORDER BY` where doing so is structurally safe;
the implementation must not remove ordering inside a user query when it affects
`LIMIT`, window functions, or other semantics.

Deep offsets can be expensive because the database may scan and discard earlier
rows. This is an accepted trade-off for a uniform first/last-page experience.
Telemetry is not added in this feature. If real workloads show unacceptable
relation-preview latency, a later design may add keyset navigation only for
relations with a proven unique ordering while preserving this model as the
fallback.

## Testing Strategy

Pure pagination-model tests cover:

- all allowed page sizes and the default;
- lookahead trimming and lower-bound calculation;
- short, full, and empty pages;
- first/previous/next/last capability derivation;
- exact last offsets for divisible and non-divisible totals;
- checked arithmetic and grouped display ranges;
- page-size changes and query-identity resets.

SQL builder tests cover SQLite, PostgreSQL, and MySQL syntax for:

- relation pages and counts with and without filters/orderings;
- plain, CTE, aggregate, and set SELECT queries;
- existing user `LIMIT`, `OFFSET`, and `FETCH` semantics;
- filtered derived results;
- trailing semicolons;
- rejection of unsafe, locked, multi-statement, and non-query SQL.

Reducer and runtime tests cover:

- request identity and stale page/count responses;
- cancellation when navigation changes rapidly;
- exact-count caching and invalidation;
- manual-transaction routing;
- count failure preserving the current page;
- one-time correction after concurrent last-page shrinkage;
- relation-edit navigation blocking.

Rendering and input tests cover:

- lower-bound and exact footer text;
- empty and one-page results;
- enabled/disabled styles and hit regions;
- page-size selector lifecycle;
- narrow-terminal degradation;
- relation and SQL result parity;
- grid and record-view reset after navigation.

SQLite integration tests provide the default end-to-end path. Adapter contract
tests verify generated page and count SQL for PostgreSQL and MySQL, with live
driver tests remaining environment-gated.

Repository-wide verification is:

```bash
cargo fmt --check
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
```

## Delivery Sequence

1. Add the shared pagination model and pure state tests.
2. Replace fixed SQL caps with shared safe page/count builders.
3. Add relation request/result pagination through all three adapters.
4. Add SQL Console base and filtered-result pagination, including manual transactions.
5. Add App actions, reset/invalidation rules, cancellation, and edit guards.
6. Add the shared footer, selector, hit regions, and responsive rendering.
7. Complete runtime, adapter, rendering, and end-to-end regression tests.
8. Remove obsolete fixed-cap labels and update user-facing help/documentation.

## Explicit Non-Goals

- Keyset pagination in the first release.
- Arbitrary page-number entry or jumping to an offset other than first/last.
- Background or eager exact counts.
- Guaranteed snapshot consistency across auto-commit page requests.
- Automatic ordering by primary key or hidden database row identifiers.
- Cross-page relation editing, selection, clipboard accumulation, or record view.
- Pagination of unsafe, multi-statement, or multi-result-set SQL execution.
- Persisting page position or exact totals across application restarts.

## Superseded Behavior

This design supersedes the user-visible fixed-cap behavior in
`2026-08-28-select-query-row-limit-design.md`. The safety objective remains:
eligible result queries are bounded before rows enter the UI. The fixed 500-row
cap becomes a default 500-row page with a maximum selectable page size of 1000
and a one-row lookahead.
