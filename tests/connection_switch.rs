use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use lazydb::{
    action::{Action, Command},
    app::App,
    db::{
        DatabaseConnection, ServerInfo,
        catalog::{
            CatalogEntry, CatalogId, CatalogKind, CatalogRequest, CatalogRequestKey, CatalogTarget,
            OptionalMetadata, QualifiedName,
        },
    },
    model::{
        execution_target::ExecutionTarget,
        explorer::{ExplorerConnectionStatus, ExplorerNodeId},
        relation::{RelationKey, RelationRequest, RelationRequestKind, RelationTab},
        relation_edit::RelationEditSession,
        tab::WorkspaceTab,
        transaction::{TransactionMode, TransactionState},
        workspace::{ConnectionIdentity, ConnectionStatus, QueryStatus},
    },
    persistence::{
        profiles::ProfileStore,
        secrets::{SecretStore, SecretStoreError, keyring_ref},
    },
    profile::{
        CatalogSelection, ConnectionProfile, CredentialPolicy, DatabaseKind, import_connection_url,
    },
    runtime::Runtime,
};
use secrecy::SecretString;
use tempfile::TempDir;
use tokio::{sync::mpsc, time::timeout};
use uuid::Uuid;

#[derive(Default)]
struct MissingSecretStore {
    get_ids: Mutex<Vec<Uuid>>,
}

impl MissingSecretStore {
    fn get_ids(&self) -> Vec<Uuid> {
        self.get_ids.lock().unwrap().clone()
    }
}

#[async_trait]
impl SecretStore for MissingSecretStore {
    async fn available(&self) -> Result<(), SecretStoreError> {
        Ok(())
    }

    async fn get(&self, profile_id: Uuid) -> Result<Option<SecretString>, SecretStoreError> {
        self.get_ids.lock().unwrap().push(profile_id);
        Ok(None)
    }

    async fn set(
        &self,
        _profile_id: Uuid,
        _password: &SecretString,
    ) -> Result<(), SecretStoreError> {
        Ok(())
    }

    async fn delete(&self, _profile_id: Uuid) -> Result<(), SecretStoreError> {
        Ok(())
    }
}

fn server(database: &str) -> ServerInfo {
    ServerInfo {
        kind: DatabaseKind::Sqlite,
        version: "3.50".into(),
        database: database.into(),
        current_user: None,
    }
}

fn memory_profile(name: &str) -> ConnectionProfile {
    import_connection_url(":memory:", Some(name))
        .unwrap()
        .profile
}

async fn file_profile(path: &std::path::Path, name: &str, sentinel: &str) -> ConnectionProfile {
    let profile = import_connection_url(&format!("sqlite://{}", path.display()), Some(name))
        .unwrap()
        .profile;
    let database = DatabaseConnection::connect(&profile, None).await.unwrap();
    database
        .execute(&format!(
            "CREATE TABLE marker (value TEXT); INSERT INTO marker VALUES ('{sentinel}');"
        ))
        .await
        .unwrap();
    database.close().await;
    profile
}

fn runtime(
    temp: &TempDir,
    profiles: Vec<ConnectionProfile>,
    secret_store: Arc<dyn SecretStore>,
    startup_password: Option<(Uuid, SecretString)>,
) -> (Runtime, mpsc::UnboundedReceiver<Action>) {
    let persisted = profiles.iter().map(|profile| profile.id).collect();
    let (sender, receiver) = mpsc::unbounded_channel();
    (
        Runtime::new(
            profiles,
            persisted,
            HashMap::new(),
            startup_password,
            ProfileStore::new(temp.path().join("connections.toml")),
            secret_store,
            sender,
        ),
        receiver,
    )
}

fn dispatch(app: &mut App, runtime: &mut Runtime, action: Action) -> Vec<Command> {
    let commands = app.update(action);
    for command in commands.iter().cloned() {
        runtime.dispatch(command);
    }
    commands
}

#[test]
fn catalog_mutation_targets_are_isolated_from_console_targets() {
    let profile_id = Uuid::new_v4();
    let console_target = ExecutionTarget {
        profile_id,
        database: "app".into(),
        schema: Some("public".into()),
    };
    let maintenance =
        lazydb::db::catalog_mutation::CatalogMutationTarget::maintenance("postgres").unwrap();
    assert_eq!(maintenance.database(), "postgres");
    assert_eq!(
        maintenance.execution_target(profile_id),
        ExecutionTarget {
            profile_id,
            database: "postgres".into(),
            schema: None,
        }
    );
    assert_eq!(console_target.database, "app");
    assert_eq!(console_target.schema.as_deref(), Some("public"));
}

async fn next_action(receiver: &mut mpsc::UnboundedReceiver<Action>) -> Action {
    timeout(Duration::from_secs(3), receiver.recv())
        .await
        .expect("runtime action timed out")
        .expect("runtime action channel closed")
}

async fn next_connection_result(
    app: &mut App,
    runtime: &mut Runtime,
    receiver: &mut mpsc::UnboundedReceiver<Action>,
    profile_id: Uuid,
) -> Action {
    loop {
        let action = next_action(receiver).await;
        match &action {
            Action::ConnectionSucceeded {
                profile_id: result_id,
                ..
            }
            | Action::ConnectionFailed {
                profile_id: result_id,
                ..
            } if *result_id == profile_id => return action,
            Action::CatalogPageLoaded(_) | Action::CatalogPageFailed { .. } => {
                dispatch(app, runtime, action);
            }
            action => panic!("unexpected action while waiting for connection result: {action:?}"),
        }
    }
}

async fn connect(
    app: &mut App,
    runtime: &mut Runtime,
    receiver: &mut mpsc::UnboundedReceiver<Action>,
    profile_id: Uuid,
) -> ConnectionIdentity {
    dispatch(app, runtime, Action::RequestConnect(profile_id));
    let connected = next_action(receiver).await;
    assert!(matches!(
        connected,
        Action::ConnectionSucceeded {
            profile_id: connected_id,
            ..
        } if connected_id == profile_id
    ));
    dispatch(app, runtime, connected);
    drain_catalog(app, runtime, receiver).await;
    app.connection.active_identity().unwrap()
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
            Action::CatalogPageLoaded(_)
                | Action::CatalogPageFailed { .. }
                | Action::DisconnectCompleted { .. }
        ));
        dispatch(app, runtime, action);
    }
}

async fn run_marker_query(
    app: &mut App,
    runtime: &mut Runtime,
    receiver: &mut mpsc::UnboundedReceiver<Action>,
) -> String {
    dispatch(
        app,
        runtime,
        Action::ReplaceEditor("SELECT value FROM marker".into()),
    );
    let commands = dispatch(app, runtime, Action::RunActiveSql);
    assert!(matches!(
        commands.as_slice(),
        [Command::RunQueryPage { .. }]
    ));
    let result = next_action(receiver).await;
    assert!(matches!(result, Action::QueryPageFinished { .. }));
    dispatch(app, runtime, result);
    app.active_console()
        .outcome
        .as_ref()
        .unwrap()
        .result_sets
        .last()
        .unwrap()
        .rows[0][0]
        .preview(40)
        .text
        .clone()
}

#[test]
fn pending_switch_keeps_active_identity_and_allows_its_existing_console() {
    let first = memory_profile("first");
    let second = memory_profile("second");
    let first_id = first.id;
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);

    let first_generation = app
        .sessions
        .start_attempt(ExecutionTarget::from_profile(
            app.profiles
                .iter()
                .find(|profile| profile.id == first_id)
                .unwrap(),
        ))
        .unwrap()
        .generation;
    app.update(Action::ConnectionSucceeded {
        profile_id: first_id,
        generation: first_generation,
        server: server("first"),
        mutation_capabilities: Default::default(),
    });
    let active_server = app.connection.server.clone();

    let second_generation = match app.update(Action::RequestConnect(second_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    assert_eq!(app.connection.profile_id, Some(first_id));
    assert_eq!(app.connection.generation, first_generation);
    assert_eq!(app.connection.server, active_server);
    assert_eq!(app.connection.pending_profile_id, Some(second_id));
    assert_eq!(app.connection.pending_generation, Some(second_generation));
    assert_eq!(app.connection.status, ConnectionStatus::Connecting);

    app.update(Action::ReplaceEditor("SELECT 1".into()));
    let commands = app.update(Action::RunActiveSql);
    assert!(
        matches!(commands.as_slice(), [Command::RunQueryPage { connection, .. }] if *connection == ConnectionIdentity { profile_id: first_id, generation: first_generation }),
        "{commands:?}"
    );
    assert_eq!(app.active_console().query_status, QueryStatus::Running);

    app.update(Action::ConnectionFailed {
        profile_id: second_id,
        generation: second_generation,
        message: "unreachable".into(),
    });
    assert_eq!(app.connection.profile_id, Some(first_id));
    assert_eq!(app.connection.status, ConnectionStatus::Connected);
    assert!(app.connection.pending_profile_id.is_none());
    assert_eq!(app.connection.server, active_server);
}

#[test]
fn failed_switch_keeps_visible_workspace_and_editor_text_unchanged() {
    let first = memory_profile("first");
    let second = memory_profile("second");
    let first_id = first.id;
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);

    let first_generation = match app.update(Action::RequestConnect(first_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id: first_id,
        generation: first_generation,
        server: server("first"),
        mutation_capabilities: Default::default(),
    });
    app.update(Action::ReplaceEditor("SELECT first".into()));
    app.update(Action::EditorKey(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('l'),
        crossterm::event::KeyModifiers::NONE,
    )));
    let first_position = app.active_editor_position();
    let first_tab = app.active_console().id;

    let second_generation = match app.update(Action::RequestConnect(second_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    assert_eq!(app.active_workspace_profile, Some(first_id));
    assert_eq!(app.active_console().id, first_tab);
    assert_eq!(app.active_editor_text().unwrap(), "SELECT first");
    assert_eq!(app.active_editor_position(), first_position);

    app.update(Action::ConnectionFailed {
        profile_id: second_id,
        generation: second_generation,
        message: "failed".into(),
    });
    assert_eq!(app.active_workspace_profile, Some(first_id));
    assert_eq!(app.active_console().id, first_tab);
    assert_eq!(app.active_editor_text().unwrap(), "SELECT first");
}

#[test]
fn successful_switch_keeps_profile_workspaces_available_together() {
    let first = memory_profile("first");
    let second = memory_profile("second");
    let first_id = first.id;
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);

    let first_generation = match app.update(Action::RequestConnect(first_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id: first_id,
        generation: first_generation,
        server: server("first"),
        mutation_capabilities: Default::default(),
    });
    app.update(Action::ReplaceEditor("SELECT first".into()));
    let first_tab = app.active_console().id;

    let second_generation = match app.update(Action::RequestConnect(second_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    let commands = app.update(Action::ConnectionSucceeded {
        profile_id: second_id,
        generation: second_generation,
        server: server("second"),
        mutation_capabilities: Default::default(),
    });
    assert_eq!(app.active_workspace_profile, Some(second_id));
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.sql_editors.len(), 2);
    assert_ne!(app.active_console().id, first_tab);
    assert_eq!(
        app.active_console()
            .execution_target
            .as_ref()
            .unwrap()
            .profile_id,
        second_id
    );
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, Command::PersistWorkspace { .. }))
    );

    assert!(app.update(Action::RequestConnect(first_id)).is_empty());
    assert_eq!(app.active_workspace_profile, Some(first_id));
    assert_eq!(app.active_console().id, first_tab);
    assert_eq!(app.active_editor_text().unwrap(), "SELECT first");
    assert_eq!(app.tabs.len(), 2);
}

#[test]
fn target_selector_switches_only_after_matching_connection_success() {
    let mut profile = memory_profile("target");
    profile.catalog_scope.databases = CatalogSelection::All;
    let profile_id = profile.id;
    let default = ExecutionTarget::from_profile(&profile);
    let alias = ExecutionTarget {
        profile_id,
        database: ":memory:".into(),
        schema: Some("attached".into()),
    };
    let mut app = App::new(vec![profile]);
    app.connection.profile_id = Some(profile_id);
    app.connection.generation = 1;
    app.connection.status = ConnectionStatus::Connected;
    app.connection.target = Some(default.clone());

    let database = CatalogEntry::database(
        CatalogId::new(profile_id, CatalogKind::Database, [":memory:"]),
        QualifiedName {
            database: Some(":memory:".into()),
            schema: None,
            object: ":memory:".into(),
        },
        "database",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    let schema = CatalogEntry::schema(
        CatalogId::new(profile_id, CatalogKind::Schema, [":memory:", "attached"]),
        database.id.clone(),
        QualifiedName {
            database: Some(":memory:".into()),
            schema: Some("attached".into()),
            object: "attached".into(),
        },
        "schema",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    app.explorer
        .normalized
        .profiles
        .get_mut(&profile_id)
        .unwrap()
        .catalog
        .insert_subtree(vec![database, schema])
        .unwrap();
    let explorer_before = app.explorer.normalized.profiles[&profile_id]
        .catalog
        .entries()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let selected_before = app.explorer.normalized.selected.clone();
    let expanded_before = app.explorer.normalized.expanded.clone();
    app.explorer.selected = 7;
    app.explorer.scroll = 3;
    let catalog_epoch_before = app.explorer.normalized.profiles[&profile_id].catalog_epoch;

    app.update(Action::OpenTargetSelector);
    let lazydb::model::workspace::Overlay::TargetSelector {
        candidates,
        selected,
        ..
    } = app.overlay.as_ref().unwrap()
    else {
        panic!("target selector did not open");
    };
    assert_eq!(candidates, &[alias.clone(), default.clone()]);
    assert_eq!(*selected, 1);
    app.update(Action::MoveTargetSelector(1));
    let commands = app.update(Action::ConfirmTargetSelector);
    let generation = match commands.as_slice() {
        [
            Command::Connect {
                target, generation, ..
            },
        ] if target == &alias => *generation,
        other => panic!("unexpected commands: {other:?}"),
    };
    assert_eq!(
        app.active_console().execution_target.as_ref(),
        Some(&default)
    );

    app.update(Action::ConnectionFailed {
        profile_id,
        generation,
        message: "switch failed".into(),
    });
    assert_eq!(
        app.active_console().execution_target.as_ref(),
        Some(&default)
    );
    assert_eq!(app.connection.target.as_ref(), Some(&default));
    assert_eq!(
        app.explorer.normalized.profiles[&profile_id].status,
        ExplorerConnectionStatus::Online
    );
    assert_eq!(
        app.explorer.normalized.profiles[&profile_id]
            .catalog
            .entries()
            .len(),
        explorer_before.len()
    );

    app.update(Action::OpenTargetSelector);
    app.update(Action::MoveTargetSelector(1));
    let generation = match app.update(Action::ConfirmTargetSelector).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        other => panic!("unexpected commands: {other:?}"),
    };
    let commands = app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: server(":memory:"),
        mutation_capabilities: Default::default(),
    });
    assert_eq!(app.active_console().execution_target.as_ref(), Some(&alias));
    assert_eq!(app.connection.target.as_ref(), Some(&alias));
    assert_eq!(
        app.explorer.normalized.profiles[&profile_id]
            .catalog
            .entries()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        explorer_before
    );
    assert_eq!(app.explorer.normalized.selected, selected_before);
    assert_eq!(app.explorer.normalized.expanded, expanded_before);
    assert_eq!(app.explorer.selected, 7);
    assert_eq!(app.explorer.scroll, 3);
    assert_eq!(
        app.explorer.normalized.profiles[&profile_id].catalog_epoch,
        catalog_epoch_before
    );
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, Command::PersistWorkspace { .. }))
    );
}

#[test]
fn switching_to_a_console_reconnects_its_target_before_execution() {
    let mut profile = memory_profile("console-target");
    profile.catalog_scope.databases = CatalogSelection::All;
    let profile_id = profile.id;
    let default = ExecutionTarget::from_profile(&profile);
    let console_target = ExecutionTarget {
        profile_id,
        database: ":memory:".into(),
        schema: Some("attached".into()),
    };
    let mut app = App::new(vec![profile]);
    app.connection.profile_id = Some(profile_id);
    app.connection.generation = 1;
    app.connection.status = ConnectionStatus::Connected;
    app.connection.target = Some(default.clone());
    app.update(Action::NewConsole);
    app.tabs[0]
        .as_console_mut()
        .expect("default console")
        .execution_target = Some(console_target.clone());
    app.tabs[1]
        .as_console_mut()
        .expect("new console")
        .execution_target = Some(default.clone());
    app.active_tab = 1;

    let commands = app.update(Action::ActivateTab(0));

    assert!(matches!(
        commands.as_slice(),
        [Command::Connect { target, .. }] if target == &console_target
    ));
    assert_eq!(
        app.connection.pending_target.as_ref(),
        Some(&console_target)
    );
}

#[test]
fn console_target_reconnect_updates_the_active_target_before_sql_runs() {
    let mut profile = memory_profile("console-target-success");
    profile.catalog_scope.databases = CatalogSelection::All;
    let profile_id = profile.id;
    let default = ExecutionTarget::from_profile(&profile);
    let console_target = ExecutionTarget {
        profile_id,
        database: ":memory:".into(),
        schema: Some("attached".into()),
    };
    let mut app = App::new(vec![profile]);
    app.connection.profile_id = Some(profile_id);
    app.connection.generation = 1;
    app.connection.status = ConnectionStatus::Connected;
    app.connection.target = Some(default);
    app.update(Action::NewConsole);
    app.tabs[0]
        .as_console_mut()
        .expect("default console")
        .execution_target = Some(console_target.clone());
    app.active_tab = 0;

    let connect = app.update(Action::ActivateTab(0));
    let generation = match connect.as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        other => panic!("unexpected commands: {other:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: server(":memory:"),
        mutation_capabilities: Default::default(),
    });
    assert_eq!(app.connection.target.as_ref(), Some(&console_target));
    app.update(Action::ReplaceEditor("SELECT 1".into()));

    assert!(matches!(
        app.update(Action::RunActiveSql).as_slice(),
        [Command::RunQueryPage { target, .. }] if target == &console_target
    ));
}

#[test]
fn query_page_result_returns_to_console_after_switching_to_another_profile() {
    let first = memory_profile("query-first");
    let second = memory_profile("query-second");
    let first_id = first.id;
    let second_id = second.id;
    let first_target = ExecutionTarget::from_profile(&first);
    let second_target = ExecutionTarget::from_profile(&second);
    let mut app = App::new(vec![first, second]);

    app.update(Action::ConnectionSucceeded {
        profile_id: first_id,
        generation: 1,
        server: server("first"),
        mutation_capabilities: Default::default(),
    });
    app.active_console_mut().execution_target = Some(first_target.clone());
    app.active_console_mut().execution_connection = Some(ConnectionIdentity {
        profile_id: first_id,
        generation: 1,
    });
    app.update(Action::ReplaceEditor("SELECT 1".into()));
    let query = app.update(Action::RunActiveSql);
    let (tab_id, generation, connection) = match query.as_slice() {
        [
            Command::RunQueryPage {
                tab_id,
                generation,
                connection,
                target,
                ..
            },
        ] => {
            assert_eq!(target, &first_target);
            (*tab_id, *generation, *connection)
        }
        commands => panic!("unexpected commands: {commands:?}"),
    };

    app.update(Action::NewConsole);
    app.tabs[1]
        .as_console_mut()
        .expect("second console")
        .execution_target = Some(second_target.clone());
    app.connection.pending_profile_id = Some(second_id);
    app.connection.pending_generation = Some(2);
    app.connection.pending_target = Some(second_target.clone());
    app.update(Action::ConnectionSucceeded {
        profile_id: second_id,
        generation: 2,
        server: server("second"),
        mutation_capabilities: Default::default(),
    });
    app.active_tab = 1;
    assert_eq!(app.connection.profile_id, Some(second_id));

    let outcome = lazydb::db::query::QueryOutcome {
        result_sets: vec![lazydb::db::query::ResultSet::default()],
        stats: lazydb::db::query::QueryStats::new(
            Duration::from_millis(1),
            Duration::from_millis(1),
            0,
        ),
    };
    app.update(Action::QueryPageFinished {
        tab_id,
        generation,
        connection,
        outcome,
        pagination: lazydb::model::pagination::ResultPagination::from_page(
            lazydb::model::pagination::PageRequest::first(
                lazydb::model::pagination::PageSize::default(),
            ),
            0,
        ),
    });

    let first_tab = app
        .tabs
        .iter()
        .find(|tab| tab.id() == tab_id)
        .and_then(WorkspaceTab::as_console)
        .expect("first console");
    assert_eq!(first_tab.query_status, QueryStatus::Idle);
    assert!(first_tab.outcome.is_some());
    assert_eq!(app.active_console().id, app.tabs[1].id());
    assert!(app.active_console().outcome.is_none());
}

#[test]
fn failed_console_target_reconnect_preserves_old_connection_and_allows_retry() {
    let mut profile = memory_profile("console-target-failure");
    profile.catalog_scope.databases = CatalogSelection::All;
    let profile_id = profile.id;
    let active_target = ExecutionTarget::from_profile(&profile);
    let console_target = ExecutionTarget {
        profile_id,
        database: ":memory:".into(),
        schema: Some("attached".into()),
    };
    let mut app = App::new(vec![profile]);
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation: 1,
        server: server(":memory:"),
        mutation_capabilities: Default::default(),
    });
    app.connection.target = Some(active_target.clone());
    app.active_console_mut().execution_target = Some(console_target.clone());
    app.update(Action::ReplaceEditor("SELECT 1".into()));

    let generation = match app.update(Action::RunActiveSql).as_slice() {
        [
            Command::Connect {
                generation, target, ..
            },
        ] => {
            assert_eq!(target, &console_target);
            *generation
        }
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionFailed {
        profile_id,
        generation,
        message: "target unavailable".into(),
    });

    assert_eq!(app.connection.target.as_ref(), Some(&active_target));
    assert_eq!(app.connection.status, ConnectionStatus::Connected);
    assert!(app.connection.pending_target.is_none());
    assert!(matches!(
        app.update(Action::RunActiveSql).as_slice(),
        [Command::Connect { target, .. }] if target == &console_target
    ));
}

#[test]
fn executing_while_console_target_is_connecting_does_not_start_a_second_connection() {
    let mut profile = memory_profile("console-target-pending");
    profile.catalog_scope.databases = CatalogSelection::All;
    let profile_id = profile.id;
    let active_target = ExecutionTarget::from_profile(&profile);
    let console_target = ExecutionTarget {
        profile_id,
        database: ":memory:".into(),
        schema: Some("attached".into()),
    };
    let mut app = App::new(vec![profile]);
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation: 1,
        server: server(":memory:"),
        mutation_capabilities: Default::default(),
    });
    app.connection.target = Some(active_target);
    app.active_console_mut().execution_target = Some(console_target);
    app.active_tab = 0;
    app.update(Action::ReplaceEditor("SELECT 1".into()));

    let first = app.update(Action::RunActiveSql);
    assert!(matches!(first.as_slice(), [Command::Connect { .. }]));
    let second = app.update(Action::RunActiveSql);

    assert!(second.is_empty());
    assert_eq!(app.connection.status, ConnectionStatus::Connecting);
}

#[test]
fn target_selector_reconnects_when_console_target_matches_selection_but_connection_does_not() {
    let mut profile = memory_profile("target-reconnect");
    profile.catalog_scope.databases = CatalogSelection::All;
    let profile_id = profile.id;
    let default = ExecutionTarget::from_profile(&profile);
    let alias = ExecutionTarget {
        profile_id,
        database: ":memory:".into(),
        schema: Some("attached".into()),
    };
    let mut app = App::new(vec![profile]);
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation: 1,
        server: server(":memory:"),
        mutation_capabilities: Default::default(),
    });
    app.connection.target = Some(default);
    app.active_console_mut().execution_target = Some(alias.clone());

    let database = CatalogEntry::database(
        CatalogId::new(profile_id, CatalogKind::Database, [":memory:"]),
        QualifiedName {
            database: Some(":memory:".into()),
            schema: None,
            object: ":memory:".into(),
        },
        "database",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    let schema = CatalogEntry::schema(
        CatalogId::new(profile_id, CatalogKind::Schema, [":memory:", "attached"]),
        database.id.clone(),
        QualifiedName {
            database: Some(":memory:".into()),
            schema: Some("attached".into()),
            object: "attached".into(),
        },
        "schema",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    app.explorer
        .normalized
        .profiles
        .get_mut(&profile_id)
        .unwrap()
        .catalog
        .insert_subtree(vec![database, schema])
        .unwrap();

    app.update(Action::OpenTargetSelector);
    let selected = match app.overlay.as_ref().unwrap() {
        lazydb::model::workspace::Overlay::TargetSelector { candidates, .. } => candidates
            .iter()
            .position(|candidate| candidate == &alias)
            .unwrap(),
        other => panic!("unexpected overlay: {other:?}"),
    };
    let commands = app.update(Action::SelectTargetSelector(selected));

    assert!(matches!(
        commands.as_slice(),
        [Command::Connect { target, .. }] if target == &alias
    ));
}

#[test]
fn target_selector_requires_an_active_connection_and_blocks_manual_transactions() {
    let profile = memory_profile("target");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.connection.profile_id = None;
    app.update(Action::OpenTargetSelector);
    assert!(app.overlay.is_none());
    assert!(app.notifications.history().any(|notification| {
        notification.level == lazydb::model::notification::NotificationLevel::Warning
            && notification.body.contains("No active connection")
    }));

    app.connection.profile_id = Some(profile_id);
    app.connection.generation = 1;
    app.connection.status = ConnectionStatus::Connected;
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation: 2,
        server: server(":memory:"),
        mutation_capabilities: Default::default(),
    });
    app.update(Action::OpenTargetSelector);
    app.active_console_mut().transaction_mode = lazydb::model::transaction::TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    assert!(app.update(Action::ConfirmTargetSelector).is_empty());
}

#[test]
fn running_sql_blocks_connection_switch_without_changing_workspace() {
    let first = memory_profile("first");
    let second = memory_profile("second");
    let first_id = first.id;
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);
    let generation = match app.update(Action::RequestConnect(first_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id: first_id,
        generation,
        server: server("first"),
        mutation_capabilities: Default::default(),
    });
    let console_id = app.active_console().id;
    app.active_console_mut().query_status = QueryStatus::Running;

    assert!(app.update(Action::RequestConnect(second_id)).is_empty());
    assert_eq!(app.connection.profile_id, Some(first_id));
    assert_eq!(app.active_console().id, console_id);
    assert!(app.notifications.history().any(|notification| {
        notification.level == lazydb::model::notification::NotificationLevel::Warning
            && notification.body.contains("running SQL")
    }));
}

#[test]
fn all_manual_console_transactions_are_deferred_and_cancel_keeps_connection() {
    let first = memory_profile("first");
    let second = memory_profile("second");
    let first_id = first.id;
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);
    let generation = match app.update(Action::RequestConnect(first_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id: first_id,
        generation,
        server: server("first"),
        mutation_capabilities: Default::default(),
    });
    let first_console = app.active_console().id;
    app.update(Action::NewConsole);
    let second_console = app.active_console().id;
    for id in [first_console, second_console] {
        let tab = app
            .tabs
            .iter_mut()
            .find(|tab| tab.id() == id)
            .and_then(WorkspaceTab::as_console_mut)
            .unwrap();
        tab.transaction_mode = TransactionMode::Manual;
        tab.transaction_state = TransactionState::Active;
    }

    let commands = app.update(Action::RequestConnect(second_id));
    assert!(matches!(commands.as_slice(), [Command::Connect { .. }]));
    assert_eq!(app.connection.pending_profile_id, Some(second_id));
}

#[test]
fn disconnecting_one_profile_does_not_review_another_profiles_transaction() {
    let first = memory_profile("first");
    let second = memory_profile("second");
    let first_id = first.id;
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);
    let generation = match app.update(Action::RequestConnect(second_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id: second_id,
        generation,
        server: server("second"),
        mutation_capabilities: Default::default(),
    });

    let first_console = app.active_console().id;
    let first_connection = ConnectionIdentity {
        profile_id: first_id,
        generation: 7,
    };
    let first_tab = app
        .tabs
        .iter_mut()
        .find(|tab| tab.id() == first_console)
        .and_then(WorkspaceTab::as_console_mut)
        .unwrap();
    first_tab.execution_connection = Some(first_connection);
    first_tab.execution_target = Some(ExecutionTarget {
        profile_id: first_id,
        database: "first".into(),
        schema: None,
    });
    first_tab.transaction_mode = TransactionMode::Manual;
    first_tab.transaction_state = TransactionState::Active;

    let commands = app.update(Action::RequestProfileDisconnect {
        profile_id: second_id,
    });

    assert!(
        matches!(commands.as_slice(), [Command::Disconnect { connection }] if *connection == ConnectionIdentity {
            profile_id: second_id,
            generation,
        })
    );
    assert!(app.overlay.is_none());
}

#[test]
fn invalidating_one_connection_preserves_other_console_transaction() {
    let first = memory_profile("first");
    let second = memory_profile("second");
    let first_id = first.id;
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);
    app.tabs
        .push(WorkspaceTab::Sql(lazydb::model::tab::ConsoleTab::new(
            "first",
        )));
    let first_console = app.tabs.last().unwrap().id();
    app.tabs
        .push(WorkspaceTab::Sql(lazydb::model::tab::ConsoleTab::new(
            "second",
        )));
    let second_console = app.tabs.last().unwrap().id();
    let first_connection = ConnectionIdentity {
        profile_id: first_id,
        generation: 1,
    };
    let second_connection = ConnectionIdentity {
        profile_id: second_id,
        generation: 2,
    };
    for (id, connection) in [
        (first_console, first_connection),
        (second_console, second_connection),
    ] {
        let tab = app
            .tabs
            .iter_mut()
            .find(|tab| tab.id() == id)
            .and_then(WorkspaceTab::as_console_mut)
            .unwrap();
        tab.execution_connection = Some(connection);
        tab.transaction_mode = TransactionMode::Manual;
        tab.transaction_state = TransactionState::Active;
    }
    app.connection.profile_id = Some(second_id);
    app.connection.generation = second_connection.generation;
    app.connection.status = ConnectionStatus::Connected;

    app.update(Action::ConnectionInvalidated {
        connection: second_connection,
        message: "closed".into(),
    });

    assert_eq!(
        app.tabs
            .iter()
            .find(|tab| tab.id() == first_console)
            .and_then(WorkspaceTab::as_console)
            .unwrap()
            .transaction_state,
        TransactionState::Active
    );
    assert_eq!(
        app.tabs
            .iter()
            .find(|tab| tab.id() == second_console)
            .and_then(WorkspaceTab::as_console)
            .unwrap()
            .transaction_state,
        TransactionState::OutcomeUnknown
    );
}

#[test]
fn dirty_relation_edit_blocks_switch_and_quit_with_explicit_message() {
    let profile = memory_profile("profile");
    let profile_id = profile.id;
    let other = memory_profile("other");
    let other_id = other.id;
    let mut app = App::new(vec![profile, other]);
    let generation = match app.update(Action::RequestConnect(profile_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: server("profile"),
        mutation_capabilities: Default::default(),
    });
    let mut relation = RelationTab::new("users");
    let mut edit = RelationEditSession::from_rows(vec![vec![]]);
    edit.rows[0].state = lazydb::model::relation_edit::EditableRowState::Updated {
        changed_columns: Default::default(),
    };
    relation.edit = Some(edit);
    app.tabs.push(WorkspaceTab::Relation(relation));
    app.active_tab = app.tabs.len() - 1;

    assert!(app.update(Action::RequestConnect(other_id)).is_empty());
    assert!(app.notifications.history().any(|notification| {
        notification.level == lazydb::model::notification::NotificationLevel::Warning
            && notification.body
                == "Commit or roll back relation edits before switching connections"
    }));
    assert!(app.update(Action::Quit).is_empty());
    assert!(!app.should_quit);
    assert!(app.notifications.history().any(|notification| {
        notification.level == lazydb::model::notification::NotificationLevel::Warning
            && notification.body == "Commit or roll back relation edits before quitting"
    }));
}

#[test]
fn profile_root_safe_switch_keeps_old_online_while_target_links_then_fails_locally() {
    let first = memory_profile("first");
    let second = memory_profile("second");
    let first_id = first.id;
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);
    let first_generation = match app
        .update(Action::RequestProfileConnect {
            profile_id: first_id,
        })
        .as_slice()
    {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id: first_id,
        generation: first_generation,
        server: server("first"),
        mutation_capabilities: Default::default(),
    });

    let second_generation = match app
        .update(Action::RequestProfileConnect {
            profile_id: second_id,
        })
        .as_slice()
    {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    assert_eq!(
        app.explorer.normalized.profiles[&first_id].status,
        ExplorerConnectionStatus::Online
    );
    assert_eq!(
        app.explorer.normalized.profiles[&second_id].status,
        ExplorerConnectionStatus::Linking
    );

    app.update(Action::ConnectionFailed {
        profile_id: second_id,
        generation: second_generation,
        message: "unreachable".into(),
    });
    assert_eq!(
        app.explorer.normalized.profiles[&first_id].status,
        ExplorerConnectionStatus::Online
    );
    assert_eq!(
        app.explorer.normalized.profiles[&second_id].status,
        ExplorerConnectionStatus::Failed
    );
    assert_eq!(
        app.explorer.normalized.profiles[&second_id]
            .last_error
            .as_deref(),
        Some("unreachable")
    );
}

#[test]
fn opening_second_profile_preserves_first_connection_state() {
    let first = memory_profile("first");
    let second = memory_profile("second");
    let first_id = first.id;
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);
    let first_generation = match app
        .update(Action::RequestProfileConnect {
            profile_id: first_id,
        })
        .as_slice()
    {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id: first_id,
        generation: first_generation,
        server: server("first"),
        mutation_capabilities: Default::default(),
    });
    app.explorer
        .normalized
        .expanded
        .insert(ExplorerNodeId::Profile(first_id));
    let first_database = CatalogEntry::database(
        CatalogId::new(first_id, CatalogKind::Database, ["first_db"]),
        QualifiedName {
            database: Some("first_db".into()),
            schema: None,
            object: "first_db".into(),
        },
        "database",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    app.explorer
        .normalized
        .profiles
        .get_mut(&first_id)
        .unwrap()
        .catalog
        .insert(first_database)
        .unwrap();

    let second_generation = match app
        .update(Action::RequestProfileConnect {
            profile_id: second_id,
        })
        .as_slice()
    {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id: second_id,
        generation: second_generation,
        server: server("second"),
        mutation_capabilities: Default::default(),
    });

    assert_eq!(
        app.explorer.normalized.profiles[&first_id].status,
        ExplorerConnectionStatus::Online
    );
    assert!(
        app.explorer
            .normalized
            .expanded
            .contains(&ExplorerNodeId::Profile(first_id))
    );
    assert!(
        app.explorer.normalized.profiles[&first_id]
            .catalog
            .get(&CatalogId::new(
                first_id,
                CatalogKind::Database,
                ["first_db"]
            ))
            .is_some()
    );
    assert_eq!(
        app.explorer.normalized.profiles[&second_id].status,
        ExplorerConnectionStatus::Syncing
    );
}

#[test]
fn late_success_for_an_older_connect_attempt_does_not_steal_selected_connection() {
    let first = memory_profile("first");
    let second = memory_profile("second");
    let first_id = first.id;
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);
    let first_generation = match app.update(Action::RequestConnect(first_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    let second_generation = match app.update(Action::RequestConnect(second_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };

    app.update(Action::ConnectionSucceeded {
        profile_id: second_id,
        generation: second_generation,
        server: server("second"),
        mutation_capabilities: Default::default(),
    });
    app.update(Action::ConnectionSucceeded {
        profile_id: first_id,
        generation: first_generation,
        server: server("first"),
        mutation_capabilities: Default::default(),
    });

    assert_eq!(app.connection.profile_id, Some(second_id));
    assert_eq!(app.connection.generation, second_generation);
    assert_eq!(
        app.sessions
            .get_by_identity(ConnectionIdentity {
                profile_id: first_id,
                generation: first_generation,
            })
            .map(|session| session.status.clone()),
        Some(lazydb::model::session::SessionStatus::Connected)
    );
    assert_eq!(
        app.sessions
            .get_by_identity(ConnectionIdentity {
                profile_id: second_id,
                generation: second_generation,
            })
            .map(|session| session.status.clone()),
        Some(lazydb::model::session::SessionStatus::Connected)
    );
}

#[test]
fn second_profile_connection_does_not_clear_first_catalog_entries() {
    let first = memory_profile("first");
    let second = memory_profile("second");
    let first_id = first.id;
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);

    let first_generation = match app
        .update(Action::RequestProfileConnect {
            profile_id: first_id,
        })
        .as_slice()
    {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id: first_id,
        generation: first_generation,
        server: server("first"),
        mutation_capabilities: Default::default(),
    });
    let first_entry = CatalogEntry::database(
        CatalogId::new(first_id, CatalogKind::Database, ["first_db"]),
        QualifiedName {
            database: Some("first_db".into()),
            schema: None,
            object: "first_db".into(),
        },
        "database",
        OptionalMetadata::Supported(None),
        true,
    )
    .unwrap();
    app.explorer
        .normalized
        .profiles
        .get_mut(&first_id)
        .unwrap()
        .catalog
        .insert(first_entry)
        .unwrap();

    let second_generation = match app
        .update(Action::RequestProfileConnect {
            profile_id: second_id,
        })
        .as_slice()
    {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id: second_id,
        generation: second_generation,
        server: server("second"),
        mutation_capabilities: Default::default(),
    });

    assert!(
        app.explorer.normalized.profiles[&first_id]
            .catalog
            .get(&CatalogId::new(
                first_id,
                CatalogKind::Database,
                ["first_db"]
            ))
            .is_some()
    );
}

#[test]
fn console_query_uses_its_session_when_another_profile_is_globally_active() {
    let first = memory_profile("first");
    let second = memory_profile("second");
    let first_id = first.id;
    let second_id = second.id;
    let first_target = ExecutionTarget::from_profile(&first);
    let mut app = App::new(vec![first, second]);

    let first_generation = match app.update(Action::RequestConnect(first_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id: first_id,
        generation: first_generation,
        server: server("first"),
        mutation_capabilities: Default::default(),
    });
    let first_identity = ConnectionIdentity {
        profile_id: first_id,
        generation: first_generation,
    };

    let second_generation = match app.update(Action::RequestConnect(second_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id: second_id,
        generation: second_generation,
        server: server("second"),
        mutation_capabilities: Default::default(),
    });
    assert_eq!(app.connection.profile_id, Some(second_id));

    app.active_console_mut().execution_target = Some(first_target.clone());
    app.active_console_mut().execution_connection = Some(first_identity);
    app.update(Action::ReplaceEditor("SELECT 1".into()));
    let commands = app.update(Action::RunActiveSql);

    assert!(matches!(
        commands.as_slice(),
        [Command::RunQueryPage { connection, target, .. }]
            if *connection == first_identity && target == &first_target
    ));
}

#[test]
fn connection_success_reconciles_without_clearing_a_newer_attempt() {
    let first = memory_profile("first");
    let second = memory_profile("second");
    let first_id = first.id;
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);

    let old_generation = app
        .sessions
        .start_attempt(ExecutionTarget::from_profile(
            app.profiles
                .iter()
                .find(|profile| profile.id == first_id)
                .unwrap(),
        ))
        .unwrap()
        .generation;
    let current_generation = match app.update(Action::RequestConnect(second_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };

    app.update(Action::ConnectionSucceeded {
        profile_id: first_id,
        generation: old_generation,
        server: server("stale"),
        mutation_capabilities: Default::default(),
    });
    app.update(Action::ConnectionFailed {
        profile_id: first_id,
        generation: old_generation,
        message: "stale".into(),
    });
    assert_eq!(app.connection.profile_id, None);
    assert_eq!(app.connection.generation, 0);
    assert_eq!(app.connection.pending_profile_id, Some(second_id));
    assert_eq!(app.connection.pending_generation, Some(current_generation));
    assert_eq!(app.connection.server, None);
    assert_eq!(app.connection.status, ConnectionStatus::Connecting);

    app.update(Action::ConnectionFailed {
        profile_id: second_id,
        generation: current_generation,
        message: "unreachable".into(),
    });
    assert_eq!(app.connection.profile_id, None);
    assert_eq!(app.connection.generation, 0);
    assert!(app.connection.pending_profile_id.is_none());
    assert_eq!(app.connection.status, ConnectionStatus::Failed);
}

#[test]
fn exhausted_connection_generation_refuses_to_wrap() {
    let profile = memory_profile("profile");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.connection.profile_id = Some(profile_id);
    app.connection.generation = u64::MAX;
    app.connection.status = ConnectionStatus::Connected;

    assert!(app.update(Action::RequestConnect(profile_id)).is_empty());
    assert_eq!(app.connection.profile_id, Some(profile_id));
    assert_eq!(app.connection.generation, u64::MAX);
    assert!(app.connection.pending_profile_id.is_none());
    assert!(
        app.connection
            .error
            .as_deref()
            .is_some_and(|message| message.contains("generation exhausted"))
    );
}

#[test]
fn disconnected_identity_cannot_be_resurrected_by_a_stale_success() {
    let profile = memory_profile("profile");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    let generation = match app.update(Action::RequestConnect(profile_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    let connection = ConnectionIdentity {
        profile_id,
        generation,
    };
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: server("profile"),
        mutation_capabilities: Default::default(),
    });
    app.update(Action::DisconnectCompleted { connection });

    assert!(
        app.update(Action::ConnectionSucceeded {
            profile_id,
            generation,
            server: server("stale"),
            mutation_capabilities: Default::default(),
        })
        .is_empty()
    );
    assert!(app.connection.profile_id.is_none());
    assert_eq!(app.connection.status, ConnectionStatus::Disconnected);
}

#[test]
fn unrelated_disconnect_completion_does_not_change_failed_state() {
    let profile = memory_profile("profile");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    let generation = match app.update(Action::RequestConnect(profile_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionFailed {
        profile_id,
        generation,
        message: "unreachable".into(),
    });

    app.update(Action::DisconnectCompleted {
        connection: ConnectionIdentity {
            profile_id,
            generation: generation + 1,
        },
    });
    assert_eq!(app.connection.status, ConnectionStatus::Failed);
    assert_eq!(app.connection.error.as_deref(), Some("unreachable"));
}

#[test]
fn active_disconnect_caches_and_hides_workspace_until_reconnect() {
    let profile = memory_profile("profile");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    let generation = match app.update(Action::RequestConnect(profile_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: server("profile"),
        mutation_capabilities: Default::default(),
    });
    let console_id = app.active_console().id;
    app.update(Action::ReplaceEditor("SELECT cached".into()));
    let commands = app.update(Action::DisconnectCompleted {
        connection: ConnectionIdentity {
            profile_id,
            generation,
        },
    });

    assert!(!app.tabs.is_empty());
    assert!(!app.sql_editors.is_empty());
    assert_eq!(app.active_editor_text().unwrap(), "SELECT cached");
    assert!(commands.iter().any(|command| matches!(
        command,
        Command::PersistWorkspace { snapshot, .. }
            if snapshot.profiles.iter().any(|workspace| {
                workspace.profile_id == profile_id
                    && workspace.consoles.iter().any(|console| console.id == console_id)
            })
    )));

    let reconnect_generation = match app.update(Action::RequestConnect(profile_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation: reconnect_generation,
        server: server("profile-again"),
        mutation_capabilities: Default::default(),
    });
    assert_eq!(app.active_console().id, console_id);
    assert_eq!(app.active_editor_text().unwrap(), "SELECT cached");
}

#[test]
fn active_invalidation_caches_and_hides_workspace_but_stale_invalidation_is_ignored() {
    let profile = memory_profile("profile");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    let generation = match app.update(Action::RequestConnect(profile_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: server("profile"),
        mutation_capabilities: Default::default(),
    });
    app.update(Action::ReplaceEditor("SELECT invalidated".into()));
    app.update(Action::ConnectionInvalidated {
        connection: ConnectionIdentity {
            profile_id,
            generation,
        },
        message: "connection lost".into(),
    });
    assert!(!app.tabs.is_empty());
    assert!(!app.sql_editors.is_empty());
    assert_eq!(app.active_editor_text().unwrap(), "SELECT invalidated");
    assert_eq!(app.connection.error.as_deref(), Some("connection lost"));

    app.update(Action::ConnectionInvalidated {
        connection: ConnectionIdentity {
            profile_id,
            generation: generation + 1,
        },
        message: "stale".into(),
    });
    assert_eq!(app.connection.error.as_deref(), Some("connection lost"));
    assert!(!app.tabs.is_empty());
}

#[tokio::test]
async fn connecting_second_profile_keeps_first_runtime_console_usable() {
    let temp = TempDir::new().unwrap();
    let first = file_profile(&temp.path().join("first.db"), "first", "alpha").await;
    let second = file_profile(&temp.path().join("second.db"), "second", "beta").await;
    let first_id = first.id;
    let second_id = second.id;
    let profiles = vec![first.clone(), second.clone()];
    let (mut runtime, mut receiver) = runtime(
        &temp,
        profiles.clone(),
        Arc::new(MissingSecretStore::default()),
        None,
    );
    let mut app = App::new(profiles);

    let first_identity = connect(&mut app, &mut runtime, &mut receiver, first_id).await;
    assert_eq!(
        run_marker_query(&mut app, &mut runtime, &mut receiver).await,
        "alpha"
    );

    let commands = dispatch(&mut app, &mut runtime, Action::RequestConnect(second_id));
    assert!(matches!(commands.as_slice(), [Command::Connect { .. }]));
    assert_eq!(app.connection.profile_id, Some(first_id));
    assert_eq!(app.connection.pending_profile_id, Some(second_id));
    let commands = dispatch(&mut app, &mut runtime, Action::RunActiveSql);
    assert!(
        matches!(commands.as_slice(), [Command::RunQueryPage { connection, .. }] if *connection == first_identity),
        "{commands:?}"
    );
    let query_finished = next_action(&mut receiver).await;
    assert!(matches!(query_finished, Action::QueryPageFinished { .. }));
    dispatch(&mut app, &mut runtime, query_finished);

    let connected = next_action(&mut receiver).await;
    assert!(matches!(
        connected,
        Action::ConnectionSucceeded {
            profile_id: connected_id,
            ..
        } if connected_id == second_id
    ));
    dispatch(&mut app, &mut runtime, connected);
    drain_catalog(&mut app, &mut runtime, &mut receiver).await;
    assert_eq!(app.connection.profile_id, Some(second_id));
    assert!(app.connection.pending_profile_id.is_none());
    assert_eq!(
        run_marker_query(&mut app, &mut runtime, &mut receiver).await,
        "beta"
    );

    runtime.dispatch(Command::RunQuery {
        connection: first_identity,
        target: ExecutionTarget::from_profile(
            app.profiles
                .iter()
                .find(|profile| profile.id == first_id)
                .unwrap(),
        ),
        tab_id: Uuid::new_v4(),
        generation: 1,
        sql: "SELECT value FROM marker".into(),
    });
    assert!(matches!(
        next_action(&mut receiver).await,
        Action::QueryFinished { connection, .. } if connection == first_identity
    ));
    let relation_request = RelationRequest {
        tab_id: Uuid::new_v4(),
        tab_generation: 0,
        request_id: 1,
        connection: first_identity,
        relation: RelationKey {
            profile_id: first_id,
            object_id: lazydb::db::catalog::CatalogId::new(
                first_id,
                CatalogKind::Table,
                ["first", "main", "marker"],
            ),
        },
        kind: RelationRequestKind::Preview,
        scope: first.catalog_scope.clone(),
        options: Default::default(),
        page: lazydb::model::pagination::PageRequest::first(
            lazydb::model::pagination::PageSize::default(),
        ),
    };
    runtime.dispatch(Command::LoadRelationPreview(relation_request.clone()));
    assert!(matches!(
        next_action(&mut receiver).await,
        Action::RelationFailed { .. }
    ));
    runtime.dispatch(Command::LoadRelationDdl(RelationRequest {
        kind: RelationRequestKind::Ddl,
        ..relation_request
    }));
    assert!(matches!(
        next_action(&mut receiver).await,
        Action::RelationFailed { .. }
    ));
    runtime.dispatch(Command::LoadCatalogPage(catalog_request(first_identity)));
    assert!(matches!(
        next_action(&mut receiver).await,
        Action::CatalogPageLoaded(page) if page.key.connection == first_identity
    ));
    runtime.shutdown().await;
}

#[tokio::test]
async fn late_disconnect_cannot_close_a_new_generation_of_the_same_profile() {
    let temp = TempDir::new().unwrap();
    let profile = file_profile(&temp.path().join("profile.db"), "profile", "current").await;
    let profile_id = profile.id;
    let (mut runtime, mut receiver) = runtime(
        &temp,
        vec![profile.clone()],
        Arc::new(MissingSecretStore::default()),
        None,
    );
    let mut app = App::new(vec![profile]);

    let old = connect(&mut app, &mut runtime, &mut receiver, profile_id).await;
    let new_generation = old.generation + 1;
    let target = ExecutionTarget::from_profile(
        app.profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .unwrap(),
    );
    app.sessions.register_attempt(
        target.clone(),
        ConnectionIdentity {
            profile_id,
            generation: new_generation,
        },
    );
    runtime.dispatch(Command::Connect {
        profile_id,
        generation: new_generation,
        target,
    });
    let connected = next_connection_result(&mut app, &mut runtime, &mut receiver, profile_id).await;
    assert!(matches!(
        connected,
        Action::ConnectionSucceeded {
            profile_id: connected_id,
            generation,
            ..
        } if connected_id == profile_id && generation == new_generation
    ));
    dispatch(&mut app, &mut runtime, connected);
    drain_catalog(&mut app, &mut runtime, &mut receiver).await;

    assert_eq!(
        app.connection.active_identity(),
        Some(ConnectionIdentity {
            profile_id,
            generation: old.generation,
        })
    );
    assert_eq!(
        app.sessions
            .get_by_identity(ConnectionIdentity {
                profile_id,
                generation: new_generation
            })
            .unwrap()
            .status,
        lazydb::model::session::SessionStatus::Connected
    );
    assert_eq!(app.connection.status, ConnectionStatus::Connected);

    runtime.dispatch(Command::Disconnect { connection: old });
    let disconnected = next_action(&mut receiver).await;
    assert_eq!(
        disconnected,
        Action::DisconnectCompleted { connection: old }
    );
    dispatch(&mut app, &mut runtime, disconnected);
    assert_eq!(
        app.connection.active_identity(),
        Some(ConnectionIdentity {
            profile_id,
            generation: new_generation,
        })
    );
    assert_eq!(
        run_marker_query(&mut app, &mut runtime, &mut receiver).await,
        "current"
    );
    runtime.shutdown().await;
}

fn catalog_request(connection: ConnectionIdentity) -> CatalogRequest {
    CatalogRequest {
        key: CatalogRequestKey {
            connection,
            catalog_epoch: 1,
            request_id: 1,
            target: CatalogTarget::Databases,
            cursor: None,
        },
        scope: lazydb::profile::CatalogScope {
            databases: CatalogSelection::All,
        },
        page_size: 100,
    }
}

#[tokio::test]
async fn runtime_accepts_same_generation_for_independent_profiles() {
    let temp = TempDir::new().unwrap();
    let first = file_profile(&temp.path().join("first.db"), "first", "alpha").await;
    let second = file_profile(&temp.path().join("second.db"), "second", "beta").await;
    let first_id = first.id;
    let second_id = second.id;
    let second_target = ExecutionTarget::from_profile(&second);
    let profiles = vec![first, second];
    let (mut runtime, mut receiver) = runtime(
        &temp,
        profiles.clone(),
        Arc::new(MissingSecretStore::default()),
        None,
    );
    let mut app = App::new(profiles);
    let first_identity = connect(&mut app, &mut runtime, &mut receiver, first_id).await;

    runtime.dispatch(Command::Connect {
        profile_id: second_id,
        generation: first_identity.generation,
        target: second_target,
    });
    assert!(matches!(
        next_action(&mut receiver).await,
        Action::ConnectionSucceeded {
            profile_id: connected_id,
            generation,
            ..
        } if connected_id == second_id && generation == first_identity.generation
    ));
    assert_eq!(
        run_marker_query(&mut app, &mut runtime, &mut receiver).await,
        "alpha"
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn runtime_rejects_target_mismatch_before_query_io() {
    let temp = TempDir::new().unwrap();
    let mut profile = file_profile(&temp.path().join("target.db"), "target", "alpha").await;
    profile.catalog_scope.databases = CatalogSelection::All;
    let profile_id = profile.id;
    let active_target = ExecutionTarget::from_profile(&profile);
    let mismatched_target = ExecutionTarget {
        profile_id,
        database: active_target.database.clone(),
        schema: Some("attached".into()),
    };
    let (mut runtime, mut receiver) = runtime(
        &temp,
        vec![profile],
        Arc::new(MissingSecretStore::default()),
        None,
    );
    runtime.dispatch(Command::Connect {
        profile_id,
        generation: 1,
        target: active_target,
    });
    assert!(matches!(
        next_action(&mut receiver).await,
        Action::ConnectionSucceeded { .. }
    ));

    runtime.dispatch(Command::RunQuery {
        connection: ConnectionIdentity {
            profile_id,
            generation: 1,
        },
        target: mismatched_target,
        tab_id: Uuid::new_v4(),
        generation: 1,
        sql: "INSERT INTO marker VALUES ('must-not-run')".into(),
    });
    assert!(matches!(
        next_action(&mut receiver).await,
        Action::QueryFailed { message, .. }
            if message.contains("does not match the execution target")
    ));
    runtime.shutdown().await;
}

#[tokio::test]
async fn failed_switch_restores_the_previous_database() {
    let temp = TempDir::new().unwrap();
    let first = file_profile(&temp.path().join("first.db"), "first", "alpha").await;
    let mut failing = import_connection_url(
        &format!("sqlite://{}", temp.path().join("missing.db").display()),
        Some("missing"),
    )
    .unwrap()
    .profile;
    failing.read_only = true;
    let first_id = first.id;
    let failing_id = failing.id;
    let profiles = vec![first, failing];
    let (mut runtime, mut receiver) = runtime(
        &temp,
        profiles.clone(),
        Arc::new(MissingSecretStore::default()),
        None,
    );
    let mut app = App::new(profiles);
    connect(&mut app, &mut runtime, &mut receiver, first_id).await;

    dispatch(&mut app, &mut runtime, Action::RequestConnect(failing_id));
    let failure = next_connection_result(&mut app, &mut runtime, &mut receiver, failing_id).await;
    assert!(matches!(
        failure,
        Action::ConnectionFailed {
            profile_id: failed_id,
            ..
        } if failed_id == failing_id
    ));
    dispatch(&mut app, &mut runtime, failure);
    assert_eq!(app.connection.profile_id, Some(first_id));
    assert_eq!(app.connection.status, ConnectionStatus::Connected);
    assert_eq!(
        run_marker_query(&mut app, &mut runtime, &mut receiver).await,
        "alpha"
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn startup_password_is_never_reused_for_another_profile() {
    let temp = TempDir::new().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut first =
        import_connection_url(&format!("postgres://127.0.0.1:{port}/first"), Some("first"))
            .unwrap()
            .profile;
    let mut second = import_connection_url(
        &format!("postgres://127.0.0.1:{port}/second"),
        Some("second"),
    )
    .unwrap()
    .profile;
    first.credential_policy = CredentialPolicy::Keyring(keyring_ref(first.id));
    second.credential_policy = CredentialPolicy::Keyring(keyring_ref(second.id));
    let first_id = first.id;
    let second_id = second.id;
    let first_target = ExecutionTarget::from_profile(&first);
    let second_target = ExecutionTarget::from_profile(&second);
    let secrets = Arc::new(MissingSecretStore::default());
    let (mut runtime, mut receiver) = runtime(
        &temp,
        vec![first, second],
        Arc::clone(&secrets) as Arc<dyn SecretStore>,
        Some((first_id, SecretString::from("startup-only".to_owned()))),
    );

    runtime.dispatch(Command::Connect {
        profile_id: second_id,
        generation: 1,
        target: second_target,
    });
    assert!(matches!(
        next_action(&mut receiver).await,
        Action::CredentialsRequired {
            profile_id: required,
            generation: 1,
            ..
        } if required == second_id
    ));
    assert_eq!(secrets.get_ids(), [second_id]);

    let server = tokio::task::spawn_blocking(move || {
        let (socket, _) = listener.accept().unwrap();
        drop(socket);
    });
    runtime.dispatch(Command::Connect {
        profile_id: first_id,
        generation: 2,
        target: first_target,
    });
    assert!(matches!(
        next_action(&mut receiver).await,
        Action::ConnectionFailed {
            profile_id: failed,
            generation: 2,
            ..
        } if failed == first_id
    ));
    server.await.unwrap();
    assert_eq!(secrets.get_ids(), [second_id]);
    runtime.shutdown().await;
}
