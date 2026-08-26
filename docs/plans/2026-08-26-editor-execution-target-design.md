# Editor Execution Target Design

**Status:** Approved

**Date:** 2026-08-26

## Summary

Every SQL console will own a truthful execution target consisting of the active
profile, database, and schema. New consoles receive the current profile default
instead of displaying `TARGET MISSING`. `Space d` opens a real selector populated
from the active profile's visible catalog, and confirmed changes affect actual
query execution rather than only the editor title.

## Scope

The selector operates only within the currently active connection profile.
Switching connection profiles remains an Explorer operation. PostgreSQL and
MySQL database changes may require rebuilding the single active pool. Schema
changes initialize backend-specific session context. SQLite aliases must have
been discovered in the active catalog.

## Target Initialization

- The initial console uses the selected or connected profile default target.
- A newly created console uses the current active profile default target.
- A console whose persisted target is absent or invalid falls back to the active
  profile default only when that profile is active and valid.
- A console without an active profile remains editable but cannot execute.

## Selector

`Space d` builds candidates from the current profile's catalog tree and scope.
Each row represents a valid `database.schema` target. The configured profile
default is always available as a fallback while catalog loading is incomplete.

The selector starts on the console's current target. `j/k` and Up/Down move,
Enter confirms, and Esc cancels. Confirmation is blocked while a query is
running or a MANUAL transaction is not Idle.

## Runtime Consistency

The active runtime connection records the exact `ExecutionTarget`. Query and
manual-transaction commands carry the target. Runtime rejects commands unless
profile, connection generation, database, and schema all match.

When a selected target differs from the active runtime target, App requests a
safe target reconnect. PostgreSQL applies the selected database and search path;
MySQL applies the selected database and keeps database/schema identical; SQLite
uses the profile file and selected discovered alias.

No query falls back to the profile default or global active pool when a console
target is missing or stale.

## Persistence And Completion

The existing workspace `ExecutionTarget` field persists per-console choices.
Target changes emit `PersistWorkspace`. Completion ranking uses the active
console target schema, falling back to the profile default only when no console
target exists.

## Error Handling

- No active connection: selector does not open and reports a clear status.
- Catalog not loaded: selector contains the valid profile default only.
- Running query: target change is rejected.
- Active, aborted, starting, committing, rolling-back, or unknown MANUAL
  transaction: target change is rejected until the transaction returns to Idle.
- Stale or missing target at execution: execution fails closed before Runtime
  dispatch.
- Failed target reconnect keeps the prior runtime target usable and leaves the
  editor target unchanged.

## Testing

Tests cover default target initialization, selector candidates and navigation,
transaction/query guards, title rendering, workspace round-trip, completion
schema, command identity, Runtime mismatch rejection, and backend target
configuration.
