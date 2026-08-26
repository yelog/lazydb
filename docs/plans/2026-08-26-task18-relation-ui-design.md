# Task 18 Relation UI Design

## Scope

Close the confirmed relation-tab UI gaps only. SQL console rendering and input behavior remain unchanged, and relation request identity fields remain authoritative.

## Design

- Route grid actions through the active tab kind. SQL tabs keep their existing result-set behavior; relation Data uses the active relation preview result-set dimensions and writes `RelationTab.grid` for keyboard and mouse selection.
- Render a retained snapshot as the relation body whenever Data or Structure is loading, failed, or cancelled with `previous`. Add a bounded, sanitized status banner and actionable retry/cancel hit regions without replacing the snapshot.
- Refresh captures and cancels the exact previous loading request before dispatching the new request. No cancellation is emitted for non-loading states.
- Sanitize and bound hostile failure text, result column names, and relation tab titles at render time.
- Render typed column metadata with bounded priority, a dedicated `TRIGGERS` section, and provenance on both relation views.
- Make relation focus traversal alternate only Explorer and Results. SQL traversal remains unchanged.

## Testing

Add focused reducer, input, and render tests for grid keyboard/mouse behavior, retained snapshots, exact refresh cancellation, hostile display values, triggers and typed metadata, Data provenance, and relation focus traversal. Run relation UI/input/app tests, all-features tests, all-target checks, strict clippy, formatting, and diff validation.
