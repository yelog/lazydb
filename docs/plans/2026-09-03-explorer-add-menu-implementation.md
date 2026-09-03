# Explorer Add Menu Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** When a database connection is selected in Explorer, pressing `a` opens a polished five-item menu for creating a connection, connection group, database, user, or role, with keyboard navigation and direct handoff to the existing editors.

**Architecture:** Add a small Explorer-specific menu state and overlay that only owns action selection. Keep connection editing in `ProfileManager`, group editing in `ProfileGroupOverlay`, and database/user/role creation in `CatalogEditorState`; confirmation closes the menu and delegates directly to one of those existing flows. Derive catalog-item availability from the selected profile, active connection, read-only state, and PostgreSQL catalog mutation capabilities rather than duplicating database support rules in the UI.

**Tech Stack:** Rust 2024, Ratatui 0.30, Crossterm 0.29, Nerd Font Symbols 0.3, existing reducer/action/overlay architecture, Rust integration tests.

---

## Product Decisions

1. The new menu opens only when the selected Explorer node is `ExplorerNodeId::Profile`. A connection group is an organizational node and cannot anchor a database mutation, so pressing `a` on `ExplorerNodeId::ConnectionGroup` continues to create a connection group directly.
2. The menu always renders exactly five rows in this order: Connection, Connection Group, Database, User, Role.
3. Connection and Connection Group are always enabled. Database, User, and Role remain visible but are disabled when the selected profile is not the active writable PostgreSQL connection.
4. Disabled catalog rows show a short reason and cannot be confirmed. Keyboard movement skips disabled rows so Enter always has a valid outcome.
5. The user-facing term is `User`; internally it maps to `CatalogObjectType::LoginRole`. `Role` maps to `CatalogObjectType::Role`.
6. Selecting Database, User, or Role opens the corresponding Catalog Editor form directly. It must not open the existing Catalog object picker and force a second selection.
7. Existing Catalog object creation on database/schema/table/group nodes remains unchanged from the user's perspective.
8. `j`, `k`, `Up`, `Down`, `Enter`, `Esc`, and `q` are owned by the new overlay while it is open. Movement clamps at the first/last enabled item rather than wrapping, matching the existing Catalog Editor picker.
9. Mouse support is included because the existing connection-group overlay already exposes clickable rows; one click selects a row and Enter confirms it. Double-click confirmation is out of scope.
10. Use the existing theme palette. Do not add new global colors or a new dependency.

## Acceptance Criteria

- With a connection selected and Explorer focused, pressing `a` opens `ADD TO CONNECTION` instead of immediately opening the group editor.
- The popup identifies the selected connection and renders Connection, Connection Group, Database, User, and Role in the required order.
- `j/k` and `Up/Down` move selection, skipping disabled catalog actions; movement does not wrap.
- `Enter` executes the selected enabled action, while `Esc` and `q` close the popup without changing application data.
- Connection opens the existing new-profile flow.
- Connection Group opens the existing new-group name editor.
- Database opens a `CatalogDraft::Database` form directly.
- User opens a login-enabled `CatalogDraft::Role` form directly.
- Role opens a non-login `CatalogDraft::Role` form directly.
- Non-PostgreSQL, disconnected, inactive, and read-only profiles still show catalog rows, but those rows are muted and unavailable with an explanatory reason.
- Pressing `a` on a connection group still starts group creation; pressing `a` on supported Catalog nodes still opens Catalog creation.
- Nerd Font, Unicode, and ASCII icon modes all render meaningful, width-safe icons.
- The popup remains readable at the application's minimum supported terminal size of `56 x 16`.
- Focused tests, the full test suite, formatting, compilation, and Clippy pass.

## Non-Goals

- Adding database/user/role creation support to MySQL, SQL Server, or SQLite.
- Changing PostgreSQL catalog mutation SQL or validation.
- Redesigning the Profile Manager, Profile Group editor, or Catalog Editor forms.
- Adding nested connection groups or creating a connection directly inside a selected group.
- Adding direct letter shortcuts such as `d`, `u`, or `r` inside the popup.
- Persisting the popup selection between openings.
- Changing `g m`, `J`, or `K` group-management shortcuts.

---

### Task 1: Add the Explorer Add Domain State

**Files:**
- Create: `src/model/explorer_add.rs`
- Modify: `src/model/mod.rs:1-19`
- Create: `tests/explorer_add.rs`

**Step 1: Write failing state tests**

Create `tests/explorer_add.rs` with focused model tests. Use an explicit availability enum instead of parallel `enabled` and `reason` fields so invalid combinations cannot be represented:

```rust
use lazydb::model::explorer_add::{
    ExplorerAddAvailability, ExplorerAddKind, ExplorerAddMenu, ExplorerAddOption,
};
use uuid::Uuid;

fn option(kind: ExplorerAddKind, enabled: bool) -> ExplorerAddOption {
    ExplorerAddOption {
        kind,
        availability: if enabled {
            ExplorerAddAvailability::Available
        } else {
            ExplorerAddAvailability::Unavailable("connect first")
        },
    }
}

#[test]
fn add_menu_starts_on_the_first_enabled_option() {
    let menu = ExplorerAddMenu::new(
        Uuid::from_u128(1),
        vec![
            option(ExplorerAddKind::Connection, true),
            option(ExplorerAddKind::Database, false),
        ],
    );
    assert_eq!(menu.selected, 0);
    assert_eq!(menu.selected_kind(), Some(ExplorerAddKind::Connection));
}

#[test]
fn movement_skips_disabled_options_and_clamps() {
    let mut menu = ExplorerAddMenu::new(
        Uuid::from_u128(1),
        vec![
            option(ExplorerAddKind::Connection, true),
            option(ExplorerAddKind::Database, false),
            option(ExplorerAddKind::Role, true),
        ],
    );
    assert!(menu.move_selection(1));
    assert_eq!(menu.selected, 2);
    assert!(!menu.move_selection(1));
    assert_eq!(menu.selected, 2);
    assert!(menu.move_selection(-1));
    assert_eq!(menu.selected, 0);
}

#[test]
fn direct_selection_rejects_disabled_options() {
    let mut menu = ExplorerAddMenu::new(
        Uuid::from_u128(1),
        vec![
            option(ExplorerAddKind::Connection, true),
            option(ExplorerAddKind::Database, false),
        ],
    );
    assert!(!menu.select(1));
    assert_eq!(menu.selected, 0);
}
```

**Step 2: Run the focused test and confirm failure**

Run: `cargo test --test explorer_add`

Expected: compilation fails because `model::explorer_add` does not exist.

**Step 3: Implement the state model**

Create `src/model/explorer_add.rs`:

```rust
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorerAddKind {
    Connection,
    ConnectionGroup,
    Database,
    User,
    Role,
}

impl ExplorerAddKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Connection => "Connection",
            Self::ConnectionGroup => "Connection Group",
            Self::Database => "Database",
            Self::User => "User",
            Self::Role => "Role",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Connection => "New server profile",
            Self::ConnectionGroup => "Organize connections",
            Self::Database => "Create a database",
            Self::User => "Login-enabled role",
            Self::Role => "Permission role",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorerAddAvailability {
    Available,
    Unavailable(&'static str),
}

impl ExplorerAddAvailability {
    pub const fn is_available(self) -> bool {
        matches!(self, Self::Available)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExplorerAddOption {
    pub kind: ExplorerAddKind,
    pub availability: ExplorerAddAvailability,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExplorerAddMenu {
    pub profile_id: Uuid,
    pub selected: usize,
    pub options: Vec<ExplorerAddOption>,
}

impl ExplorerAddMenu {
    pub fn new(profile_id: Uuid, options: Vec<ExplorerAddOption>) -> Self {
        let selected = options
            .iter()
            .position(|option| option.availability.is_available())
            .unwrap_or(0);
        Self {
            profile_id,
            selected,
            options,
        }
    }

    pub fn selected_option(&self) -> Option<&ExplorerAddOption> {
        self.options
            .get(self.selected)
            .filter(|option| option.availability.is_available())
    }

    pub fn selected_kind(&self) -> Option<ExplorerAddKind> {
        self.selected_option().map(|option| option.kind)
    }

    pub fn select(&mut self, index: usize) -> bool {
        if self
            .options
            .get(index)
            .is_none_or(|option| !option.availability.is_available())
        {
            return false;
        }
        let changed = self.selected != index;
        self.selected = index;
        changed
    }

    pub fn move_selection(&mut self, delta: isize) -> bool {
        if delta == 0 || self.options.is_empty() {
            return false;
        }
        let step = delta.signum();
        let mut index = self.selected as isize;
        loop {
            let next = index + step;
            if next < 0 || next >= self.options.len() as isize {
                return false;
            }
            index = next;
            if self.options[index as usize].availability.is_available() {
                self.selected = index as usize;
                return true;
            }
        }
    }
}
```

Export it from `src/model/mod.rs`:

```rust
pub mod explorer_add;
```

If Rust 1.94 or the configured lints reject `Option::is_none_or`, use the equivalent `map_or(true, ...)` expression without changing behavior.

**Step 4: Run model tests**

Run: `cargo test --test explorer_add`

Expected: all three tests pass.

**Step 5: Commit the state model**

```bash
git add src/model/explorer_add.rs src/model/mod.rs tests/explorer_add.rs
git commit -m "feat(explorer): add connection action menu state"
```

---

### Task 2: Add Direct Catalog Object Selection

**Files:**
- Modify: `src/model/catalog_editor.rs:1176-1254` and the remainder of `select_option`
- Modify: `tests/catalog_editor_state.rs:107-145`

**Step 1: Write failing direct-selection tests**

Add tests to `tests/catalog_editor_state.rs` proving callers can enter the form without constructing a one-item picker:

```rust
#[test]
fn profile_object_type_selection_opens_database_form_directly() {
    let mut editor = CatalogEditorState::new(
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Profile {
            profile_id: profile(),
        },
        1,
        vec![],
    );

    assert!(editor.select_object_type(CatalogObjectType::Catalog(
        lazydb::db::catalog::CatalogKind::Database,
    )));
    assert_eq!(editor.page, CatalogEditorPage::Form);
    assert!(matches!(editor.draft, Some(CatalogDraft::Database(_))));
}

#[test]
fn profile_object_type_selection_distinguishes_user_and_role() {
    for (object_type, expected_login) in [
        (CatalogObjectType::LoginRole, true),
        (CatalogObjectType::Role, false),
    ] {
        let mut editor = CatalogEditorState::new(
            CatalogMutationMode::Create,
            CatalogMutationAnchor::Profile {
                profile_id: profile(),
            },
            1,
            vec![],
        );
        assert!(editor.select_object_type(object_type));
        let Some(CatalogDraft::Role(draft)) = editor.draft else {
            panic!("role draft expected");
        };
        assert_eq!(draft.login, expected_login);
    }
}
```

Retain the existing `profile_role_picker_initializes_login_and_non_login_drafts` test. It protects the old picker path.

**Step 2: Run the focused tests and confirm failure**

Run: `cargo test --test catalog_editor_state profile_object_type_selection`

Expected: compilation fails because `CatalogEditorState::select_object_type` does not exist.

**Step 3: Extract the existing draft initialization**

In `CatalogEditorState`, change `select_option` into a thin picker adapter:

```rust
pub fn select_option(&mut self, selected: usize) -> bool {
    if self.is_busy() {
        return false;
    }
    let Some(option) = self.options.get(selected) else {
        return false;
    };
    self.selected_option = selected;
    self.select_object_type(option.object_type)
}
```

Add:

```rust
pub fn select_object_type(&mut self, object_type: CatalogObjectType) -> bool {
    if self.is_busy() || self.mode != CatalogMutationMode::Create {
        return false;
    }
    self.object_type = Some(object_type);
    self.page = CatalogEditorPage::Form;
    self.draft = None;

    // Move the current draft-construction body from select_option here unchanged.
    // Replace reads of self.object_type with the object_type argument where practical.
    // Database, role, schema, table, view, materialized-view, sequence, index,
    // column, and constraint initialization must all retain current behavior.

    self.draft.is_some()
}
```

The implementation step is an extraction, not a rewrite: move all existing draft branches currently following `self.page = CatalogEditorPage::Form` into `select_object_type`. Return `true` only when a supported draft was created. Do not alter field defaults or anchor interpretation.

**Step 4: Run Catalog Editor state tests**

Run: `cargo test --test catalog_editor_state`

Expected: the new direct-selection tests and all existing picker/draft tests pass.

**Step 5: Commit the extraction**

```bash
git add src/model/catalog_editor.rs tests/catalog_editor_state.rs
git commit -m "refactor(catalog): support direct create type selection"
```

---

### Task 3: Add Overlay Actions and Reducer Transitions

**Files:**
- Modify: `src/action.rs:118-190`
- Modify: `src/model/workspace.rs:102-165`
- Modify: `src/app.rs:1892-4470` for action dispatch
- Modify: `src/app.rs` near `resolve_explorer_mutation_intent` for helpers
- Modify: `tests/explorer_add.rs`

**Step 1: Write failing reducer tests for opening the menu**

Extend `tests/explorer_add.rs` with an application fixture. Use a PostgreSQL profile and mark it connected through the existing reducer action:

```rust
use lazydb::{
    action::Action,
    app::App,
    db::ServerInfo,
    model::{
        catalog_editor::CatalogDraft,
        explorer::ExplorerNodeId,
        explorer_add::{ExplorerAddAvailability, ExplorerAddKind},
        workspace::{Focus, Overlay},
    },
    profile::{DatabaseKind, import_connection_url},
};

fn connected_postgres() -> (App, Uuid) {
    let profile = import_connection_url("postgres://localhost/app", Some("production"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.focus = Focus::Explorer;
    app.explorer.normalized.selected = Some(ExplorerNodeId::Profile(profile_id));
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation: 1,
        server: ServerInfo {
            kind: DatabaseKind::Postgres,
            version: "16.4".into(),
            database: "app".into(),
        },
    });
    (app, profile_id)
}

#[test]
fn opening_add_menu_builds_the_required_order() {
    let (mut app, profile_id) = connected_postgres();
    app.update(Action::OpenExplorerAdd);
    let Some(Overlay::ExplorerAdd(menu)) = app.overlay.as_ref() else {
        panic!("explorer add menu expected");
    };
    assert_eq!(menu.profile_id, profile_id);
    assert_eq!(
        menu.options.iter().map(|option| option.kind).collect::<Vec<_>>(),
        vec![
            ExplorerAddKind::Connection,
            ExplorerAddKind::ConnectionGroup,
            ExplorerAddKind::Database,
            ExplorerAddKind::User,
            ExplorerAddKind::Role,
        ]
    );
    assert!(menu.options.iter().all(|option| {
        option.availability == ExplorerAddAvailability::Available
    }));
}
```

Add separate tests for disabled reasons. Each fixture must still have five options:

```rust
#[test]
fn disconnected_postgres_disables_catalog_actions() { /* reason: "Connect this connection first" */ }

#[test]
fn non_postgres_disables_catalog_actions() { /* reason: "PostgreSQL only" */ }

#[test]
fn read_only_postgres_disables_catalog_actions() { /* reason: "Read-only connection" */ }

#[test]
fn inactive_postgres_disables_catalog_actions() { /* reason: "Activate this connection first" */ }
```

For every case, assert Connection and Connection Group are available and indexes `2..=4` are unavailable with the expected reason.

**Step 2: Run the reducer tests and confirm failure**

Run: `cargo test --test explorer_add opening_add_menu_builds_the_required_order`

Expected: compilation fails because `Action::OpenExplorerAdd` and `Overlay::ExplorerAdd` do not exist.

**Step 3: Add action and overlay variants**

Add to `Action`:

```rust
OpenExplorerAdd,
ExplorerAddMove(isize),
ExplorerAddSelect(usize),
ExplorerAddConfirm,
ExplorerAddCancel,
```

Add to `Overlay`:

```rust
ExplorerAdd(crate::model::explorer_add::ExplorerAddMenu),
```

Do not add a generic callback or boxed action to the model. Explicit enum variants preserve `Clone`, `Debug`, `Eq`, and reducer testability.

**Step 4: Implement menu option construction**

Add an `App` helper with this responsibility:

```rust
fn explorer_add_options(
    &self,
    profile_id: Uuid,
) -> Vec<crate::model::explorer_add::ExplorerAddOption>
```

Always start with available Connection and Connection Group options. Determine one context-level catalog failure in this priority order:

1. Missing profile: do not open the menu.
2. Non-PostgreSQL profile: `"PostgreSQL only"`.
3. Read-only profile: `"Read-only connection"`.
4. No active connection identity: `"Connect this connection first"`.
5. Active connection belongs to another profile: `"Activate this connection first"`.

When the context is valid, call:

```rust
crate::db::postgres::PostgresAdapter::catalog_mutation_capabilities()
    .create_options(
        &CatalogMutationAnchor::Profile { profile_id },
        None,
    )
```

Map support by object type:

```rust
ExplorerAddKind::Database => CatalogObjectType::Catalog(CatalogKind::Database),
ExplorerAddKind::User => CatalogObjectType::LoginRole,
ExplorerAddKind::Role => CatalogObjectType::Role,
```

If a valid PostgreSQL context does not advertise a specific type, mark only that row unavailable with `"Unsupported by this server"`. Do not hide it.

Handle `Action::OpenExplorerAdd` only when direct selection is a Profile:

```rust
let Some(ExplorerNodeId::Profile(profile_id)) =
    self.explorer.normalized.selected.as_ref()
else {
    return Vec::new();
};
let profile_id = *profile_id;
if !self.profiles.iter().any(|profile| profile.id == profile_id) {
    return Vec::new();
}
let options = self.explorer_add_options(profile_id);
self.overlay = Some(Overlay::ExplorerAdd(ExplorerAddMenu::new(profile_id, options)));
Vec::new()
```

Implement move, direct select, and cancel as state-only reducer transitions. Cancel closes only `Overlay::ExplorerAdd`.

**Step 5: Write failing confirmation tests**

Add reducer tests covering all enabled outcomes:

```rust
#[test]
fn confirming_connection_opens_new_profile_manager() { /* selected index 0 */ }

#[test]
fn confirming_connection_group_opens_group_editor() { /* selected index 1 */ }

#[test]
fn confirming_database_opens_database_form_directly() { /* selected index 2 */ }

#[test]
fn confirming_user_opens_login_role_form_directly() { /* selected index 3 */ }

#[test]
fn confirming_role_opens_non_login_role_form_directly() { /* selected index 4 */ }

#[test]
fn confirming_an_unavailable_option_keeps_the_menu_open() { /* defensive invariant */ }
```

For Catalog outcomes assert all of the following:

```rust
assert!(matches!(app.overlay, Some(Overlay::CatalogEditor)));
assert_eq!(editor.page, CatalogEditorPage::Form);
assert!(editor.options.is_empty());
assert!(matches!(editor.draft, Some(CatalogDraft::Database(_))));
```

For User and Role destructure `CatalogDraft::Role` and assert `draft.login`.

**Step 6: Implement confirmation delegation**

Before dispatching, take the selected `(profile_id, kind)` from the menu. If no enabled option is selected, return without closing it.

Map non-catalog options to existing actions:

```rust
ExplorerAddKind::Connection => self.update(Action::ProfileStartNew),
ExplorerAddKind::ConnectionGroup => self.update(Action::ProfileGroupCreate),
```

For catalog items, add:

```rust
fn open_profile_catalog_create(
    &mut self,
    profile_id: Uuid,
    object_type: CatalogObjectType,
) -> Vec<Command>
```

This helper must re-check that the selected profile is still the active writable PostgreSQL connection, because state may change while an overlay is open. If invalid, keep/reopen the add menu and notify a warning instead of opening a broken editor.

For a valid context:

```rust
let catalog_epoch = self
    .explorer
    .normalized
    .profiles
    .get(&profile_id)
    .map_or(0, |state| state.catalog_epoch);
let mut editor = CatalogEditorState::new(
    CatalogMutationMode::Create,
    CatalogMutationAnchor::Profile { profile_id },
    catalog_epoch,
    Vec::new(),
);
if !editor.select_object_type(object_type) {
    return Vec::new();
}
self.catalog_editor = Some(editor);
self.overlay = Some(Overlay::CatalogEditor);
Vec::new()
```

Use this exact mapping:

```rust
ExplorerAddKind::Database => CatalogObjectType::Catalog(CatalogKind::Database),
ExplorerAddKind::User => CatalogObjectType::LoginRole,
ExplorerAddKind::Role => CatalogObjectType::Role,
```

**Step 7: Run reducer and Catalog Editor tests**

Run: `cargo test --test explorer_add --test catalog_editor_state`

Expected: all tests pass.

**Step 8: Commit reducer behavior**

```bash
git add src/action.rs src/model/workspace.rs src/app.rs tests/explorer_add.rs
git commit -m "feat(explorer): route connection add actions"
```

---

### Task 4: Route Keyboard Input Through the New Overlay

**Files:**
- Modify: `src/input/keymap.rs:62-153,1845-1869`
- Modify: `tests/keymap.rs:465-526,1612-1625`

**Step 1: Update the failing profile shortcut regression**

Rename and change the existing test at `tests/keymap.rs:1613`:

```rust
#[test]
fn explorer_a_on_a_profile_opens_add_menu() {
    let profile = import_connection_url(":memory:", Some("test"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile]);
    app.focus = Focus::Explorer;
    let mut keymap = Keymap::default();

    assert_eq!(
        keymap.map(key(KeyCode::Char('a')), &app),
        Some(Action::OpenExplorerAdd)
    );
}
```

Update `explorer_catalog_shortcuts_are_not_advertised_for_synthetic_rows` at `tests/keymap.rs:508`: its initial Profile assertion must also expect `OpenExplorerAdd`; retain the Status-row assertion that `a` returns `None`.

Add a connection-group regression by assigning an `ExplorerNodeId::ConnectionGroup` selection and asserting `a` still returns `ProfileGroupCreate`.

**Step 2: Run the shortcut tests and confirm failure**

Run: `cargo test --test keymap explorer_a_on_a_profile_opens_add_menu`

Expected: assertion fails because the current mapping returns `ProfileGroupCreate`.

**Step 3: Change only the Profile branch of `map_explorer`**

Replace the current combined Profile/ConnectionGroup condition at `src/input/keymap.rs:1857` with:

```rust
KeyCode::Char('a') => {
    if matches!(
        app.explorer.normalized.selected,
        Some(ExplorerNodeId::Profile(_))
    ) {
        return Some(Action::OpenExplorerAdd);
    }
    if matches!(
        app.explorer.normalized.selected,
        Some(ExplorerNodeId::ConnectionGroup { .. })
    ) {
        return Some(Action::ProfileGroupCreate);
    }
    return crate::help::shortcut_is_available_in_app(
        app,
        crate::help::HelpShortcutId::ExplorerCreateCatalog,
    )
    .then_some(Action::OpenCatalogCreate);
}
```

**Step 4: Write failing overlay key ownership tests**

Create a menu through `app.update(Action::OpenExplorerAdd)` and assert:

```rust
assert_eq!(keymap.map(key(KeyCode::Char('j')), &app), Some(Action::ExplorerAddMove(1)));
assert_eq!(keymap.map(key(KeyCode::Down), &app), Some(Action::ExplorerAddMove(1)));
assert_eq!(keymap.map(key(KeyCode::Char('k')), &app), Some(Action::ExplorerAddMove(-1)));
assert_eq!(keymap.map(key(KeyCode::Up), &app), Some(Action::ExplorerAddMove(-1)));
assert_eq!(keymap.map(key(KeyCode::Enter), &app), Some(Action::ExplorerAddConfirm));
assert_eq!(keymap.map(key(KeyCode::Esc), &app), Some(Action::ExplorerAddCancel));
assert_eq!(keymap.map(key(KeyCode::Char('q')), &app), Some(Action::ExplorerAddCancel));
assert_eq!(keymap.map(key(KeyCode::Char('a')), &app), None);
```

The last assertion proves overlay ownership prevents `a` from opening a second flow.

**Step 5: Add the overlay keymap branch**

Place it near the existing Profile Access and Profile Group overlay handling, before normal Explorer input:

```rust
if matches!(app.overlay, Some(Overlay::ExplorerAdd(_))) {
    self.pending = None;
    return match event.code {
        KeyCode::Up | KeyCode::Char('k') => Some(Action::ExplorerAddMove(-1)),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::ExplorerAddMove(1)),
        KeyCode::Enter => Some(Action::ExplorerAddConfirm),
        KeyCode::Esc | KeyCode::Char('q') => Some(Action::ExplorerAddCancel),
        _ => None,
    };
}
```

**Step 6: Run all keymap tests**

Run: `cargo test --test keymap`

Expected: all tests pass, including existing Catalog Editor and Profile Group key ownership tests.

**Step 7: Commit keyboard behavior**

```bash
git add src/input/keymap.rs tests/keymap.rs
git commit -m "feat(explorer): open add menu from connection shortcut"
```

---

### Task 5: Add Mode-Safe Icons and the Polished Popup

**Files:**
- Modify: `src/ui/icons.rs:19-43,159-297,299-442`
- Modify: `src/ui/mod.rs:150-192,630-783,2817-3140`
- Modify: `tests/ui_render.rs`

**Step 1: Write failing icon-mode tests**

Add a public or crate-visible icon category in `src/ui/icons.rs` and tests for all modes:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorerAddIcon {
    Connection,
    ConnectionGroup,
    User,
    Role,
}
```

In the icon test module, assert every icon is non-empty in each `IconMode`, and explicitly assert ASCII fallbacks:

```rust
assert_eq!(ascii.explorer_add(ExplorerAddIcon::Connection), "CN");
assert_eq!(ascii.explorer_add(ExplorerAddIcon::ConnectionGroup), "GR");
assert_eq!(ascii.explorer_add(ExplorerAddIcon::User), "US");
assert_eq!(ascii.explorer_add(ExplorerAddIcon::Role), "RL");
```

Database intentionally continues to use `icons.catalog(CatalogKind::Database)` and does not need a duplicate add-menu icon variant.

**Step 2: Run icon tests and confirm failure**

Run: `cargo test ui::icons::tests`

Expected: compilation fails because `ExplorerAddIcon` and `IconSet::explorer_add` do not exist.

**Step 3: Implement icon mappings**

Add `IconSet::explorer_add`. Use constants available from the existing `nerd_font_symbols::md` module; verify the exact constant names by compiling rather than hard-coding private-use glyph strings. Preferred semantics are:

```rust
IconMode::NerdFont => match kind {
    ExplorerAddIcon::Connection => md::MD_CONNECTION,
    ExplorerAddIcon::ConnectionGroup => md::MD_FOLDER_PLUS,
    ExplorerAddIcon::User => md::MD_ACCOUNT_PLUS,
    ExplorerAddIcon::Role => md::MD_SHIELD_ACCOUNT,
},
IconMode::Unicode => match kind {
    ExplorerAddIcon::Connection => "⇄",
    ExplorerAddIcon::ConnectionGroup => "▰",
    ExplorerAddIcon::User => "●",
    ExplorerAddIcon::Role => "◇",
},
IconMode::Ascii => match kind {
    ExplorerAddIcon::Connection => "CN",
    ExplorerAddIcon::ConnectionGroup => "GR",
    ExplorerAddIcon::User => "US",
    ExplorerAddIcon::Role => "RL",
},
```

If a preferred Nerd Font constant is unavailable in `nerd-font-symbols 0.3`, choose the closest exported `md` constant. Do not use a raw Nerd Font glyph.

**Step 4: Write failing popup render tests**

Add a connected PostgreSQL fixture to `tests/ui_render.rs`, open `Action::OpenExplorerAdd`, and render at `80 x 24`. Assert:

```rust
assert!(output.contains("ADD TO CONNECTION"), "{output}");
assert!(output.contains("production"), "{output}");
assert!(output.contains("Connection"), "{output}");
assert!(output.contains("Connection Group"), "{output}");
assert!(output.contains("Database"), "{output}");
assert!(output.contains("User"), "{output}");
assert!(output.contains("Role"), "{output}");
assert!(output.contains("j/k"), "{output}");
assert!(output.contains("Enter"), "{output}");
assert!(output.contains("Esc"), "{output}");
```

Add a disconnected fixture and assert its output contains `Connect this connection first`.

Render the enabled fixture with `IconSet::new(IconMode::Ascii)` and assert `CN`, `GR`, `DB`, `US`, and `RL` appear. Add a compact `56 x 16` render and assert the title, all five primary labels, footer, and rounded border are still present.

**Step 5: Run popup tests and confirm failure**

Run: `cargo test --test ui_render explorer_add`

Expected: tests fail because the new overlay is not rendered.

**Step 6: Register overlay rendering and animation identity**

In `render_overlay`, add:

```rust
Overlay::ExplorerAdd(menu) => {
    render_explorer_add(frame, area, app, menu, state, theme, icons)
}
```

In `overlay_key`, assign the new overlay a unique value, for example `20`. Do not reuse another overlay key because the animation state uses this identity.

**Step 7: Implement `render_explorer_add`**

Use a centered `64 x 14` popup clamped to the available area:

```rust
let popup = centered(area, 64.min(area.width), 14.min(area.height));
frame.render_widget(Clear, popup);
let block = panel_block(" ADD TO CONNECTION ", true, theme);
let inner = block.inner(popup);
frame.render_widget(block, popup);
```

Render these regions:

- Header row at `inner.y`: `TARGET  <sanitized profile name> · <database kind>`.
- Blank separator row when height permits.
- Five fixed option rows.
- Footer at `inner.bottom() - 1`: `j/k · ↑/↓ select   Enter continue   Esc close`.

For terminals too narrow for descriptions, compute whether the longest `icon + label + description/reason` fits. Hide only the right-hand description; never hide icons or labels. Use `UnicodeWidthStr`/the existing `cell_width()` helper for terminal width, not byte length.

Each option row should be assembled from styled spans:

- Selected marker `›`: `theme.accent`.
- Type icon: Connection `theme.action`, Group `theme.warning`, Database `theme.accent`, User `theme.success`, Role `theme.warning`.
- Enabled label: `theme.text`, bold only when selected.
- Description: `theme.muted`.
- Disabled icon, label, and reason: `theme.muted`.
- Selected row background: `theme.selection`; unselected row background: `theme.surface`.

Sanitize the profile name with `sanitize_terminal_text`. Labels and static descriptions do not require sanitization.

Use these icon sources:

```rust
ExplorerAddKind::Connection => icons.explorer_add(ExplorerAddIcon::Connection),
ExplorerAddKind::ConnectionGroup => icons.explorer_add(ExplorerAddIcon::ConnectionGroup),
ExplorerAddKind::Database => icons.catalog(CatalogKind::Database),
ExplorerAddKind::User => icons.explorer_add(ExplorerAddIcon::User),
ExplorerAddKind::Role => icons.explorer_add(ExplorerAddIcon::Role),
```

**Step 8: Run icon and render tests**

Run: `cargo test ui::icons::tests && cargo test --test ui_render explorer_add`

Expected: icon-mode and popup tests pass at normal and compact sizes.

**Step 9: Commit visual behavior**

```bash
git add src/ui/icons.rs src/ui/mod.rs tests/ui_render.rs
git commit -m "feat(ui): render connection add action menu"
```

---

### Task 6: Add Mouse Selection and Contextual Help

**Files:**
- Modify: `src/ui/mod.rs:150-192` and `render_explorer_add`
- Modify: `src/input/mouse.rs:49-151,270-285`
- Modify: `src/help.rs:280-290,1017-1047,2253-2402,2920-2970`
- Modify: `tests/mouse.rs`
- Modify: `tests/ui_render.rs`
- Modify: help tests in `src/help.rs`

**Step 1: Write failing hit-region and mouse tests**

Add `HitTarget::ExplorerAddOption(usize)`.

In `tests/ui_render.rs`, render the popup with state and assert five option hit regions exist with indexes `0..5`. Disabled rows should still be hit-testable for selection feedback only if `ExplorerAddSelect` rejects them; the recommended simpler behavior is to register hit regions only for enabled rows.

In `tests/mouse.rs`, construct/open the overlay, supply a hit region for an enabled option, click it, and assert:

```rust
Some(Action::ExplorerAddSelect(index))
```

Also assert clicking outside registered popup controls returns `None` while the overlay is open.

**Step 2: Run focused tests and confirm failure**

Run: `cargo test --test mouse explorer_add && cargo test --test ui_render explorer_add`

Expected: compilation fails because `HitTarget::ExplorerAddOption` does not exist.

**Step 3: Register and map menu hit regions**

For each enabled option row in `render_explorer_add`, push:

```rust
state.hit_regions.push(HitRegion {
    area: row,
    target: HitTarget::ExplorerAddOption(index),
});
```

Allow this target through the overlay mouse filter in `src/input/mouse.rs`, then map it:

```rust
HitTarget::ExplorerAddOption(index) => Some(Action::ExplorerAddSelect(index)),
```

Add the new target to any exhaustive `HitTarget` matches in mouse-up/right-click handling as a no-op where appropriate.

**Step 4: Write failing help capability tests**

Adjust existing help tests so a selected Profile exposes one action:

```text
a  add to connection
```

A selected connection group should expose:

```text
a  new connection group
```

A supported Catalog anchor should continue to expose:

```text
a  add object
```

Do not advertise both `add object` and `new connection group` for the same Profile selection.

**Step 5: Update help IDs and capabilities**

Add a dedicated `HelpShortcutId::ExplorerAddToConnection` row:

```rust
row!(
    ExplorerAddToConnection,
    [Explorer],
    "a",
    "add to connection",
    ProfileEditAvailable,
    executable
),
```

Map this executable help action to `Action::OpenExplorerAdd` in the same `App` help dispatch match that currently maps Explorer shortcut IDs. Keep `ExplorerCreateCatalog` for Catalog nodes and `ExplorerCreateGroup` for connection-group nodes.

Refine help filtering so:

- `ExplorerAddToConnection` is available only for `ExplorerNodeId::Profile`.
- `ExplorerCreateGroup` is available only for `ExplorerNodeId::ConnectionGroup`.
- `ExplorerCreateCatalog` is available only when `catalog_editor_capabilities` reports a supported Catalog anchor; a Profile no longer needs this help entry because its catalog actions live behind the new menu.

Prefer adding explicit booleans to `ShortcutCapabilities` if required. Do not infer availability from shortcut description text.

**Step 6: Run mouse and help tests**

Run: `cargo test --test mouse && cargo test help::`

Expected: all mouse and help tests pass with no duplicate `a` entry for Profile nodes.

**Step 7: Commit mouse and help behavior**

```bash
git add src/ui/mod.rs src/input/mouse.rs src/help.rs tests/mouse.rs tests/ui_render.rs
git commit -m "feat(explorer): add menu mouse and help integration"
```

---

### Task 7: Centralize Existing Catalog Create Options

**Files:**
- Modify: `src/app.rs:2892-2992`
- Modify: `tests/catalog_editor_reducer.rs:27-84`
- Modify: or add focused capability tests in `tests/catalog_mutation.rs`

**Step 1: Add a regression test for capability-derived options**

Build a connected writable PostgreSQL app with a Profile selection, then invoke the existing `Action::OpenCatalogCreate` directly. This action remains a valid internal/help execution path even though Profile `a` now opens the new menu. Assert its object picker options equal the adapter's `create_options` result in order and labels come from `display_label()`.

Add a Schema anchor regression asserting all adapter-supported Schema create types appear. This test must catch the current duplicate Schema match arm in `src/app.rs`, where the first branch makes a later `View + Sequence` branch unreachable.

**Step 2: Run reducer tests and confirm the Schema assertion fails**

Run: `cargo test --test catalog_editor_reducer catalog_create_options`

Expected: the Schema test fails because the current hand-built match does not reflect all adapter capability results.

**Step 3: Replace hand-built options with the capability source**

In `Action::OpenCatalogCreate`:

1. Resolve the `CatalogMutationAnchor` as today.
2. Resolve the selected Catalog entry for `CatalogMutationAnchor::Catalog`; use `None` for Profile/Group anchors.
3. Obtain `PostgresAdapter::catalog_mutation_capabilities()`.
4. Call `create_options(&anchor, entry)`.
5. Convert each returned type to the UI model:

```rust
let options = object_types
    .into_iter()
    .map(|object_type| CatalogMutationOption {
        object_type,
        label: object_type.display_label().into(),
    })
    .collect::<Vec<_>>();
```

6. If capability resolution fails or produces no options, notify a Catalog warning and do not open an empty overlay.
7. Remove the entire hand-written anchor match, including both duplicate Schema arms.

Do not combine this helper with `ExplorerAddOption`: the former describes database object types while the latter includes non-catalog actions and availability presentation.

**Step 4: Run Catalog tests**

Run: `cargo test --test catalog_editor_reducer --test catalog_mutation --test catalog_editor_state`

Expected: all tests pass and Schema options now match adapter capabilities.

**Step 5: Commit capability cleanup**

```bash
git add src/app.rs tests/catalog_editor_reducer.rs tests/catalog_mutation.rs
git commit -m "refactor(catalog): derive create options from capabilities"
```

---

### Task 8: Full Verification and Manual Acceptance

**Files:**
- Modify only files required to fix issues revealed by verification.

**Step 1: Format the code**

Run: `cargo fmt --all`

Expected: command succeeds.

**Step 2: Check formatting**

Run: `cargo fmt --all --check`

Expected: command succeeds with no diff.

**Step 3: Compile all targets**

Run: `cargo check --all-targets --all-features`

Expected: command succeeds without warnings or errors.

**Step 4: Run focused integration tests**

Run:

```bash
cargo test --test explorer_add
cargo test --test keymap
cargo test --test catalog_editor_state
cargo test --test catalog_editor_reducer
cargo test --test ui_render
cargo test --test mouse
```

Expected: every command passes.

**Step 5: Run the complete test suite**

Run: `cargo test --all-targets --all-features`

Expected: all unit and integration tests pass.

**Step 6: Run Clippy with warnings denied**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: command succeeds without diagnostics.

**Step 7: Perform interactive keyboard acceptance**

Run the application using the project's normal development command and verify:

1. Select a PostgreSQL connection and press `a`; confirm the five-row popup appears.
2. Press `j`, `k`, `Up`, and `Down`; confirm selection movement and boundary clamping.
3. Select Connection and press Enter; confirm the new connection form opens.
4. Reopen the popup, select Connection Group, and press Enter; confirm the group-name editor opens.
5. On an active writable PostgreSQL connection, open Database, User, and Role; confirm each goes directly to the correct form without a second picker.
6. Press Esc and `q` in the popup; confirm both close it without opening another overlay.
7. Select a SQLite/MySQL/SQL Server, disconnected PostgreSQL, inactive PostgreSQL, and read-only PostgreSQL profile; confirm all five rows remain visible and unavailable catalog rows show the correct reason.
8. Select a connection group and press `a`; confirm direct group creation still works.
9. Select a supported Catalog node and press `a`; confirm normal Catalog object creation still works.
10. Launch once with each configured icon mode and confirm icons align without corrupting row width.
11. Resize the terminal to `56 x 16`; confirm title, five labels, footer, and border remain readable.

**Step 8: Inspect the final diff**

Run: `git diff --check && git status --short && git diff`

Expected: no whitespace errors; only the files listed by this plan are changed, apart from pre-existing unrelated worktree changes.

**Step 9: Commit final verification fixes if any**

Only if verification required additional source changes:

```bash
git add <only-files-changed-for-this-feature>
git commit -m "fix(explorer): address add menu regressions"
```

Do not create an empty commit. Do not stage unrelated worktree changes.

---

## Implementation Notes

- Before editing each symbol, use `codegraph_explore` to refresh its current source and blast radius. The worktree may have changed since this plan was written.
- Keep exactly one overlay active. Confirmation replaces `Overlay::ExplorerAdd` with the destination overlay; it must not retain a hidden menu underneath.
- Sanitize only dynamic terminal text such as the profile name and unavailability text if it becomes dynamic. Static labels are trusted constants.
- Revalidate the catalog action at confirmation time. Rendering availability is not an authorization or correctness boundary.
- Avoid introducing a generic menu framework in this iteration. The new state is small, and existing pickers have domain-specific behavior; generalization is not justified until another identical menu is needed.
- Preserve existing PostgreSQL role semantics: User means `LOGIN`, Role means `NOLOGIN` through the existing `RoleDraft::new(bool)` path.
- Do not change Catalog mutation SQL, runtime commands, refresh behavior, or success/error notifications.
- The commit steps are execution checkpoints, not authorization to commit during planning. Only create commits when the user explicitly requests plan execution with commits.
