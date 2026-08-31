# Restored Relation Catalog Readiness Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make a restored active relation tab load automatically after its identity enters the active catalog, without showing the transient `relation is not present in the active catalog snapshot` failure or requiring the user to press `r`.

**Architecture:** Keep `Runtime::load_relation` as the final ownership and active-catalog safety boundary. Add a small App-layer readiness check so relation requests wait while catalog discovery is still pending, then re-run the existing idempotent `load_active_relation(false)` path after each accepted catalog page; once the target relation appears, the normal preview or DDL command is emitted exactly once.

**Tech Stack:** Rust 2024, Tokio command runtime, existing App reducer and normalized catalog model, Rust integration tests.

---

### Task 1: Lock In The Startup Race With A Failing Test

**Files:**
- Modify: `tests/workspace_tabs.rs:1-13,561-635`
- Test: `tests/workspace_tabs.rs`

**Step 1: Rename and update the existing restored-relation test**

Rename:

```rust
fn connection_install_loads_only_active_restored_relation()
```

to:

```rust
fn restored_relation_waits_for_catalog_before_loading()
```

Keep the existing two restored relation tabs, with `first` active and `second` inactive. Change the assertions immediately after `Action::ConnectionSucceeded` to require:

```rust
assert_eq!(
    commands
        .iter()
        .filter(|command| matches!(command, Command::LoadCatalogPage(_)))
        .count(),
    1
);
assert!(!commands
    .iter()
    .any(|command| matches!(command, Command::LoadRelationPreview(_))));
assert!(matches!(app.tabs[0], WorkspaceTab::Relation(ref tab)
    if matches!(tab.data, lazydb::model::relation::RelationLoad::Empty)));
assert!(matches!(app.tabs[1], WorkspaceTab::Relation(ref tab)
    if matches!(tab.data, lazydb::model::relation::RelationLoad::Empty)));
```

Remove the old expectation that connection success immediately emits one `LoadRelationPreview`. Preserve the inactive-tab assertion and later activation coverage, but update it in Task 3 after catalog fixtures exist.

**Step 2: Run the focused test and verify it fails**

Run:

```bash
cargo test --test workspace_tabs restored_relation_waits_for_catalog_before_loading -- --exact
```

Expected: FAIL because `Action::ConnectionSucceeded` currently emits `Command::LoadRelationPreview` before the root catalog request has completed.

**Step 3: Record the invariant in the test name and assertions**

The test must establish all of these conditions before implementation:

- Connection success still starts catalog discovery.
- The active restored relation remains `RelationLoad::Empty` while discovery is pending.
- The inactive restored relation also remains `RelationLoad::Empty`.
- No relation request reaches Runtime before catalog identity is available.

Do not introduce sleeps, timers, mock retries, or assertions against the runtime error string.

---

### Task 2: Gate Relation Requests On App Catalog Readiness

**Files:**
- Modify: `src/app.rs:31-42,7945-8051`
- Test: `tests/workspace_tabs.rs:561-635`

**Step 1: Add a private readiness type near the relation helper functions**

Add a private enum near `pending_relation_request` and `cancel_relation_load`:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RelationCatalogReadiness {
    Present,
    Loading,
    Missing,
}
```

The three states have distinct meanings:

- `Present`: the exact persisted `CatalogId` exists in the active profile's `CatalogTree`.
- `Loading`: it is not present yet, but the profile has pending catalog requests or is `ExplorerConnectionStatus::Syncing`.
- `Missing`: catalog discovery is currently settled and the exact identity is absent.

Do not add this state to persisted models or Runtime. It is a private reducer decision only.

**Step 2: Add an App helper that derives readiness from existing state**

Add a private method close to `load_active_relation`:

```rust
fn relation_catalog_readiness(
    &self,
    connection: ConnectionIdentity,
    relation: &crate::db::catalog::CatalogId,
) -> RelationCatalogReadiness {
    let Some(state) = self
        .explorer
        .normalized
        .profiles
        .get(&connection.profile_id)
    else {
        return RelationCatalogReadiness::Loading;
    };

    if state.catalog.get(relation).is_some() {
        RelationCatalogReadiness::Present
    } else if state.status == ExplorerConnectionStatus::Syncing
        || !state.pending_requests.is_empty()
    {
        RelationCatalogReadiness::Loading
    } else {
        RelationCatalogReadiness::Missing
    }
}
```

Use the full `CatalogId`, not `qualified_name`, title, or native object name. Runtime's `known_relations` uses the same identity, so App and Runtime must agree on the key.

**Step 3: Apply the readiness gate before mutating the relation load state**

In `load_active_relation`, after validating active connection/profile ownership/scope but before incrementing `next_request_id` or replacing `RelationLoad`, derive readiness from the tab's `descriptor.key.object_id`.

Implement this behavior:

```rust
match readiness {
    RelationCatalogReadiness::Loading => return Vec::new(),
    RelationCatalogReadiness::Present | RelationCatalogReadiness::Missing => {}
}
```

`Loading` must leave all relation state untouched:

- Keep `RelationLoad::Empty`, `Failed`, or `Cancelled` as-is.
- Do not consume `next_request_id`.
- Do not cancel a prior request.
- Do not emit a Runtime command.

Allow `Missing` through to Runtime once discovery is settled. This preserves the existing definitive safety error for a deleted, stale, corrupted, or out-of-catalog relation instead of leaving the tab waiting forever.

Structure the borrows so readiness is computed before holding the mutable relation-tab borrow. A minimal pattern is to clone only the active tab's `CatalogId`, evaluate readiness, then reacquire the mutable tab and continue through the existing request construction.

**Step 4: Run the focused startup test**

Run:

```bash
cargo test --test workspace_tabs restored_relation_waits_for_catalog_before_loading -- --exact
```

Expected: PASS for the startup assertions. If the test still contains the old tab-activation assertion, temporarily keep that assertion scoped to “no command while catalog is pending”; Task 3 adds the successful wake-up path.

**Step 5: Run relation reducer tests to detect behavior regressions**

Run:

```bash
cargo test --test relation_tabs
cargo test --test relation_runtime
```

Expected: PASS. Existing refresh, request cancellation, stale-result rejection, and snapshot attribution behavior remain unchanged.

---

### Task 3: Wake The Active Relation When Its Catalog Page Arrives

**Files:**
- Modify: `src/app.rs:5963-6144`
- Modify: `tests/workspace_tabs.rs:1-13,561-635`
- Test: `tests/workspace_tabs.rs`

**Step 1: Extend the restored-relation test with a real catalog hierarchy**

Import the existing catalog domain types needed to construct valid pages:

```rust
use lazydb::db::catalog::{
    CatalogCount, CatalogEntry, CatalogGroupSummary, CatalogPage, CatalogTarget,
    ObjectGroup, OptionalMetadata, QualifiedName,
};
```

In `restored_relation_waits_for_catalog_before_loading`, give the persisted relation a valid SQLite-style identity shared with the catalog entry, for example:

```rust
let database_id = CatalogId::new(profile.id, CatalogKind::Database, ["main"]);
let schema_id = CatalogId::new(profile.id, CatalogKind::Schema, ["main", "main"]);
let first_relation_id = CatalogId::new(
    profile.id,
    CatalogKind::Table,
    ["main", "main", "first"],
);
```

Drive the existing automatic discovery sequence using the pending requests stored in:

```rust
app.explorer.normalized.profiles[&profile.id].pending_requests
```

Apply valid pages in order:

1. `CatalogTarget::Databases` containing `main`.
2. `CatalogTarget::Schemas` containing the `main` schema.
3. `CatalogTarget::Groups` containing the `Tables` summary.
4. `CatalogTarget::Objects { group: Tables, .. }` containing the `first` relation.

For each of the first three pages, assert no `LoadRelationPreview` command is emitted. For the final objects page, assert exactly one request for the active restored relation:

```rust
assert!(matches!(
    commands.as_slice(),
    [Command::LoadRelationPreview(request)]
        if request.tab_id == first_id
            && request.relation.object_id == first_relation_id
));
assert!(matches!(app.tabs[0], WorkspaceTab::Relation(ref tab)
    if matches!(tab.data, lazydb::model::relation::RelationLoad::Loading { .. })));
assert!(matches!(app.tabs[1], WorkspaceTab::Relation(ref tab)
    if matches!(tab.data, lazydb::model::relation::RelationLoad::Empty)));
```

Extract only a small test-local `pending_request` helper if repeated indexing makes the test hard to read. Do not add production APIs solely for fixture construction.

**Step 2: Run the focused test and verify the wake-up assertion fails**

Run:

```bash
cargo test --test workspace_tabs restored_relation_waits_for_catalog_before_loading -- --exact
```

Expected: FAIL because `Action::CatalogPageLoaded` currently updates the catalog but never retries the deferred active relation load.

**Step 3: Reuse the existing idempotent loader after accepting each catalog page**

At the end of `App::accept_catalog_page`, after:

- Applying the page to `self.explorer.normalized`.
- Updating load states and pending requests.
- Starting continuation or child catalog requests.
- Rebuilding the projection and completion index.

append:

```rust
commands.extend(self.load_active_relation(false));
```

Place this after the `match &request.key.target` block that schedules child requests. This ordering is required: an intermediate database/schema/group page must see the newly scheduled child request and remain `RelationCatalogReadiness::Loading`, while the objects page containing the relation sees `Present`.

Do not call `load_active_relation(true)`. The non-refresh path guarantees that a relation already in `Loading` or `Ready` does not issue duplicate requests when additional catalog pages arrive.

**Step 4: Run the focused test and verify it passes**

Run:

```bash
cargo test --test workspace_tabs restored_relation_waits_for_catalog_before_loading -- --exact
```

Expected: PASS. The active relation remains empty during discovery and transitions directly to loading when its exact catalog page arrives.

**Step 5: Verify inactive restored tabs remain lazy**

After the active relation request is emitted, simulate its success or leave it loading, then activate the second restored relation while its identity is absent from the settled catalog.

Assert:

- No second relation was loaded merely because the first relation page arrived.
- `Action::ActivateTab(1)` follows existing behavior and emits at most the request for that newly active tab.
- No duplicate request for `first_id` is emitted.

If the second identity is intentionally missing from a settled catalog, expect Runtime to remain the final source of the existing missing-relation error; do not emulate a retry loop in App.

---

### Task 4: Cover DDL, Pagination, And Idempotency

**Files:**
- Modify: `tests/workspace_tabs.rs`
- Test: `tests/workspace_tabs.rs`

**Step 1: Add a restored DDL-tab regression test**

Add:

```rust
#[test]
fn restored_ddl_relation_loads_after_catalog_identity_arrives() {
    // Restore an active relation with RelationView::Ddl.
    // Connect and assert no relation command while catalog discovery is pending.
    // Apply the valid hierarchy and target objects page.
    // Assert exactly one Command::LoadRelationDdl for the restored tab.
}
```

Expected final assertion:

```rust
assert!(matches!(
    commands.as_slice(),
    [Command::LoadRelationDdl(request)]
        if request.tab_id == relation_tab_id
            && request.relation.object_id == relation_id
));
```

**Step 2: Run the DDL test and verify it passes with the shared loader**

Run:

```bash
cargo test --test workspace_tabs restored_ddl_relation_loads_after_catalog_identity_arrives -- --exact
```

Expected: PASS without DDL-specific production logic. `load_active_relation(false)` must select the command from `RelationView` exactly as it does today.

**Step 3: Add a pagination regression test**

Add:

```rust
#[test]
fn restored_relation_waits_until_later_catalog_page_contains_identity() {
    // Restore the target relation.
    // Reach the Tables objects target.
    // Apply page 1 without the target and with next_cursor = Some(...).
    // Assert only a continuation LoadCatalogPage is emitted.
    // Apply page 2 containing the target.
    // Assert exactly one LoadRelationPreview is emitted.
}
```

Use `CatalogCursor::from_keyset(...)` or the existing cursor fixture constructor already used by catalog tests; do not construct private cursor internals.

The page-one assertion must ensure the relation loader observes the continuation request as pending:

```rust
assert!(commands
    .iter()
    .any(|command| matches!(command, Command::LoadCatalogPage(_))));
assert!(!commands
    .iter()
    .any(|command| matches!(command, Command::LoadRelationPreview(_))));
```

**Step 4: Add an idempotency assertion**

After the target relation is in `RelationLoad::Loading`, apply another unrelated valid catalog page or invoke the non-refresh activation path. Assert no additional request for the same tab is emitted:

```rust
assert!(!commands.iter().any(|command| matches!(
    command,
    Command::LoadRelationPreview(request) if request.tab_id == relation_tab_id
)));
```

This confirms that catalog fan-out cannot create duplicate database work.

**Step 5: Run all workspace-tab tests**

Run:

```bash
cargo test --test workspace_tabs
```

Expected: PASS, including relation persistence, workspace switching, tab ordering, and restored relation loading.

---

### Task 5: Verify Runtime Safety And Full Reducer Behavior

**Files:**
- Verify only: `src/runtime.rs:941-1004`
- Verify only: `tests/relation_runtime.rs:374-395`
- Verify only: `tests/catalog_reducer.rs`

**Step 1: Keep Runtime's catalog membership guard unchanged**

Confirm this check remains in `Runtime::load_relation`:

```rust
if !self.known_relations.lock().is_ok_and(|known| {
    known.contains(&(request.connection, request.relation.object_id.clone()))
}) {
    // RelationFailed: relation is not present in the active catalog snapshot
}
```

The fix must not weaken profile ownership checks, active connection generation checks, relation-kind checks, or catalog membership checks.

**Step 2: Run catalog and runtime regression suites**

Run:

```bash
cargo test --test catalog_reducer
cargo test --test relation_runtime
cargo test --test connection_switch
```

Expected: PASS. Catalog fan-out, request cancellation, connection generation isolation, and unknown relation rejection remain intact.

**Step 3: Run formatting and static checks**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: both commands succeed with no formatting diff and no warnings.

**Step 4: Run the full test suite**

Run:

```bash
cargo test --all-targets --all-features
```

Expected: all unit and integration tests pass.

**Step 5: Perform a manual startup acceptance check**

Use an existing saved connection with at least one persisted relation tab:

1. Start `lazydb`.
2. Open/connect the saved profile.
3. Leave the restored relation tab active.
4. Observe catalog discovery.
5. Confirm the relation transitions from empty/loading directly to data or DDL.
6. Confirm `relation is not present in the active catalog snapshot` is not displayed transiently.
7. Confirm no focus switch and no `r` key press are required.

Also verify one genuinely deleted persisted relation still reaches a stable missing-relation failure after catalog discovery settles, rather than waiting forever or retrying repeatedly.

---

## Acceptance Criteria

- A restored active relation emits no preview/DDL command while catalog discovery is pending and its exact `CatalogId` is absent.
- The corresponding catalog objects page automatically emits exactly one relation command.
- Data tabs emit `LoadRelationPreview`; DDL tabs emit `LoadRelationDdl`.
- Relations on continuation pages wait for the page containing their identity.
- Inactive restored relation tabs are not eagerly loaded.
- Additional catalog pages do not duplicate a relation request already in `Loading` or `Ready`.
- A relation absent after settled discovery still reaches Runtime's existing definitive catalog-membership failure.
- Runtime ownership, connection-generation, kind, scope, and `known_relations` validation remain unchanged.
- Focus changes and manual `r` retries are no longer required for valid restored relations.

## Non-Goals

- Do not remove or relax `Runtime::known_relations` validation.
- Do not add fixed delays, polling loops, retry counters, or error-string matching.
- Do not persist catalog readiness or relation load state.
- Do not eagerly load every restored relation tab.
- Do not change database adapter catalog or relation APIs.
- Do not change workspace file format.

## Commit Policy

Do not create commits unless the user explicitly requests them. If requested, keep the regression test and implementation in one focused commit, for example:

```text
fix(relation): wait for catalog before restoring relation
```
