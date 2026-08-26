use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use lazydb::{
    action::Action,
    app::App,
    model::{tab::CompletionPopup, workspace::ConnectionStatus},
    persistence::{profiles::ProfileStore, secrets::NativeSecretStore},
    profile::import_connection_url,
    runtime::Runtime,
};
use tempfile::TempDir;
use tokio::{sync::mpsc, time::timeout};

#[test]
fn typing_dismisses_stale_completion_and_updates_the_editor() {
    let mut app = App::new(Vec::new());
    app.active_console_mut().completion = Some(CompletionPopup::default());

    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::NONE,
    )));

    assert_eq!(app.active_editor_text().unwrap(), "s");
    assert!(app.active_console().completion.is_none());
}

fn editor_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.update(Action::EditorKey(KeyEvent::new(code, modifiers)));
}

#[test]
fn normal_mode_motions_do_not_insert_literal_keys_through_app() {
    let mut app = App::new(Vec::new());
    app.update(Action::ReplaceEditor("one two\nthree".into()));

    for code in [
        KeyCode::Char('h'),
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::Char('l'),
        KeyCode::Char('w'),
        KeyCode::Char('b'),
        KeyCode::Char('e'),
        KeyCode::Char('0'),
        KeyCode::Char('$'),
        KeyCode::Char('G'),
    ] {
        editor_key(&mut app, code, KeyModifiers::NONE);
    }

    assert_eq!(app.active_editor_text().unwrap(), "one two\nthree");
    assert_eq!(
        app.active_editor_mode(),
        lazydb::model::editor::EditorMode::Normal
    );
}

#[test]
fn vim_operator_text_object_visual_and_undo_sequences_use_app_pipeline() {
    let mut app = App::new(Vec::new());
    app.update(Action::ReplaceEditor("one two three".into()));

    for code in [KeyCode::Char('c'), KeyCode::Char('i'), KeyCode::Char('w')] {
        editor_key(&mut app, code, KeyModifiers::NONE);
    }
    for code in [KeyCode::Char('X'), KeyCode::Esc] {
        editor_key(&mut app, code, KeyModifiers::NONE);
    }
    assert_eq!(app.active_editor_text().unwrap(), "X two three");

    editor_key(&mut app, KeyCode::Char('v'), KeyModifiers::NONE);
    editor_key(&mut app, KeyCode::Char('l'), KeyModifiers::NONE);
    editor_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(
        app.active_editor_mode(),
        lazydb::model::editor::EditorMode::Normal
    );

    editor_key(&mut app, KeyCode::Char('u'), KeyModifiers::NONE);
    assert_eq!(app.active_editor_text().unwrap(), "one two three");
    editor_key(&mut app, KeyCode::Char('r'), KeyModifiers::CONTROL);
    assert_eq!(app.active_editor_text().unwrap(), "X two three");
}

#[test]
fn accepting_completion_places_cursor_after_inserted_text() {
    let mut app = App::new(Vec::new());
    for character in ['s', 'e', 'l'] {
        editor_key(&mut app, KeyCode::Char(character), KeyModifiers::NONE);
    }
    app.update(Action::CompletionExplicit);
    assert!(app.active_console().completion.is_some());

    app.update(Action::CompletionAccept);
    assert_eq!(app.active_editor_text().unwrap(), "SELECT");
    let snapshot = app
        .active_editor_render_snapshot(lazydb::model::editor::EditorViewport {
            width: 80,
            height: 5,
        })
        .unwrap();
    assert_eq!(
        snapshot.cursor,
        lazydb::model::editor::EditorPosition { line: 0, column: 6 }
    );

    editor_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    editor_key(&mut app, KeyCode::Char('u'), KeyModifiers::NONE);
    assert_eq!(app.active_editor_text().unwrap(), "sel");
}

#[tokio::test]
async fn connects_loads_catalog_and_executes_through_runtime() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("flow.db");
    let imported =
        import_connection_url(&format!("sqlite://{}", path.display()), Some("flow")).unwrap();
    let profile = imported.profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile.clone()]);
    let (events, mut receiver) = mpsc::unbounded_channel();
    let mut runtime = Runtime::new(
        vec![profile],
        HashSet::from([profile_id]),
        HashMap::new(),
        None,
        ProfileStore::new(temp.path().join("connections.toml")),
        Arc::new(NativeSecretStore),
        events,
    );

    dispatch(&mut app, &mut runtime, Action::RequestConnect(profile_id));
    let action = timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    dispatch(&mut app, &mut runtime, action);
    assert_eq!(app.connection.status, ConnectionStatus::Connected);

    let action = timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    dispatch(&mut app, &mut runtime, action);

    dispatch(
        &mut app,
        &mut runtime,
        Action::ReplaceEditor(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);\n\
             INSERT INTO users VALUES (1, 'Ada');\n\
             SELECT id, name FROM users;"
                .into(),
        ),
    );
    dispatch(&mut app, &mut runtime, Action::RunAllSql);
    assert!(matches!(
        app.overlay,
        Some(lazydb::model::workspace::Overlay::ExecutionConfirm { .. })
    ));
    dispatch(
        &mut app,
        &mut runtime,
        Action::ToggleExecutionConfirmationFocus,
    );
    dispatch(&mut app, &mut runtime, Action::ConfirmExecution);
    let action = timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    dispatch(&mut app, &mut runtime, action);

    let outcome = app.active_console().outcome.as_ref().unwrap();
    assert_eq!(outcome.stats.row_count, 1);
    assert_eq!(
        outcome.result_sets.last().unwrap().rows[0][1]
            .preview(20)
            .text,
        "Ada"
    );

    dispatch(&mut app, &mut runtime, Action::RefreshCatalog);
    let action = timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    dispatch(&mut app, &mut runtime, action);
    assert!(app.explorer.nodes.iter().any(|node| node.name == "users"));

    let users_visible_index = app
        .explorer
        .visible()
        .iter()
        .position(|visible| app.explorer.nodes[visible.node_index].name == "users")
        .unwrap();
    app.explorer.selected = users_visible_index;
    dispatch(&mut app, &mut runtime, Action::PreviewSelected);
    let action = timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    dispatch(&mut app, &mut runtime, action);
    assert_eq!(app.active_console().name, "users data");
    assert!(app.active_editor_text().unwrap().contains("LIMIT 500"));
    assert_eq!(
        app.active_console()
            .outcome
            .as_ref()
            .unwrap()
            .stats
            .row_count,
        1
    );

    dispatch(&mut app, &mut runtime, Action::DdlSelected);
    let action = timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    dispatch(&mut app, &mut runtime, action);
    assert_eq!(app.active_console().name, "users DDL");
    assert!(
        app.active_editor_text()
            .unwrap()
            .contains("CREATE TABLE users")
    );

    app.update(Action::NewConsole);
    assert_eq!(app.tabs.len(), 4);
    runtime.shutdown().await;
}

fn dispatch(app: &mut App, runtime: &mut Runtime, action: Action) {
    for command in app.update(action) {
        runtime.dispatch(command);
    }
}

#[test]
fn ex_quit_is_reduced_by_app_not_called_directly() {
    let mut app = App::new(Vec::new());
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )));
    app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Char(':'),
        KeyModifiers::NONE,
    )));
    app.update(Action::EditorPaste("q".into()));
    let commands = app.update(Action::EditorKey(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert!(app.should_quit);
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, lazydb::action::Command::Quit))
    );
}
