# Explorer Catalog Object Drop Design

**Status:** Approved

**Date:** 2026-09-01

## Summary

LazyDB will make the Explorer `d` shortcut depend on the directly selected node.
Selecting a profile root will retain the existing connection-profile deletion
flow. Selecting a catalog object will request a database object drop instead of
deleting the owning connection. Synthetic Explorer rows will never inherit a
destructive action from their owning profile.

Database adapters will generate and validate the exact dialect-specific SQL.
The UI will display that SQL in a dedicated confirmation overlay, but it will
not construct or accept arbitrary drop statements. Execution requires typing
exactly lowercase `y` and then pressing Enter. The generated statement will not
include `CASCADE` or `IF EXISTS`.

All `CatalogKind` variants have an explicit outcome: the adapter either returns
a safe, unambiguous drop plan or rejects the request with a user-facing reason.
Unsupported operations are never approximated with guessed SQL.

## Existing Problem

`map_explorer` currently derives only the owning profile UUID from the selected
Explorer node. Pressing `d` then always emits `ProfileRequestDelete`, even when
the selected node is a table, column, index, group, status row, or another
catalog descendant. This loses the selected node's semantics before the action
reaches the application reducer.

The existing profile deletion confirmation is also unsuitable for catalog
objects. It permits Enter as immediate confirmation, belongs to the profile
manager lifecycle, and does not display executable SQL.

## Shortcut Semantics

The Explorer keymap will dispatch `d` from the direct `ExplorerNodeId`:

| Selected node | Behavior |
| --- | --- |
| `Profile` | Request deletion of that connection profile |
| `Catalog` | Request a drop plan for that catalog object |
| `Group` | Report that the presentation group cannot be dropped |
| `Status`, `LoadMore`, `Empty` | Report that the synthetic row cannot be dropped |
| `EmptyProfiles`, `Others` | No destructive action; report unavailability when useful |

Catalog descendants must not fall back to deleting their owning relation or
profile. Each directly selected catalog object is handled according to its own
kind.

The catalog path uses a dedicated action such as:

```rust
Action::RequestDropCatalogObject { id: CatalogId }
```

Using `Drop` distinguishes database DDL from deleting local profile metadata.

## Adapter-Owned Drop Plans

The active database adapter will create an immutable plan before confirmation:

```rust
pub struct CatalogDropPlan {
    pub connection: ConnectionIdentity,
    pub catalog_generation: u64,
    pub object_id: CatalogId,
    pub object_kind: CatalogKind,
    pub display_name: String,
    pub sql: String,
}
```

The exact representation may change during implementation, but the plan must
bind the statement to the connection identity, catalog generation, and catalog
object that were validated.

Plan construction must:

- Verify that the object belongs to the active connection.
- Resolve the current catalog entry and any required owner information.
- Use adapter-owned identifier quoting and namespace rules.
- Fully qualify names wherever the database supports doing so.
- Return exactly one unambiguous DDL operation.
- Omit `CASCADE` and use the database's default dependency restriction.
- Omit `IF EXISTS` so a stale target does not appear to succeed silently.
- Reject system objects, unsupported namespaces, and insufficient metadata.
- Return a specific user-facing reason when no safe plan can be generated.

The UI must never construct SQL by concatenating `CatalogKind`, display labels,
or `native_path` components.

## Catalog Kind Coverage

All current catalog kinds are valid plan requests:

- Database
- Schema
- Table
- View
- Materialized view
- Column
- Index
- Primary key
- Unique constraint
- Foreign key
- Check constraint
- Function
- Procedure
- Trigger
- Sequence
- Type

Support remains capability-based per adapter. Typical statements include
`DROP TABLE`, `DROP VIEW`, `DROP MATERIALIZED VIEW`, and owner-qualified
`ALTER TABLE ... DROP ...` operations. The adapter controls the final syntax.

Known cases that must be rejected unless sufficient native metadata and safe
syntax are available include:

- SQLite `main`, `temp`, and attached database aliases as database drops.
- SQLite operations that require implicit table reconstruction.
- PostgreSQL overloaded functions or procedures without complete argument type
  identities.
- MySQL objects whose database/schema representation cannot be mapped without
  ambiguity.
- Indexes, constraints, and triggers without their required owning relation or
  native identity.
- A database currently required by the active connection when the adapter
  cannot safely execute its deletion from that connection.

An unsupported request produces a status such as:

```text
Cannot drop function public.calculate_total: parameter signature is unavailable
```

It does not open an executable confirmation overlay.

## Confirmation Overlay

Catalog drops use a dedicated overlay rather than `ProfileManagerState` or the
generic SQL execution confirmation. The state contains the trusted plan, a
small confirmation input, request state, and any execution error.

The overlay displays:

```text
 DROP TABLE

This operation will execute:

DROP TABLE "mes_idg"."etms_line"

This action cannot be undone.
Type y and press Enter to execute:

> _
```

Interaction rules are strict:

- Only input exactly equal to lowercase `y` is valid.
- Typing `y` alone never executes.
- Enter executes only when the input is exactly `y`.
- Enter with any other input keeps the overlay open and explains the required
  confirmation.
- Uppercase `Y` is not accepted.
- Backspace edits the input and Ctrl-U clears it.
- Esc cancels and closes the overlay.
- Execution disables further input and duplicate submission.
- The displayed SQL is read-only, wraps without hiding qualified identifiers,
  and is the exact statement represented by the trusted plan.

There is no default-focused permanent-delete button and no Enter-only path.

## Execution Flow

Execution uses a dedicated command rather than injecting SQL into the active
console:

```rust
Command::DropCatalogObject {
    request_id: u64,
    plan: CatalogDropPlan,
}
```

The runtime validates that the active connection identity and catalog
generation still match the plan. It also rejects execution for a read-only
profile, a conflicting running operation, or an invalidated target. The runtime
must execute only adapter-produced plans; the overlay must not become a generic
arbitrary-SQL entry point.

Catalog DDL does not join an active console manual transaction. This avoids
unexpected transaction coupling and MySQL implicit-commit behavior.

The runtime reports a dedicated success or failure action containing the
request ID and target object identity. Stale responses are ignored.

## Concurrency And Safety

Plan generation or execution is blocked when:

- The selected object does not belong to the active connection.
- The profile is read-only.
- A query or conflicting catalog operation is running.
- The target relation has uncommitted relation edits.
- A console manual transaction makes the destructive operation unsafe.
- The active connection or catalog generation changed after plan creation.
- Removing a current database or schema would invalidate the execution target
  without a safe adapter-defined transition.

The implementation should reuse existing workspace-exit and transaction safety
checks where their semantics match. It must not silently commit, roll back, or
cancel unrelated user work.

## Success State Reconciliation

After a successful drop, the app will:

- Close the confirmation overlay.
- Remove the target and its descendants from the normalized Explorer tree.
- Invalidate completion entries for the removed subtree.
- Advance or refresh catalog state so stale catalog responses cannot restore the
  object.
- Refresh the nearest valid parent target instead of reloading the whole
  connection when possible.
- Reconcile execution targets if a namespace was removed.
- Retain affected open relation tabs only as read-only snapshots and label them
  as dropped-object snapshots, rather than presenting them as live data.
- Show a concise success status containing the object kind and qualified name.

Removing a container removes all known descendants from local catalog and
completion state.

## Failure Handling

If plan creation fails, LazyDB leaves Explorer unchanged and shows the adapter's
specific unsupported or validation reason.

If execution fails, LazyDB:

- Keeps the object and its descendants in Explorer.
- Keeps the confirmation overlay open with the generated SQL and database
  error.
- Clears the confirmation input before another attempt.
- Allows Esc to cancel or a new lowercase `y` plus Enter to retry.
- Does not refresh or mutate completion and relation state as if the drop had
  succeeded.

Terminal control characters in database error messages continue to use the
existing sanitization boundary.

## Contextual Help

The Explorer documentation and contextual hints will distinguish profile
deletion from database object drops:

- Profile root: `d delete connection`
- Supported catalog object: `d drop table`, `d drop view`, or the corresponding
  kind
- Unsupported or synthetic node: no destructive hint, with a specific status
  if `d` is pressed

`docs/keybindings.md` will describe `d` as dropping the directly selected
database object and deleting a connection only when its profile root is
selected.

## Testing

Keymap and reducer tests will verify:

- A table or other catalog node emits a catalog drop request, never a profile
  deletion request.
- A profile root retains profile deletion behavior.
- Synthetic rows and presentation groups cannot delete their owning profile.
- Every `CatalogKind` reaches an explicit supported or unsupported adapter path.

Adapter tests will verify:

- PostgreSQL, MySQL, and SQLite identifier quoting for reserved words, spaces,
  quote characters, and mixed case.
- Correct qualification independent of the current default schema.
- No generated statement contains `CASCADE` or `IF EXISTS`.
- Owner-dependent statements use the correct relation identity.
- Unsupported SQLite namespace operations and ambiguous overloaded routines are
  rejected with useful reasons.

Confirmation tests will verify:

- Enter alone does not execute.
- `y` alone does not execute.
- Lowercase `y` followed by Enter emits exactly one command.
- Uppercase `Y` and all other input do not execute.
- Busy state prevents duplicate execution.
- Esc, Backspace, and Ctrl-U behave as specified.

Lifecycle tests will verify:

- A changed connection identity or catalog generation invalidates an old plan.
- Success removes the subtree, invalidates completion state, refreshes the
  parent, and marks affected relation tabs as dropped snapshots.
- Failure preserves catalog state and displays the error.
- Read-only and transaction-conflict cases are blocked without changing user
  work.

UI render tests will cover the SQL preview, confirmation input, busy state,
error state, and contextual key hints. Final verification includes formatting,
focused tests, the full Rust test suite, and Clippy with warnings denied.

## Delivery Strategy

The architecture and explicit capability boundary land first. Adapters then
enable each object type only when they can produce a safe, fully validated
statement. This makes every catalog object behave predictably from the first
release without claiming unsafe cross-database parity.

The implementation sequence is:

1. Correct direct-node shortcut dispatch and add the drop plan/action model.
2. Add adapter planning, strict confirmation, dedicated runtime execution, and
   stale-response protection.
3. Add success/failure catalog, completion, execution-target, and relation-tab
   reconciliation.
4. Enable and test object kinds per adapter; keep unsafe combinations explicitly
   disabled.
5. Update contextual hints and operational documentation.
