# Explorer Find Match Centering Design

## Goal

When Explorer's `/` find feature selects a match, position that row as close as
possible to the vertical center of the visible Explorer viewport. This applies
while entering a query, on `Enter`, and when cycling with `n`/`N`.

## Approach

Reuse the existing `ExplorerNodeAlignment::Middle` behavior rather than adding
a second scrolling algorithm. After resolving a find match against the current
visible projection, select it, call `align_selected(Middle)`, and synchronize
the compatibility selection and scroll fields. The existing alignment method
already clamps the scroll offset at both the beginning and end of the tree and
handles zero-height viewports safely.

The normal Explorer tree remains authoritative. Search snapshots, match order,
query semantics, and ordinary `j`/`k` navigation are unchanged. Empty queries,
missing matches, and disappeared rows remain no-ops.

## Verification

Add model tests that use a viewport taller than one row and assert the first
match and a later `n`/`N` match receive the middle alignment. Run the focused
Explorer state tests and the complete test suite.
