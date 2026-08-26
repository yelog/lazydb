use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use lazydb::{
    action::Action,
    app::App,
    cli::ConfirmationPolicy,
    db::{
        ServerInfo,
        catalog::{
            CatalogCount, CatalogCursor, CatalogEntry, CatalogId, CatalogKind, ObjectGroup,
            OptionalMetadata, QualifiedName,
        },
        query::{ColumnMeta, QueryOutcome, QueryStats, ResultSet},
        value::CellValue,
    },
    model::{
        explorer::{CatalogGroupState, ExplorerLoadState, ExplorerNodeId, ExplorerOwnerId},
        profile_manager::{ProfileField, ProfileManagerPage, ProfileOperation},
        relation::RelationTab,
        tab::CompletionPopup,
        tab::WorkspaceTab,
        workspace::{ConnectionStatus, Focus},
    },
    persistence::secrets::keyring_ref,
    profile::{DatabaseKind, Environment, import_connection_url},
    sql::{CompletionCandidate, CompletionKind, CompletionScore, TextRange},
    ui::{self, HitTarget, ProfileButton, UiState},
};
use ratatui::{Terminal, backend::TestBackend};

fn fixture() -> App {
    let profile = import_connection_url("sqlite::memory:", Some("orbital-lab"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile.clone()]);
    app.connection.profile_id = Some(profile.id);
    app.connection.status = ConnectionStatus::Connected;
    app.connection.server = Some(ServerInfo {
        kind: DatabaseKind::Sqlite,
        version: "3.50.0".into(),
        database: ":memory:".into(),
    });
    app.update(Action::ReplaceEditor(
        "SELECT id, name, active\nFROM users\nWHERE active = true;".into(),
    ));
    let tab_id = app.active_console().id;
    let generation = app.active_console().generation;
    app.update(Action::QueryFinished {
        tab_id,
        generation,
        connection: app.connection.active_identity().unwrap(),
        outcome: QueryOutcome {
            result_sets: vec![ResultSet {
                columns: vec![
                    ColumnMeta {
                        name: "id".into(),
                        type_name: "INTEGER".into(),
                    },
                    ColumnMeta {
                        name: "name".into(),
                        type_name: "TEXT".into(),
                    },
                    ColumnMeta {
                        name: "active".into(),
                        type_name: "BOOLEAN".into(),
                    },
                ],
                rows: vec![vec![
                    CellValue::Integer(42),
                    CellValue::Text("Ada".into()),
                    CellValue::Boolean(true),
                ]],
                affected_rows: 0,
            }],
            stats: QueryStats::new(Duration::from_millis(24), Duration::from_millis(352), 1),
        },
    });
    app
}

fn render(app: &App, width: u16, height: u16) -> String {
    render_with_state(app, width, height).0
}

fn render_with_state(app: &App, width: u16, height: u16) -> (String, UiState) {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = UiState::new(true);
    terminal
        .draw(|frame| ui::render_with_state(frame, app, &mut state))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let mut output = String::new();
    for y in 0..height {
        for x in 0..width {
            output.push_str(buffer[(x, y)].symbol());
        }
        output.push('\n');
    }
    (output, state)
}

#[test]
fn active_profile_explorer_renders_normalized_group_permission_and_load_more_rows() {
    let profile = import_connection_url(":memory:", Some("catalog-ui"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile.clone()]);
    app.connection.profile_id = Some(profile.id);
    app.connection.generation = 1;
    app.connection.status = ConnectionStatus::Connected;
    let database = CatalogEntry::database(
        CatalogId::new(profile.id, CatalogKind::Database, ["app"]),
        QualifiedName {
            database: Some("app".into()),
            schema: None,
            object: "app".into(),
        },
        "database",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    let schema = CatalogEntry::schema(
        CatalogId::new(profile.id, CatalogKind::Schema, ["app", "public"]),
        database.id.clone(),
        QualifiedName {
            database: Some("app".into()),
            schema: Some("public".into()),
            object: "public".into(),
        },
        "schema",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    let owner = ExplorerOwnerId::Group {
        parent: schema.id.clone(),
        group: ObjectGroup::Tables,
    };
    let state = app
        .explorer
        .normalized
        .profiles
        .get_mut(&profile.id)
        .unwrap();
    state
        .catalog
        .insert_subtree(vec![database.clone(), schema.clone()])
        .unwrap();
    state
        .catalog
        .set_group_state(
            &schema.id,
            ObjectGroup::Tables,
            CatalogGroupState {
                count: CatalogCount::Exact(2),
                completeness: lazydb::db::catalog::CatalogCompleteness::Partial,
            },
        )
        .unwrap();
    state.load_states.insert(
        owner.clone(),
        ExplorerLoadState::PermissionDenied { request_id: 4 },
    );
    state
        .load_errors
        .insert(owner.clone(), "permission denied".into());
    app.explorer.normalized.expanded.extend([
        ExplorerNodeId::Catalog(database.id),
        ExplorerNodeId::Catalog(schema.id.clone()),
        ExplorerNodeId::Group {
            parent: schema.id.clone(),
            group: ObjectGroup::Tables,
        },
    ]);
    app.explorer.rebuild_projection(profile.id);
    let permission = render(&app, 120, 36);
    assert!(permission.contains("Tables"));
    assert!(permission.contains("Permission"), "{permission}");

    app.explorer
        .normalized
        .profiles
        .get_mut(&profile.id)
        .unwrap()
        .load_states
        .insert(
            owner,
            ExplorerLoadState::Loaded {
                next_cursor: Some(CatalogCursor::from_keyset("users", "users").unwrap()),
            },
        );
    app.explorer.rebuild_projection(profile.id);
    let load_more = render(&app, 120, 36);
    assert!(load_more.contains("Load more..."));
}

#[test]
fn explorer_width_is_adaptive_and_clamped_in_split_layouts() {
    let app = fixture();
    for (width, expected) in [(120, 40), (180, 56), (300, 56)] {
        let (_, state) = render_with_state(&app, width, 36);
        let explorer = state
            .hit_regions
            .iter()
            .find(|region| region.target == HitTarget::Focus(Focus::Explorer))
            .unwrap();
        assert_eq!(explorer.area.width, expected);
    }
}

#[test]
fn explorer_hostile_metadata_is_sanitized_and_name_type_stay_first() {
    let profile = import_connection_url("sqlite::memory:", Some("safe\x1b[31m-name"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile.clone()]);
    app.focus = Focus::Explorer;
    app.connection.profile_id = Some(profile.id);
    app.connection.status = ConnectionStatus::Connected;
    let database = CatalogEntry::database(
        CatalogId::new(profile.id, CatalogKind::Database, ["db\x1b[31m"]),
        QualifiedName {
            database: Some("db\x1b[31m".into()),
            schema: None,
            object: "db\x1b[31m".into(),
        },
        "db\x1b[31m",
        OptionalMetadata::Supported(Some("comment\x1b[2J".into())),
        true,
    )
    .unwrap();
    app.explorer
        .normalized
        .profiles
        .get_mut(&profile.id)
        .unwrap()
        .catalog
        .insert(database)
        .unwrap();
    app.explorer
        .normalized
        .expanded
        .insert(ExplorerNodeId::Profile(profile.id));
    app.explorer.rebuild_projection(profile.id);
    for (width, height) in [(80, 24), (120, 36), (180, 50)] {
        let output = render(&app, width, height);
        assert!(!output.contains('\x1b'));
        assert!(output.contains("safe<ESC>[31m-name"));
        assert!(output.contains("db<ESC>[31m"));
    }
}

#[test]
fn relation_hostile_title_is_sanitized_in_placeholder() {
    let mut app = App::new(Vec::new());
    let raw_title = "users\x1b[31m\x07";
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new(raw_title)));
    app.active_tab = 1;
    app.focus = Focus::Results;
    assert_eq!(app.tabs[1].title(), raw_title);

    let output = render(&app, 80, 24);

    assert!(!output.contains('\x1b'));
    assert!(output.contains("users<ESC>[31m<0x07>"));
}

#[test]
fn relation_title_is_bounded_in_workspace_tab_bar() {
    let mut app = App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new(format!(
            "{}END",
            "x".repeat(200)
        ))));
    app.active_tab = 1;
    let output = render(&app, 120, 36);
    assert!(!output.contains(&"x".repeat(200)));
    assert!(output.contains(&"x".repeat(48)));
}

#[test]
fn relation_loading_with_previous_snapshot_keeps_data_visible_and_exposes_cancel() {
    let mut app = fixture();
    let mut relation = RelationTab::new("users");
    relation.data = lazydb::model::relation::RelationLoad::Loading {
        request: lazydb::model::relation::RelationRequest {
            tab_id: relation.id,
            tab_generation: relation.generation,
            request_id: 1,
            connection: lazydb::identity::ConnectionIdentity {
                profile_id: uuid::Uuid::nil(),
                generation: 0,
            },
            relation: relation.descriptor.key.clone(),
            kind: lazydb::model::relation::RelationRequestKind::Preview,
            scope: lazydb::profile::CatalogScope::for_profile(DatabaseKind::Sqlite, "db", None),
        },
        previous: Some(lazydb::model::relation::OwnedSnapshot::new(
            lazydb::db::RelationPreview {
                sql: "SELECT previous".into(),
                result: app.active_console().outcome.clone().unwrap(),
            },
            lazydb::identity::ConnectionIdentity {
                profile_id: uuid::Uuid::nil(),
                generation: 0,
            },
            lazydb::profile::CatalogScope::for_profile(DatabaseKind::Sqlite, "db", None),
        )),
    };
    app.tabs.push(WorkspaceTab::Relation(relation));
    app.active_tab = 1;
    let output = render(&app, 120, 36);
    assert!(output.contains("RELATION DATA"), "{output}");
    assert!(output.contains("Refreshing"), "{output}");
}

#[test]
fn relation_page_renders_data_structure_selectors_and_relation_layout() {
    let mut app = App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;
    app.focus = Focus::Results;

    for (width, height) in [(80, 24), (120, 36), (180, 50)] {
        let (output, state) = render_with_state(&app, width, height);
        assert!(output.contains("DATA"), "{width}x{height}: {output}");
        assert!(output.contains("STRUCTURE"), "{width}x{height}: {output}");
        if width >= 100 {
            assert!(output.contains("EXPLORER"), "{width}x{height}: {output}");
        }
        assert!(state.hit_regions.iter().any(|region| region.target
            == HitTarget::RelationView(lazydb::model::relation::RelationView::Data)));
        assert!(state.hit_regions.iter().any(|region| region.target
            == HitTarget::RelationView(lazydb::model::relation::RelationView::Structure)));
    }
}

#[test]
fn explorer_metadata_keeps_name_type_before_flags_and_comments() {
    let profile = import_connection_url(":memory:", Some("metadata-order"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile.clone()]);
    let database = CatalogEntry::database(
        CatalogId::new(profile.id, CatalogKind::Database, ["db"]),
        QualifiedName {
            database: Some("db".into()),
            schema: None,
            object: "db".into(),
        },
        "DATABASE",
        OptionalMetadata::Supported(Some("database comment".into())),
        true,
    )
    .unwrap();
    app.explorer
        .normalized
        .profiles
        .get_mut(&profile.id)
        .unwrap()
        .catalog
        .insert(database)
        .unwrap();
    app.explorer
        .normalized
        .expanded
        .insert(ExplorerNodeId::Profile(profile.id));
    app.explorer.rebuild_projection(profile.id);
    let row = app.explorer.visible().into_iter().find(
        |row| matches!(row.id, ExplorerNodeId::Catalog(ref id) if id.kind == CatalogKind::Database),
    );
    let row = row.unwrap();
    assert_eq!(row.label, "db  DATABASE");
    assert_eq!(row.detail.as_deref(), Some("database comment"));
}

#[test]
fn explorer_local_status_rows_render_at_supported_sizes() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    for (width, height) in [(80, 24), (120, 36), (180, 50)] {
        let output = render(&app, width, height);
        assert!(output.contains("No profiles"), "{width}x{height}: {output}");
    }
}

#[test]
fn editor_prompt_is_rendered_as_inert_display_text() {
    let mut app = fixture();
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )));
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char(':'),
        KeyModifiers::NONE,
    )));
    app.update(Action::EditorPaste("run\x1b[31m".into()));
    let rendered = render(&app, 120, 30);
    assert!(rendered.contains(":run<ESC>[31m"));
    assert!(!rendered.contains('\x1b'));
}

#[test]
fn execution_confirmation_preview_is_sanitized_and_shows_scope() {
    let profile = import_connection_url("sqlite::memory:", Some("preview-db"))
        .unwrap()
        .profile;
    let mut app = App::with_confirmation_policy(vec![profile.clone()], ConfirmationPolicy::Always);
    app.connection.profile_id = Some(profile.id);
    app.connection.generation = 1;
    app.connection.status = ConnectionStatus::Connected;
    app.update(Action::ReplaceEditor("SELECT 1;\x1b]8;;bad\x07".into()));
    app.update(Action::RunAllSql);

    let output = render(&app, 120, 36);
    assert!(output.contains("EXECUTION CONFIRMATION"));
    assert!(output.contains("FullBuffer"));
    assert!(!output.contains('\x1b'));
}

#[test]
fn completion_popup_is_anchored_below_the_editor_cursor() {
    let mut app = fixture();
    app.focus = Focus::Editor;
    app.update(Action::ReplaceEditor(String::new()));
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char('i'),
        KeyModifiers::NONE,
    )));
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::NONE,
    )));
    app.active_console_mut().completion = Some(CompletionPopup {
        candidates: vec![CompletionCandidate {
            label: "SELECT".into(),
            insert_text: "SELECT".into(),
            kind: CompletionKind::Keyword,
            detail: Some("keyword".into()),
            replace: TextRange::new(0, 1),
            score: CompletionScore {
                context: 4,
                prefix: 1,
                schema: 0,
            },
        }],
        selected: 0,
    });

    let (_, state) = render_with_state(&app, 120, 36);
    let editor = state
        .hit_regions
        .iter()
        .find_map(|region| {
            (region.target == HitTarget::Focus(Focus::Editor)).then_some(region.area)
        })
        .unwrap();
    let popup = state.completion_popup.unwrap();

    assert_eq!(popup.y, editor.y + 2);
    assert!(popup.x < editor.x + editor.width / 2);
    assert!(popup.right() <= editor.right());
    assert!(popup.bottom() <= editor.bottom());
}

#[test]
fn standard_layout_shows_stable_workspace_regions() {
    let output = render(&fixture(), 120, 36);

    assert!(output.contains("LAZYDB"));
    assert!(output.contains("orbital-lab"));
    assert!(output.contains("EXPLORER"));
    assert!(output.contains("console"));
    assert!(output.contains("SELECT"));
    assert!(output.contains("DATA"));
    assert!(output.contains("OUTPUT"));
    assert!(output.contains("Ada"));
    assert!(output.contains("F1 help"));
}

#[test]
fn compact_layout_uses_the_focused_panel() {
    let mut app = fixture();
    app.focus = Focus::Editor;
    let output = render(&app, 80, 24);

    assert!(output.contains("LAZYDB"));
    assert!(output.contains("SQL EDITOR"));
    assert!(output.contains("WHERE"));
    assert!(!output.contains("EXPLORER"));
}

#[test]
fn wide_layout_remains_readable() {
    let mut app = fixture();
    app.update(Action::ToggleResultView);
    let output = render(&app, 180, 50);

    assert!(output.contains("EXPLORER"));
    assert!(output.contains("SQL EDITOR"));
    assert!(output.contains("1 row(s) retrieved in 376 ms"));
}

#[test]
fn help_overlay_is_contextual() {
    let mut app = fixture();
    app.focus = Focus::Explorer;
    app.update(Action::ShowHelp);
    let output = render(&app, 120, 36);

    assert!(output.contains("KEYMAP // EXPLORER"));
    assert!(output.contains("expand / open"));
    assert!(output.contains("Esc"));
}

#[test]
fn tiny_terminal_gets_an_actionable_message() {
    let output = render(&fixture(), 40, 10);

    assert!(output.contains("TERMINAL TOO SMALL"));
    assert!(output.contains("Resize"));
}

#[test]
fn explorer_roots_show_connection_metadata_and_semantic_hit_regions() {
    let mut primary = import_connection_url(
        "postgres://alice@db.example.com:5432/app",
        Some("production"),
    )
    .unwrap()
    .profile;
    primary.environment = Environment::Production;
    primary.read_only = true;
    let replica =
        import_connection_url("mysql://report@mysql.example.com/metrics", Some("reports"))
            .unwrap()
            .profile;
    let primary_id = primary.id;
    let mut app = App::new(vec![primary, replica]);
    app.focus = Focus::Explorer;
    app.connection.profile_id = Some(primary_id);
    app.connection.status = ConnectionStatus::Connected;
    let (output, state) = render_with_state(&app, 120, 36);
    assert!(output.contains("EXPLORER"));
    assert!(output.contains("production"));
    assert!(output.contains("SAVED"));
    assert!(output.contains("production"));
    assert!(output.contains("reports"));
    assert!(state.hit_regions.iter().any(|region| matches!(
        region.target,
        HitTarget::ExplorerRow(ExplorerNodeId::Profile(_))
    )));
}

#[test]
fn server_profile_form_shows_all_fields_and_never_reveals_passwords() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    let manager = app.profile_manager.as_mut().unwrap();
    let draft = manager.draft.as_mut().unwrap();
    draft.name.set("primary");
    draft.user.set("operator");
    draft.database.set("warehouse");
    draft.set_password("super-secret");

    let (output, state) = render_with_state(&app, 120, 36);
    for label in [
        "DRIVER",
        "NAME",
        "HOST",
        "PORT",
        "USER",
        "PASSWORD",
        "DATABASE",
        "VISIBLE OBJECTS",
        "SSL MODE",
        "ENVIRONMENT",
        "READ ONLY",
        "REMEMBER PASSWORD",
        "TEST",
        "SAVE",
        "SAVE & CONNECT",
        "CANCEL",
    ] {
        assert!(output.contains(label), "missing {label}");
    }
    assert!(!output.contains("super-secret"));
    assert!(output.contains("••••••••••••"));
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| { region.target == HitTarget::ProfileField(ProfileField::Password) })
    );
    assert!(state.hit_regions.iter().any(|region| {
        region.target == HitTarget::ProfileButton(ProfileButton::SaveAndConnect)
    }));
}

#[test]
fn stored_password_is_described_without_rendering_a_secret() {
    let mut profile =
        import_connection_url("postgres://alice@db.example.com/app", Some("remembered"))
            .unwrap()
            .profile;
    profile.secret_ref = Some(keyring_ref(profile.id));
    let mut app = App::new(vec![profile]);
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileStartEdit {
        profile_id: app.profiles[0].id,
    });

    let output = render(&app, 120, 36);
    assert!(output.contains("Stored in system keyring"));
    assert!(!output.contains("keyring:dev.lazydb"));
}

#[test]
fn mysql_and_sqlite_forms_only_show_relevant_fields() {
    let mut mysql = App::new(Vec::new());
    mysql.update(Action::OpenProfileManager);
    mysql.update(Action::ProfileCycle(1));
    let mysql_output = render(&mysql, 120, 36);
    assert!(mysql_output.contains("MYSQL"));
    assert!(mysql_output.contains("HOST"));
    assert!(!mysql_output.contains("MEMORY DATABASE"));

    let mut sqlite_file = App::new(Vec::new());
    sqlite_file.update(Action::OpenProfileManager);
    sqlite_file.update(Action::ProfileCycle(2));
    let sqlite_file_output = render(&sqlite_file, 120, 36);
    assert!(sqlite_file_output.contains("SQLITE"));
    assert!(sqlite_file_output.contains("PATH"));
    assert!(sqlite_file_output.contains("MEMORY DATABASE"));
    assert!(!sqlite_file_output.contains("HOST"));
    assert!(!sqlite_file_output.contains("PASSWORD"));

    sqlite_file.update(Action::ProfileFocusField(ProfileField::SqliteMemory));
    sqlite_file.update(Action::ProfileToggle);
    let sqlite_memory_output = render(&sqlite_file, 120, 36);
    assert!(sqlite_memory_output.contains("MEMORY DATABASE"));
    assert!(!sqlite_memory_output.contains("PATH"));
}

#[test]
fn profile_form_remains_actionable_in_compact_layout() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    let output = render(&app, 80, 24);

    assert!(output.contains("NEW CONNECTION"));
    assert!(output.contains("HOST"));
    assert!(output.contains("PASSWORD"));
    assert!(output.contains("SAVE & CONNECT"));
    assert!(output.contains("Esc cancel"));
}

#[test]
fn profile_manager_renders_confirmation_busy_errors_and_warnings() {
    let profile = import_connection_url(":memory:", Some("throwaway"))
        .unwrap()
        .profile;
    let mut deleting = App::new(vec![profile]);
    deleting.update(Action::OpenProfileManager);
    deleting.update(Action::ProfileRequestDelete {
        profile_id: deleting.profiles[0].id,
    });
    let confirmation = render(&deleting, 100, 30);
    assert!(confirmation.contains("DELETE CONNECTION"));
    assert!(confirmation.contains("throwaway"));
    assert!(confirmation.contains("DELETE PERMANENTLY"));

    let mut busy = App::new(Vec::new());
    busy.update(Action::OpenProfileManager);
    {
        let manager = busy.profile_manager.as_mut().unwrap();
        let draft = manager.draft.as_mut().unwrap();
        draft.name.set("busy");
        draft.database.set("app");
    }
    assert!(!busy.update(Action::ProfileTest).is_empty());
    assert_eq!(
        busy.profile_manager.as_ref().unwrap().operation,
        Some(ProfileOperation::Testing)
    );
    let (busy_output, busy_state) = render_with_state(&busy, 100, 30);
    assert!(busy_output.contains("TESTING CONNECTION"));
    assert!(busy_output.contains("BUSY"));
    assert!(!busy_state.hit_regions.iter().any(|region| {
        matches!(
            region.target,
            HitTarget::ProfileField(_) | HitTarget::ProfileToggle(_) | HitTarget::ProfileButton(_)
        )
    }));

    let mut connecting = App::new(vec![
        import_connection_url(":memory:", Some("connecting"))
            .unwrap()
            .profile,
    ]);
    connecting.update(Action::OpenProfileManager);
    connecting.profile_manager.as_mut().unwrap().operation = Some(ProfileOperation::Connecting);
    let (_, connecting_state) = render_with_state(&connecting, 100, 30);
    assert!(!connecting_state.hit_regions.iter().any(|region| matches!(
        region.target,
        HitTarget::ProfileField(_) | HitTarget::ProfileButton(_)
    )));

    let mut invalid = App::new(Vec::new());
    invalid.update(Action::OpenProfileManager);
    invalid.update(Action::ProfileSave { connect: false });
    let invalid_output = render(&invalid, 100, 30);
    assert!(invalid_output.contains("profile name is required"));

    invalid.profile_manager.as_mut().unwrap().message =
        Some("Native password store is unavailable; the password is session-only".into());
    let warning_output = render(&invalid, 100, 30);
    assert!(warning_output.contains("Native password store is unavailable"));
    assert!(warning_output.contains("session-only"));
}

#[test]
fn tiny_terminal_wins_over_profile_overlay() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    assert_eq!(
        app.profile_manager.as_ref().unwrap().page,
        ProfileManagerPage::Form
    );
    let output = render(&app, 40, 10);
    assert!(output.contains("TERMINAL TOO SMALL"));
    assert!(!output.contains("NEW CONNECTION"));
}

#[test]
fn disconnected_explorer_points_to_the_profile_manager() {
    let output = render(&App::new(Vec::new()), 120, 36);
    assert!(output.contains("No profiles"));
    assert!(output.contains("NEW"));
}

#[test]
fn explorer_root_projection_keeps_ordered_roots_visible() {
    let profiles = (0..7)
        .map(|index| {
            import_connection_url(":memory:", Some(&format!("profile-{index:02}")))
                .unwrap()
                .profile
        })
        .collect();
    let mut app = App::new(profiles);
    app.focus = Focus::Explorer;
    let (output, state) = render_with_state(&app, 120, 36);
    assert!(output.contains("profile-00"));
    assert!(state.hit_regions.iter().any(|region| matches!(
        region.target,
        HitTarget::ExplorerRow(ExplorerNodeId::Profile(_))
    )));
}

#[test]
fn minimum_supported_form_scrolls_to_the_selected_field() {
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileFocusField(ProfileField::RememberPassword));

    let (output, state) = render_with_state(&app, 56, 16);
    assert!(output.contains("REMEMBER PASSWORD"));
    assert!(output.contains("Esc cancel"));
    assert!(state.hit_regions.iter().any(|region| {
        region.target == HitTarget::ProfileToggle(ProfileField::RememberPassword)
    }));
}

#[test]
fn hostile_and_long_form_values_render_safely_at_the_cursor() {
    let mut hostile = App::new(Vec::new());
    hostile.update(Action::OpenProfileManager);
    hostile.update(Action::ProfileFocusField(ProfileField::Name));
    hostile.update(Action::ProfilePaste("\n\u{1b}".into()));
    let hostile_output = render(&hostile, 80, 24);
    assert!(hostile_output.contains("<LF><ESC>"));

    let mut long = App::new(Vec::new());
    long.update(Action::OpenProfileManager);
    long.update(Action::ProfileFocusField(ProfileField::Name));
    long.profile_manager
        .as_mut()
        .unwrap()
        .draft
        .as_mut()
        .unwrap()
        .name
        .set(format!("{}VISIBLE-END", "prefix-".repeat(20)));
    let long_output = render(&long, 80, 24);
    assert!(long_output.contains("VISIBLE-END"));
}

#[test]
fn profile_modal_hides_the_workspace_cursor_unless_editing_text() {
    let mut list = fixture();
    list.update(lazydb::action::Action::EditorKey(
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        ),
    ));
    list.update(Action::OpenProfileManager);
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = UiState::new(true);
    terminal
        .draw(|frame| ui::render_with_state(frame, &list, &mut state))
        .unwrap();
    assert!(!terminal.backend().cursor_visible());

    list.update(Action::ProfileStartEdit {
        profile_id: list.profiles[0].id,
    });
    list.update(Action::ProfileFocusField(ProfileField::Name));
    terminal
        .draw(|frame| ui::render_with_state(frame, &list, &mut state))
        .unwrap();
    assert!(terminal.backend().cursor_visible());
}

#[test]
fn empty_profile_list_only_exposes_actionable_buttons() {
    let app = App::new(Vec::new());
    let (_, state) = render_with_state(&app, 120, 36);
    assert!(state.hit_regions.iter().any(|region| matches!(
        region.target,
        HitTarget::ExplorerRow(ExplorerNodeId::EmptyProfiles)
    )));
}

#[test]
fn editor_snapshot_projects_hostile_controls_to_inert_display_text() {
    let mut app = App::new(Vec::new());
    app.update(Action::ReplaceEditor(
        "safe\u{1b}]52;c;secret\u{7}\u{1b}[2J\u{00}\tend".into(),
    ));
    let snapshot = app
        .active_editor_render_snapshot(lazydb::model::editor::EditorViewport {
            width: 80,
            height: 4,
        })
        .unwrap();
    let display = snapshot.lines[0].spans[0].text.as_str();
    assert!(!display.contains('\u{1b}'));
    assert!(display.contains("<ESC>"));
    assert!(display.contains("<0x07>"));
    assert!(display.contains("<0x00>"));
    assert!(display.ends_with("end"));
    let output = render(&app, 80, 24);
    assert!(!output.contains('\u{1b}'));
    assert!(!output.contains('\u{7}'));
    assert!(output.contains("<ESC>"));
}

#[test]
fn editor_snapshot_maps_cjk_emoji_and_tabs_to_display_cells() {
    let mut app = App::new(Vec::new());
    app.update(Action::ReplaceEditor("数据🙂\tX".into()));
    let snapshot = app
        .active_editor_render_snapshot(lazydb::model::editor::EditorViewport {
            width: 40,
            height: 4,
        })
        .unwrap();
    assert_eq!(
        snapshot.lines[0].source_to_display_cells,
        vec![0, 2, 4, 6, 8, 9]
    );
    assert_eq!(snapshot.cursor_screen_cell, Some((0, 0)));

    for _ in 0..3 {
        app.update(Action::EditorKey(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('l'),
            crossterm::event::KeyModifiers::NONE,
        )));
    }
    let snapshot = app
        .active_editor_render_snapshot(lazydb::model::editor::EditorViewport {
            width: 40,
            height: 4,
        })
        .unwrap();
    assert_eq!(snapshot.cursor_screen_cell, Some((6, 0)));
}

#[test]
fn editor_snapshot_scrolls_without_projecting_offscreen_lines() {
    let mut app = App::new(Vec::new());
    let text = (0..10_000)
        .map(|line| format!("line-{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.update(Action::ReplaceEditor(text));
    app.update(Action::EditorScroll {
        rows: 5_000,
        columns: 0,
    });
    let snapshot = app
        .active_editor_render_snapshot(lazydb::model::editor::EditorViewport {
            width: 40,
            height: 3,
        })
        .unwrap();
    assert_eq!(snapshot.first_line, 5_000);
    assert_eq!(snapshot.lines.len(), 5);
    assert_eq!(snapshot.lines[0].line, 5_000);
    assert!(!snapshot.lines[0].spans[0].text.contains("line-4999"));
}
