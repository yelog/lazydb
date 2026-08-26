use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use lazydb::{
    action::{Action, Command},
    app::App,
    persistence::{profiles::ProfileStore, secrets::NativeSecretStore},
    runtime::Runtime,
};
use tempfile::TempDir;
use tokio::{sync::mpsc, time::timeout};

fn dispatch(app: &mut App, runtime: &mut Runtime, action: Action) -> Vec<Command> {
    let commands = app.update(action);
    for command in commands.iter().cloned() {
        runtime.dispatch(command);
    }
    commands
}

async fn next_action(receiver: &mut mpsc::UnboundedReceiver<Action>) -> Action {
    timeout(Duration::from_secs(3), receiver.recv())
        .await
        .expect("runtime action timed out")
        .expect("runtime action channel closed")
}

async fn apply_next(
    app: &mut App,
    runtime: &mut Runtime,
    receiver: &mut mpsc::UnboundedReceiver<Action>,
) -> Action {
    let action = next_action(receiver).await;
    dispatch(app, runtime, action.clone());
    action
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

fn set_sqlite_draft(app: &mut App, name: &str, path: &std::path::Path) {
    app.update(Action::ProfileCycle(2));
    let manager = app.profile_manager.as_mut().unwrap();
    let draft = manager.draft.as_mut().unwrap();
    draft.name.set(name);
    draft.sqlite_path.set(path.to_string_lossy());
    draft.sqlite_memory = false;
}

async fn test_profile(
    app: &mut App,
    runtime: &mut Runtime,
    receiver: &mut mpsc::UnboundedReceiver<Action>,
) {
    let commands = dispatch(app, runtime, Action::ProfileTest);
    assert!(matches!(commands.as_slice(), [Command::TestProfile { .. }]));
    assert!(matches!(
        apply_next(app, runtime, receiver).await,
        Action::ProfileTestSucceeded { .. }
    ));
}

async fn save_and_connect(
    app: &mut App,
    runtime: &mut Runtime,
    receiver: &mut mpsc::UnboundedReceiver<Action>,
) {
    let commands = dispatch(app, runtime, Action::ProfileSave { connect: true });
    assert!(matches!(
        commands.as_slice(),
        [Command::SaveProfile { connect: true, .. }]
    ));
    let saved = apply_next(app, runtime, receiver).await;
    assert!(matches!(saved, Action::ProfileSaved { connect: true, .. }));
    let connected = apply_next(app, runtime, receiver).await;
    assert!(matches!(connected, Action::ConnectionSucceeded { .. }));
    drain_catalog(app, runtime, receiver).await;
}

async fn query(
    app: &mut App,
    runtime: &mut Runtime,
    receiver: &mut mpsc::UnboundedReceiver<Action>,
    sql: &str,
) {
    dispatch(app, runtime, Action::ReplaceEditor(sql.to_owned()));
    let commands = dispatch(app, runtime, Action::RunAllSql);
    assert!(commands.is_empty());
    dispatch(app, runtime, Action::ToggleExecutionConfirmationFocus);
    let commands = dispatch(app, runtime, Action::ConfirmExecution);
    assert!(matches!(commands.as_slice(), [Command::RunQuery { .. }]));
    let action = apply_next(app, runtime, receiver).await;
    assert!(matches!(action, Action::QueryFinished { .. }));
    assert!(app.active_console().outcome.is_some());
}

#[tokio::test]
async fn two_sqlite_profiles_complete_the_full_runtime_lifecycle() {
    let temp = TempDir::new().unwrap();
    let path_a = temp.path().join("alpha.db");
    let path_b = temp.path().join("beta.db");
    let store_path = temp.path().join("connections.toml");
    let (events, mut receiver) = mpsc::unbounded_channel();
    let mut runtime = Runtime::new(
        Vec::new(),
        HashSet::new(),
        HashMap::new(),
        None,
        ProfileStore::new(store_path.clone()),
        Arc::new(NativeSecretStore),
        events,
    );
    let mut app = App::new(Vec::new());

    dispatch(&mut app, &mut runtime, Action::OpenProfileManager);
    set_sqlite_draft(&mut app, "alpha", &path_a);
    test_profile(&mut app, &mut runtime, &mut receiver).await;
    assert!(
        ProfileStore::new(store_path.clone())
            .load()
            .unwrap()
            .is_empty()
    );
    save_and_connect(&mut app, &mut runtime, &mut receiver).await;
    let alpha_id = app.connection.profile_id.unwrap();
    query(
        &mut app,
        &mut runtime,
        &mut receiver,
        "CREATE TABLE marker (value TEXT); INSERT INTO marker VALUES ('alpha');",
    )
    .await;

    dispatch(&mut app, &mut runtime, Action::ProfileStartNew);
    set_sqlite_draft(&mut app, "beta", &path_b);
    save_and_connect(&mut app, &mut runtime, &mut receiver).await;
    let beta_id = app.connection.profile_id.unwrap();
    assert_ne!(alpha_id, beta_id);
    query(
        &mut app,
        &mut runtime,
        &mut receiver,
        "CREATE TABLE marker (value TEXT); INSERT INTO marker VALUES ('beta');",
    )
    .await;

    dispatch(
        &mut app,
        &mut runtime,
        Action::RequestProfileConnect {
            profile_id: alpha_id,
        },
    );
    assert!(matches!(
        apply_next(&mut app, &mut runtime, &mut receiver).await,
        Action::ConnectionSucceeded { .. }
    ));
    drain_catalog(&mut app, &mut runtime, &mut receiver).await;
    assert_eq!(app.connection.profile_id, Some(alpha_id));
    query(
        &mut app,
        &mut runtime,
        &mut receiver,
        "SELECT value FROM marker;",
    )
    .await;
    assert_eq!(
        app.active_console()
            .outcome
            .as_ref()
            .unwrap()
            .result_sets
            .last()
            .unwrap()
            .rows[0][0]
            .preview(20)
            .text,
        "alpha"
    );

    dispatch(
        &mut app,
        &mut runtime,
        Action::ProfileStartEdit {
            profile_id: alpha_id,
        },
    );
    app.profile_manager
        .as_mut()
        .unwrap()
        .draft
        .as_mut()
        .unwrap()
        .name
        .set("alpha-renamed");
    let old_order = app
        .profiles
        .iter()
        .map(|profile| profile.id)
        .collect::<Vec<_>>();
    dispatch(
        &mut app,
        &mut runtime,
        Action::ProfileSave { connect: false },
    );
    assert!(matches!(
        apply_next(&mut app, &mut runtime, &mut receiver).await,
        Action::ProfileSaved { connect: false, .. }
    ));
    assert_eq!(app.connection.profile_id, Some(alpha_id));
    assert_eq!(
        app.connection.status,
        lazydb::model::workspace::ConnectionStatus::Connected
    );
    assert_eq!(app.profiles[0].id, old_order[0]);
    assert_eq!(app.profiles[0].name, "alpha-renamed");
    let persisted_profiles = ProfileStore::new(store_path.clone()).load().unwrap();
    assert_eq!(
        persisted_profiles
            .iter()
            .map(|profile| profile.id)
            .collect::<Vec<_>>(),
        old_order
    );
    assert_eq!(persisted_profiles[0].name, "alpha-renamed");

    let reloaded_ids = persisted_profiles
        .iter()
        .map(|profile| profile.id)
        .collect();
    let (reload_events, _reload_receiver) = mpsc::unbounded_channel();
    let reloaded_runtime = Runtime::new(
        persisted_profiles.clone(),
        reloaded_ids,
        HashMap::new(),
        None,
        ProfileStore::new(store_path.clone()),
        Arc::new(NativeSecretStore),
        reload_events,
    );
    reloaded_runtime.shutdown().await;

    dispatch(
        &mut app,
        &mut runtime,
        Action::RequestProfileConnect {
            profile_id: alpha_id,
        },
    );
    assert!(matches!(
        apply_next(&mut app, &mut runtime, &mut receiver).await,
        Action::ConnectionSucceeded { .. }
    ));
    drain_catalog(&mut app, &mut runtime, &mut receiver).await;

    dispatch(
        &mut app,
        &mut runtime,
        Action::ProfileRequestDelete {
            profile_id: beta_id,
        },
    );
    dispatch(&mut app, &mut runtime, Action::ProfileConfirmDelete);
    let deleted = apply_next(&mut app, &mut runtime, &mut receiver).await;
    assert!(matches!(
        deleted,
        Action::ProfileDeleted {
            profile_id,
            active_connection: None,
            ..
        } if profile_id == beta_id
    ));
    assert_eq!(app.profiles.len(), 1);
    assert_eq!(
        ProfileStore::new(store_path.clone()).load().unwrap().len(),
        1
    );

    dispatch(
        &mut app,
        &mut runtime,
        Action::ProfileRequestDelete {
            profile_id: alpha_id,
        },
    );
    dispatch(&mut app, &mut runtime, Action::ProfileConfirmDelete);
    let deleted = apply_next(&mut app, &mut runtime, &mut receiver).await;
    assert!(matches!(
        deleted,
        Action::ProfileDeleted {
            profile_id,
            active_connection: Some(_),
            ..
        } if profile_id == alpha_id
    ));
    assert!(matches!(
        apply_next(&mut app, &mut runtime, &mut receiver).await,
        Action::DisconnectCompleted { .. }
    ));
    assert!(app.profiles.is_empty());
    assert!(app.connection.profile_id.is_none());
    assert!(
        ProfileStore::new(store_path.clone())
            .load()
            .unwrap()
            .is_empty()
    );

    runtime.shutdown().await;
}
