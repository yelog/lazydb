# Explorer Slash Search Start Design

**Goal:** Make the first `/` search result start at the currently selected Explorer node and continue with existing wraparound behavior.

**Architecture:** Keep the existing visible-row snapshot and cyclic match navigation. When the query is edited, derive the current node's position from that snapshot, then select the first matching row at or after that position; if none exists, select the first match. Preserve the existing cyclic navigation after confirmation.

**Testing:** Add model-level regression tests covering a selection in the middle of the visible tree and the fallback to the first match when no later match exists.

**Files:**
- Modify `src/model/workspace.rs` to choose the initial visible `/` search match relative to the current Explorer selection.
- Modify `tests/explorer_state.rs` with targeted regression coverage.
