use std::time::Duration;

use lazydb::{
    action::Action,
    app::App,
    db::{
        ServerInfo,
        query::{ColumnMeta, QueryOutcome, QueryStats, ResultSet},
        value::CellValue,
    },
    model::{
        profile_manager::{ProfileField, ProfileManagerPage, ProfileOperation},
        workspace::{ConnectionStatus, Focus},
    },
    persistence::secrets::keyring_ref,
    profile::{DatabaseKind, Environment, import_connection_url},
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
fn profile_list_shows_connection_metadata_and_semantic_hit_regions() {
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
    app.connection.profile_id = Some(primary_id);
    app.connection.status = ConnectionStatus::Connected;
    app.update(Action::OpenProfileManager);

    let (output, state) = render_with_state(&app, 120, 36);
    assert!(output.contains("CONNECTIONS"));
    assert!(output.contains("production"));
    assert!(output.contains("POSTGRES"));
    assert!(output.contains("db.example.com:5432/app"));
    assert!(output.contains("PRODUCTION"));
    assert!(output.contains("READ ONLY"));
    assert!(output.contains("ACTIVE"));
    assert!(output.contains("reports"));
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| { region.target == HitTarget::ProfileRow(0) })
    );
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| { region.target == HitTarget::ProfileButton(ProfileButton::New) })
    );
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
        "SCHEMA",
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
    app.update(Action::ProfileStartEdit);

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
    deleting.update(Action::ProfileRequestDelete);
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
    assert!(
        !connecting_state
            .hit_regions
            .iter()
            .any(|region| matches!(region.target, HitTarget::ProfileRow(_)))
    );

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
    assert!(output.contains("Press Space c"));
    assert!(output.contains("create"));
    assert!(output.contains("profile"));
}

#[test]
fn narrow_profile_list_scrolls_to_keep_the_selection_visible() {
    let profiles = (0..7)
        .map(|index| {
            import_connection_url(":memory:", Some(&format!("profile-{index:02}")))
                .unwrap()
                .profile
        })
        .collect();
    let mut app = App::new(profiles);
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileMove(isize::MAX));

    let (output, state) = render_with_state(&app, 56, 16);
    assert!(output.contains("profile-06"));
    assert!(!output.contains("profile-00"));
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| { region.target == HitTarget::ProfileRow(6) })
    );
}

#[test]
fn minimum_supported_form_scrolls_to_the_selected_field() {
    let mut app = App::new(Vec::new());
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
    list.active_console_mut().editor.mode = lazydb::model::editor::EditorMode::Insert;
    list.update(Action::OpenProfileManager);
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut state = UiState::new(true);
    terminal
        .draw(|frame| ui::render_with_state(frame, &list, &mut state))
        .unwrap();
    assert!(!terminal.backend().cursor_visible());

    list.update(Action::ProfileStartEdit);
    list.update(Action::ProfileFocusField(ProfileField::Name));
    terminal
        .draw(|frame| ui::render_with_state(frame, &list, &mut state))
        .unwrap();
    assert!(terminal.backend().cursor_visible());
}

#[test]
fn empty_profile_list_only_exposes_actionable_buttons() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    let manager = app.profile_manager.as_mut().unwrap();
    manager.page = ProfileManagerPage::List;
    manager.draft = None;

    let (_, state) = render_with_state(&app, 80, 24);
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| { region.target == HitTarget::ProfileButton(ProfileButton::New) })
    );
    assert!(
        state
            .hit_regions
            .iter()
            .any(|region| { region.target == HitTarget::ProfileButton(ProfileButton::Close) })
    );
    assert!(!state.hit_regions.iter().any(|region| {
        matches!(
            region.target,
            HitTarget::ProfileButton(
                ProfileButton::Edit | ProfileButton::Connect | ProfileButton::Delete
            )
        )
    }));
}
