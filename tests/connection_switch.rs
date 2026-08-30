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
        relation::{RelationKey, RelationRequest, RelationRequestKind},
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

async fn next_action(receiver: &mut mpsc::UnboundedReceiver<Action>) -> Action {
    timeout(Duration::from_secs(3), receiver.recv())
        .await
        .expect("runtime action timed out")
        .expect("runtime action channel closed")
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
            Action::CatalogPageLoaded(_) | Action::CatalogPageFailed { .. }
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
    assert!(matches!(commands.as_slice(), [Command::RunQuery { .. }]));
    let result = next_action(receiver).await;
    assert!(matches!(result, Action::QueryFinished { .. }));
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
fn pending_switch_keeps_active_identity_and_rejects_new_queries() {
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
    assert!(app.update(Action::RunActiveSql).is_empty());
    assert_eq!(app.active_console().query_status, QueryStatus::Idle);

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
    });
    app.update(Action::ReplaceEditor("SELECT first".into()));
    let first_tab = app.active_console().id;

    let second_generation = match app.update(Action::RequestConnect(second_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    assert_eq!(app.active_workspace_profile, Some(first_id));
    assert_eq!(app.active_console().id, first_tab);
    assert_eq!(app.active_editor_text().unwrap(), "SELECT first");

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
fn successful_switch_caches_and_restores_profile_workspace_once() {
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
    });
    assert_eq!(app.active_workspace_profile, Some(second_id));
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.sql_editors.len(), 1);
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
            .any(|command| matches!(command, Command::PersistWorkspace(_)))
    );

    let first_generation = match app.update(Action::RequestConnect(first_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id: first_id,
        generation: first_generation,
        server: server("first-again"),
    });
    assert_eq!(app.active_workspace_profile, Some(first_id));
    assert_eq!(app.active_console().id, first_tab);
    assert_eq!(app.active_editor_text().unwrap(), "SELECT first");
    assert_eq!(app.tabs.len(), 1);
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

    app.update(Action::OpenTargetSelector);
    let lazydb::model::workspace::Overlay::TargetSelector {
        candidates,
        selected,
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
    });
    assert_eq!(app.active_console().execution_target.as_ref(), Some(&alias));
    assert_eq!(app.connection.target.as_ref(), Some(&alias));
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, Command::PersistWorkspace(_)))
    );
}

#[test]
fn target_selector_requires_an_active_connection_and_blocks_manual_transactions() {
    let profile = memory_profile("target");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.connection.profile_id = None;
    app.update(Action::OpenTargetSelector);
    assert!(app.overlay.is_none());
    assert!(
        app.connection
            .error
            .as_deref()
            .unwrap()
            .contains("No active connection")
    );

    app.connection.profile_id = Some(profile_id);
    app.connection.generation = 1;
    app.connection.status = ConnectionStatus::Connected;
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation: 2,
        server: server(":memory:"),
    });
    app.update(Action::OpenTargetSelector);
    app.active_console_mut().transaction_mode = lazydb::model::transaction::TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;
    assert!(app.update(Action::ConfirmTargetSelector).is_empty());
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
fn profile_root_successful_switch_clears_old_catalog_and_syncs_target() {
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
    });
    app.explorer
        .normalized
        .expanded
        .insert(ExplorerNodeId::Profile(first_id));

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
    });

    assert_eq!(
        app.explorer.normalized.profiles[&first_id].status,
        ExplorerConnectionStatus::Offline
    );
    assert!(
        app.explorer.normalized.profiles[&first_id]
            .catalog
            .is_empty()
    );
    assert!(
        !app.explorer
            .normalized
            .expanded
            .contains(&ExplorerNodeId::Profile(first_id))
    );
    assert_eq!(
        app.explorer.normalized.profiles[&second_id].status,
        ExplorerConnectionStatus::Syncing
    );
}

#[test]
fn installed_connection_success_reconciles_without_clearing_a_newer_attempt() {
    let first = memory_profile("first");
    let second = memory_profile("second");
    let first_id = first.id;
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);

    let old_generation = match app.update(Action::RequestConnect(first_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    let current_generation = match app.update(Action::RequestConnect(second_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };

    app.update(Action::ConnectionSucceeded {
        profile_id: first_id,
        generation: old_generation,
        server: server("stale"),
    });
    app.update(Action::ConnectionFailed {
        profile_id: first_id,
        generation: old_generation,
        message: "stale".into(),
    });
    assert_eq!(app.connection.profile_id, Some(first_id));
    assert_eq!(app.connection.generation, old_generation);
    assert_eq!(app.connection.pending_profile_id, Some(second_id));
    assert_eq!(app.connection.pending_generation, Some(current_generation));
    assert_eq!(app.connection.server, Some(server("stale")));
    assert_eq!(app.connection.status, ConnectionStatus::Connecting);

    app.update(Action::ConnectionFailed {
        profile_id: second_id,
        generation: current_generation,
        message: "unreachable".into(),
    });
    assert_eq!(app.connection.profile_id, Some(first_id));
    assert!(app.connection.pending_profile_id.is_none());
    assert_eq!(app.connection.status, ConnectionStatus::Connected);
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
    });
    app.update(Action::DisconnectCompleted { connection });

    assert!(
        app.update(Action::ConnectionSucceeded {
            profile_id,
            generation,
            server: server("stale"),
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

#[tokio::test]
async fn successful_switch_installs_the_new_database_and_rejects_stale_commands() {
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
    assert!(dispatch(&mut app, &mut runtime, Action::RunActiveSql).is_empty());

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
        Action::QueryFailed { .. }
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
        Action::CatalogPageFailed { key, .. } if key.connection == first_identity
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
    let commands = dispatch(&mut app, &mut runtime, Action::RequestConnect(profile_id));
    let new_generation = match commands.as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    let connected = next_action(&mut receiver).await;
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
            generation: new_generation,
        })
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
async fn runtime_rejects_a_reused_connection_generation() {
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
    assert!(
        timeout(Duration::from_millis(100), receiver.recv())
            .await
            .is_err()
    );
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
    let failure = next_action(&mut receiver).await;
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
