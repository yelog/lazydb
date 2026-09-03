# Catalog Create Capabilities Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the Explorer `a` action show every catalog object that is both valid for the selected anchor and supported by the active database connection, including Table, View, Materialized View, and Sequence for a writable PostgreSQL schema.

**Architecture:** Keep `CatalogMutationCapabilities` as the single source of truth for supported mutations and anchor-to-object compatibility. Add one side-effect-free `App` query that resolves the selected anchor, catalog entry, connection constraints, and runtime capabilities; both Help/keymap availability and `OpenCatalogCreate` consume that query. Store the capabilities produced by the actual connected adapter so PostgreSQL version-dependent form options do not fall back to the static PostgreSQL 12 profile.

**Tech Stack:** Rust 2024, Tokio, SQLx, Ratatui, Cargo integration tests.

---

## Scope And Constraints

This plan includes:

- Table, View, Materialized View, and Sequence creation from a PostgreSQL Schema node.
- Object-specific creation from Tables, Views, Materialized Views, and Sequences group nodes.
- Creation draft initialization for all object types exposed by the picker.
- One shared availability/query path for Help, keymap, and the reducer.
- Runtime PostgreSQL version capabilities, especially View options.
- Read-only, disconnected, wrong-profile, stale-entry, and wrong-target safeguards.

This plan deliberately excludes Function, Procedure, Type, and Trigger creation. Those objects are discoverable in Explorer, but there are no matching `CatalogObjectDefinition`, Draft, form, validation, and PostgreSQL planner implementations. They must remain unavailable until that complete vertical slice exists.

Do not modify or revert unrelated working-tree changes in `config/default.toml`, `src/config.rs`, `src/input/keymap.rs`, `src/runtime.rs`, `tests/ui_render.rs`, or other files. Where this plan touches an already modified file, preserve the existing changes and apply only the targeted hunks.

## Target Behavior

| Selected Explorer node | PostgreSQL create picker |
|---|---|
| Profile | Database, Login Role, Role |
| Database | Schema |
| Schema | Table, View, Materialized View, Sequence |
| Tables group | Table |
| Views group | View |
| Materialized Views group | Materialized View |
| Sequences group | Sequence |
| Table | Primary Key, Unique, Foreign Key, Check |
| Materialized View | No create action until Index creation UI is complete |

The picker must show an option only when its creation Draft, editable form, and planner are all usable. Current inspection confirms that Column creation is not implemented: the PostgreSQL Column planner requires a Column catalog anchor and a loaded Table baseline, which is an edit-only flow. Index SQL planning exists, but `IndexDraft` has no creation constructor and the reducer/keymap has no Index form input handling. Mark both create capabilities unavailable in this change; retain their existing edit capabilities and planners. Do not leave a selectable option that produces `draft == None` or a display-only form.

### Task 1: Correct Schema Anchor Capabilities

**Files:**
- Modify: `src/db/catalog_mutation.rs:188-250`
- Modify: `src/db/postgres.rs:563-608`
- Test: `tests/catalog_mutation.rs:510-524`
- Test: `tests/catalog_mutation.rs:790-857`
- Test: `tests/catalog_mutation.rs:1240-1270`

**Step 1: Write the failing Schema capability test**

Add a focused test near the existing mutation capability tests. Use a PostgreSQL capability set and a Schema `CatalogMutationAnchor`:

```rust
#[test]
fn postgres_schema_create_options_include_every_implemented_schema_object() {
    let profile = Uuid::new_v4();
    let schema = id(profile, CatalogKind::Schema, &["app", "public"]);
    let options = lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities()
        .create_options(&CatalogMutationAnchor::Catalog(schema), None)
        .unwrap();

    assert_eq!(
        options,
        vec![
            CatalogObjectType::Catalog(CatalogKind::Table),
            CatalogObjectType::Catalog(CatalogKind::View),
            CatalogObjectType::Catalog(CatalogKind::MaterializedView),
            CatalogObjectType::Catalog(CatalogKind::Sequence),
        ]
    );
}
```

Also add assertions for the four corresponding `ObjectGroup` anchors so group behavior remains narrow and deterministic.

Add a second test that records the fail-closed capability boundary:

```rust
#[test]
fn postgres_create_capabilities_hide_incomplete_column_and_index_flows() {
    let capabilities =
        lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities();
    for kind in [CatalogKind::Column, CatalogKind::Index] {
        assert!(matches!(
            capabilities.create_availability(CatalogObjectType::Catalog(kind)),
            Some(CatalogMutationAvailability::Unavailable { .. })
        ));
        assert_eq!(
            capabilities.edit_availability(CatalogObjectType::Catalog(kind)),
            Some(CatalogMutationAvailability::Available)
        );
    }
}
```

Update `postgres_index_capabilities_cover_table_and_materialized_view`: it must now assert that `create_options()` does not offer Index from either relation until the creation form is implemented. Keep the Index planner tests; planner support remains valid and can be re-exposed later.

**Step 2: Run the test to verify it fails**

Run:

```bash
cargo test --test catalog_mutation postgres_schema_create_options_include_every_implemented_schema_object -- --exact
```

Expected: FAIL because `Sequence` is absent from the Schema options; the incomplete-flow test also fails because Column and Index are currently advertised as available.

**Step 3: Add Sequence to the Schema anchor rule**

Update the `CatalogKind::Schema` branch in `CatalogMutationCapabilities::create_options()`:

```rust
CatalogKind::Schema => vec![
    CatalogObjectType::Catalog(CatalogKind::Table),
    CatalogObjectType::Catalog(CatalogKind::View),
    CatalogObjectType::Catalog(CatalogKind::MaterializedView),
    CatalogObjectType::Catalog(CatalogKind::Sequence),
],
```

Keep the existing intersection with `self.create`; anchor validity and adapter support must both be required.

In `PostgresAdapter::catalog_mutation_capabilities_for_version()`, change only the `create` entries for Column and Index:

```rust
CatalogMutationOption {
    object_type: CatalogObjectType::Catalog(CatalogKind::Column),
    availability: CatalogMutationAvailability::Unavailable {
        reason: "column creation form is not implemented",
    },
},
CatalogMutationOption {
    object_type: CatalogObjectType::Catalog(CatalogKind::Index),
    availability: CatalogMutationAvailability::Unavailable {
        reason: "index creation form is not implemented",
    },
},
```

Do not change their `edit` entries.

**Step 4: Run the focused capability tests**

Run:

```bash
cargo test --test catalog_mutation postgres_schema_create_options_include_every_implemented_schema_object -- --exact
cargo test --test catalog_mutation postgres_create_capabilities_hide_incomplete_column_and_index_flows -- --exact
cargo test --test catalog_mutation mutation_model_capabilities_have_labels_and_validate_selection -- --exact
cargo test --test catalog_mutation postgres_index_capabilities_cover_table_and_materialized_view -- --exact
```

Expected: all PASS.

**Step 5: Commit this isolated model change**

```bash
git add src/db/catalog_mutation.rs src/db/postgres.rs tests/catalog_mutation.rs
git commit -m "fix(catalog): align create capabilities with implemented flows"
```

Only commit if explicitly requested by the user. Otherwise leave the verified changes uncommitted.

### Task 2: Guarantee A Draft For Every Exposed Create Option

**Files:**
- Modify: `src/model/catalog_editor.rs:248-309`
- Modify: `src/model/catalog_editor.rs:948-992`
- Modify: `src/model/catalog_editor.rs:1213-1369`
- Test: `tests/catalog_editor_state.rs:63-129`

**Step 1: Write failing creation-Draft tests**

Add a helper that creates a `CatalogEditorState` for a schema and selects one option:

```rust
fn select_schema_create(object_type: CatalogObjectType) -> CatalogEditorState {
    let mut editor = CatalogEditorState::new(
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Catalog(lazydb::db::catalog::CatalogId::new(
            profile(),
            lazydb::db::catalog::CatalogKind::Schema,
            ["app", "public"],
        )),
        1,
        vec![CatalogMutationOption {
            object_type,
            label: object_type.display_label().into(),
        }],
    );
    assert!(editor.select_option(0));
    editor
}
```

Add a table test:

```rust
#[test]
fn schema_table_create_selection_initializes_table_draft() {
    let editor = select_schema_create(CatalogObjectType::Catalog(
        lazydb::db::catalog::CatalogKind::Table,
    ));
    let Some(CatalogDraft::Table(draft)) = editor.draft else {
        panic!("table draft expected");
    };
    assert_eq!(draft.schema.value(), "public");
    assert!(draft.name.value().is_empty());
    assert!(draft.columns.is_empty());
}
```

Add a table-driven test for View, Materialized View, and Sequence asserting that each selection creates the matching `CatalogDraft` and initializes schema to `public`.

Add equivalent group-anchor tests. The group carries the same Schema ID and must initialize the same Draft as direct Schema selection.

Add a Database-anchor test for Schema creation. This is another currently exposed option that must initialize a real Draft:

```rust
#[test]
fn database_schema_create_selection_initializes_schema_draft() {
    let mut editor = CatalogEditorState::new(
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Catalog(lazydb::db::catalog::CatalogId::new(
            profile(),
            lazydb::db::catalog::CatalogKind::Database,
            ["app"],
        )),
        1,
        vec![CatalogMutationOption {
            object_type: CatalogObjectType::Catalog(
                lazydb::db::catalog::CatalogKind::Schema,
            ),
            label: "Schema".into(),
        }],
    );

    assert!(editor.select_option(0));
    assert!(matches!(editor.draft, Some(CatalogDraft::Schema(_))));
}
```

**Step 2: Run the tests to verify Table fails**

Run:

```bash
cargo test --test catalog_editor_state schema_table_create_selection_initializes_table_draft -- --exact
```

Expected: FAIL because selecting Table currently leaves `editor.draft` as `None`.

**Step 3: Add a minimal TableDraft constructor**

Add a constructor next to `TableDraft::from_definition()`:

```rust
pub fn new(schema: impl Into<String>) -> Self {
    Self {
        name: TextInput::default(),
        schema: schema.into().into(),
        owner: TextInput::default(),
        comment: TextInput::default(),
        columns: Vec::new(),
        selected_section: CatalogEditorSection::General,
        selected_column: 0,
        indexes: Vec::new(),
        constraints: Vec::new(),
    }
}
```

If `impl Into<String>` causes ambiguous `.into()` inference, bind it first:

```rust
let schema: String = schema.into();
```

Then use `schema.into()` for `TextInput`.

Do not add a synthetic empty column in this task. The current table validator permits zero columns, and introducing implicit row state is a separate UX decision.

**Step 4: Refactor create selection into one typed Draft factory**

Replace the series of independent create-only `if` blocks in `CatalogEditorState::select_option()` with one internal helper. Keep edit/loading behavior unchanged.

Suggested shape:

```rust
fn create_draft(
    anchor: &CatalogMutationAnchor,
    object_type: CatalogObjectType,
) -> Option<CatalogDraft> {
    let schema = match anchor {
        CatalogMutationAnchor::Catalog(id) if id.kind == CatalogKind::Schema => Some(id),
        CatalogMutationAnchor::Group { schema, .. } => Some(schema),
        _ => None,
    };

    match (anchor, schema, object_type) {
        (CatalogMutationAnchor::Profile { .. }, _, CatalogObjectType::Catalog(CatalogKind::Database)) => {
            Some(CatalogDraft::Database(DatabaseDraft::new("")))
        }
        (CatalogMutationAnchor::Profile { .. }, _, CatalogObjectType::LoginRole) => {
            Some(CatalogDraft::Role(RoleDraft::new(true)))
        }
        (CatalogMutationAnchor::Profile { .. }, _, CatalogObjectType::Role) => {
            Some(CatalogDraft::Role(RoleDraft::new(false)))
        }
        (CatalogMutationAnchor::Catalog(database), _, CatalogObjectType::Catalog(CatalogKind::Schema))
            if database.kind == CatalogKind::Database =>
        {
            Some(CatalogDraft::Schema(SchemaDraft {
                name: TextInput::default(),
                owner: TextInput::default(),
                comment: TextInput::default(),
            }))
        }
        (_, Some(schema), CatalogObjectType::Catalog(CatalogKind::Table)) => {
            Some(CatalogDraft::Table(TableDraft::new(schema_name(schema))))
        }
        (_, Some(schema), CatalogObjectType::Catalog(CatalogKind::View)) => {
            Some(CatalogDraft::View(new_view_draft(schema_name(schema))))
        }
        (_, Some(schema), CatalogObjectType::Catalog(CatalogKind::MaterializedView)) => {
            Some(CatalogDraft::MaterializedView(new_materialized_view_draft(schema_name(schema))))
        }
        (_, Some(schema), CatalogObjectType::Catalog(CatalogKind::Sequence)) => {
            Some(CatalogDraft::Sequence(new_sequence_draft(schema_name(schema))))
        }
        _ => create_relation_child_draft(anchor, object_type),
    }
}
```

Use small private helpers only where they remove duplicated struct literals. Keep this in `src/model/catalog_editor.rs`; do not introduce a new module.

Set `self.draft` once:

```rust
if self.mode == CatalogMutationMode::Create {
    self.draft = create_draft(&self.anchor, self.options[selected].object_type);
}
```

The helper must preserve current Database, Role, Constraint, View, Materialized View, and Sequence defaults exactly.

**Step 5: Decide and enforce invariant behavior**

`select_option()` currently returns `true` even when no Draft exists. Tighten it for create mode so unsupported combinations fail closed:

```rust
if self.mode == CatalogMutationMode::Create && draft.is_none() {
    self.error = Some("The selected object type cannot be created from this target".into());
    return false;
}
```

Do not update `object_type` or move to `Form` until the Draft is successfully constructed. Add a test with an invalid anchor/object combination and assert:

- `select_option(0)` returns `false`.
- `page` stays `ObjectPicker`.
- `object_type` stays `None`.
- `draft` stays `None`.
- `error` is populated.

**Step 6: Run Catalog Editor state tests**

Run:

```bash
cargo test --test catalog_editor_state
```

Expected: all PASS.

**Step 7: Run PostgreSQL planner tests for newly exposed objects**

Run:

```bash
cargo test --test catalog_mutation postgres_table_create_and_edit_plan_is_quoted_ordered_and_destructive_when_needed -- --exact
cargo test --test catalog_mutation sequence_bounds_keep_unset_distinct_from_no_limit_and_validate_numbers -- --exact
cargo test --test postgres_adapter materialized -- --nocapture
```

Expected: all selected tests PASS. If the substring filter matches zero tests, list tests with `cargo test --test postgres_adapter -- --list` and run the exact materialized-view create planner test shown by that command.

**Step 8: Commit the Draft invariant change**

```bash
git add src/model/catalog_editor.rs tests/catalog_editor_state.rs
git commit -m "fix(catalog): initialize drafts for schema object creation"
```

Only commit if explicitly requested.

### Task 3: Add One App-Level Create Option Query

**Files:**
- Modify: `src/app.rs:6613-6618`
- Modify: `src/help.rs:2353-2395`
- Test: `tests/catalog_editor_reducer.rs`
- Test: `tests/keymap.rs:1230-1360`

**Step 1: Write failing reducer tests for Schema picker contents**

In `tests/catalog_editor_reducer.rs`, add a reusable PostgreSQL fixture that:

- Creates a writable PostgreSQL `ConnectionProfile` targeting database `app` and schema `public`.
- Drives `Action::RequestConnect` and matching `Action::ConnectionSucceeded`.
- Inserts Database and Schema entries into the normalized catalog.
- Selects the Schema node.

Then add:

```rust
#[test]
fn opening_create_on_schema_uses_capability_ordered_options() {
    let mut app = connected_postgres_schema_app();
    app.update(Action::OpenCatalogCreate);

    let editor = app.catalog_editor.as_ref().expect("catalog editor");
    assert_eq!(editor.page, CatalogEditorPage::ObjectPicker);
    assert_eq!(
        editor.options.iter().map(|option| option.object_type).collect::<Vec<_>>(),
        vec![
            CatalogObjectType::Catalog(CatalogKind::Table),
            CatalogObjectType::Catalog(CatalogKind::View),
            CatalogObjectType::Catalog(CatalogKind::MaterializedView),
            CatalogObjectType::Catalog(CatalogKind::Sequence),
        ]
    );
}
```

Add a label assertion using `display_label()`, rather than duplicating literal labels in fixture setup.

**Step 2: Run the test to reproduce the reported bug**

Run:

```bash
cargo test --test catalog_editor_reducer opening_create_on_schema_uses_capability_ordered_options -- --exact
```

Expected: FAIL with actual options equal to `[View]`.

**Step 3: Introduce a side-effect-free App query**

Add a crate-visible return type near `CatalogRequestIntent` or use a tuple if it remains readable:

```rust
pub(crate) struct CatalogCreateSelection {
    pub anchor: CatalogMutationAnchor,
    pub catalog_epoch: u64,
    pub options: Vec<CatalogObjectType>,
}
```

Add a method near `resolve_explorer_mutation_intent()`:

```rust
pub(crate) fn selected_catalog_create_options(&self) -> Option<CatalogCreateSelection>
```

It must perform these checks in order:

1. Resolve `ExplorerMutationIntent::Create(anchor)` from the direct Explorer selection.
2. Resolve the selected `profile_id` and matching profile.
3. Require a writable profile.
4. Require a connected `database_command_identity()` for the same profile.
5. Require a valid current connection target for the profile.
6. Resolve the selected profile's normalized catalog state and copy its `catalog_epoch`; do not use the global frontend `explorer.catalog_generation` as a mutation epoch.
7. For Catalog anchors, fetch and validate the selected `CatalogEntry` from that profile's normalized catalog.
8. For Schema and Group anchors, require the anchor database to equal `connection.target.database`.
9. Ask the active mutation capabilities for `create_options(&anchor, entry)`.
10. Return `None` if the list is empty.

At this stage, before Task 5 adds runtime storage, isolate capability lookup in one helper:

```rust
fn catalog_mutation_capabilities_for_profile(
    &self,
    profile: &ConnectionProfile,
) -> CatalogMutationCapabilities
```

For PostgreSQL, temporarily return `PostgresAdapter::catalog_mutation_capabilities()`; for all other kinds return their adapter's static capability method. This temporary helper provides one replacement point in Task 5 and removes the direct PostgreSQL-only check from Help.

Do not emit notifications from this query. Help rendering and key mapping call it frequently and must stay side-effect free.

**Step 4: Use the shared query in Help**

Replace `help.rs::catalog_editor_capabilities()`'s independent profile-kind, entry, anchor, and static capability logic with:

```rust
let create = app.selected_catalog_create_options().is_some();
```

Keep profile-edit resolution separate. For edit support, either retain the existing logic in this task or add a symmetric `selected_catalog_edit_available()` only if doing so shortens the function without changing behavior.

The keymap already calls `shortcut_is_available_in_app()`, so no keymap production change should be necessary.

**Step 5: Run Help/keymap tests**

Run:

```bash
cargo test --test keymap catalog
cargo test --test catalog_editor_reducer help_edit_shortcut_uses_direct_selection_resolution -- --exact
```

Expected: all PASS.

**Step 6: Add guard-condition tests**

Add reducer or keymap tests asserting the shared query and `a` shortcut are unavailable when:

- The PostgreSQL profile is read-only.
- The selected Schema belongs to a different profile.
- The selected Schema database differs from the active connection target.
- The profile is disconnected.
- The selection is a status/loading row.
- The adapter capabilities return no supported mutations, as with current MySQL, SQLite, and SQL Server implementations.

Prefer table-driven tests for database kinds to avoid duplicating setup.

**Step 7: Delete the hard-coded picker match**

Replace `src/app.rs:2903-2985` with conversion of the shared query result:

```rust
let Some(selection) = self.selected_catalog_create_options() else {
    self.notify_warning("Catalog", "No catalog objects can be created from this selection");
    return Vec::new();
};
let options = selection
    .options
    .into_iter()
    .map(|object_type| crate::model::catalog_editor::CatalogMutationOption {
        object_type,
        label: object_type.display_label().into(),
    })
    .collect();
self.catalog_editor = Some(CatalogEditorState::new(
    CatalogMutationMode::Create,
    selection.anchor,
    selection.catalog_epoch,
    options,
));
self.overlay = Some(Overlay::CatalogEditor);
```

`CatalogEditorState::new()` already chooses `ObjectPicker` for create mode. Remove the redundant `has_options` variable and manual page assignment.

**Step 8: Run the bug regression test**

Run:

```bash
cargo test --test catalog_editor_reducer opening_create_on_schema_uses_capability_ordered_options -- --exact
```

Expected: PASS with the four Schema object types.

**Step 9: Verify group-specific picker behavior**

Add and run a table-driven reducer test that selects each group, dispatches `OpenCatalogCreate`, and verifies exactly one corresponding option:

```bash
cargo test --test catalog_editor_reducer opening_create_on_group_shows_only_group_object -- --exact
```

Expected: PASS.

**Step 10: Commit the shared-query integration**

```bash
git add src/app.rs src/help.rs tests/catalog_editor_reducer.rs tests/keymap.rs
git commit -m "fix(catalog): derive create picker from capabilities"
```

Only commit if explicitly requested.

### Task 4: Cover Picker Rendering

**Files:**
- Modify: `tests/ui_render.rs`
- Verify: `src/ui/catalog_editor.rs:53-93`

**Step 1: Add a focused picker rendering test**

Construct a `CatalogEditorState` with the four Schema options and render it using the existing `TestBackend` helpers. Assert the buffer contains:

```text
Choose an object type
Table
View
Materialized View
Sequence
```

Also assert the selected row uses the expected selection marker or style according to the existing UI test conventions.

Do not alter `src/ui/catalog_editor.rs` unless this test reveals clipping or layout issues. The current list renderer already iterates all `editor.options`.

**Step 2: Run the rendering test**

Run:

```bash
cargo test --test ui_render catalog_editor_create_picker_lists_schema_objects -- --exact
```

Expected: PASS without production UI changes.

**Step 3: Run all Catalog Editor UI tests**

Run:

```bash
cargo test --test ui_render catalog_editor
```

Expected: all matching tests PASS.

**Step 4: Commit the rendering regression test**

```bash
git add tests/ui_render.rs
git commit -m "test(catalog): cover schema create picker rendering"
```

Only commit if explicitly requested.

### Task 5: Carry Runtime Mutation Capabilities Through Connection State

**Files:**
- Modify: `src/action.rs:542-546`
- Modify: `src/runtime.rs:1016-1073`
- Modify: `src/model/workspace.rs:205-240`
- Modify: `src/app.rs:5335-5400`
- Modify: `src/app.rs:4540-4570`
- Modify: `src/app.rs:5545-5565`
- Modify: `src/app.rs:7840-7895`
- Modify: all test fixtures constructing `Action::ConnectionSucceeded`
- Test: `tests/catalog_editor_reducer.rs`
- Test: `tests/connection_switch.rs`
- Test: `tests/profile_runtime.rs`

**Step 1: Write a failing runtime-version capability test**

Add a reducer test that connects with an explicit capability payload representing PostgreSQL 15 and opens a View create Draft. Assert:

```rust
assert!(draft.security_invoker.availability.is_available());
```

Add the complementary PostgreSQL 14 assertion:

```rust
assert!(!draft.security_invoker.availability.is_available());
```

The test should not infer capabilities by parsing `ServerInfo.version`; construct them with:

```rust
PostgresAdapter::catalog_mutation_capabilities_for_version(150_000)
PostgresAdapter::catalog_mutation_capabilities_for_version(140_000)
```

Expected before implementation: the action/state cannot carry this payload, or View creation retains the default unavailable options.

**Step 2: Extend the connection success action**

Change the action to:

```rust
ConnectionSucceeded {
    profile_id: Uuid,
    generation: u64,
    server: ServerInfo,
    mutation_capabilities: CatalogMutationCapabilities,
},
```

Import `CatalogMutationCapabilities` in `src/action.rs`.

Do not add capabilities to `ServerInfo`; server identity and client feature capabilities have different lifecycles.

**Step 3: Produce capabilities from the connected adapter**

In `src/runtime.rs`, compute capabilities before moving the `DatabaseConnection` into `ActiveConnection`:

```rust
let mutation_capabilities = database.catalog_mutation_capabilities();
```

Send them in `Action::ConnectionSucceeded`. Because this invokes the concrete PostgreSQL adapter's `mutation_capabilities()`, it preserves the actual `server_version_num` obtained during connect.

**Step 4: Store capabilities in ConnectionState**

Add:

```rust
pub mutation_capabilities: CatalogMutationCapabilities,
```

`CatalogMutationCapabilities` already implements `Default`, so `ConnectionState::default()` remains straightforward.

In `App::update(Action::ConnectionSucceeded { ... })`, assign the received value only after all existing generation, target, and profile checks pass:

```rust
self.connection.mutation_capabilities = mutation_capabilities;
```

On every path that clears `connection.profile_id`/`connection.server`, also reset:

```rust
self.connection.mutation_capabilities = Default::default();
```

This prevents stale PostgreSQL capabilities from surviving disconnect or profile switches.

**Step 5: Update all ConnectionSucceeded fixtures mechanically**

For tests that do not inspect mutation behavior, add:

```rust
mutation_capabilities: Default::default(),
```

For PostgreSQL Catalog Editor tests, provide:

```rust
mutation_capabilities:
    lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities_for_version(150_000),
```

Update exhaustive pattern matches to include `..` where fields are irrelevant. Do not change wildcard matches that already compile.

Likely affected tests, based on current source search:

- `tests/catalog_editor_reducer.rs`
- `tests/catalog_reducer.rs`
- `tests/connection_switch.rs`
- `tests/keymap.rs`
- `tests/profile_reducer.rs`
- `tests/relation_runtime.rs`
- `tests/sql_completion.rs`
- `tests/sql_execution.rs`
- `tests/ui_render.rs`
- `tests/workspace_tabs.rs`
- Internal tests in `src/app.rs`

Use compiler errors to locate any remaining constructors; do not perform unrelated rewrites.

**Step 6: Replace static capability lookup**

Update `App::selected_catalog_create_options()` to use:

```rust
&self.connection.mutation_capabilities
```

after verifying that the selected profile matches the active connection identity. Delete the temporary static adapter helper from Task 3.

Update Help indirectly through the shared App query; `src/help.rs` must no longer call `PostgresAdapter::catalog_mutation_capabilities()`.

**Step 7: Apply View option capabilities when constructing its Draft**

Pass `ViewMutationCapabilities` into the Draft factory or set the options immediately after successful View Draft construction:

```rust
security_barrier: ViewOption {
    availability: capabilities.view_options.security_barrier,
    value: None,
},
security_invoker: ViewOption {
    availability: capabilities.view_options.security_invoker,
    value: None,
},
check_option: ViewOption {
    availability: capabilities.view_options.check_option,
    value: None,
},
```

Prefer passing only `ViewMutationCapabilities` to `CatalogEditorState::new()` or a create-Draft method rather than storing the entire adapter capability set in editor state. The editor needs a snapshot for form construction, not a second mutable source of truth.

If changing `CatalogEditorState::new()` would force broad unrelated fixture churn, add an explicit method used before the picker is displayed:

```rust
editor.set_view_mutation_capabilities(capabilities.view_options);
```

Then ensure `select_option()` consumes that snapshot. Choose the smaller API after checking compiler impact.

**Step 8: Run runtime-version tests**

Run:

```bash
cargo test --test catalog_editor_reducer view_create_uses_connected_postgres_version_capabilities -- --exact
cargo test --test connection_switch
cargo test --test profile_runtime
```

Expected: all PASS; PostgreSQL 14 and 15 differ only where the adapter declares version-dependent support.

**Step 9: Run all compile checks to catch action constructors**

Run:

```bash
cargo check --all-targets
```

Expected: PASS with no missing `mutation_capabilities` fields.

**Step 10: Commit runtime capability propagation**

```bash
git add src/action.rs src/runtime.rs src/model/workspace.rs src/app.rs src/help.rs tests
git commit -m "refactor(catalog): use connected mutation capabilities"
```

Before staging, inspect `git diff` and stage only hunks belonging to this task because several listed files currently contain unrelated local changes. Only commit if explicitly requested.

### Task 6: End-To-End Safety And Consistency Tests

**Files:**
- Modify: `tests/catalog_editor_reducer.rs`
- Modify: `tests/keymap.rs`
- Modify: `tests/catalog_mutation.rs`
- Modify: `tests/ui_render.rs`

**Step 1: Add Help/picker consistency test**

For every supported selection fixture, assert:

- `shortcut_is_available_in_app(... ExplorerCreateCatalog)` is true exactly when `selected_catalog_create_options()` returns a non-empty list.
- Dispatching `OpenCatalogCreate` produces `editor.options` with the exact same ordered object types.
- The created editor stores the selected profile state's `catalog_epoch`, not `explorer.catalog_generation`.

This test is the regression barrier against reintroducing separate Help and reducer rule sets.

**Step 2: Add unsupported-object test**

Assert that Functions, Procedures, Types, and Triggers groups do not expose `a`, do not open Catalog Editor, and do not appear in Schema picker options.

**Step 3: Add invalid selection tests**

Assert no picker opens for:

- Status rows.
- Catalog entries missing from normalized catalog.
- Cross-profile anchors.
- Wrong target database.
- Read-only profiles.
- Disconnected profiles.

For a direct action dispatch, assert a warning notification is emitted. For Help/keymap checks, assert no side effects.

**Step 4: Run all catalog-focused suites**

Run:

```bash
cargo test --test catalog_mutation
cargo test --test catalog_editor_state
cargo test --test catalog_editor_reducer
cargo test --test postgres_adapter
cargo test --test keymap catalog
cargo test --test ui_render catalog_editor
```

Expected: all PASS.

**Step 5: Run formatting, linting, and complete test suite**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

Expected: all PASS.

If formatting fails, run `cargo fmt --all`, inspect the resulting diff, then rerun all three commands. Do not claim full verification if the environment prevents database-backed or platform-specific tests from running; record the exact command and failure.

**Step 6: Manual TUI verification**

Using a writable PostgreSQL profile:

1. Connect to PostgreSQL and expand Database > Schema.
2. Select the Schema and press `a`.
3. Verify the picker order is Table, View, Materialized View, Sequence.
4. Select each option and verify the corresponding form opens with schema prefilled.
5. Select each group and press `a`; verify only that group's object is offered.
6. Reconnect using a read-only profile; verify `a` is unavailable for catalog creation.
7. If PostgreSQL 14 and 15 environments are available, verify `security_invoker` is unavailable on 14 and available on 15.

Expected: Help, keymap, picker contents, form initialization, and planner behavior agree.

**Step 7: Review the final diff**

Run:

```bash
git status --short
```

Expected:

- No whitespace errors.
- No duplicate Schema `match` branches.
- No hard-coded Catalog Editor object labels outside `display_label()`.
- No direct static PostgreSQL mutation capability lookup in Help or picker code.
- No unrelated local changes included in the implementation diff.

**Step 8: Commit final regression coverage**

```bash
git add tests/catalog_editor_reducer.rs tests/keymap.rs tests/catalog_mutation.rs tests/ui_render.rs
git commit -m "test(catalog): enforce create capability consistency"
```

Only commit if explicitly requested.

## Acceptance Criteria

- On a writable connected PostgreSQL Schema, pressing `a` shows Table, View, Materialized View, and Sequence.
- Every selectable option initializes a matching non-empty typed Draft and opens a functional form.
- Direct Schema and group selections use the same capability model.
- Help visibility, keymap dispatch, and actual picker contents are derived from one App query.
- Read-only, disconnected, wrong-profile, stale-entry, and wrong-database selections cannot open creation UI.
- PostgreSQL version-dependent View options use capabilities from the concrete active adapter.
- MySQL, SQLite, and SQL Server do not expose catalog mutation until their adapters declare supported mutation capabilities.
- Function, Procedure, Type, and Trigger remain hidden until their full mutation vertical slices are implemented.
- `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets` pass, or any environment-specific exception is explicitly documented.

## Rollback Boundaries

- Task 1 is a pure capability-model change and can be reverted independently.
- Task 2 is confined to Catalog Editor state and Draft construction.
- Tasks 3 and 4 remove UI duplication and add regression coverage without changing runtime connection transport.
- Task 5 changes the `ConnectionSucceeded` action shape and should be reverted as one unit with all fixture updates.
- Task 6 contains only consistency and end-to-end tests.

This sequence keeps the reported UI bug fixed and tested before the broader runtime capability propagation, while still ending with one authoritative, version-aware architecture.
