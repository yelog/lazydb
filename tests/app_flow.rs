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

#[tokio::test]
async fn connects_loads_catalog_and_executes_through_runtime() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("flow.db");
    let imported =
        import_connection_url(&format!("sqlite://{}", path.display()), Some("flow")).unwrap();
    let profile = imported.profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile.clone()]);
    let tab_id = app.active_console().id;
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

    drain_catalog(&mut app, &mut runtime, &mut receiver).await;

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

    let original_sql = app.active_editor_text().unwrap();
    let outcome = app.active_console().outcome.as_ref().unwrap();
    assert_eq!(outcome.stats.row_count, 1);
    assert_eq!(
        outcome.result_sets.last().unwrap().rows[0][1]
            .preview(20)
            .text,
        "Ada"
    );

    dispatch(&mut app, &mut runtime, Action::RefreshCatalog);
    drain_catalog(&mut app, &mut runtime, &mut receiver).await;
    assert!(app.explorer.nodes.iter().any(|node| node.name == "users"));

    let users = app.explorer.normalized.profiles[&profile_id]
        .catalog
        .entries()
        .iter()
        .find(|(_, entry)| entry.qualified_name.object == "users")
        .map(|(id, _)| id.clone())
        .unwrap();
    app.explorer.normalized.selected =
        Some(lazydb::model::explorer::ExplorerNodeId::Catalog(users));
    dispatch(&mut app, &mut runtime, Action::PreviewSelected);
    let action = timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    dispatch(&mut app, &mut runtime, action);
    assert!(matches!(
        app.tabs[app.active_tab],
        lazydb::model::tab::WorkspaceTab::Relation(lazydb::model::relation::RelationTab {
            data: lazydb::model::relation::RelationLoad::Ready(_),
            ..
        })
    ));

    dispatch(
        &mut app,
        &mut runtime,
        Action::SetRelationView(lazydb::model::relation::RelationView::Structure),
    );
    let action = timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    dispatch(&mut app, &mut runtime, action);
    assert!(matches!(
        app.tabs[app.active_tab],
        lazydb::model::tab::WorkspaceTab::Relation(lazydb::model::relation::RelationTab {
            structure: lazydb::model::relation::RelationLoad::Ready(_),
            ..
        })
    ));
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.editor_text(tab_id).unwrap(), original_sql);

    app.update(Action::NewConsole);
    assert_eq!(app.tabs.len(), 3);
    runtime.shutdown().await;
}

fn dispatch(app: &mut App, runtime: &mut Runtime, action: Action) {
    for command in app.update(action) {
        runtime.dispatch(command);
    }
}

async fn drain_catalog(
    app: &mut App,
    runtime: &mut Runtime,
    receiver: &mut mpsc::UnboundedReceiver<Action>,
) {
    loop {
        let Ok(Some(action)) = timeout(Duration::from_millis(100), receiver.recv()).await else {
            break;
        };
        assert!(matches!(
            action,
            Action::CatalogPageLoaded(_) | Action::CatalogPageFailed { .. }
        ));
        dispatch(app, runtime, action);
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
