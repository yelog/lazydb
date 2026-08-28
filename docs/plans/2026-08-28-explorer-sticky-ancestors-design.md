# Explorer Sticky Ancestors Design

**Status:** Approved

**Date:** 2026-08-28

## Summary

The Explorer will keep the selected node's location visible while browsing large
expanded groups. Ancestors that have scrolled above the viewport will be rendered
as stacked sticky tree rows at the top of the Explorer. The rows retain their
normal hierarchy, presentation, stable identity, and mouse interactions.

Sticky ancestors apply to normal tree browsing and in-tree `/` find. Flat catalog
search remains unchanged because its rows are search results rather than one
continuous tree projection.

## Goals

- Keep the selected node's connection, database, schema, group, relation, and any
  other visible ancestors available after they scroll above the viewport.
- Preserve normal tree-row appearance and complete mouse interaction for sticky
  rows.
- Keep the selected row visible below the sticky region during keyboard movement,
  paging, alignment, mouse scrolling, and find navigation.
- Degrade predictably in very short Explorer viewports.
- Preserve the existing stable-ID tree model and 10,000-object performance
  contract.

## Non-goals

- Adding ancestor rows to flat catalog-search results.
- Persisting sticky rows as independent Explorer state.
- Replacing the current Explorer with a third-party tree or virtual-list widget.
- Introducing a compact single-line breadcrumb in place of tree rows.
- Changing catalog loading, expansion, selection identity, or connection state.

## Current Behavior

`ExplorerTreeState::visible()` projects the expanded tree into an ordered list.
`ExplorerTreeState::scroll` identifies the first normal row in the viewport, and
`render_explorer()` currently renders:

```text
visible.skip(scroll).take(viewport_height)
```

This makes every row above `scroll` unavailable, including all location context
for a selected table in a large group. The model already has the information
needed to solve this: stable `ExplorerNodeId` values, catalog parent links,
presentation-group ownership, current selection, expansion state, and the normal
scroll offset.

## Interaction Model

For the current selected node, the Explorer derives its visible ancestor chain in
root-to-parent order. An ancestor becomes sticky only when its normal projected
index is less than the current normal scroll offset.

For example, when all ancestors have crossed the top boundary:

```text
▾ connection-a
  ▾ business_db
    ▾ public
      ▾ Tables  120
        audit_log
        customer
      > customer_order
        invoice
```

Sticky rows appear progressively as scrolling crosses each ancestor. An ancestor
that remains in the normal body is not duplicated in the sticky region.

Sticky rows use the same stable row target as normal rows. They support the
existing complete interaction set:

- Single click selects the ancestor.
- Double click executes its normal primary action.
- Expansion and collapse retain their existing semantics.
- Clicking a sticky row focuses the Explorer in the same way as a normal row.

Selecting, collapsing, deleting, refreshing, or replacing a subtree does not
mutate separate sticky state. The next projection derives the new sticky chain
from authoritative Explorer state.

## Viewport Projection

Introduce a UI-neutral viewport projection that separates sticky rows from the
normal scrolling body. The exact Rust names may vary, but the result needs these
concepts:

```rust
struct ExplorerViewport {
    pinned: Vec<VisibleCatalogNode>,
    rows: Vec<VisibleCatalogNode>,
    hidden_ancestor_count: usize,
}
```

The projection receives:

- The ordered visible tree rows.
- The selected stable node ID.
- The normal scroll offset.
- The available viewport height.
- Any mode-specific reserved rows.

It returns:

- Sticky ancestors in root-to-parent order.
- Normal body rows beginning at the effective scroll offset.
- The number of far ancestors omitted by compact-height degradation.
- Stable node IDs for every interactive screen row.

The projection remains independent of Ratatui. The UI remains responsible for
icons, spans, colors, truncation, drawing, and hit-region registration.

## Ancestor Resolution

Ancestor resolution must follow real Explorer ownership rather than infer parents
from indentation depth. The chain must support:

- Profile roots.
- Catalog database, schema, relation, and relation-child nodes.
- Presentation groups.
- Status, load-more, and empty rows owned by a profile, catalog node, or group.

The chain excludes the selected node itself. It includes only ancestors that are
currently part of the visible expanded projection. Parent traversal is bounded by
tree depth and runs once for the selected node per projection.

## Scroll Semantics

Sticky rows consume vertical space. The effective body height is:

```text
Explorer inner height
- mode-reserved rows
- sticky ancestor rows
- optional omitted-ancestor indicator
= normal body height
```

Normal browsing reserves no mode row. In-tree `/` find reserves its one-line find
input before sticky and body rows are calculated.

Selection visibility, page movement, half-page movement, and selected-row
alignment must use the effective body height. A row is not considered visible
when its screen position would be covered by the sticky region.

Sticky count depends on scroll, while effective body height affects scroll. This
must be resolved by one centralized pure calculation that stabilizes the
candidate scroll and sticky chain. A bounded iteration is acceptable because the
maximum number of changes is bounded by ancestor depth. The calculation must not
be duplicated across rendering and movement methods.

The normal scroll offset remains an index into the complete visible row
projection. Sticky rows are derived screen content and do not receive independent
scroll positions.

## Compact-height Degradation

The selected normal row has priority over sticky context.

The degradation rules are:

1. Reserve at least one body row whenever the viewport has usable height.
2. Fill remaining capacity with the ancestors nearest to the selected node.
3. Remove farthest ancestors from the root side first.
4. Show a muted omitted-ancestor indicator when space permits.
5. Do not use an indicator when it would displace the selected row or a retained
   nearest ancestor.
6. At one usable row, show only the selected/body row.
7. At two usable rows, prefer the nearest ancestor and selected/body row without
   an indicator.

The indicator is non-interactive. Its exact copy may be concise, such as
`⋮ 2 ancestors`, and must pass through the same safe terminal rendering rules as
other text.

## Rendering

Sticky rows reuse the existing tree-row presentation:

- Original indentation and expansion marker.
- Database, catalog, and group icons.
- Label, metadata, count, comment, endpoint, provenance, and connection status.
- Existing terminal-text sanitization and width truncation.
- Existing selection style if a sticky row becomes selected during an
  interaction frame.

The sticky region should remain visually quiet. It must not add `PINNED`, `PATH`,
or similar labels. A subtle existing surface style or a bottom boundary on the
last sticky row may distinguish it from the body without consuming another row.

Rendering should share one row-construction helper between sticky and body rows
so their semantic and truncation behavior cannot drift.

## Mouse Mapping

Hit regions must be generated from the final screen projection rather than from
`scroll + screen_row` assumptions. Every sticky and normal node row maps to:

```rust
HitTarget::ExplorerRow(node_id)
```

The omitted-ancestor indicator has no hit target. A node must not appear in both
sticky and normal regions in the same frame. Reprojection after selecting or
collapsing a sticky ancestor updates all hit regions on the next frame.

## Find and Search Modes

### Normal Browsing

Sticky ancestors are fully enabled.

### In-tree Find

The `/` input remains the first Explorer content row. Sticky ancestors are placed
below it, followed by the normal tree body. Find match alignment and visibility
use the reduced body height.

```text
/ customer (3/18)
▾ connection-a
  ▾ business_db
    ▾ public
      customer_order
```

### Flat Catalog Search

Flat search remains unchanged and has no sticky region. Search rows can span
unrelated connections and branches, so imposing the selected result's ancestors
would change the established result-list scrolling model.

## Performance

The implementation must not scan the catalog tree once per visible row. It may:

- Reuse the existing visible projection.
- Build or reuse a stable-ID-to-index map for that projection.
- Traverse only the selected node's ancestor chain.
- Format only final sticky and body viewport rows.

Additional ancestor work should be `O(depth)`, independent of the number of
objects in an expanded group. The existing 10,000-object projection and rendering
performance contract remains authoritative.

## Error and State Handling

- A missing or no-longer-visible selected node produces no stale sticky rows and
  follows the existing selection fallback behavior.
- Catalog refresh or replacement immediately derives sticky rows from the new
  parent relationships.
- Collapsing a sticky ancestor uses the existing descendant-selection fallback
  and then recalculates scroll and sticky rows.
- A zero-height viewport produces no rows and does not panic.
- Invalid or unavailable owner chains fail closed by omitting unresolved
  ancestors rather than rendering stale identities.

## Testing Strategy

### Model and Projection Tests

- No sticky ancestors before the first ancestor crosses the top boundary.
- Connection, database, schema, group, and relation ancestors progressively
  become sticky in root-to-parent order.
- Ancestors still present in the body are not duplicated.
- Switching selection to another branch replaces the sticky chain.
- Collapsing or removing a sticky ancestor produces the correct selection
  fallback and viewport.
- Line, half-page, and page movement keep selection in the effective body.
- Top, middle, and bottom alignment account for sticky height.
- Heights zero, one, and two do not panic and follow compact degradation.
- Far ancestors are omitted before near ancestors, with an accurate indicator.
- Status, load-more, and empty owner chains resolve correctly.

### UI Tests

- Sticky rows render at the top with their normal icons, indentation, counts, and
  statuses.
- The selected body row is below the sticky region and visible.
- Sticky nodes are not duplicated in the normal body.
- Every sticky node has the correct `ExplorerRow` hit target.
- The omitted-ancestor indicator is muted and non-interactive.
- In-tree find keeps the input above sticky rows.
- Flat catalog search renders no sticky ancestors.
- Supported compact terminal sizes degrade correctly.

### Mouse Tests

- Clicking a sticky schema selects it.
- Double-clicking a sticky group executes the same action as its normal row.
- Collapsing a sticky ancestor removes obsolete descendant hit targets.
- Mouse-wheel scrolling regenerates sticky hit coordinates correctly.

### Performance Tests

- A 10,000-object expanded group does not introduce per-object ancestor
  traversal.
- Row formatting remains bounded to final viewport and sticky rows.

## Acceptance Criteria

- Every selected node ancestor that has scrolled above the Explorer body is
  stacked at the top in hierarchy order.
- Connections, databases, schemas, groups, relations, and deeper supported nodes
  participate correctly.
- An ancestor still visible in the body is never duplicated.
- The selected node remains visible below the sticky region during all navigation
  and alignment operations.
- Sticky rows have complete normal-node mouse interaction.
- Normal browsing and in-tree find use sticky ancestors; flat catalog search does
  not.
- Compact heights prioritize the selected row and nearest ancestors without
  panic or unstable scrolling.
- Refresh, deletion, collapse, and connection changes cannot leave stale sticky
  rows.
- The 10,000-object Explorer remains navigable without new catalog-wide parent
  scans.
