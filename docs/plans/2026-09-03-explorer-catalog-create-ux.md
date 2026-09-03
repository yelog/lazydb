# Explorer Catalog Create UX Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make Explorer catalog creation faster and visually consistent by showing object icons in every create picker, skipping the picker when only one object type can be created, and rendering catalog create forms with the same panel hierarchy and field treatment as the connection-group editor.

**Architecture:** Keep `CatalogMutationCapabilities::create_options()` as the only source of truth for which object types are valid. Add one shared App-level selection function so manual picker confirmation and automatic single-option selection initialize exactly the same draft and runtime View capabilities. Keep presentation logic in `src/ui`: extend `IconSet` to map `CatalogObjectType`, pass icons into Catalog Editor rendering, and add Catalog Editor-local panel/field helpers modeled on the existing connection-group editor rather than coupling catalog forms to profile-specific components.

**Tech Stack:** Rust 1.94 / edition 2024, Ratatui 0.30.2, Crossterm 0.29, Nerd Font Symbols 0.3, existing reducer/action/overlay architecture, Cargo integration tests.

---

## Product Decisions

1. Single-option behavior is generic. It applies to every Catalog create context that resolves to exactly one available option, not only Tables and Views groups.
2. Database -> Schema, Tables -> Table, Views -> View, Materialized Views -> Materialized View, and Sequences -> Sequence open the corresponding form directly.
3. A Schema node continues to show the picker because it currently offers Table, View, Materialized View, and Sequence.
4. Empty option sets continue to suppress Catalog Editor opening. This behavior remains owned by `App::selected_catalog_create_options()`.
5. The shortcut mapping in `src/input/keymap.rs` does not change. `a` still produces `Action::OpenCatalogCreate`; the reducer decides whether a picker is necessary.
6. Automatic selection and Enter-based selection must use the same function. In particular, a View opened directly from the Views group must receive the active connection's version-specific `ViewMutationCapabilities`.
7. Icons are presentation data. Do not add icon strings to `CatalogMutationOption` or any database/model capability structure.
8. `CatalogObjectType::Catalog(kind)` uses the existing `IconSet::catalog(kind)` mapping. `LoginRole` and `Role` reuse the existing Explorer add User and Role icons.
9. Catalog Editor uses the same visual language as `NEW CONNECTION GROUP`: rounded focused panel, themed surface, uppercase details/status line, `›` active-field marker, fixed label/value columns, selected-value background, `×` error line, centered action/hint rows.
10. Catalog Editor retains its wider `92 x 22` maximum footprint because Table, View, Database, Role, and Sequence forms contain more fields than connection-group creation.
11. The redesign applies to all existing `CatalogDraft` variants so the Catalog Editor does not switch visual systems between object types. Database, Schema, Table, and View are the required acceptance paths; Role, Materialized View, Sequence, Index, and Constraint must retain their existing content and behavior in the new shell.
12. Schema input behavior must be completed as part of this work. The model validates both name and owner, but the current UI only renders/edits name. A styled but unusable owner row is not acceptable.
13. Do not redesign SQL Preview, mutation planning, SQL generation, validation rules, or apply confirmation beyond making the outer panel and footer visually consistent.
14. Do not add a new dependency, configuration option, persisted state, mouse interaction model, or compatibility path for the old picker.
15. Commit commands in this plan are optional checkpoints. Run them only when the user explicitly asks for commits.

## Target Behavior Matrix

| Explorer selection | Available options | Result after `a` |
|---|---:|---|
| Database | Schema | Open `NEW SCHEMA` form directly |
| Schema | Table, View, Materialized View, Sequence | Open object picker with icons |
| Tables group | Table | Open `NEW TABLE` form directly |
| Views group | View | Open `NEW VIEW` form directly |
| Materialized Views group | Materialized View | Open matching form directly |
| Sequences group | Sequence | Open matching form directly |
| Table | Capability-dependent child types | Picker when multiple; direct form when exactly one |
| Unsupported/read-only/disconnected context | None | No Catalog Editor overlay, unchanged |

## Target Rendering

Multi-option picker in ASCII mode:

```text
╭─ NEW CATALOG OBJECT ─────────────────────────────────────────╮
│ TARGET  app.public                                           │
│                                                              │
│ › TB  Table                                                  │
│   VW  View                                                   │
│   MV  Materialized View                                      │
│   SQ  Sequence                                               │
│                                                              │
│       j/k · ↑/↓ select   Enter continue   Esc close          │
╰──────────────────────────────────────────────────────────────╯
```

Direct Table form:

```text
╭─ NEW TABLE ──────────────────────────────────────────────────╮
│ TABLE DETAILS                              TARGET  app.public │
│                                                              │
│ › Name             orders                                   │
│   Schema           public                                   │
│   Owner            postgres                                 │
│   Comment                                                   │
│                                                              │
│ General  Columns  Indexes  Constraints                       │
│                                         [ Preview SQL ]       │
│       Tab/Shift-Tab fields   Enter preview   Esc cancel      │
╰──────────────────────────────────────────────────────────────╯
```

Exact border fill depends on terminal width and Ratatui. Tests should assert semantic labels, icons, placement, styles, and retained content rather than snapshotting the entire frame.

## Acceptance Criteria

- Pressing `a` on Tables and Views groups opens `CatalogEditorPage::Form` immediately.
- Every other create context with exactly one option also skips `ObjectPicker`.
- A Schema node still opens `ObjectPicker` with the existing option order.
- Directly opened View drafts receive the active connection's real View capabilities.
- Catalog picker rows display the correct icon in Nerd Font, Unicode, and ASCII modes.
- Role and Login Role have meaningful icons even though they are not `CatalogKind` variants.
- Catalog Editor uses a rounded, themed panel and dynamic titles such as `NEW TABLE`, `NEW VIEW`, and `EDIT DATABASE`.
- Picker/Form headers display a sanitized, user-facing target without Rust debug formatting such as `Tables` or `Views`.
- Active editable fields have a `›` marker, action-colored label, and selection-colored value area.
- Validation errors render as `× <sanitized message>` in `theme.error`.
- Footer hints remain visible and centered at `56 x 16`, `80 x 24`, and `100 x 30` where the form has enough room.
- Schema name, owner, and comment can all be selected and edited; validation can succeed after required fields are entered.
- Existing Catalog Editor keyboard behavior, SQL preview, apply flow, secret redaction, and object-specific content remain intact.
- Focused tests, full tests, formatting, compilation, and Clippy pass.

---

### Task 1: Make Catalog Option Selection One Shared Operation

**Files:**
- Modify: `src/app.rs:2940-2960` (`Action::OpenCatalogCreate`)
- Modify: `src/app.rs:3220-3248` (`Action::CatalogEditorSelect`)
- Add helper near: `src/app.rs:9490-9551`
- Test: `tests/catalog_editor_reducer.rs:184-287`

**Step 1: Add a reusable connected PostgreSQL fixture helper**

Extract the connection setup currently embedded in `opening_create_on_schema_uses_capability_ordered_options()` so new tests use the same active profile, execution target, catalog epoch, and PostgreSQL 15 capabilities. The helper should return `(App, Uuid, CatalogId, CatalogId)` for app/profile/database/schema.

Use the existing setup operations exactly:

```rust
fn connected_postgres_catalog_fixture() -> (App, Uuid, CatalogId, CatalogId) {
    let profile = import_connection_url(
        "postgres://localhost/app",
        Some("postgres-test"),
    )
    .unwrap()
    .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    let generation = match app.update(Action::RequestConnect(profile_id)).as_slice() {
        [lazydb::action::Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: lazydb::db::ServerInfo {
            kind: lazydb::profile::DatabaseKind::Postgres,
            version: "PostgreSQL 15".into(),
            database: "app".into(),
        },
        mutation_capabilities:
            lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities_for_version(
                150_000,
            ),
    });

    let database = CatalogId::new(profile_id, CatalogKind::Database, ["app"]);
    let schema = CatalogId::new(profile_id, CatalogKind::Schema, ["app", "public"]);
    let catalog = &mut app
        .explorer
        .normalized
        .profiles
        .get_mut(&profile_id)
        .unwrap()
        .catalog;
    catalog
        .insert(
            CatalogEntry::database(
                database.clone(),
                QualifiedName {
                    database: Some("app".into()),
                    schema: None,
                    object: "app".into(),
                },
                "database",
                OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        )
        .unwrap();
    catalog
        .insert(
            CatalogEntry::schema(
                schema.clone(),
                database.clone(),
                QualifiedName {
                    database: Some("app".into()),
                    schema: Some("public".into()),
                    object: "public".into(),
                },
                "schema",
                OptionalMetadata::Supported(None),
                true,
            )
            .unwrap(),
        )
        .unwrap();
    (app, profile_id, database, schema)
}
```

Keep insertion in this helper and do not introduce a production test utility.

**Step 2: Write failing direct-open reducer tests**

Add a table-driven test for group anchors:

```rust
#[test]
fn single_option_catalog_groups_open_the_matching_form_directly() {
    for (group, object_type, expected_draft) in [
        (ObjectGroup::Tables, CatalogKind::Table, "table"),
        (ObjectGroup::Views, CatalogKind::View, "view"),
        (
            ObjectGroup::MaterializedViews,
            CatalogKind::MaterializedView,
            "materialized-view",
        ),
        (ObjectGroup::Sequences, CatalogKind::Sequence, "sequence"),
    ] {
        let (mut app, _, _, schema) = connected_postgres_catalog_fixture();
        app.explorer.normalized.selected = Some(ExplorerNodeId::Group {
            parent: schema,
            group,
        });

        app.update(Action::OpenCatalogCreate);

        let editor = app.catalog_editor.as_ref().expect("catalog editor");
        assert_eq!(editor.page, CatalogEditorPage::Form);
        assert_eq!(
            editor.object_type,
            Some(CatalogObjectType::Catalog(object_type))
        );
        assert_draft_kind(editor.draft.as_ref(), expected_draft);
        assert_eq!(app.overlay, Some(Overlay::CatalogEditor));
    }
}
```

Implement `assert_draft_kind` as an exhaustive test-only `match` over the four expected strings, or split this into four explicit tests if that produces clearer failures. Do not compare `Debug` output.

Add a Database-anchor test:

```rust
#[test]
fn database_single_schema_option_opens_schema_form_directly() {
    let (mut app, _, database, _) = connected_postgres_catalog_fixture();
    app.explorer.normalized.selected = Some(ExplorerNodeId::Catalog(database));

    app.update(Action::OpenCatalogCreate);

    let editor = app.catalog_editor.as_ref().expect("catalog editor");
    assert_eq!(editor.page, CatalogEditorPage::Form);
    assert_eq!(
        editor.object_type,
        Some(CatalogObjectType::Catalog(CatalogKind::Schema))
    );
    assert!(matches!(editor.draft, Some(CatalogDraft::Schema(_))));
}
```

Update imports to include `ObjectGroup`, `CatalogDraft`, and `CatalogEditorPage`.

**Step 3: Write a failing View capability parity test**

The test must prove more than `draft.is_some()`. PostgreSQL 15 makes `security_invoker` available, while a newly constructed View draft starts unavailable:

```rust
#[test]
fn directly_opened_view_uses_active_server_capabilities() {
    let (mut app, _, _, schema) = connected_postgres_catalog_fixture();
    app.explorer.normalized.selected = Some(ExplorerNodeId::Group {
        parent: schema,
        group: ObjectGroup::Views,
    });

    app.update(Action::OpenCatalogCreate);

    let Some(CatalogDraft::View(draft)) = app
        .catalog_editor
        .as_ref()
        .and_then(|editor| editor.draft.as_ref())
    else {
        panic!("view draft expected");
    };
    assert!(draft.security_invoker.availability.is_available());
}
```

**Step 4: Run the new tests and verify failure**

Run:

```bash
cargo test --test catalog_editor_reducer single_option_catalog_groups_open_the_matching_form_directly -- --exact
cargo test --test catalog_editor_reducer database_single_schema_option_opens_schema_form_directly -- --exact
cargo test --test catalog_editor_reducer directly_opened_view_uses_active_server_capabilities -- --exact
```

Expected: the editors remain on `CatalogEditorPage::ObjectPicker`; the View capability assertion cannot yet reach a selected View draft.

**Step 5: Extract one selection function in `src/app.rs`**

Add a module-private free function near the Catalog Editor helpers. It must own all side effects currently performed by `Action::CatalogEditorSelect`:

```rust
fn select_catalog_editor_option(
    editor: &mut CatalogEditorState,
    selected: usize,
    view_capabilities: crate::db::catalog_mutation::ViewMutationCapabilities,
) -> bool {
    if !editor.select_option(selected) {
        return false;
    }
    if let Some(crate::model::catalog_editor::CatalogDraft::View(draft)) =
        editor.draft.as_mut()
    {
        draft.security_barrier = crate::db::catalog_mutation::ViewOption {
            availability: view_capabilities.security_barrier,
            value: None,
        };
        draft.security_invoker = crate::db::catalog_mutation::ViewOption {
            availability: view_capabilities.security_invoker,
            value: None,
        };
        draft.check_option = crate::db::catalog_mutation::ViewOption {
            availability: view_capabilities.check_option,
            value: None,
        };
    }
    true
}
```

The draft variant is sufficient to detect View. Do not duplicate an `editor.object_type` check unless needed to satisfy an invariant enforced elsewhere.

Replace `Action::CatalogEditorSelect` with:

```rust
Action::CatalogEditorSelect => {
    let view_capabilities = self.connection.mutation_capabilities.view_options;
    if let Some(editor) = self.catalog_editor.as_mut() {
        let selected = editor.selected_option;
        select_catalog_editor_option(editor, selected, view_capabilities);
    }
    Vec::new()
}
```

**Step 6: Auto-select exactly one option in `OpenCatalogCreate`**

Build the editor first, then use the same selection function before exposing it:

```rust
let mut editor = CatalogEditorState::new(
    CatalogMutationMode::Create,
    selection.anchor,
    selection.catalog_epoch,
    options,
);
if editor.options.len() == 1 {
    let view_capabilities = self.connection.mutation_capabilities.view_options;
    select_catalog_editor_option(&mut editor, 0, view_capabilities);
}
self.catalog_editor = Some(editor);
self.overlay = Some(Overlay::CatalogEditor);
```

Do not put this rule in `CatalogEditorState::new()`, `map_explorer()`, or `ui::catalog_editor::render()`. Keep the existing `selected_catalog_create_options()` empty-option guard.

**Step 7: Verify direct and multi-option behavior**

Run:

```bash
cargo test --test catalog_editor_reducer single_option_catalog_groups_open_the_matching_form_directly -- --exact
cargo test --test catalog_editor_reducer database_single_schema_option_opens_schema_form_directly -- --exact
cargo test --test catalog_editor_reducer directly_opened_view_uses_active_server_capabilities -- --exact
cargo test --test catalog_editor_reducer opening_create_on_schema_uses_capability_ordered_options -- --exact
```

Expected: all PASS. The existing Schema test must still see four options and manually select View.

**Step 8: Optional checkpoint commit**

```bash
git add src/app.rs tests/catalog_editor_reducer.rs
git commit -m "feat(catalog): open single create option directly"
```

Only run when explicitly requested.

---

### Task 2: Add a Complete Catalog Object Icon Mapping

**Files:**
- Modify: `src/ui/icons.rs:5-10` (imports)
- Modify: `src/ui/icons.rs:206-286` (`IconSet` mappings)
- Test: `src/ui/icons.rs:375-424`

**Step 1: Write failing icon coverage assertions**

Extend `every_mode_has_safe_mappings()` with every object type the picker can display:

```rust
const CATALOG_OBJECT_TYPES: &[CatalogObjectType] = &[
    CatalogObjectType::Catalog(CatalogKind::Database),
    CatalogObjectType::Catalog(CatalogKind::Schema),
    CatalogObjectType::Catalog(CatalogKind::Table),
    CatalogObjectType::Catalog(CatalogKind::View),
    CatalogObjectType::Catalog(CatalogKind::MaterializedView),
    CatalogObjectType::Catalog(CatalogKind::Column),
    CatalogObjectType::Catalog(CatalogKind::Index),
    CatalogObjectType::Catalog(CatalogKind::PrimaryKey),
    CatalogObjectType::Catalog(CatalogKind::UniqueConstraint),
    CatalogObjectType::Catalog(CatalogKind::ForeignKey),
    CatalogObjectType::Catalog(CatalogKind::CheckConstraint),
    CatalogObjectType::Catalog(CatalogKind::Function),
    CatalogObjectType::Catalog(CatalogKind::Procedure),
    CatalogObjectType::Catalog(CatalogKind::Trigger),
    CatalogObjectType::Catalog(CatalogKind::Sequence),
    CatalogObjectType::Catalog(CatalogKind::Type),
    CatalogObjectType::LoginRole,
    CatalogObjectType::Role,
];
```

For each icon mode, assert:

```rust
for object_type in CATALOG_OBJECT_TYPES {
    let icon = icons.catalog_object(*object_type);
    assert!(!icon.is_empty());
    assert!(icon.chars().all(|character| !character.is_control()));
    if mode == IconMode::Ascii {
        assert!(icon.is_ascii());
    }
    if mode == IconMode::Unicode {
        assert!(!icon.chars().any(is_private_use));
    }
}
```

**Step 2: Run the icon test and verify compilation failure**

Run:

```bash
cargo test ui::icons::tests::every_mode_has_safe_mappings -- --exact
```

Expected: compilation fails because `IconSet::catalog_object` does not exist.

**Step 3: Implement the semantic mapping**

Import `CatalogObjectType` and add:

```rust
pub(crate) const fn catalog_object(
    self,
    object_type: CatalogObjectType,
) -> &'static str {
    match object_type {
        CatalogObjectType::Catalog(kind) => self.catalog(kind),
        CatalogObjectType::LoginRole => self.explorer_add(ExplorerAddIcon::User),
        CatalogObjectType::Role => self.explorer_add(ExplorerAddIcon::Role),
    }
}
```

Keep `catalog()` and `explorer_add()` unchanged so existing Explorer, completion, and add-menu callers retain their current API.

**Step 4: Run icon tests**

Run:

```bash
cargo test ui::icons::tests::every_mode_has_safe_mappings -- --exact
cargo test ui::icons::tests -- --nocapture
```

Expected: PASS in Nerd Font, Unicode, and ASCII modes.

**Step 5: Optional checkpoint commit**

```bash
git add src/ui/icons.rs
git commit -m "feat(ui): map catalog object icons"
```

Only run when explicitly requested.

---

### Task 3: Complete Schema Form Editing Before Restyling It

**Files:**
- Modify: `src/model/catalog_editor.rs:134-139` (`SchemaDraft`)
- Modify: `src/model/catalog_editor.rs:1043-1061` (`SchemaDraft` implementation)
- Modify: `src/model/catalog_editor.rs:1076-1164` (`CatalogDraft` editing delegation)
- Modify: `src/model/catalog_editor.rs:1270-1279` (create Draft initialization)
- Modify: `src/app.rs:3123-3136` (loaded Schema initialization)
- Update literals in: `tests/catalog_editor_state.rs`
- Update literals in: `tests/catalog_mutation.rs`
- Update literals in: `tests/postgres_adapter.rs`
- Test: `tests/catalog_editor_state.rs`

**Step 1: Write failing Schema editing tests**

Add a focused test proving field navigation and input delegation:

```rust
#[test]
fn schema_draft_edits_name_owner_and_comment() {
    let mut draft = CatalogDraft::Schema(SchemaDraft::new());

    draft.insert('a');
    draft.move_field(1);
    draft.insert('o');
    draft.move_field(1);
    draft.insert('c');

    let CatalogDraft::Schema(draft) = draft else {
        unreachable!();
    };
    assert_eq!(draft.name.value(), "a");
    assert_eq!(draft.owner.value(), "o");
    assert_eq!(draft.comment.value(), "c");
    assert_eq!(draft.selected_field, 2);
}
```

Add editing-command coverage so Schema behaves like other text-input drafts:

```rust
#[test]
fn schema_draft_delegates_cursor_and_delete_commands() {
    let mut draft = CatalogDraft::Schema(SchemaDraft::new());
    for character in "owner".chars() {
        draft.insert(character);
    }
    draft.move_home();
    draft.move_right();
    draft.delete();
    draft.move_end();
    draft.backspace();

    let CatalogDraft::Schema(draft) = draft else {
        unreachable!();
    };
    assert_eq!(draft.name.value(), "one");
}
```

The expected value is `one`: deleting at cursor position one removes `w`, then Backspace at the end removes `r`.

**Step 2: Run tests and verify failure**

Run:

```bash
cargo test --test catalog_editor_state schema_draft_edits_name_owner_and_comment -- --exact
cargo test --test catalog_editor_state schema_draft_delegates_cursor_and_delete_commands -- --exact
```

Expected: compilation fails because `SchemaDraft::new()` and `selected_field` do not exist, and `CatalogDraft` does not delegate Schema editing.

**Step 3: Add explicit Schema field state**

Change the type to:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemaDraft {
    pub name: TextInput,
    pub owner: TextInput,
    pub comment: TextInput,
    pub selected_field: usize,
}
```

Add methods alongside `validate()`:

```rust
impl SchemaDraft {
    pub fn new() -> Self {
        Self {
            name: TextInput::default(),
            owner: TextInput::default(),
            comment: TextInput::default(),
            selected_field: 0,
        }
    }

    pub fn move_field(&mut self, delta: isize) {
        self.selected_field =
            (self.selected_field as isize + delta).rem_euclid(3) as usize;
    }

    fn selected_input_mut(&mut self) -> &mut TextInput {
        match self.selected_field {
            0 => &mut self.name,
            1 => &mut self.owner,
            _ => &mut self.comment,
        }
    }

    pub fn insert(&mut self, character: char) {
        self.selected_input_mut().insert(character);
    }

    pub fn backspace(&mut self) {
        self.selected_input_mut().backspace();
    }

    pub fn delete(&mut self) {
        self.selected_input_mut().delete();
    }

    pub fn delete_previous_word(&mut self) {
        self.selected_input_mut().delete_previous_word();
    }

    pub fn delete_to_start(&mut self) {
        self.selected_input_mut().delete_to_start();
    }

    pub fn move_left(&mut self) {
        self.selected_input_mut().move_left();
    }

    pub fn move_right(&mut self) {
        self.selected_input_mut().move_right();
    }

    pub fn move_home(&mut self) {
        self.selected_input_mut().move_home();
    }

    pub fn move_end(&mut self) {
        self.selected_input_mut().move_end();
    }

    // Keep the existing validate() implementation unchanged.
}
```

Do not derive `Default` unless it reduces literal churn without changing semantics. `new()` is the preferred creation path.

**Step 4: Delegate every supported edit operation**

Add the corresponding `Self::Schema(d)` arm to all applicable `CatalogDraft` methods:

- `move_field`
- `insert`
- `backspace`
- `delete`
- `delete_previous_word`
- `delete_to_start`
- `move_left`
- `move_right`
- `move_home`
- `move_end`

This is intentionally exhaustive. Supporting only insertion would leave Ctrl-W/U/A/E and arrow navigation inconsistent.

While touching these matches, preserve all existing Database, Role, View, Materialized View, and Sequence arms. Do not refactor unrelated editing behavior.

**Step 5: Replace production Schema literals**

For create initialization in `CatalogEditorState::select_object_type()`, use:

```rust
self.draft = Some(CatalogDraft::Schema(SchemaDraft::new()));
```

For loaded definitions in `Action::CatalogObjectDefinitionLoaded`, include:

```rust
selected_field: 0,
```

Do not use `SchemaDraft::new()` for loaded definitions if doing so would require mutating all three values afterward; an explicit complete literal is clearer there.

**Step 6: Update test fixtures**

Every existing `SchemaDraft` struct literal must add `selected_field: 0`, unless it represents a blank create Draft and is clearer as `SchemaDraft::new()`.

Expected affected files from the current tree:

- `tests/catalog_editor_state.rs`
- `tests/catalog_mutation.rs`
- `tests/postgres_adapter.rs`
- `src/app.rs`
- `src/model/catalog_editor.rs`

Do not perform a blind textual replacement; preserve fixture-specific name/owner/comment values.

**Step 7: Run state and mutation tests**

Run:

```bash
cargo test --test catalog_editor_state
cargo test --test catalog_mutation
cargo test --test postgres_adapter
```

Expected: PASS. In particular, existing Schema validation and SQL planner tests must remain unchanged semantically.

**Step 8: Optional checkpoint commit**

```bash
git add src/model/catalog_editor.rs src/app.rs tests/catalog_editor_state.rs tests/catalog_mutation.rs tests/postgres_adapter.rs
git commit -m "fix(catalog): make schema fields editable"
```

Only run when explicitly requested.

---

### Task 4: Render Catalog Picker Rows With Icons and Shared Panel Styling

**Files:**
- Modify: `src/ui/mod.rs:2937-2960` (pass icons to Catalog Editor)
- Modify: `src/ui/catalog_editor.rs:1-93` (render signature, panel shell, picker)
- Test: `tests/ui_render.rs:262-299`

**Step 1: Strengthen the picker rendering test**

Rename `catalog_editor_overlay_renders_picker_shell_and_context` to describe the new contract, then render with ASCII icons:

```rust
#[test]
fn catalog_editor_picker_renders_object_icons_target_and_panel_controls() {
    let mut app = catalog_picker_fixture();
    let (buffer, _) = render_buffer_with_icons(
        &app,
        100,
        30,
        IconSet::new(IconMode::Ascii),
    );
    let output = buffer_to_string(&buffer);

    assert!(output.contains("NEW CATALOG OBJECT"), "{output}");
    assert!(output.contains("TARGET"), "{output}");
    assert!(output.contains("TB  Table"), "{output}");
    assert!(output.contains("VW  View"), "{output}");
    assert!(output.contains("MV  Materialized View"), "{output}");
    assert!(output.contains("SQ  Sequence"), "{output}");
    assert!(output.contains("Enter continue"), "{output}");
    assert!(output.contains("Esc close"), "{output}");
}
```

Extract these two small test helpers:

- `catalog_picker_fixture()` for the existing four-option editor state.
- `buffer_to_string()` because existing helpers return a string only while rendering; this test also needs the Buffer for style assertions.

Also verify the selected row style using `find_text_cell()`:

```rust
let (x, y) = find_text_cell(&buffer, "TB  Table").expect("table option");
assert_eq!(buffer[(x, y)].bg, Color::Rgb(26, 55, 70));
```

Prefer comparison with the same concrete selection color used elsewhere in `ui_render.rs`; do not expose `Theme` internals solely for this test.

**Step 2: Add narrow rendering coverage**

Add:

```rust
#[test]
fn catalog_editor_picker_remains_readable_at_minimum_terminal_size() {
    let app = catalog_picker_fixture();
    let output = render_with_icons(
        &app,
        56,
        16,
        IconSet::new(IconMode::Ascii),
    )
    .0;

    assert!(output.contains("NEW CATALOG OBJECT"), "{output}");
    assert!(output.contains("TB"), "{output}");
    assert!(output.contains("Table"), "{output}");
    assert!(output.contains("Esc"), "{output}");
}
```

**Step 3: Run tests and verify failure**

Run:

```bash
cargo test --test ui_render catalog_editor_picker -- --nocapture
```

Expected: FAIL because the current picker has no icons, target line, dynamic panel title, rounded themed shell, or new footer wording.

**Step 4: Pass `IconSet` into Catalog Editor rendering**

Change `src/ui/mod.rs`:

```rust
Overlay::CatalogEditor => {
    catalog_editor::render(frame, area, app, state, theme, icons)
}
```

Change the renderer signature:

```rust
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &App,
    ui: &mut UiState,
    theme: Theme,
    icons: IconSet,
) {
```

Import `super::icons::IconSet`. Pass `icons` only to picker/form functions that need it.

**Step 5: Use the existing `panel_block()` shell**

Replace the custom plain-border Block with the existing parent helper:

```rust
let popup = super::centered(area, 92.min(area.width), 22.min(area.height));
frame.render_widget(Clear, popup);
let title = panel_title(editor);
let block = super::panel_block(&title, true, theme);
let inner = block.inner(popup);
frame.render_widget(block, popup);
```

Add dynamic title helpers:

```rust
fn panel_title(editor: &CatalogEditorState) -> String {
    match editor.page {
        CatalogEditorPage::ObjectPicker => " NEW CATALOG OBJECT ".into(),
        CatalogEditorPage::Loading => " CATALOG EDITOR // LOADING ".into(),
        CatalogEditorPage::Form => {
            let verb = match editor.mode {
                CatalogMutationMode::Create => "NEW",
                CatalogMutationMode::Edit => "EDIT",
            };
            format!(
                " {verb} {} ",
                editor.object_type.map_or("OBJECT", |kind| kind.display_label())
            )
        }
        CatalogEditorPage::SqlPreview => " REVIEW SQL ".into(),
    }
}
```

Remove `mode_label()` after replacing its final call with `panel_title()`. Use `NEW` only for create Forms; Loading and SQL Preview use their exact titles above.

The block already supplies rounded borders, `theme.surface`, title style, and focused accent color. Do not duplicate those styles in Catalog Editor.

**Step 6: Make target labels user-facing and sanitized**

Update `target_label()`:

```rust
fn target_label(editor: &CatalogEditorState) -> String {
    match &editor.anchor {
        CatalogMutationAnchor::Profile { profile_id } => format!("profile {profile_id}"),
        CatalogMutationAnchor::Catalog(id) => id.native_path.join("."),
        CatalogMutationAnchor::Group { schema, group } => format!(
            "{} / {}",
            schema.native_path.join("."),
            group_label(*group),
        ),
    }
}

fn group_label(group: ObjectGroup) -> &'static str {
    match group {
        ObjectGroup::Tables => "tables",
        ObjectGroup::Views => "views",
        ObjectGroup::MaterializedViews => "materialized views",
        ObjectGroup::Sequences => "sequences",
        ObjectGroup::Functions => "functions",
        ObjectGroup::Procedures => "procedures",
        ObjectGroup::Types => "types",
        ObjectGroup::Triggers => "triggers",
    }
}
```

The existing Explorer `group_label()` is private, so add the local exhaustive mapping above; do not use `{:?}`. Sanitize the final text at rendering time with `sanitize_terminal_text()`.

**Step 7: Replace picker `ListItem<String>` rows with styled spans**

Render a target line, blank separator, one row per option, and a footer. Follow `render_explorer_add()` behavior:

```rust
let icon = icons.catalog_object(option.object_type);
let background = if selected { theme.selection } else { theme.surface };
let row = Line::from(vec![
    Span::styled(if selected { "› " } else { "  " }, marker_style),
    Span::styled(format!("{icon} "), icon_style),
    Span::styled(sanitize_terminal_text(&option.label), label_style),
]);
```

Requirements:

- Use `›` for consistency with Explorer Add and Connection Group.
- Use `theme.selection` for the selected row background.
- Use `theme.text + BOLD` for the selected label.
- Color Catalog icons by object kind using the existing UI semantic palette.
- Use `theme.surface` on all unselected spans so cleared popup cells do not inherit workspace colors.
- Footer text: `j/k · ↑/↓ select   Enter continue   Esc close`.
- Center the footer.
- Do not add mouse hit regions in this task; Catalog Editor currently has no picker hit target and the requirement does not request one.

Add a local color helper rather than moving unrelated Explorer code:

```rust
fn object_color(object_type: CatalogObjectType, theme: Theme) -> Color {
    match object_type {
        CatalogObjectType::Catalog(CatalogKind::Database | CatalogKind::Schema) => theme.action,
        CatalogObjectType::Catalog(
            CatalogKind::Table | CatalogKind::View | CatalogKind::MaterializedView,
        ) => theme.text,
        CatalogObjectType::Catalog(
            CatalogKind::PrimaryKey | CatalogKind::UniqueConstraint,
        ) => theme.warning,
        CatalogObjectType::Catalog(CatalogKind::ForeignKey | CatalogKind::Trigger) => theme.accent,
        CatalogObjectType::LoginRole => theme.success,
        CatalogObjectType::Role => theme.warning,
        CatalogObjectType::Catalog(_) => theme.muted,
    }
}
```

Import `Color`, `CatalogKind`, and `CatalogObjectType` in `src/ui/catalog_editor.rs`.

**Step 8: Verify picker rendering and existing overlays**

Run:

```bash
cargo test --test ui_render catalog_editor_picker -- --nocapture
cargo test --test ui_render explorer_add_overlay -- --nocapture
cargo test --test ui_render profile_group_overlay -- --nocapture
```

Expected: all PASS. Explorer Add and Profile Group output must remain unchanged.

**Step 9: Optional checkpoint commit**

```bash
git add src/ui/mod.rs src/ui/catalog_editor.rs tests/ui_render.rs
git commit -m "feat(ui): add icons to catalog create picker"
```

Only run when explicitly requested.

---

### Task 5: Apply the Connection-Group Form Language to Catalog Forms

**Files:**
- Modify: `src/ui/catalog_editor.rs:118-522`
- Test: `tests/ui_render.rs:382-617`
- Test: `tests/ui_render.rs` near profile-group overlay tests at `1190-1260`

**Step 1: Add core form contract tests**

Build focused fixtures for Database, Schema, Table, and View. Reuse existing fixtures where possible rather than introducing a generic builder with many optional parameters.

Add assertions for a Database form:

```rust
#[test]
fn database_create_form_uses_structured_catalog_panel() {
    let app = database_create_form_fixture();
    let (buffer, _) = render_buffer_with_icons(
        &app,
        100,
        30,
        IconSet::new(IconMode::Ascii),
    );
    let output = buffer_to_string(&buffer);

    assert!(output.contains("NEW DATABASE"), "{output}");
    assert!(output.contains("DATABASE DETAILS"), "{output}");
    assert!(output.contains("TARGET"), "{output}");
    assert!(output.contains("› Name"), "{output}");
    assert!(output.contains("Owner"), "{output}");
    assert!(output.contains("[ Preview SQL ]"), "{output}");
    assert!(output.contains("Enter preview"), "{output}");
}
```

Add these semantic tests:

- Schema: title, Name/Owner/Comment, and active marker.
- Table: title, General/Columns/Indexes/Constraints retained, Name/Schema/Owner/Comment retained.
- View: title, Name/Schema/Owner/Comment/Output columns/Query retained, capability status retained.

Do not assert a whole frame snapshot.

**Step 2: Add active-row style and error tests**

Use `find_text_cell()` to verify:

- The selected label uses the action color or bold modifier.
- The selected value cell uses the existing selection background.
- An unselected row does not use the selection background.

Add an error fixture:

```rust
app.catalog_editor.as_mut().unwrap().error =
    Some("schema owner is required".into());
```

Assert output contains `× schema owner is required`, and inspect the `×` cell foreground color using the same error color asserted elsewhere in `ui_render.rs`.

**Step 3: Add Schema interaction-to-render coverage**

Use reducer actions, not direct field mutation:

```rust
app.update(Action::CatalogEditorInsert('n'));
app.update(Action::CatalogEditorFieldNext);
app.update(Action::CatalogEditorInsert('o'));
app.update(Action::CatalogEditorFieldNext);
app.update(Action::CatalogEditorInsert('c'));
```

Render and assert all three values appear and `› Comment` is active. This proves the model work from Task 3 is connected to the actual reducer and renderer.

**Step 4: Run tests and verify failure**

Run:

```bash
cargo test --test ui_render catalog_panel -- --nocapture
cargo test --test ui_render schema_create_form -- --nocapture
cargo test --test ui_render table_editor_renders_general_and_columns_sections -- --exact
cargo test --test ui_render view_editor_renders_query_and_output_columns -- --exact
```

Expected: new visual-contract tests fail; existing content tests should still pass before the refactor.

**Step 5: Add a Catalog form layout**

Introduce a small local structure and function:

```rust
struct CatalogFormLayout {
    header: Rect,
    body: Rect,
    feedback: Rect,
    actions: Rect,
    hint: Rect,
}

fn form_layout(inner: Rect) -> CatalogFormLayout {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    CatalogFormLayout {
        header: chunks[0],
        body: chunks[2],
        feedback: chunks[3],
        actions: chunks[4],
        hint: chunks[5],
    }
}
```

The unused separator row is `chunks[1]`. If the terminal is too short, collapse the separator before dropping feedback/actions/hint. Keep the final implementation saturating and panic-free for small `Rect`s.

**Step 6: Add reusable Catalog field renderers**

Use local helpers modeled on `profiles::render_field`:

```rust
fn render_input_field(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    input: &TextInput,
    active: bool,
    editable: bool,
    ui: &mut UiState,
    theme: Theme,
) {
    let label_width = area.width.min(22);
    let label_area = Rect::new(area.x, area.y, label_width, 1);
    let value_area = Rect::new(
        area.x.saturating_add(label_width),
        area.y,
        area.width.saturating_sub(label_width),
        1,
    );
    render_field_label(frame, label_area, label, active, theme);
    let value_style = Style::new()
        .fg(if editable { theme.text } else { theme.muted })
        .bg(if active { theme.selection } else { theme.surface });
    if active && editable {
        render_text_input(frame, value_area, "", input, value_style, ui);
    } else {
        frame.render_widget(
            Paragraph::new(sanitize_terminal_text(input.value())).style(value_style),
            value_area,
        );
    }
}
```

Add complementary helpers:

- `render_value_field` for booleans/enums/status values that are not `TextInput`.
- `render_field_label` for `›`, fixed label width, active action color, and bold modifier.
- `render_form_header` for `<OBJECT> DETAILS` and right-aligned target context when width permits.
- `render_form_feedback` for sanitized errors or blank space.
- `render_form_actions` for centered `[ Preview SQL ]`.
- `render_form_hint` for centered contextual controls.

Do not import or generalize `profiles::render_field()`: it is coupled to `ProfileDraft`, `ProfileField`, profile HitTargets, and driver icons.

**Step 7: Define exhaustive field mappings**

Render each current Draft using its existing `selected_field` semantics. Do not reorder fields independently of model indices.

Database (`selected_field` 0-12):

| Index | Label | Editable |
|---:|---|---|
| 0 | Name | yes |
| 1 | Owner | yes |
| 2 | Template | create only |
| 3 | Encoding | create only |
| 4 | Locale provider | create only |
| 5 | Locale | create only |
| 6 | Collation | create only |
| 7 | Ctype | create only |
| 8 | Tablespace | create only |
| 9 | Connection limit | yes |
| 10 | Allow connections | toggle/status; preserve current behavior |
| 11 | Is template | toggle/status; preserve current behavior |
| 12 | Comment | yes |

Schema (`selected_field` 0-2): Name, Owner, Comment; all editable.

View (`selected_field` 0-5): Name, Schema, Owner, Comment, Query, Output columns. Preserve security barrier/invoker/check availability below the editable rows as muted status lines.

Materialized View (`selected_field` 0-5): Name, Schema, Owner, Comment, Query, Tablespace. Render WITH DATA separately, preserving Space behavior and query read-only state during edit.

Sequence (`selected_field` 0-9): Name, Schema, Owner, Comment, Type, Increment, Start, Restart, Cache, Owned by. Preserve min/max/cycle as status/value rows until their existing input model supports direct field selection.

Role (`selected_field` 0-10): preserve every existing role flag and status, including password redaction. Never render secret contents.

Table:

- Preserve section tabs and selected section behavior.
- In General, render Name/Schema/Owner/Comment using structured rows.
- In Columns, preserve the current Table widget and selected-column marker.
- Keep the currently non-interactive Indexes/Constraints labels as-is; do not invent editing behavior.

Index and Constraint:

- Convert every existing line that represents one field to a label/value row. Keep compound constraint clauses such as `MATCH`, `ON UPDATE`, and `ON DELETE` together because they form one semantic setting.
- Preserve all fields and values.
- Do not add new field selection or edit behavior in this task.

**Step 8: Update `form()` orchestration**

The new flow should be:

```rust
let layout = form_layout(area);
render_form_header(frame, layout.header, editor, theme, icons);
match editor.draft.as_ref() {
    Some(CatalogDraft::Database(draft)) => {
        render_database(frame, layout.body, draft, ui, theme);
    }
    Some(CatalogDraft::Role(draft)) => {
        render_role(frame, layout.body, draft, ui, theme);
    }
    Some(CatalogDraft::Schema(draft)) => {
        render_schema(frame, layout.body, draft, ui, theme);
    }
    Some(CatalogDraft::Table(draft)) => {
        render_table(frame, layout.body, draft, ui, theme);
    }
    Some(CatalogDraft::Index(draft)) => {
        render_index(frame, layout.body, draft, theme);
    }
    Some(CatalogDraft::Constraint(draft)) => {
        render_constraint(frame, layout.body, draft, theme);
    }
    Some(CatalogDraft::View(draft)) => {
        render_view(frame, layout.body, draft, ui, theme);
    }
    Some(CatalogDraft::MaterializedView(draft)) => {
        render_materialized_view(frame, layout.body, draft, ui, theme);
    }
    Some(CatalogDraft::Sequence(draft)) => {
        render_sequence(frame, layout.body, draft, ui, theme);
    }
    None => {
        frame.render_widget(
            Paragraph::new("Definition form is unavailable")
                .style(Style::new().fg(theme.muted).bg(theme.surface)),
            layout.body,
        );
    }
}
render_form_feedback(frame, layout.feedback, editor.error.as_deref(), theme);
render_form_actions(frame, layout.actions, editor.is_busy(), theme);
render_form_hint(frame, layout.hint, editor, theme);
```

Pass `UiState` into concrete renderers that render active text inputs. Avoid mutable model access from UI; rendering remains read-only.

**Step 9: Keep content visible in constrained height**

Database and Role have more fields than fit in the body at the minimum terminal height. Implement a deterministic viewport based on `selected_field`, following the existing `profiles::viewport_start()` logic:

```rust
fn viewport_start(selected: usize, total: usize, capacity: usize) -> usize {
    if capacity >= total {
        0
    } else {
        selected
            .saturating_sub(capacity / 2)
            .min(total.saturating_sub(capacity))
    }
}
```

Use it for row-based forms. The active field must always remain visible. Do not add scroll state to `CatalogEditorState`; selection already determines the viewport.

**Step 10: Render errors and controls consistently**

Error:

```rust
Paragraph::new(format!("× {}", sanitize_terminal_text(error)))
    .style(Style::new().fg(theme.error).bg(theme.surface))
```

Normal feedback row remains blank rather than duplicating the footer hint.

Action row:

```text
[ Preview SQL ]
```

Use `theme.action` and bold when the editor is not busy. Catalog planning is triggered by Enter, so no new HitTarget is required.

Footer:

- Default: `Tab/Shift-Tab fields   Enter preview   Esc cancel`
- Materialized View create: include `Space toggle data`
- Center-align and use `theme.muted` + `theme.surface`.

**Step 11: Run focused form tests**

Run:

```bash
cargo test --test ui_render database_create_form_uses_structured_catalog_panel -- --exact
cargo test --test ui_render schema_create_form -- --nocapture
cargo test --test ui_render table_editor_renders_general_and_columns_sections -- --exact
cargo test --test ui_render view_editor_renders_query_and_output_columns -- --exact
cargo test --test ui_render materialized_view_editor_renders_data_state_and_read_only_query -- --exact
cargo test --test ui_render role_editor_renders_secret_as_status_only -- --exact
```

Expected: PASS. Secret output must remain redacted.

**Step 12: Test narrow and hostile content**

Add or extend tests to render:

- `56 x 16` Schema and View forms.
- A long target path.
- Field values containing newline and terminal escape characters.
- A long validation error.

Assert sanitized markers such as `<LF>`/`<ESC>` appear and raw control characters do not affect adjacent rows. Use existing sanitization test patterns from `hostile_and_long_form_values_render_safely_at_the_cursor`.

Run:

```bash
cargo test --test ui_render catalog_editor -- --nocapture
cargo test --test ui_render hostile_and_long_form_values_render_safely_at_the_cursor -- --exact
```

**Step 13: Optional checkpoint commit**

```bash
git add src/ui/catalog_editor.rs tests/ui_render.rs
git commit -m "feat(ui): unify catalog create form styling"
```

Only run when explicitly requested.

---

### Task 6: Verify the Complete Explorer Add and Catalog Mutation Flow

**Files:**
- No production changes expected
- Update tests only if verification exposes a real regression

**Step 1: Run focused state and reducer suites**

```bash
cargo test --test explorer_add
cargo test --test catalog_editor_state
cargo test --test catalog_editor_reducer
cargo test --test catalog_mutation
```

Expected: PASS.

**Step 2: Run input and UI regression suites**

```bash
cargo test --test keymap
cargo test --test mouse
cargo test --test ui_render
```

Expected: PASS. Confirm specifically:

- Profile + `a` still opens Explorer Add.
- Connection Group + `a` still opens new-group editing directly.
- Schema + `a` still opens the multi-option Catalog picker.
- Tables/Views + `a` reach Form without a second Enter.
- Explorer Add ASCII icons are unchanged.
- Profile Group mouse hit regions are unchanged.

**Step 3: Run formatting and compilation checks**

```bash
cargo fmt --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: PASS with no warnings.

If `cargo fmt --check` fails due to this change, run `cargo fmt`, inspect the diff, and rerun the check. Do not format or revert unrelated files manually.

**Step 4: Run the full suite**

```bash
cargo test --all-targets --all-features
```

Expected: PASS.

**Step 5: Manual TUI acceptance pass**

Run LazyDB against a writable PostgreSQL profile and verify:

1. Select a Schema and press `a`: picker appears with icons and four options.
2. Select Tables and press `a`: `NEW TABLE` appears immediately.
3. Select Views and press `a`: `NEW VIEW` appears immediately.
4. Select a Database and press `a`: `NEW SCHEMA` appears immediately.
5. In Schema form, enter Name, Tab to Owner, enter Owner, Tab to Comment, and enter Comment.
6. Press Enter: SQL Preview appears; Esc returns to the form with values intact.
7. Switch `--icon-mode` among `nerd-font`, `unicode`, and `ascii`: every picker row remains meaningful and aligned.
8. Resize to `56 x 16`: title, selected option/field, and Esc hint remain visible.
9. Trigger validation with missing required data: error displays inline with `×` and the form remains open.

**Step 6: Inspect the final diff**

```bash
git status --short
git diff -- src/app.rs src/model/catalog_editor.rs src/ui/icons.rs src/ui/mod.rs src/ui/catalog_editor.rs tests/catalog_editor_state.rs tests/catalog_editor_reducer.rs tests/catalog_mutation.rs tests/postgres_adapter.rs tests/ui_render.rs
```

Confirm:

- No database capability or SQL planner rules changed.
- No keymap changes were introduced.
- No unrelated files were reverted or reformatted.
- Icon strings remain centralized in `IconSet`.
- Single-option logic exists only in the Catalog create reducer path.
- Manual and automatic option selection call the same helper.
- Every `SchemaDraft` literal initializes `selected_field`.

**Step 7: Optional final commit**

```bash
git add src/app.rs src/model/catalog_editor.rs src/ui/icons.rs src/ui/mod.rs src/ui/catalog_editor.rs tests/catalog_editor_state.rs tests/catalog_editor_reducer.rs tests/catalog_mutation.rs tests/postgres_adapter.rs tests/ui_render.rs
git commit -m "feat(catalog): streamline explorer create experience"
```

Only run when explicitly requested.

---

## Risk Controls

- **View capability regression:** Prevented by one shared selection helper and an explicit PostgreSQL 15 direct-open test.
- **Picker accidentally skipped for Schema:** Prevented by retaining and rerunning the existing four-option order test.
- **Model/UI mismatch for Schema:** Prevented by adding field state and exhaustive edit delegation before rendering additional rows.
- **Icon-mode rendering breakage:** Prevented by iterating every `CatalogObjectType` in all three modes and checking private-use/control characters.
- **Small-terminal clipping:** Prevented by selected-field-driven row viewport and explicit `56 x 16` tests.
- **Secret exposure:** Prevented by retaining the existing Role redaction test during renderer conversion.
- **Terminal injection:** Prevented by sanitizing labels, target text, errors, and values before rendering; hostile-value tests remain mandatory.
- **Unrelated behavior changes:** `CatalogMutationCapabilities`, key mappings, planners, apply logic, and Explorer Add state remain unchanged.

## Definition of Done

The work is complete only when all acceptance criteria pass, all verification commands that can run locally have actually been run, and any skipped manual/database-backed check is explicitly reported with its remaining risk. Do not claim completion based only on compilation or screenshots.
