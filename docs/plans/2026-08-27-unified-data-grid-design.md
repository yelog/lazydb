# Unified Data Grid Design

## Goal

Use one Data Grid component for SQL console results and relation previews. Both
surfaces receive identical column sizing, horizontal and vertical navigation,
selection, mouse hit testing, and query-bar behavior. Their data-source
controllers remain separate so relation catalog identity and SQL execution
semantics stay correct.

## Current State

Both DATA surfaces render the same `ResultSet` model but duplicate table headers,
cell preview and styling, selected-cell state, mouse regions, and viewport logic.
Relation DATA additionally provides content-aware widths, horizontal column
visibility, width overrides, WHERE/ORDER BY inputs, request attribution, and a
hard 500-row preview limit. SQL DATA uses equal widths and stores row/column
selection as separate console fields.

The common component boundary is `ResultSet`, not `RelationTab`. Relation tabs
also own catalog keys, scope checks, request identity, retained snapshots,
structure metadata, and provenance; these do not belong in a generic grid.

## Shared Model

Replace the current split state with:

```text
DataGridState
  selected_row
  selected_column
  row_offset
  column_offset
  column_widths: Vec<Option<u16>>
```

Both `ConsoleTab` and `RelationTab` own `DataGridState`. Selection is clamped
whenever the displayed result changes. Column overrides are aligned to the
current result's columns and stale overrides are removed.

Pure helpers calculate content-aware automatic widths, visible columns, visible
rows, selection-driven viewport movement, and mouse coordinates. Width
calculation sanitizes headers, measures Unicode display cells, previews bounded
values, and clamps automatic widths to `6..40`; explicit widths clamp to
`6..80`.

## Shared Renderer

Create `src/ui/data_grid.rs`. It renders:

- sanitized headers and values;
- NULL and Unsupported styles;
- content-aware automatic widths;
- explicit width overrides;
- horizontal and vertical viewports;
- keyboard/mouse selected cells;
- cell and column-boundary hit regions;
- zero-row result sets that retain columns;
- affected-row results with no columns;
- an optional title/footer supplied by the source controller.

SQL and relation renderers prepare a `DataGridView` and delegate all table work.
Relation status/provenance and SQL result tabs/stats remain outside the component.

## Shared Input

Rename relation-specific width actions to grid actions:

```text
GridMove
GridSelect
GridResizeColumn
GridResetColumnWidth
GridStartColumnResize
GridSetColumnWidth
GridEndColumnResize
```

Both DATA surfaces support `[`/`]` resize, `=` reset, selection-driven scrolling,
wheel navigation, cell clicks, and column-boundary dragging. App routes actions
to the active tab's grid state and current displayed `ResultSet` dimensions.

## Query Bar

Generalize the relation query state:

```text
DataQueryState
  where_input
  order_by_input
  submitted
  focus
  error
  capability
```

The same query-bar renderer and input actions are used by both DATA surfaces.
Capability determines submission behavior:

- Relation: regenerate the verified relation preview request.
- Derived SQL: execute a derived read-only query.
- Unavailable: render a bounded reason and reject editing.

The existing fragment validation becomes generic. WHERE and ORDER BY fragments
must parse in the active dialect and cannot contain comments, statement
separators, limits, fetch clauses, or locks.

## SQL Derived Queries

For a safe SQL result, submitting the query bar executes:

```sql
SELECT *
FROM (
  <immutable executed SQL>
) AS __lazydb_result
WHERE <where fragment>
ORDER BY <order fragment>
LIMIT 500
```

The source is the immutable execution draft retained at execution time, never
the current mutable editor buffer.

Derived filtering is enabled only when all of these hold:

- the latest source execution succeeded;
- exactly one statement was executed;
- the statement is a fully parsed, read-only query;
- it is not EXPLAIN, DML RETURNING, transaction control, procedure/function
  invocation, or a lock-bearing query;
- connection identity and execution target still match;
- transaction state permits safe replay;
- the active database adapter supports the derived-table shape.

Multi-statement outcomes and other unsafe sources show a disabled reason instead
of accepting input.

The wrapper is built through dialect-aware SQL helpers and the final complete SQL
is parsed again before dispatch. User fragments cannot override the outer
500-row limit.

## Base and Derived Results

SQL console DATA keeps the original successful outcome as `base`. A submitted
derived query produces a separate displayed outcome. Clearing both clauses
immediately restores `base` without database I/O.

Derived requests carry console UUID, a dedicated result generation, source
execution generation, connection identity, exact target, and immutable derived
SQL. A late derived response cannot overwrite a newer source execution, another
derived request, a target change, or a disconnected console.

Running F5 always creates a new base result, clears prior derived results, resets
submitted query-bar options, clamps the grid, and recomputes derived capability.

## Multiple Result Sets

Add `active_result_set` to SQL console result state. Query completion selects the
last result set containing columns or rows, otherwise the final statement
result. The UI displays `RESULT n/m` and allows result-set navigation.

Derived WHERE/ORDER BY is disabled for multi-statement execution because the
mapping between a displayed result set and a safely replayable source statement
is not authoritative. Grid state is clamped when switching result sets.

## Relation Boundary

Relation previews continue to own:

- `RelationKey` and descriptor;
- active profile/scope verification;
- runtime known-relation verification;
- request generation and stale-response rejection;
- retained snapshot behavior during load/failure/cancel;
- adapter-generated quoted relation SQL;
- structure view and snapshot provenance;
- the hard 500-row limit.

Only their grid and query-bar presentation/state primitives become shared.

## Error Handling

- Invalid fragments remain editable and display a sanitized bounded error.
- Derived-query database failures retain the base result and query drafts.
- Clearing clauses restores base even after a derived failure.
- Duplicate SQL result column names may produce a database ambiguity error; the
  first version does not silently rewrite names or ordinals.
- Missing/changed connection targets disable replay rather than reconnecting to
  a different namespace.
- Grid state never indexes outside the current rows or columns.

## Delivery Phases

1. Extract shared `DataGridState`, helpers, renderer, and generic width actions.
2. Move Relation DATA and SQL DATA to the shared grid without changing queries.
3. Extract the shared query bar and preserve Relation behavior.
4. Add SQL derived-query capability analysis, execution, and base/derived state.
5. Add multiple result-set selection and polish disabled reasons/completion.

Each phase must leave all existing SQL and relation behavior usable and tested.

## Testing

- SQL and relation DATA produce identical table geometry and styling for the
  same `ResultSet`.
- Automatic widths, horizontal overflow, explicit resize/reset, Unicode, NULL,
  Unsupported, empty rows, and no-column outcomes work on both surfaces.
- Vertical scrolling followed by mouse selection targets the displayed row.
- Query-bar editing and fragment validation are shared.
- Relation requests preserve catalog identity, scope, attribution, and stale
  rejection.
- Safe single read-only queries can execute derived WHERE/ORDER BY wrappers.
- Multi-statement, mutation, EXPLAIN, lock, procedure, and stale-target sources
  are disabled with stable reasons.
- Derived requests enforce 500 rows and stale results cannot overwrite newer
  base or derived results.
- Clearing clauses restores base without another command.
- Result-set selection shows the selected set's stats and clamps grid state.
