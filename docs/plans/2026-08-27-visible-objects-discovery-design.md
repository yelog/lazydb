# Visible Objects Discovery Design

**Status:** Approved on 2026-08-27

## Goal

Make the Visible Objects picker automatically load every database and schema visible to the draft credentials while preserving saved selections and tolerating per-database PostgreSQL failures.

## Architecture

Catalog discovery becomes an independent profile-manager operation instead of a side effect available only through Test Connection. Opening or refreshing the Scope page validates the draft, starts a fingerprinted discovery request, and immediately renders saved selections while the runtime resolves draft credentials and performs discovery without mutating the active connection.

PostgreSQL discovery is orchestrated by the runtime because cross-database schema discovery needs both the draft profile and resolved password. It lists connectable databases from the initial connection, opens bounded temporary connections to each database, and returns successful database/schema results plus sanitized per-database warnings. MySQL and SQLite retain their existing single-connection discovery behavior.

## State And Results

Add a discovery-specific pending request to `ProfileManagerState`; do not reuse `ProfileOperation::Testing`, because Scope loading must not masquerade as a connection test or block unrelated form semantics. Responses are accepted only when both request id and `DiscoveryFingerprint` match the current draft.

`CatalogDiscovery` gains a warning list. A PostgreSQL database whose schema query fails remains visible with no discovered schemas and a warning; successful databases remain usable. Saved selections continue to be unioned with discovered candidates and are never changed by refresh.

## Interaction

- Opening Visible Objects starts discovery when no fresh matching snapshot exists.
- The picker initially shows saved selections and `Discovering databases and schemas...`.
- `r` forces a refresh.
- Success replaces candidate discovery data but preserves checkboxes, expansion, selection, and viewport where possible.
- Global failure keeps saved selections and shows the actual sanitized error.
- Partial failure shows discovered objects and a concise warning count/detail.

## PostgreSQL Semantics

The initial connection queries non-template, connectable databases visible to the current role. Schema discovery then connects to each database with a concurrency limit of four and runs the existing non-system-schema query. The configured database uses the existing connection where possible. Individual connection/query failures become warnings rather than failing the entire result.

## Testing

Reducer tests cover automatic open, refresh, stale response rejection, preserved scope, and loading/error state. Runtime tests cover credential reuse. PostgreSQL integration tests cover multiple databases when configured and unit-level result assembly covers partial failures. UI/keymap tests cover loading text, warning text, and `r` refresh.
