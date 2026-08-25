use std::{collections::HashMap, time::Duration};

use lazydb::{
    action::Action, app::App, model::workspace::ConnectionStatus, profile::import_connection_url,
    runtime::Runtime,
};
use tempfile::TempDir;
use tokio::{sync::mpsc, time::timeout};

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
    let mut runtime = Runtime::new(vec![profile], HashMap::new(), events);

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
    dispatch(&mut app, &mut runtime, Action::RunActiveSql);
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
    assert!(app.active_console().editor.text().contains("LIMIT 500"));
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
        app.active_console()
            .editor
            .text()
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
