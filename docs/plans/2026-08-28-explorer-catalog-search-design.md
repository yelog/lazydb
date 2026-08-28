# Explorer Catalog Search Design

**Status:** Approved

**Date:** 2026-08-28

## Summary

When Explorer has focus, `/` opens an inline search mode that searches every
catalog object visible to the active connection, including objects that have not
been expanded or loaded by normal Explorer pagination. Search is case-insensitive
and matches both the object name and its qualified path.

Search results are a temporary projection, not another Explorer subtree. Pressing
Enter locates the selected hit by merging its object and required ancestors into
the normal tree and expanding that path. Search remains open so the user can keep
browsing results; Esc closes search and leaves the located object selected.

## Goals

- Search all actual objects in the active connection's configured catalog scope.
- Include database, schema, relation, routine, type, sequence, column, index,
  constraint, and trigger kinds supported by each adapter.
- Find objects that normal lazy loading and pagination have not materialized.
- Preserve the current reducer/runtime/adapter architecture and stale-response
  protections.
- Keep the interaction fast, keyboard-first, and usable at 80x24.

## Non-goals

- Searching inactive profiles or outside the saved catalog scope.
- Searching Explorer groups, status rows, empty rows, or pagination controls.
- Fuzzy matching, regular expressions, search history, or query syntax.
- Adding search hits to SQL completion.

## Confirmed Interaction

- `/` enters search while Explorer has focus.
- Printable characters edit the query; Backspace deletes and Ctrl-U clears it.
- `j/k`, arrows, Home, and End navigate results.
- Enter locates the selected result while keeping search open.
- Esc exits search and returns to the located normal-tree selection.
- Matching is case-insensitive across object names and qualified paths.
- Empty queries do not execute database work.

## Architecture

Search uses a dedicated catalog contract rather than overloading normal paged
tree requests:

```rust
struct CatalogSearchRequest {
    connection: ConnectionIdentity,
    generation: u64,
    query: String,
    scope: CatalogScope,
    limit: usize,
}

struct CatalogSearchHit {
    entry: CatalogEntry,
    ancestors: Vec<CatalogEntry>,
}

struct CatalogSearchPage {
    hits: Vec<CatalogSearchHit>,
    total_count: Option<usize>,
    truncated: bool,
}
```

The exact implementation may use an adapter method whose arguments are equivalent.
Every request and result carries enough connection and generation identity to
reject responses from an old connection or superseded query.

The existing unidirectional flow remains authoritative:

```text
Key input -> Action -> App::update -> Command -> Runtime -> DatabaseConnection
Database result -> Action -> App::update -> Explorer search projection
```

Search state is independent from `CatalogTree` and contains the query, lifecycle,
generation, hits, selected result, scroll position, and last located node. A short
debounce prevents one database query per keystroke. A new query keeps previous
results visible while loading.

## Adapter Contract

Each concrete adapter pushes the configured `CatalogScope` and a case-insensitive
name/path predicate into native catalog queries. Results are bounded to 100 rows
and ordered by relevance: exact object-name match, object-name prefix, object-name
substring, then qualified-path substring. Stable qualified path is the tie-breaker.

PostgreSQL searches the configured database's visible schemas through `pg_catalog`
and `information_schema`. MySQL searches permitted schemas through
`information_schema`. SQLite searches permitted attached databases through
`sqlite_schema` and relevant PRAGMA metadata for relation children.

Every hit contains the real catalog entry plus the database/schema/relation
ancestors required for navigation. Presentation groups remain Explorer concepts
and are not emitted as fake native catalog entries.

## Tree Location

Search results do not mutate normal tree loading or completion state. On Enter:

1. Validate that the hit still belongs to the active connection and scope.
2. Insert or update its real ancestors and object in `CatalogTree`.
3. Restore required group state without marking an incomplete normal page complete.
4. Expand the profile, catalog ancestors, and presentation group path.
5. Select the hit's stable `ExplorerNodeId` and make it visible.
6. Keep the search projection open and mark that hit as located.

Esc removes only the search projection. The normal tree remains at the located
object.

## UI/UX

Search replaces the Explorer panel's inner projection without opening an overlay:

```text
 / user
 > users                  public.users
   user_id       public.users.user_id
   user_roles       auth.user_roles
 3 results      Enter locate   Esc close
```

The first line is always the input. Results are flat for scan speed: the object
name is primary text and its qualified path is secondary text. Existing catalog
kind icons and colors are reused. A non-color marker identifies the located hit.
Narrow layouts preserve the object name and truncate the qualified path first.

Lifecycle states are explicit:

- Empty query: `Type to search all objects`.
- Loading: keep old results and show `Searching...`.
- Empty result: `No objects match "query"`.
- Truncated: `100+ results - refine your search`.
- Failure: sanitized contextual error plus `r retry`.

## Error and Concurrency Handling

- Connection switch and disconnect cancel or invalidate active searches.
- Query edits increment search generation; older responses are ignored.
- Permission errors are distinct from no matches.
- Errors never clear the normal Explorer tree.
- Adapter text and errors are sanitized before rendering.
- Empty query and inactive connection produce no runtime command.

## Testing

- Model tests cover query editing, navigation, lifecycle, stale generations, tree
  location, expansion, and selection retention.
- Keymap tests cover `/`, printable input, Backspace, Ctrl-U, navigation, Enter,
  Esc, retry, and focus isolation.
- Reducer/runtime tests cover debounce, request identity, cancellation/invalidation,
  stale responses, and connection changes.
- Adapter tests cover case-insensitive name/path matching, scope pushdown, all
  supported actual kinds, ordering, limits, ancestors, and safe qualification.
- UI tests cover empty, loading, ready, empty-result, truncated, failed, and
  located states at compact and standard terminal sizes.
- A large synthetic catalog test verifies search does not first materialize the
  complete normal Explorer tree.

## Acceptance Criteria

- `/` in Explorer opens inline search and an empty query performs no database work.
- Objects outside loaded pages are searchable within the active profile scope.
- Every adapter returns all actual object kinds it truthfully supports.
- Group and status rows never appear as hits.
- Old connection and old query responses cannot replace current results.
- Enter locates and expands a hit without closing search or corrupting pagination.
- Esc returns to the normal tree with the located object selected.
- Search remains usable at 80x24 and does not expose unsanitized database text.
