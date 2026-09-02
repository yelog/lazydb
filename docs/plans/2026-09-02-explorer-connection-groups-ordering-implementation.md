# Explorer Connection Groups and Ordering Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add persistent custom connection groups, group membership management, region-aware group projection in Explorer, and persistent connection reordering without changing connection, catalog, or workspace behavior.

**Architecture:** Persist groups as globally identified labels beside the ordered profile list in `connections.toml`; each saved profile has at most one optional `group_id`. Keep `ProfileAccess` as the only source of current-project/global/other placement, then project the same group separately into the primary and `others` regions. Keep the profile vector/runtime registry order as the only connection-order source, and serialize every profile/group/order mutation through the existing runtime `profile_mutation` lock.

**Tech Stack:** Rust 2024, Serde/TOML 0.9, UUID 1.18, Tokio 1.47, Ratatui 0.30, Crossterm 0.29, existing reducer/action/runtime architecture.

---

## Product Decisions

1. A group is global application configuration, not project-owned configuration.
2. A connection belongs to zero or one group. Multiple group membership is out of scope because it would duplicate one profile/catalog subtree in Explorer and require occurrence-aware node identities.
3. `ProfileAccess` remains unchanged and independently determines whether a connection is shown in the primary region or under `others`.
4. The same `group_id` may be projected once in the primary region and once under `others`.
5. Group projection nodes include their region in their identity so expansion and selection are independent.
6. Empty groups are persisted but omitted from Explorer. They remain available in the group picker.
7. Ungrouped connections are rendered directly in their region; do not add a permanent `Ungrouped` tree node.
8. Deleting a group never deletes connections. Its members become ungrouped in the same atomic persistence transaction.
9. Group names are trimmed, non-empty, at most 64 Unicode scalar values, and unique using Unicode lowercase comparison. Preserve the user's original casing.
10. Group display order is `ProfileCollection.groups` order. This iteration does not add group reordering.
11. Connection display order is `ProfileCollection.profiles` order. Do not add `sort_index`, ranks, or per-group member arrays.
12. Move-up/down only swaps the selected profile with the previous/next visible sibling having the same region and `group_id`; it never changes access or group membership.
13. Group membership and ordering changes do not reconnect, clear catalog state, close tabs, or change the active profile.
14. Keep `App::new(Vec<ConnectionProfile>)` as a test/API convenience that constructs an empty group list. Add a full-collection constructor for startup.

## Acceptance Criteria

- Users can create, rename, and delete custom groups from Explorer.
- Users can move a connection to any existing group or back to ungrouped.
- A group containing current-project/global and other-project connections appears in both regions with only the applicable members.
- Expanding the primary occurrence of a group does not expand its `others` occurrence.
- Empty groups do not appear in Explorer but remain selectable for membership assignment.
- Deleting a group preserves every connection and removes all references to the deleted group.
- Connection order persists across restart and remains stable after profile edits.
- Reordering is constrained to visible siblings in the same region and group.
- Moving or reordering a connected profile preserves its connection status, loaded catalog, active workspace, and tabs.
- Existing V2-V5 connection files load successfully; V5 profile order is preserved and all migrated profiles are ungrouped.
- Headless agent commands continue to consume only `collection.profiles` and preserve existing selection behavior.
- `cargo fmt --check`, `cargo check`, focused tests, the full test suite, and Clippy with warnings denied pass.

## Non-Goals

- Multiple groups per connection.
- Nested groups.
- Project-specific group definitions or project-specific connection order.
- Drag-and-drop in the terminal.
- Group colors, icons, descriptions, or manual group ordering.
- Exposing groups through the agent/MCP response schema.
- Changing `ProfileAccess`, credential storage, connection lifecycle, catalog loading, or workspace persistence.

---

### Task 1: Add the Group Domain Model

**Files:**
- Modify: `src/profile.rs:221-309`
- Modify: `src/model/profile_manager.rs:919-950,1105-1140`
- Modify: `src/persistence/profiles.rs:81-103`
- Modify: `tests/execution_target.rs:11-35`
- Modify: `tests/agent_context.rs:9-35`
- Modify: `tests/agent_policy.rs:9-35`
- Modify: `tests/agent_mcp.rs:15-45`
- Modify: `tests/profile_draft.rs:23-55`
- Modify: `tests/relation_tabs.rs:764-790`
- Modify: `tests/relation_runtime.rs:457-485`
- Modify: any remaining `ConnectionProfile { ... }` literal identified by `cargo check`
- Test: `tests/profile_url.rs`

**Step 1: Write failing model tests**

Add tests proving imported profiles are ungrouped and a collection preserves explicit group/profile order:

```rust
#[test]
fn imported_profiles_are_ungrouped() {
    let imported = import_connection_url("postgres://localhost/app", Some("app"))
        .unwrap()
        .profile;
    assert_eq!(imported.group_id, None);
}

#[test]
fn profile_collection_preserves_declared_order() {
    let first = ConnectionGroup::new(Uuid::from_u128(1), "Production").unwrap();
    let second = ConnectionGroup::new(Uuid::from_u128(2), "Development").unwrap();
    let collection = ProfileCollection {
        groups: vec![first.clone(), second.clone()],
        profiles: vec![],
    };
    assert_eq!(collection.groups, vec![first, second]);
}
```

**Step 2: Run the focused test and confirm failure**

Run: `cargo test --test profile_url imported_profiles_are_ungrouped`

Expected: compilation fails because `ConnectionGroup`, `ProfileCollection`, and `group_id` do not exist.

**Step 3: Add the domain types**

Add before `ConnectionProfile`:

```rust
pub const MAX_CONNECTION_GROUP_NAME_CHARS: usize = 64;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionGroup {
    pub id: Uuid,
    pub name: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProfileCollection {
    pub groups: Vec<ConnectionGroup>,
    pub profiles: Vec<ConnectionProfile>,
}
```

Add a small validation constructor/error in `src/profile.rs`:

```rust
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ConnectionGroupNameError {
    #[error("group name cannot be empty")]
    Empty,
    #[error("group name cannot exceed {MAX_CONNECTION_GROUP_NAME_CHARS} characters")]
    TooLong,
}

impl ConnectionGroup {
    pub fn new(id: Uuid, name: impl Into<String>) -> Result<Self, ConnectionGroupNameError> {
        let name = name.into().trim().to_owned();
        if name.is_empty() {
            return Err(ConnectionGroupNameError::Empty);
        }
        if name.chars().count() > MAX_CONNECTION_GROUP_NAME_CHARS {
            return Err(ConnectionGroupNameError::TooLong);
        }
        Ok(Self { id, name })
    }

    pub fn normalized_name(&self) -> String {
        self.name.to_lowercase()
    }
}
```

Add to `ConnectionProfile` after `access`:

```rust
#[serde(default)]
pub group_id: Option<Uuid>,
```

Set `group_id: None` in `import_connection_url`, legacy conversions, test fixtures, and all explicit profile literals. Do not add group editing to `ProfileDraft`; profile form saves must retain the original `group_id` unchanged.

**Step 4: Run model tests**

Run: `cargo test --test profile_url`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/profile.rs tests/profile_url.rs
git commit -m "feat(profiles): add connection group model"
```

---

### Task 2: Upgrade Connection Persistence to V6

**Files:**
- Modify: `src/persistence/profiles.rs:17-239`
- Create: `tests/profile_groups.rs`
- Modify: `src/agent/service.rs:32-52`

**Step 1: Write failing V5 migration and V6 round-trip tests**

Create `tests/profile_groups.rs` with helpers that write real TOML to a temp directory. Cover:

```rust
#[test]
fn v5_migrates_to_empty_groups_without_reordering_profiles() { /* first, second */ }

#[test]
fn v6_round_trip_preserves_group_and_profile_order() { /* two groups, three profiles */ }

#[test]
fn missing_file_loads_an_empty_collection() { /* groups and profiles empty */ }
```

Use fixed UUIDs and assert full vectors rather than searching by ID, so ordering regressions fail clearly.

**Step 2: Run and confirm failure**

Run: `cargo test --test profile_groups`

Expected: compilation fails because `ProfileStore::load` still returns `Vec<ConnectionProfile>` and V6 is unsupported.

**Step 3: Introduce explicit current and V5 file structs**

Set:

```rust
const PROFILE_FILE_VERSION: u16 = 6;
```

Use an explicit V6 file:

```rust
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProfileFile {
    version: u16,
    #[serde(default)]
    groups: Vec<ConnectionGroup>,
    profiles: Vec<ConnectionProfile>,
}
```

Define `ConnectionProfileV5` without `group_id` and `ProfileFileV5`. Convert V5 profiles with `group_id: None`. Keep the existing V2 conversion and V3/V4 credential/access normalization behavior; do not parse old versions with the V6 struct because `ConnectionProfile` now has different semantics despite serde defaults.

Change APIs to:

```rust
pub fn load(&self) -> Result<ProfileCollection, PersistenceError>;
pub fn save(&self, collection: &ProfileCollection) -> Result<(), PersistenceError>;
```

For versions V2-V5 return `ProfileCollection { groups: vec![], profiles }`. For V6 return both serialized vectors.

**Step 4: Preserve canonical output without changing semantic order**

Clone the collection before save. Sort only `ProfileAccess::Projects.roots`, as today. Do not sort groups or profiles.

Update `AgentService::load` to use:

```rust
let profiles = ProfileStore::new(profile_path)
    .load()
    .map_err(/* existing mapping */)?
    .profiles;
```

The agent intentionally ignores group metadata.

**Step 5: Run persistence tests**

Run: `cargo test --test profile_groups && cargo test --test profile_runtime`

Expected: new tests pass; existing runtime tests may still require mechanical `.profiles` or collection fixture updates, but no behavioral assertion changes.

**Step 6: Commit**

```bash
git add src/persistence/profiles.rs src/agent/service.rs tests/profile_groups.rs tests/profile_runtime.rs
git commit -m "feat(persistence): store connection groups in profile file v6"
```

---

### Task 3: Validate Group Integrity at the Persistence Boundary

**Files:**
- Modify: `src/persistence/profiles.rs:24-42,120-239`
- Test: `tests/profile_groups.rs`

**Step 1: Add failing invalid-file tests**

Add one test for each contract:

```rust
#[test]
fn duplicate_group_ids_are_rejected() {}

#[test]
fn case_insensitive_duplicate_group_names_are_rejected() {}

#[test]
fn blank_and_overlong_group_names_are_rejected() {}

#[test]
fn unknown_profile_group_references_are_rejected() {}
```

Also test that `" Production "` cannot be persisted; names must already be canonical at the persistence boundary so hand-edited files do not silently change.

**Step 2: Run and confirm failure**

Run: `cargo test --test profile_groups`

Expected: invalid collections currently load or save successfully.

**Step 3: Add typed errors and collection validation**

Extend `PersistenceError`:

```rust
#[error("connection group UUID {0} appears more than once")]
DuplicateGroupId(Uuid),
#[error("connection group name `{0}` appears more than once")]
DuplicateGroupName(String),
#[error("connection group `{0}` has an invalid name")]
InvalidGroupName(String),
#[error("profile {profile_id} references missing connection group {group_id}")]
UnknownProfileGroup { profile_id: Uuid, group_id: Uuid },
```

Implement one `validate_collection(&ProfileCollection)` called by both load and save. It must:

- call existing profile ID/access validation;
- reject duplicate group IDs;
- reject names where `ConnectionGroup::new(id, name) != original`;
- reject case-insensitive duplicate normalized names;
- reject every `profile.group_id` absent from the group ID set.

**Step 4: Run tests**

Run: `cargo test --test profile_groups`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/persistence/profiles.rs tests/profile_groups.rs
git commit -m "feat(persistence): validate connection group integrity"
```

---

### Task 4: Carry Groups and Order Through Startup and Runtime Registry

**Files:**
- Modify: `src/runtime.rs:107-200,206-247,2836-3140,3364-3404,3869-3940`
- Modify: `src/runtime.rs:3564-3585` (application bootstrap consuming `StartupProfiles`)
- Modify: `tests/profile_runtime.rs`
- Modify: `tests/connection_switch.rs`
- Modify: `tests/relation_runtime.rs`

**Step 1: Add failing runtime persistence tests**

Add tests that dispatch an organization save followed immediately by a normal profile edit and assert the final file retains both changes:

```rust
#[tokio::test]
async fn organization_and_profile_edits_share_one_serialized_registry() {}

#[tokio::test]
async fn runtime_reconstruction_preserves_groups_and_profile_order() {}
```

The first test is the race regression: dispatch group assignment/reorder, then save a renamed profile, consume success actions, reload the file, and assert group, membership, order, and renamed profile.

**Step 2: Run and confirm failure**

Run: `cargo test --test profile_runtime organization_and_profile_edits_share_one_serialized_registry`

Expected: compilation fails because runtime registry has no groups and no organization command.

**Step 3: Extend the registry, startup result, and constructors**

Add `groups: Vec<ConnectionGroup>` to `ProfileRegistry` and make its existing `order` remain authoritative. Change startup loading to hold:

```rust
pub struct StartupProfiles {
    pub collection: ProfileCollection,
    // existing persisted/secrets/selected/store fields
}
```

Compute `persisted` and CLI selection from `collection.profiles`. Append a direct CLI profile only to `collection.profiles`; it remains ungrouped and session-only.

Change `Runtime::new` to accept `ProfileCollection`. Update its limited direct call sites and test helper to wrap old vectors as:

```rust
ProfileCollection { groups: vec![], profiles }
```

Do not add a second mutation lock. Continue to use `profile_mutation` for every write to `connections.toml`.

**Step 4: Save one consistent registry snapshot**

Replace helpers that save only profile vectors with a helper that, while holding the registry lock long enough to clone a consistent state, builds:

```rust
ProfileCollection {
    groups: registry.groups.clone(),
    profiles: registry.order.iter()
        .filter_map(|id| registry.profiles.get(id).cloned())
        .collect(),
}
```

Release the registry lock before blocking file I/O, but retain the existing `profile_mutation` guard across registry mutation, persistence, and rollback/commit. Existing save/delete/access transactions must persist this complete collection so they cannot erase groups or order.

**Step 5: Run runtime regression tests**

Run: `cargo test --test profile_runtime && cargo test --test connection_switch && cargo test --test relation_runtime`

Expected: PASS.

**Step 6: Commit**

```bash
git add src/runtime.rs tests/profile_runtime.rs tests/connection_switch.rs tests/relation_runtime.rs
git commit -m "refactor(runtime): persist complete profile collections"
```

---

### Task 5: Implement Pure Organization Mutations

**Files:**
- Create: `src/model/profile_organization.rs`
- Modify: `src/model/mod.rs`
- Test: inline unit tests in `src/model/profile_organization.rs`

**Step 1: Write failing unit tests**

Test these pure operations without App or Runtime:

- create appends a canonical group and rejects duplicate names;
- rename preserves ID/order/members and rejects duplicate names;
- delete removes the group and clears all matching memberships;
- assignment accepts `None` and known IDs but rejects unknown IDs;
- visible sibling reorder swaps only same-region/same-group peers;
- first/last sibling moves are no-ops;
- moving one peer preserves all unrelated profile relative order.

**Step 2: Run and confirm failure**

Run: `cargo test profile_organization`

Expected: module/functions do not exist.

**Step 3: Implement the minimal pure API**

Use typed errors and mutate a `ProfileCollection` only after all validation succeeds:

```rust
pub enum OrganizationError {
    GroupNotFound(Uuid),
    ProfileNotFound(Uuid),
    DuplicateGroupName(String),
    InvalidGroupName(ConnectionGroupNameError),
    NoSibling,
}

pub enum MoveDirection { Up, Down }

pub fn create_group(collection: &mut ProfileCollection, id: Uuid, name: String)
    -> Result<(), OrganizationError>;
pub fn rename_group(collection: &mut ProfileCollection, id: Uuid, name: String)
    -> Result<(), OrganizationError>;
pub fn delete_group(collection: &mut ProfileCollection, id: Uuid)
    -> Result<usize, OrganizationError>;
pub fn assign_profile(collection: &mut ProfileCollection, profile_id: Uuid, group_id: Option<Uuid>)
    -> Result<(), OrganizationError>;
pub fn move_profile(collection: &mut ProfileCollection, profile_id: Uuid, sibling_ids: &[Uuid], direction: MoveDirection)
    -> Result<bool, OrganizationError>;
```

`move_profile` receives the already-filtered sibling IDs from App and swaps positions in the master `profiles` vector. Return `Ok(false)` at a boundary; do not use `NoSibling` for normal boundary behavior.

**Step 4: Run unit tests**

Run: `cargo test profile_organization`

Expected: PASS.

**Step 5: Commit**

```bash
git add src/model/mod.rs src/model/profile_organization.rs
git commit -m "feat(model): add connection organization operations"
```

---

### Task 6: Add Region-Aware Group Nodes to Explorer

**Files:**
- Modify: `src/model/explorer.rs:32-145,768-840,901-1287,1310-1760`
- Modify: `src/model/workspace.rs:241-251,788-988`
- Test: `tests/explorer_state.rs`
- Test: `tests/explorer_performance.rs`

**Step 1: Write failing projection tests**

Add fixtures with fixed group/profile UUIDs and test exact visible IDs/depths:

```rust
#[test]
fn one_group_is_projected_independently_in_primary_and_others() {}

#[test]
fn expanding_primary_group_does_not_expand_others_group() {}

#[test]
fn empty_group_occurrences_are_omitted() {}

#[test]
fn ungrouped_profiles_render_directly_in_each_region() {}

#[test]
fn group_members_follow_profile_order() {}
```

Expected shape after expansion:

```text
ConnectionGroup { group_id, region: Primary } depth 0
Profile(current) depth 1
Profile(global) depth 1
Others depth 0
ConnectionGroup { group_id, region: Others } depth 1
Profile(other) depth 2
```

**Step 2: Run and confirm failure**

Run: `cargo test --test explorer_state group_`

Expected: `ConnectionGroup` and region node IDs do not exist.

**Step 3: Add node identity and metadata**

Add:

```rust
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ProfileRegion { Primary, Others }

ExplorerNodeId::ConnectionGroup {
    group_id: Uuid,
    region: ProfileRegion,
}
```

Add `group_id: Option<Uuid>` to `ExplorerProfileState`. Add ordered `groups: Vec<ConnectionGroup>` to `ExplorerTreeState`, or provide it during projection; prefer storing it beside `profile_order` because visibility, node existence, labels, and selection fallback all need it.

Extend `add_profile_with_placement` with `group_id`, and add a synchronization method that updates group definitions and `profile_order` without replacing `ExplorerProfileState` values:

```rust
pub fn sync_organization(&mut self, groups: Vec<ConnectionGroup>, profile_order: Vec<Uuid>)
```

This method must retain catalog/load/status state and prune only nonexistent expansion IDs.

**Step 4: Replace placement-only projection with region/group projection**

For the primary region, include `CurrentProject` and `Global`. For `others`, include `OtherProject`. Within each region:

1. Iterate `groups` in persisted order.
2. Collect matching profile IDs by iterating `profile_order`.
3. Omit empty group occurrences.
4. Append the group node and its members if expanded.
5. Append ungrouped profiles in `profile_order` order.

Keep `Others` as the parent virtual node. Add group-aware handling to:

- `visible_parent`;
- `node_exists`;
- `is_expandable`/expand/collapse;
- ancestor lookup and sticky rows;
- selection fallback after synchronization;
- search frontend projection, ensuring a matching profile brings its occurrence group ancestor into the result.

**Step 5: Map group nodes to visible rows**

In `src/model/workspace.rs`, produce:

- label from `ExplorerTreeState.groups`;
- metadata as the member count for that region;
- `expandable = true`;
- no database kind, placement badge, endpoint, or connection status.

**Step 6: Preserve performance characteristics**

Avoid scanning the catalog for organization projection. Extend `tests/explorer_performance.rs` to prove collapsed groups do not visit catalog entries and visible projection remains proportional to visible profiles plus visited catalog nodes. A simple precomputed map local to `visible_with_visit_count` is sufficient; do not add persistent duplicate membership indexes yet.

**Step 7: Run Explorer tests**

Run: `cargo test --test explorer_state && cargo test --test explorer_performance`

Expected: PASS.

**Step 8: Commit**

```bash
git add src/model/explorer.rs src/model/workspace.rs tests/explorer_state.rs tests/explorer_performance.rs
git commit -m "feat(explorer): project connection groups by region"
```

---

### Task 7: Synchronize App State Without Losing Connection State

**Files:**
- Modify: `src/app.rs:191-221` and profile initialization/reducer helpers
- Modify: `tests/profile_reducer.rs`
- Modify: `tests/startup_profiles.rs`

**Step 1: Write failing App-state tests**

Add tests for:

```rust
#[test]
fn app_collection_constructor_projects_groups_and_order() {}

#[test]
fn organization_sync_preserves_profile_catalog_and_status() {}

#[test]
fn profile_edit_preserves_group_membership_and_order() {}
```

For catalog preservation, populate an Explorer profile catalog, mark it online, apply an organization change, then assert the same catalog entry and status remain.

**Step 2: Run and confirm failure**

Run: `cargo test --test profile_reducer organization_`

Expected: App has no group collection/synchronization.

**Step 3: Add App collection state and constructor**

Add:

```rust
pub connection_groups: Vec<ConnectionGroup>,
```

Keep:

```rust
pub fn new(profiles: Vec<ConnectionProfile>) -> Self {
    Self::from_profile_collection(ProfileCollection { groups: vec![], profiles })
}
```

Add `from_profile_collection` for startup. Build Explorer profiles in profile vector order, passing `group_id`, then synchronize group definitions. Startup must use the full collection constructor.

**Step 4: Centralize App organization snapshots**

Add helpers:

```rust
fn profile_collection(&self) -> ProfileCollection;
fn sync_explorer_organization(&mut self);
```

`sync_explorer_organization` only updates group metadata, group IDs, placements, and `profile_order`; it must not rebuild the normalized Explorer map.

When a profile save succeeds, preserve the submitted profile's existing `group_id` for edit operations unless the command explicitly changes group membership. Existing test `save_success_upserts_without_reordering_profiles` must continue to pass.

**Step 5: Run reducer/startup tests**

Run: `cargo test --test profile_reducer && cargo test --test startup_profiles`

Expected: PASS.

**Step 6: Commit**

```bash
git add src/app.rs tests/profile_reducer.rs tests/startup_profiles.rs
git commit -m "feat(app): synchronize connection organization state"
```

---

### Task 8: Persist Group CRUD, Membership, and Ordering Transactions

**Files:**
- Modify: `src/action.rs:200-260,760-810`
- Modify: `src/runtime.rs:206-247` and profile transaction section
- Modify: `src/app.rs` profile reducer section
- Modify: `tests/profile_runtime.rs`
- Modify: `tests/profile_reducer.rs`

**Step 1: Add failing reducer/runtime tests**

Cover success and failure for:

- create group;
- rename group;
- delete group and ungroup members;
- assign and unassign profile;
- move up/down;
- persistence failure leaves App unchanged;
- operations do not emit `Connect`, `Disconnect`, workspace deletion, or catalog-load commands.

**Step 2: Run and confirm failure**

Run: `cargo test --test profile_reducer profile_group && cargo test --test profile_runtime profile_group`

Expected: actions and commands do not exist.

**Step 3: Add one organization command protocol**

Avoid separate runtime code paths for every mutation. Define serializable intent:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProfileOrganizationMutation {
    CreateGroup { id: Uuid, name: String },
    RenameGroup { group_id: Uuid, name: String },
    DeleteGroup { group_id: Uuid },
    AssignProfile { profile_id: Uuid, group_id: Option<Uuid> },
    MoveProfile { profile_id: Uuid, sibling_ids: Vec<Uuid>, direction: MoveDirection },
}
```

Add:

```rust
Command::UpdateProfileOrganization { request_id, mutation }
Action::ProfileOrganizationSaved { request_id, collection: ProfileCollection }
Action::ProfileOrganizationSaveFailed { request_id, message: String }
```

Do not optimistically mutate App. App emits a command and marks the relevant overlay busy; only the success action installs the returned collection. This matches existing profile-save failure semantics and avoids App/runtime divergence.

**Step 4: Implement one runtime transaction**

While holding `profile_mutation`:

1. lock registry;
2. materialize a `ProfileCollection` in registry order;
3. apply the pure mutation from Task 5;
4. persist the complete collection atomically;
5. update registry groups/order/profiles only after save succeeds;
6. emit the saved collection;
7. on any failure, leave registry unchanged and emit a sanitized error.

No credential operations belong in this transaction.

**Step 5: Apply successful collections in App**

On success:

- replace `connection_groups` and reorder/update `profiles`;
- synchronize Explorer organization in place;
- retain active connection/workspaces/tabs;
- keep selection on the affected profile when it still exists;
- if a selected group occurrence disappears, select the first affected profile or its region parent;
- close the overlay and show a concise success notification.

**Step 6: Run tests**

Run: `cargo test --test profile_reducer && cargo test --test profile_runtime`

Expected: PASS.

**Step 7: Commit**

```bash
git add src/action.rs src/runtime.rs src/app.rs tests/profile_runtime.rs tests/profile_reducer.rs
git commit -m "feat(profiles): persist connection organization changes"
```

---

### Task 9: Add Group Management Overlay State

**Files:**
- Create: `src/model/profile_group.rs`
- Modify: `src/model/mod.rs`
- Modify: `src/model/workspace.rs` (`Overlay` definition)
- Modify: `src/action.rs`
- Modify: `src/app.rs`
- Test: `tests/profile_reducer.rs`

**Step 1: Write failing overlay-state tests**

Test keyboard-independent reducer behavior:

- opening picker preselects current group or Ungrouped;
- picker wraps selection and includes `Create group...`;
- create/rename trims and validates input before command emission;
- duplicate names remain in overlay with an error;
- delete confirmation reports affected member count;
- busy overlays ignore duplicate confirmation.

**Step 2: Run and confirm failure**

Run: `cargo test --test profile_reducer group_overlay`

Expected: overlay states/actions do not exist.

**Step 3: Implement focused state types**

Create small model types using existing `TextInput`:

```rust
pub enum ProfileGroupOverlay {
    Picker { profile_id: Uuid, selected: usize, busy: bool },
    Edit { group_id: Option<Uuid>, name: TextInput, error: Option<String>, busy: bool },
    DeleteConfirm { group_id: Uuid, member_count: usize, busy: bool },
}
```

Add `Overlay::ProfileGroup(ProfileGroupOverlay)`. Do not put this state in `ProfileManagerState` because groups are Explorer organization, not connection settings.

**Step 4: Add reducer actions**

Add open/move/insert/backspace/confirm/cancel actions. Generate UUID only when the create action is confirmed, then carry it in the command so retry/result matching is deterministic.

When opening edit/delete, require the selected node to be `ConnectionGroup`. When opening membership, require `Profile`. Session-only direct URL profiles must not be mutable through persisted organization controls; show no command or a notification explaining they are session connections.

**Step 5: Run reducer tests**

Run: `cargo test --test profile_reducer group_overlay`

Expected: PASS.

**Step 6: Commit**

```bash
git add src/model/mod.rs src/model/profile_group.rs src/model/workspace.rs src/action.rs src/app.rs tests/profile_reducer.rs
git commit -m "feat(explorer): add connection group interaction state"
```

---

### Task 10: Render Groups and Organization Overlays

**Files:**
- Create: `src/ui/profile_groups.rs`
- Modify: `src/ui/mod.rs:137-187,1410-1564` and overlay dispatcher
- Modify: `src/ui/icons.rs`
- Test: `tests/ui_render.rs`
- Test: `tests/mouse.rs`

**Step 1: Add failing render tests**

Use `TestBackend` snapshots/assertions for:

- group rows show folder/group icon, name, and region member count;
- primary/others group indentation is correct;
- group rows do not show PROJECT/GLOBAL/OTHER badges;
- picker shows `Ungrouped`, all groups in persisted order, and `+ Create group...`;
- create/rename displays validation errors;
- delete confirmation includes group name and member count;
- no password, URL, or endpoint data appears in group overlays.

Add mouse tests for picker rows and confirm/cancel controls.

**Step 2: Run and confirm failure**

Run: `cargo test --test ui_render profile_group && cargo test --test mouse profile_group`

Expected: group node/overlay rendering is absent.

**Step 3: Render group rows through the existing Explorer list**

Extend `explorer_list_item` to recognize `ConnectionGroup` separately from catalog `Group`. Use a neutral folder/tag-like icon from `IconSet`; do not reuse database catalog `ObjectGroup` icons semantically. Preserve selected, collapsed, and expanded styles and the existing hit-region mechanism.

**Step 4: Render compact overlays**

Use one centered bordered panel, no nested cards:

- picker max width 56, height bounded by available rows;
- create/rename max width 56 with one text input and error row;
- delete confirmation max width 64 with explicit `Delete` and `Cancel` actions;
- use existing theme action/error/selection styles;
- sanitize every group name before rendering.

Add specific `HitTarget` variants for option rows and buttons; do not infer actions from coordinates in App.

**Step 5: Run UI and mouse tests**

Run: `cargo test --test ui_render profile_group && cargo test --test mouse profile_group`

Expected: PASS.

**Step 6: Commit**

```bash
git add src/ui/profile_groups.rs src/ui/mod.rs src/ui/icons.rs tests/ui_render.rs tests/mouse.rs
git commit -m "feat(ui): render connection groups and management dialogs"
```

---

### Task 11: Wire Keyboard and Mouse Commands

**Files:**
- Modify: `src/input/keymap.rs:75-280` and Explorer key mapping section
- Modify: `src/runtime.rs` mouse-event mapping if required by existing architecture
- Modify: `src/help.rs`
- Test: keymap unit/integration tests colocated with existing keymap tests
- Test: `tests/mouse.rs`

**Step 1: Add failing keymap tests**

Lock down these context-sensitive keys:

- `g` on a saved profile opens membership picker;
- `a` on Explorer opens create-group input;
- `e` on a group opens rename;
- `d` on a group opens group delete confirmation, while `d` on a profile keeps deleting the profile;
- `K` moves a profile up among visible siblings;
- `J` moves a profile down among visible siblings;
- picker: `j/k` or arrows move, Enter confirms, Esc cancels;
- editor: characters insert, Backspace edits, Enter confirms, Esc cancels;
- delete confirmation: Enter/`y` confirms, Esc/`n` cancels;
- none of these bindings trigger on catalog object nodes.

Before finalizing, search existing key IDs and resolve conflicts without replacing established Explorer search/catalog shortcuts.

**Step 2: Run and confirm failure**

Run: `cargo test keymap -- profile_group`

Expected: actions are not mapped.

**Step 3: Implement overlay-first key handling**

Handle `Overlay::ProfileGroup` before generic Explorer mappings, following the existing `ProfileAccess` overlay pattern. Clear pending Vim sequences when an organization overlay is active.

For `J/K`, App derives sibling IDs from the current Explorer region and group. Do not let Runtime derive visibility from project paths because App already owns the current projection.

**Step 4: Add help entries**

Add concise Explorer help rows for group picker, create group, edit group, delete group, and move connection. Reuse keys contextually in descriptions.

**Step 5: Run input/mouse tests**

Run: `cargo test keymap && cargo test --test mouse`

Expected: PASS.

**Step 6: Commit**

```bash
git add src/input/keymap.rs src/runtime.rs src/help.rs tests/mouse.rs
git commit -m "feat(input): add connection organization shortcuts"
```

---

### Task 12: Cover Search, Selection, and Lifecycle Regressions

**Files:**
- Modify: `tests/explorer_state.rs`
- Modify: `tests/profile_reducer.rs`
- Modify: `tests/connection_switch.rs`
- Modify: `tests/startup_profiles.rs`
- Modify: `tests/workspace_persistence.rs` if constructor updates require it

**Step 1: Add high-risk regression tests**

Add end-to-end state tests proving:

1. Search results include the correct region-specific group ancestor.
2. Reveal of a grouped catalog object expands its profile, group occurrence, and `others` when applicable.
3. Sticky ancestors include the group occurrence and do not collide across regions.
4. Moving a connected profile between groups preserves online status and loaded catalog.
5. Reordering a connected profile emits no connection command and preserves active profile.
6. Group deletion while a member has open tabs preserves all tabs/workspaces.
7. Renaming a profile after reordering preserves its new position.
8. Changing `ProfileAccess` moves the profile and its group occurrence between regions without changing membership.
9. Deleting a profile removes it from its group projection but preserves the group definition.
10. Session profiles remain ungrouped and are omitted from persistent organization mutations.

**Step 2: Run focused regressions and fix only demonstrated defects**

Run:

```bash
cargo test --test explorer_state
cargo test --test profile_reducer
cargo test --test connection_switch
cargo test --test startup_profiles
cargo test --test workspace_persistence
```

Expected: PASS. Keep fixes localized; do not redesign catalog IDs or workspace snapshots.

**Step 3: Commit**

```bash
git add tests/explorer_state.rs tests/profile_reducer.rs tests/connection_switch.rs tests/startup_profiles.rs tests/workspace_persistence.rs src
git commit -m "test(explorer): cover grouped connection lifecycle"
```

---

### Task 13: Document Configuration and Complete Verification

**Files:**
- Modify: `README.md` connection management section
- Modify: `CHANGELOG.md` unreleased section if present
- Modify: example connection configuration documentation if present

**Step 1: Document user-visible behavior**

Document:

- group CRUD and shortcuts;
- a connection can belong to at most one group;
- the same group appears in both primary and `others` when it has members there;
- deleting a group ungroups rather than deletes connections;
- `J/K` ordering scope;
- the V6 TOML shape without credentials.

Example:

```toml
version = 6

[[groups]]
id = "11111111-1111-1111-1111-111111111111"
name = "Production"

[[profiles]]
id = "22222222-2222-2222-2222-222222222222"
name = "Billing"
group_id = "11111111-1111-1111-1111-111111111111"
# existing connection fields follow
```

**Step 2: Format and compile**

Run: `cargo fmt --check && cargo check`

Expected: PASS with no warnings/errors.

If formatting fails, run `cargo fmt`, inspect the diff, then rerun the check.

**Step 3: Run focused suites**

Run:

```bash
cargo test --test profile_groups
cargo test --test profile_runtime
cargo test --test profile_reducer
cargo test --test explorer_state
cargo test --test explorer_performance
cargo test --test ui_render
cargo test --test mouse
cargo test --test connection_switch
cargo test --test startup_profiles
```

Expected: PASS.

**Step 4: Run full verification**

Run:

```bash
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS.

**Step 5: Inspect the final diff**

Run: `git diff --check && git status --short`

Expected: no whitespace errors; only intended source, test, and documentation files are changed.

**Step 6: Commit**

```bash
git add README.md CHANGELOG.md docs src tests
git commit -m "docs: document Explorer connection groups"
```

---

## Manual Acceptance Checklist

1. Start with a V5 `connections.toml`; verify all existing connections appear in their old order and no group is shown.
2. Create `Production` and `Development`; verify empty groups do not clutter Explorer.
3. Assign a current-project connection and an other-project connection to `Production`; verify `Production` appears both above and inside `others`.
4. Expand only the primary `Production`; verify the `others` occurrence stays collapsed.
5. Move one connection back to ungrouped; verify its active connection, catalog expansion, and tabs remain intact.
6. Rename `Production` to `Critical`; verify both occurrences update together.
7. Reorder two connections in one group; restart LazyDB and verify the order persists.
8. Try moving the first connection up and last connection down; verify both are harmless no-ops.
9. Delete `Critical`; verify its connections remain and become ungrouped.
10. Change one grouped connection from current-project to another project using the existing access UI; verify it moves under the same group in `others`.
11. Open with a direct `--url` session profile; verify group persistence controls do not modify it.
12. Force a profile-file write failure; verify the visible group/order state does not claim success and an error notification appears.

## Rollback and Compatibility Notes

- The migration is read-compatible with V2-V5 and write-only as V6 after the first successful mutation.
- V6 is not expected to be readable by older LazyDB builds; document this in release notes.
- No database or workspace data migration is involved.
- If a V6 save fails, the existing atomic temp-file/rename behavior must leave the prior file intact.
- Runtime transactions must update in-memory registry state only after persistence succeeds, making rollback a no-op rather than reconstructing state.
