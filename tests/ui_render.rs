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
        workspace::{ConnectionStatus, Focus, Overlay},
    },
    persistence::secrets::keyring_ref,
    profile::{DatabaseKind, Environment, import_connection_url},
    sql::{CompletionCandidate, CompletionKind, CompletionScore, TextRange},
    ui::{
        self, HitTarget, ProfileButton, UiState,
        icons::{IconMode, IconSet},
    },
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

#[test]
fn sql_editor_underlines_only_the_statement_at_the_cursor() {
    let mut app = fixture();
    app.update(Action::ReplaceEditor("SELECT 1;\nSELECT 2;".into()));
    let snapshot = app
        .active_editor_render_snapshot(lazydb::model::editor::EditorViewport {
            width: 120,
            height: 10,
        })
        .unwrap();
    assert!(
        snapshot.lines[0]
            .spans
            .iter()
            .any(|span| span.current_statement)
    );
    assert!(
        snapshot.lines[1]
            .spans
            .iter()
            .all(|span| !span.current_statement)
    );
}

fn render(app: &App, width: u16, height: u16) -> String {
    render_with_state(app, width, height).0
}

fn render_with_state(app: &App, width: u16, height: u16) -> (String, UiState) {
    render_with_icons(app, width, height, IconSet::default())
}

fn render_with_icons(app: &App, width: u16, height: u16, icons: IconSet) -> (String, UiState) {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = UiState::new();
    terminal
        .draw(|frame| ui::render_with_state_using_icons(frame, app, &mut state, icons))
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
fn explorer_uses_selected_icon_mode() {
    let app = fixture();

    let nerd = render_with_icons(&app, 120, 36, IconSet::new(IconMode::NerdFont)).0;
    assert!(nerd.contains(nerd_font_symbols::dev::DEV_SQLITE), "{nerd}");

    let unicode = render_with_icons(&app, 120, 36, IconSet::new(IconMode::Unicode)).0;
    assert!(unicode.contains("SQ "), "{unicode}");

    let ascii = render_with_icons(&app, 120, 36, IconSet::new(IconMode::Ascii)).0;
    assert!(ascii.contains("SQ "), "{ascii}");
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
            options: lazydb::model::relation::RelationPreviewOptions::default(),
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
fn relation_page_renders_contextual_help_overlay() {
    let mut app = App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;
    app.focus = Focus::Results;
    app.overlay = Some(Overlay::Help(Focus::Results));

    let (output, state) = render_with_state(&app, 120, 36);

    assert!(output.contains("KEYMAP // RESULTS"), "{output}");
    assert!(!state.hit_regions.is_empty());
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
    assert_eq!(row.label, "db");
    assert_eq!(row.metadata, None);
    assert_eq!(row.comment.as_deref(), Some("database comment"));
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
fn cursor_style_follows_editor_mode() {
    let mut app = fixture();
    let (_, normal_state) = render_with_state(&app, 120, 36);
    assert_eq!(
        normal_state.cursor_style,
        Some(lazydb::ui::CursorStyle::Block)
    );

    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char('i'),
        KeyModifiers::NONE,
    )));
    let (_, insert_state) = render_with_state(&app, 120, 36);
    assert_eq!(
        insert_state.cursor_style,
        Some(lazydb::ui::CursorStyle::Bar)
    );

    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )));
    let (_, returned_normal_state) = render_with_state(&app, 120, 36);
    assert_eq!(
        returned_normal_state.cursor_style,
        Some(lazydb::ui::CursorStyle::Block)
    );
}

#[test]
fn editor_prompt_uses_bar_cursor() {
    let mut app = fixture();
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char(':'),
        KeyModifiers::NONE,
    )));

    let (_, state) = render_with_state(&app, 120, 36);
    assert_eq!(state.cursor_style, Some(lazydb::ui::CursorStyle::Bar));
}

#[test]
fn replace_mode_uses_underline_cursor() {
    let mut app = fixture();
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char('R'),
        KeyModifiers::NONE,
    )));

    let (_, state) = render_with_state(&app, 120, 36);
    assert_eq!(state.cursor_style, Some(lazydb::ui::CursorStyle::Underline));
}

#[test]
fn footer_and_header_show_transaction_state_and_controls() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = lazydb::model::transaction::TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    let (output, _) = render_with_state(&app, 120, 36);
    assert!(output.contains("TX MANUAL:ACTIVE"));
}

#[test]
fn editor_title_owns_target_and_transaction_context() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = lazydb::model::transaction::TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    let (output, _) = render_with_state(&app, 120, 36);
    assert!(output.contains("orbital-lab"));
    assert!(output.contains("TX MANUAL:ACTIVE"));
    assert!(!output.contains("TX AUTO"));
    assert!(!output.lines().take(2).any(|line| line.contains("TX ")));
}

#[test]
fn editor_help_documents_target_context_controls() {
    let mut app = fixture();
    app.update(Action::ShowHelp);
    let (output, _) = render_with_state(&app, 80, 24);
    for text in [
        "Space d",
        ":connection",
        ":database",
        ":schema",
        "Space tt",
        "Space tc",
        "Space tr",
    ] {
        assert!(output.contains(text), "missing {text}");
    }
}

#[test]
fn editor_context_keeps_transaction_visible_when_narrow() {
    let mut app = fixture();
    app.active_console_mut().transaction_mode = lazydb::model::transaction::TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    for width in [120, 80, 56] {
        let output = render(&app, width, 24);
        assert!(output.contains("TX MANUAL:ACTIVE"), "width={width}");
    }
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
fn target_selector_renders_real_target_and_navigation_hint() {
    let mut app = fixture();
    app.update(Action::OpenTargetSelector);
    let output = render(&app, 120, 36);

    assert!(output.contains("EXECUTION TARGET"));
    assert!(output.contains(":memory:.main"));
    assert!(output.contains("current"));
    assert!(output.contains("Enter confirm"));
    assert!(!output.contains("Target selector is available"));
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
    assert!(output.contains("toggle expand / collapse"));
    assert!(output.contains("open table preview / activate"));
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
    app.explorer
        .normalized
        .profiles
        .get_mut(&primary_id)
        .unwrap()
        .status = lazydb::model::explorer::ExplorerConnectionStatus::Online;
    let (output, state) = render_with_state(&app, 120, 36);
    assert!(output.contains("EXPLORER"));
    assert!(output.contains("production"));
    assert!(!output.contains("SAVED"));
    assert!(output.contains("●"));
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
        "URL FORMAT",
        "URL",
        "NAME",
        "HOST",
        "PORT",
        "USER",
        "PASSWORD",
        "DATABASE",
        "DEFAULT SCHEMA",
        "VISIBLE OBJECTS",
        "SSL MODE",
        "ENVIRONMENT",
        "READ ONLY",
        "PASSWORD STORAGE",
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
fn visible_objects_scope_shows_discovery_loading_and_refresh_hint() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    {
        let draft = app
            .profile_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap();
        draft.name.set("primary");
        draft.database.set("warehouse");
    }
    assert!(matches!(
        app.update(Action::ProfileOpenScope).as_slice(),
        [lazydb::action::Command::DiscoverProfileCatalog { .. }]
    ));

    let output = render(&app, 120, 36);
    assert!(output.contains("Discovering databases and schemas..."));
    assert!(output.contains("r refresh"));
    assert!(output.contains("warehouse"));
}

#[test]
fn visible_objects_scope_renders_partial_database_without_all_schemas_row() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    {
        let draft = app
            .profile_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap();
        draft.name.set("primary");
        draft.database.set("warehouse");
    }
    let (request_id, fingerprint) = match app.update(Action::ProfileOpenScope).as_slice() {
        [
            lazydb::action::Command::DiscoverProfileCatalog {
                request_id,
                submission,
            },
        ] => (*request_id, submission.discovery_fingerprint),
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ProfileCatalogDiscoverySucceeded {
        request_id,
        fingerprint,
        server: ServerInfo {
            kind: DatabaseKind::Postgres,
            version: "16".into(),
            database: "warehouse".into(),
        },
        capabilities: lazydb::db::catalog::CatalogCapabilities {
            namespace_model: lazydb::db::catalog::NamespaceModel::DatabaseAndSchema,
            top_level_groups: vec![],
            column_metadata: Default::default(),
            supports_lazy_children: false,
        },
        discovery: lazydb::db::catalog::CatalogDiscovery {
            databases: vec![lazydb::db::catalog::DiscoveredDatabase {
                name: "warehouse".into(),
                schemas: vec!["analytics".into(), "public".into()],
            }],
            warnings: Vec::new(),
        },
    });

    let output = render(&app, 120, 36);
    assert!(output.contains("[-] warehouse"), "{output}");
    assert!(output.contains("[x] public"), "{output}");
    assert!(output.contains("[ ] analytics"), "{output}");
    assert!(!output.contains("All schemas"), "{output}");
}

#[test]
fn pending_url_redacts_an_embedded_password_before_commit() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileFocusField(ProfileField::Url));
    let draft = app
        .profile_manager
        .as_mut()
        .unwrap()
        .draft
        .as_mut()
        .unwrap();
    draft.move_home(ProfileField::Url);
    draft.paste(
        ProfileField::Url,
        "postgresql://alice:never-render-this@db.example/app?sslmode=require",
    );

    let output = render(&app, 120, 36);
    assert!(!output.contains("never-render-this"));
    assert!(output.contains("[REDACTED]"));
}

#[test]
fn stored_password_is_described_without_rendering_a_secret() {
    let mut profile =
        import_connection_url("postgres://alice@db.example.com/app", Some("remembered"))
            .unwrap()
            .profile;
    profile.credential_policy = lazydb::profile::CredentialPolicy::Keyring(keyring_ref(profile.id));
    let mut app = App::new(vec![profile]);
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileStartEdit {
        profile_id: app.profiles[0].id,
    });

    let output = render(&app, 120, 36);
    assert!(output.contains("Stored in system credential store"));
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
    assert!(!mysql_output.contains("DEFAULT SCHEMA"));
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
    assert!(output.contains("POSTGRES MYSQL SQLITE"), "{output}");
    assert!(output.contains("HOST"));
    assert!(output.contains("PASSWORD"));
    assert!(output.contains("SAVE & CONNECT"));
    assert!(output.contains("Esc cancel"));
}

#[test]
fn driver_options_have_individual_targets_and_selected_style_survives_field_blur() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileFocusField(ProfileField::Name));

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = UiState::new();
    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();

    let options = [
        DatabaseKind::Postgres,
        DatabaseKind::MySql,
        DatabaseKind::Sqlite,
    ]
    .map(|kind| {
        state
            .hit_regions
            .iter()
            .find(|region| region.target == HitTarget::ProfileDriver(kind))
            .unwrap()
    });
    assert!(
        options
            .windows(2)
            .all(|pair| pair[0].area.right() < pair[1].area.x)
    );
    let selected = options[0].area;
    let unselected = options[1].area;
    let buffer = terminal.backend().buffer();
    assert_eq!(
        buffer[(selected.x, selected.y)].bg,
        ui::theme::Theme::default().accent
    );
    assert_ne!(
        buffer[(unselected.x, unselected.y)].bg,
        ui::theme::Theme::default().accent
    );
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
            HitTarget::ProfileField(_)
                | HitTarget::ProfileDriver(_)
                | HitTarget::ProfileToggle(_)
                | HitTarget::ProfileButton(_)
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
    app.update(Action::ProfileFocusField(ProfileField::PasswordStorage));

    let (output, state) = render_with_state(&app, 56, 16);
    assert!(output.contains("PASSWORD STORAGE"));
    assert!(output.contains("Esc cancel"));
    assert!(
        state.hit_regions.iter().any(|region| {
            region.target == HitTarget::ProfileField(ProfileField::PasswordStorage)
        })
    );
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
    let mut state = UiState::new();
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
