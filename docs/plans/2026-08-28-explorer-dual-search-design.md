# Explorer Dual Search Design

**Status:** Approved

**Date:** 2026-08-28

## Summary

Explorer provides two distinct search interactions:

- `/` performs a local find over the rows visible when find mode opens. It does
  not load, expand, or search hidden nodes. Matches remain in the normal tree,
  matching text is highlighted, and the status displays the current match and
  total count. After Enter confirms the query, `n` and `N` cycle through matches.
- `f` performs the existing server-backed catalog search across the active
  connection, including objects that normal lazy loading has not materialized.
  Results are rendered as a temporary tree containing each match and its ancestor
  path, rather than as a flat result list.

The two interactions use separate state because they have different data sources,
lifecycles, and effects. They share text matching, highlighting, match counters,
and two-phase keyboard semantics where practical.

## Goals

- Make `/` behave as an immediate, zero-I/O find over the currently visible tree.
- Never let `/` discover descendants hidden by a collapsed node or unloaded page.
- Highlight every matching label and show `(current/total)`.
- Let Enter confirm a query and `n`/`N` cycle through results with wraparound.
- Move the current all-catalog search behavior to `f`.
- Preserve hierarchy, indentation, icons, and match highlighting in `f` results.
- Reuse the existing adapter search contracts, debounce, cancellation, and stale
  response protection.
- Keep normal lazy-tree expansion, pagination, selection, and scroll state isolated
  from the temporary `f` result projection.

## Non-goals

- Fuzzy matching, regular expressions, search history, or query syntax.
- Searching secondary row metadata, endpoints, comments, or status details with
  `/`.
- Changing the adapter-specific PostgreSQL, MySQL, or SQLite catalog search SQL.
- Materializing a complete normal Explorer tree before either search mode.
- Persisting search sessions across connection changes or application restarts.

## Confirmed Interaction

### Visible Find (`/`)

- `/` opens find editing while Explorer has focus.
- The searchable rows are the normal tree rows visible through expansion at the
  moment find opens, not merely the rows inside the terminal viewport.
- Matching uses each row's primary label only, case-insensitively.
- Profile, database, schema, presentation group, object, status, empty, and load
  rows may match when their primary label matches.
- Hidden descendants and objects absent from loaded pages cannot match.
- Printable characters edit the query; Backspace deletes and Ctrl-U clears it.
- During editing, all matching label fragments are highlighted and the counter is
  updated without database work.
- Enter confirms the query and selects the first match.
- In confirmed mode, `n` selects the next match and `N` selects the previous match;
  both wrap at the ends.
- Esc during editing cancels find and restores the selection from before `/`.
- Esc after confirmation clears the find highlights while retaining the current
  selected node.
- Clearing find restores the normal meaning of `n`, which creates a profile.

### Catalog Search (`f`)

- `f` opens server-backed catalog search while Explorer has focus.
- Search continues to cover every actual object in the active connection's
  configured scope, including unloaded objects and relation children.
- Matching remains case-insensitive over object names and qualified paths.
- Results retain each hit's profile, catalog ancestors, and presentation group.
- Shared ancestors are deduplicated, non-matching sibling branches are omitted,
  and all paths containing a hit are expanded in the temporary result projection.
- Matching labels are highlighted.
- Search input uses an editing phase so `n` and `N` remain valid query characters.
- Enter confirms the query; `n` and `N` then cycle through actual matching nodes.
- Arrow keys and `j`/`k` may navigate all visible result-tree rows.
- Enter on an actual result may locate it in the normal tree using the existing
  merge-and-expand behavior.
- Esc closes catalog search and cancels an outstanding request.
- Closing without locating restores the original normal-tree selection, scroll,
  and expansion. Closing after locating retains the located normal-tree path.

## Architecture

### Separate State Machines

Local find and catalog search must not share a single state containing optional
fields. A local find is synchronous and snapshot-based, while catalog search is
asynchronous, connection-bound, and generation-checked.

The Explorer state gains a dedicated local find:

```rust
struct ExplorerFindState {
    phase: ExplorerSearchPhase,
    query: String,
    rows: Vec<ExplorerFindRow>,
    matches: Vec<ExplorerNodeId>,
    current: usize,
    original_selected: Option<ExplorerNodeId>,
}

struct ExplorerFindRow {
    id: ExplorerNodeId,
    label: String,
}

enum ExplorerSearchPhase {
    Editing,
    Confirmed,
}
```

The existing `ExplorerSearchState` remains the catalog-search state and is renamed
or otherwise made explicit as catalog search where that improves call-site
clarity. It gains the input phase, a temporary hierarchical projection, and the
normal-tree state required for restoration.

Only one Explorer search mode may be active at a time. Opening `/` closes catalog
search and cancels its request. Opening `f` clears local find.

### Visible Snapshot

Opening `/` calls the normal `ExplorerState::visible()` projection once and stores
the stable node ID and primary label for every projected row. This includes rows
outside the terminal viewport but excludes descendants suppressed by collapsed
ancestors and entries absent from loaded pages.

Query edits recompute `matches` from this snapshot. They do not call
`CatalogTree::entries()`, alter `expanded`, or emit a command. Because normal tree
navigation and expansion keys are preempted while editing, the snapshot remains a
truthful representation of what was expanded when find began.

Navigation resolves each stored ID against the current normal projection before
selection. Missing IDs are skipped defensively, although ordinary find input does
not mutate the tree.

### Catalog Result Tree

The database contract already returns:

```rust
struct CatalogSearchHit {
    entry: CatalogEntry,
    ancestors: Vec<CatalogEntry>,
}
```

This is sufficient to construct a temporary result tree. No adapter contract
change is required.

For every hit, the model inserts this logical path into a result-tree builder:

```text
Profile -> catalog ancestors -> presentation group -> hit
```

Presentation groups are derived with the existing object-kind-to-group mapping.
Nodes use stable `ExplorerNodeId` values and are deduplicated by ID. Every ancestor
with a matching descendant is included; unrelated sibling branches are excluded.
All included ancestor paths are considered expanded by the result projection and
do not mutate `ExplorerTreeState::expanded`.

The projection records whether each row is an actual search match. `n` and `N`
navigate only these match rows. Normal result-tree movement may select ancestors
as well.

The result builder uses normal Explorer ordering where available. Its fallback is
a stable ordering by native or qualified path, avoiding response-order-dependent
rendering.

## Data Flow

Visible find is entirely local:

```text
/ -> open find -> snapshot ExplorerState::visible()
query edit -> recompute local matches -> render normal tree with highlights
Enter -> confirm and select first match
n/N -> wrap through stable matching node IDs
```

Catalog search preserves the existing asynchronous boundary:

```text
f -> open catalog search
query edit -> debounce -> Command::SearchCatalog
Runtime -> DatabaseConnection -> CatalogSearchPage
App generation/connection validation -> build result-tree projection
render temporary result tree with highlighted matches
```

Connection switches and disconnects invalidate both modes. Catalog results from an
old connection, session, or generation remain rejected.

## Rendering

### Normal Tree With Find

Local find does not replace `render_explorer` with another list. The normal tree
renderer receives optional find information and splits matching labels into styled
spans.

Style precedence is:

1. Preserve the selected row background.
2. Render the current match fragment with accent color and strong emphasis.
3. Render other match fragments with the action color and emphasis.
4. Render unmatched text with the normal label style.

Every occurrence inside a matching label is highlighted, but the counter and
navigation count matching nodes, not substring occurrences. This gives stable
node-oriented semantics such as `(1/3)`.

The input/status line displays:

```text
/ user                         (1/3)
```

An empty query or a query without matches displays `(0/0)`. Confirmed mode also
advertises `n/N` and Esc when space permits. Narrow layouts truncate guidance
before truncating the query and counter.

### Catalog Result Tree

Catalog search retains its inline input and lifecycle status but renders tree rows
using the normal Explorer visual language:

- depth indentation;
- expanded branch markers;
- profile, group, and catalog icons;
- normal kind colors;
- selected-row background;
- highlighted matching label fragments.

Loading may retain the previous result tree, matching the current behavior. Empty,
failed, and truncated states remain explicit. The counter describes actual hits,
not ancestor rows.

## Keyboard Routing

Explorer input precedence is:

1. Active catalog-search editing or confirmed mode.
2. Active local-find editing or confirmed mode.
3. Normal Explorer bindings.

In editing mode, printable `n` and `N` append to the query. In confirmed local-find
mode, they navigate matches. With no confirmed find, lowercase `n` retains its
existing `ProfileStartNew` action.

`/` opens local find and `f` opens catalog search only in normal Explorer mode.
The Relation Data `/` binding and Editor search bindings remain unchanged.

## Restoration And Mutation

Local find never changes expansion or loading. It changes normal selection only
after confirmation or match navigation.

Catalog search stores the normal selection, scroll, and expansion at entry. Its
temporary result expansion is independent. If no hit is located, closing restores
the stored normal state. If a hit is located, the existing location operation
merges the required real entries and expands the required normal path; that
located state becomes authoritative and is retained on close.

Search result merging must continue to avoid marking incomplete normal pages as
fully loaded or adding search-only entries to completion state.

## Error Handling

- Local find has no database failure state.
- Missing local snapshot IDs are skipped during navigation.
- Empty local queries perform no matching work beyond clearing the match list.
- Empty catalog queries emit no database command.
- Catalog permission and adapter failures retain sanitized contextual messages.
- Stale catalog responses cannot replace the current result tree.
- Closing catalog search emits `CancelCatalogSearch` even if no request is known to
  be active; cancellation remains idempotent.
- Search errors never clear or replace the normal Explorer tree.

## Testing

### Model Tests

- Local find snapshots only normal visible rows.
- A loaded descendant beneath a collapsed node does not match.
- An object absent from loaded pages does not match.
- Matching is case-insensitive and uses only the primary label.
- Multiple occurrences in one label produce one navigable result.
- `n` and `N` wrap and update normal selection and scroll.
- Editing cancellation restores the original selection.
- Confirmed-find clearing retains the current selection.
- Catalog tree projection deduplicates shared ancestors.
- Catalog projection includes presentation groups and complete hit paths.
- Unrelated siblings are excluded and hit paths are expanded.
- Closing catalog search restores the normal state unless a hit was located.
- Existing stale generation and connection rejection remains covered.

### Keymap Tests

- `/` opens local find and `f` opens catalog search in Explorer only.
- Printable input, Backspace, Ctrl-U, Enter, and Esc respect each phase.
- `n` and `N` are query characters while editing.
- `n` and `N` navigate after confirmation.
- Lowercase `n` creates a profile after local find is cleared.
- Existing Editor and Relation Data search bindings are unaffected.

### UI Tests

- Normal tree remains visible during `/` find.
- Matching label fragments, rather than complete rows, are highlighted.
- Current and non-current matches have distinct styles.
- `(1/3)` and `(0/0)` render correctly.
- Catalog results preserve ancestor hierarchy, indentation, markers, and icons.
- Empty, loading, ready, truncated, failed, and located states remain legible.
- Compact 80x24 and standard 120x36 layouts remain usable.

### Regression Tests

- Local find emits no `Command::SearchCatalog`.
- Catalog search retains debounce, cancellation, and stale-response behavior.
- Normal expansion and lazy pagination are unchanged after a non-locating search.
- Locating a catalog hit still preserves incomplete pagination semantics.

## Acceptance Criteria

- `/` can never find a node that was hidden by collapse or absent from loaded
  pages when find opened.
- `/` highlights matching label text in the normal tree and displays
  `(current/total)`.
- Enter confirms `/`; `n` and `N` then cycle through matches with wraparound.
- `f` searches the active connection's complete configured catalog scope.
- `f` results render as a deduplicated ancestor-preserving tree with highlighted
  matches and automatically expanded hit paths.
- Closing a non-locating `f` search restores the original normal-tree state.
- Locating an `f` result retains the selected object's normal-tree path.
- Existing adapter search implementations and asynchronous stale-result safeguards
  continue to work.
