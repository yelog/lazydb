# Explorer Frontend Catalog Search Design

**Status:** Approved

**Date:** 2026-08-28

## Summary

Explorer `f` search becomes an immediate frontend filter over the authoritative
`CatalogTree`. Search no longer builds a second catalog tree from server search
hits or uses a separate visual language. The filtered projection preserves the
normal Explorer hierarchy and row presentation while omitting unrelated branches
and every relation child.

Catalog loading continues asynchronously. Already loaded objects are searchable
immediately, additional searchable object groups are loaded in the background,
and active results update as catalog pages arrive.

## Goals

- Make query edits synchronous and independent of database round trips.
- Render search results with the same labels, icons, colors, indentation, and
  metadata as the normal Explorer tree.
- Treat search as a filtered view of the normal catalog rather than another tree.
- Exclude columns and all other relation children from search.
- Treat relations as leaves in search results even when their children are loaded
  and expanded in the normal tree.
- Preserve broad catalog coverage by preloading searchable non-relation groups in
  the background.
- Locate the selected normal-tree object and close search on Enter.

## Search Scope

Searchable kinds are database, schema, table, view, materialized view, function,
procedure, sequence, type, and non-relation-owned trigger. Profile and presentation
group rows may appear only as ancestors. Status, empty, and pagination rows do not
participate.

Column, index, constraint, and relation-owned trigger entries are excluded. More
generally, an entry whose owning relation differs from itself is a relation child
and is not indexed or projected. A relation match is always a leaf in the search
projection.

## Architecture

`CatalogTree` remains the only catalog data model. The normal and filtered views
both return normal Explorer row identities:

```text
CatalogTree
|- visible()                 normal expansion-aware projection
`- filtered_visible(query)   ancestor-preserving search projection
```

The filtered projection finds matching catalog entries, marks their catalog
ancestors and presentation groups as included, and flattens only those paths.
Logical expansion in this projection does not mutate `ExplorerTreeState::expanded`.
Relations terminate traversal, so loaded relation children never appear.

Search state retains the query, phase, filtered row IDs, matching node IDs,
selection, scroll, and the original normal-tree selection and scroll. It no longer
stores `CatalogSearchHit` values or `ExplorerCatalogSearchRow` objects.

The first implementation may linearly scan cached catalog entries. Normalized
labels and qualified paths should be cached only if profiling shows repeated
lowercasing is material; no new indexing dependency is required.

## Data Flow

```text
f -> open frontend search -> filter current CatalogTree
query edit -> synchronously recompute filtered rows and matches
catalog page accepted -> recompute active filtered search
Enter -> expand normal ancestor/group path, select object, close search
Esc -> close search and restore original selection/scroll
```

No query edit emits `Command::SearchCatalog`. Existing server search contracts may
remain temporarily unused and can be removed separately after call sites are
verified.

## Background Loading

Connection catalog discovery already loads databases, schemas, groups, and the
object groups used by SQL completion. Extend automatic object loading to every
adapter-supported group that can contain searchable non-relation objects,
including sequences, types, and triggers where supported. Relation children remain
on-demand and are never preloaded for search.

Search opens immediately while background loading continues. Existing results are
usable, new matching objects appear without stealing selection, and the status line
shows `Indexing catalog...` while relevant catalog owners are pending. A target-
local load failure retains current results and exposes the existing retry path.

## Rendering

Normal and search modes share one row-rendering path. It resolves every row from
the normal Explorer model and applies the same profile label, expansion marker,
icon, kind color, selected background, metadata, comment, and indentation. Search
adds only match highlighting and its input/status lines.

Search-specific projection semantics control expansion markers: included namespace
and group ancestors are open, while relation rows are leaves. This does not change
their normal-tree expansion state.

## Interaction

- `f` opens search and focuses the query.
- Printable input, Backspace, and Ctrl-U update results immediately.
- `j`/`k`, arrows, Home, and End navigate filtered rows.
- `n`/`N` after confirmation navigate matching catalog objects only.
- Enter expands the object's normal Profile/Database/Schema/Group path, selects and
  centers it, closes search, and never expands the relation itself.
- Esc closes search and restores the pre-search normal selection and scroll.

## Error Handling

- Empty queries perform no matching work and show an instructional state.
- Connection changes invalidate search.
- Catalog mutations remove missing matches and retain the selected stable ID when
  possible; otherwise selection clamps to the nearest surviving row.
- Background failures never clear already indexed results.
- Search completeness is explicit while relevant catalog requests are pending.

## Testing

- Model tests compare normal and filtered row identities and hierarchy.
- Projection tests cover ancestor retention, sibling omission, stable order, and
  deduplication.
- Relation-child tests exclude columns, indexes, constraints, and owned triggers,
  and verify expanded relations remain leaves.
- Reducer tests assert query edits emit no search command and catalog page arrival
  refreshes active results without stealing stable selection.
- Keymap tests cover editing, result navigation, Enter-close, and Esc restoration.
- UI tests verify normal and filtered rows share presentation and show indexing.
- A large synthetic catalog test covers frontend filtering with at least 10,000
  objects.
