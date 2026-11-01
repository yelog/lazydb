use lazydb::{
    action::{Action, Command},
    app::App,
    db::ServerInfo,
    identity::ConnectionIdentity,
    model::{
        profile_manager::ProfileChange,
        relation::RelationTab,
        relation_edit::{EditableRowState, RelationEditSession},
        tab::WorkspaceTab,
        transaction::{
            DeferredTransactionTarget, TransactionExitChoice, TransactionMode, TransactionState,
        },
        workspace::{ConnectionStatus, Overlay, QueryStatus},
    },
    persistence::profiles::ProfileStore,
    profile::{ConnectionProfile, import_connection_url},
};
use tempfile::tempdir;

fn profile(name: &str) -> lazydb::profile::ConnectionProfile {
    import_connection_url("postgresql://localhost/app", Some(name))
        .unwrap()
        .profile
}

#[test]
fn reconcile_save_merges_external_add_and_rejects_same_profile_conflicts() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("connections.toml");
    let store = ProfileStore::new(path);
    let first = profile("first");
    store.save(vec![first.clone()]).unwrap();

    let expected = store.load().unwrap();
    let added = profile("added-externally");
    store
        .mutate(|collection| {
            collection.profiles.push(added.clone());
            Ok(((), true))
        })
        .unwrap();

    let mut desired = expected.clone();
    desired.profiles[0].name = "first-renamed".to_owned();
    store.reconcile_save(&expected, desired.clone()).unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.profiles.len(), 2);
    assert_eq!(loaded.profiles[0].name, "first-renamed");
    assert_eq!(loaded.profiles[1].id, added.id);

    let expected = store.load().unwrap();
    let mut desired = expected.clone();
    desired.profiles[0].name = "stale-edit".to_owned();
    store
        .mutate(|collection| {
            collection.profiles[0].name = "external-edit".to_owned();
            Ok(((), true))
        })
        .unwrap();
    let error = store.reconcile_save(&expected, desired).unwrap_err();
    assert!(error.to_string().contains("profile_conflict"));
    assert_eq!(store.load().unwrap().profiles[0].name, "external-edit");
}

fn connected_app() -> (App, ConnectionProfile, ConnectionIdentity) {
    let profile = profile("transaction");
    let mut app = App::new(vec![profile.clone()]);
    app.update(Action::NewConsole);
    let commands = app.update(Action::RequestProfileConnect {
        profile_id: profile.id,
    });
    let generation = commands
        .iter()
        .find_map(|command| match command {
            Command::Connect { generation, .. } => Some(*generation),
            _ => None,
        })
        .unwrap();
    app.update(Action::ConnectionSucceeded {
        profile_id: profile.id,
        generation,
        server: ServerInfo {
            kind: profile.kind,
            version: "test".into(),
            database: "app".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });
    let connection = app.connection.active_identity().unwrap();
    (app, profile, connection)
}

fn save_and_reconnect(
    app: &mut App,
    profile: &ConnectionProfile,
    credentials: bool,
) -> Vec<Command> {
    app.update(Action::ProfileStartEdit {
        profile_id: profile.id,
    });
    let commands = app.update(Action::ProfileSave { connect: true });
    let request_id = commands
        .iter()
        .find_map(|command| match command {
            Command::SaveProfile { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .unwrap();
    let mut saved = profile.clone();
    if !credentials {
        saved.read_only = true;
    }
    app.update(Action::ProfileSaved {
        request_id,
        profile: saved,
        warning: None,
        change: ProfileChange {
            connection_settings_changed: !credentials,
            credentials_changed: credentials,
            catalog_scope_changed: false,
            display_only_changed: false,
        },
        connect: true,
    })
}

fn assert_connection_preserved(app: &App, connection: ConnectionIdentity) {
    assert_eq!(app.connection.active_identity(), Some(connection));
    assert_eq!(app.connection.status, ConnectionStatus::Connected);
    assert!(!app.sessions.is_retired(connection));
}

fn assert_reconnected(commands: &[Command], app: &App, connection: ConnectionIdentity) {
    let disconnect = commands
        .iter()
        .position(|command| {
            matches!(command,
        Command::Disconnect { connection: retired } if *retired == connection)
        })
        .unwrap();
    let connect = commands
        .iter()
        .position(|command| {
            matches!(command,
        Command::Connect { profile_id, generation, .. }
            if *profile_id == connection.profile_id && *generation > connection.generation)
        })
        .unwrap();
    assert!(disconnect < connect);
    assert!(app.sessions.is_retired(connection));
    assert_eq!(app.connection.status, ConnectionStatus::Connecting);
}

#[test]
fn saved_profile_reconnect_waits_for_commit_or_rollback_of_idle_manual_transactions() {
    for credentials in [false, true] {
        for choice in [
            TransactionExitChoice::Commit,
            TransactionExitChoice::Rollback,
        ] {
            let (mut app, profile, connection) = connected_app();
            app.active_console_mut().transaction_mode = TransactionMode::Manual;
            app.active_console_mut().transaction_state = TransactionState::Active;
            let console_id = app.active_console().id;
            assert_eq!(app.active_console().query_status, QueryStatus::Idle);

            assert!(save_and_reconnect(&mut app, &profile, credentials).is_empty());
            assert_connection_preserved(&app, connection);
            assert!(app.profile_manager.is_none());
            assert!(matches!(app.overlay,
                Some(Overlay::TransactionExitConfirm { prompt, .. })
                    if prompt.target == DeferredTransactionTarget::Console(console_id)));

            let commands = app.update(Action::ConfirmTransactionExitChoice(choice));
            let completion = match commands.as_slice() {
                [
                    Command::ManualCommit {
                        connection: actual,
                        tab_id,
                        query_generation,
                        transaction_generation,
                    },
                ] if choice == TransactionExitChoice::Commit && *actual == connection => {
                    Action::ManualCommitted {
                        connection,
                        tab_id: *tab_id,
                        query_generation: *query_generation,
                        transaction_generation: *transaction_generation,
                    }
                }
                [
                    Command::ManualRollback {
                        connection: actual,
                        tab_id,
                        query_generation,
                        transaction_generation,
                    },
                ] if choice == TransactionExitChoice::Rollback && *actual == connection => {
                    Action::ManualRolledBack {
                        connection,
                        tab_id: *tab_id,
                        query_generation: *query_generation,
                        transaction_generation: *transaction_generation,
                    }
                }
                _ => panic!("unexpected resolution commands: {commands:?}"),
            };
            assert_connection_preserved(&app, connection);
            let commands = app.update(completion);
            assert_eq!(
                app.active_console().transaction_state,
                TransactionState::Idle
            );
            assert_reconnected(&commands, &app, connection);
        }
    }
}

#[test]
fn cancelling_saved_profile_reconnect_preserves_transaction_and_session() {
    let (mut app, profile, connection) = connected_app();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state = TransactionState::Active;
    assert!(save_and_reconnect(&mut app, &profile, false).is_empty());

    assert!(app.update(Action::CancelTransactionExit).is_empty());

    assert_connection_preserved(&app, connection);
    assert_eq!(
        app.active_console().transaction_state,
        TransactionState::Active
    );
    assert!(app.overlay.is_none());
    assert!(
        app.profiles[0].read_only,
        "the profile save itself already succeeded"
    );
}

#[test]
fn failed_transaction_resolution_does_not_retire_saved_profile_connection() {
    let (mut app, profile, connection) = connected_app();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state = TransactionState::Active;
    assert!(save_and_reconnect(&mut app, &profile, true).is_empty());
    let commands = app.update(Action::ConfirmTransactionExitChoice(
        TransactionExitChoice::Commit,
    ));
    let [
        Command::ManualCommit {
            tab_id,
            query_generation,
            transaction_generation,
            ..
        },
    ] = commands.as_slice()
    else {
        panic!("unexpected commands: {commands:?}");
    };

    let commands = app.update(Action::ManualCommitFailed {
        tab_id: *tab_id,
        query_generation: *query_generation,
        transaction_generation: *transaction_generation,
        connection,
        message: "commit rejected".into(),
        unknown: false,
    });

    assert!(commands.is_empty());
    assert_connection_preserved(&app, connection);
    assert!(matches!(
        app.overlay,
        Some(Overlay::TransactionExitConfirm { .. })
    ));
}

#[test]
fn saved_profile_reconnect_waits_for_every_console_transaction() {
    let (mut app, profile, connection) = connected_app();
    let target = app.active_console().execution_target.clone();
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state = TransactionState::Active;
    app.update(Action::NewConsole);
    app.active_console_mut().execution_target = target;
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.active_console_mut().transaction_state = TransactionState::Active;
    assert!(save_and_reconnect(&mut app, &profile, false).is_empty());

    for index in 0..2 {
        let commands = app.update(Action::ConfirmTransactionExitChoice(
            TransactionExitChoice::Rollback,
        ));
        let [
            Command::ManualRollback {
                tab_id,
                query_generation,
                transaction_generation,
                ..
            },
        ] = commands.as_slice()
        else {
            panic!("unexpected commands: {commands:?}");
        };
        let commands = app.update(Action::ManualRolledBack {
            tab_id: *tab_id,
            query_generation: *query_generation,
            transaction_generation: *transaction_generation,
            connection,
        });
        if index == 0 {
            assert!(commands.is_empty());
            assert_connection_preserved(&app, connection);
            assert!(matches!(
                app.overlay,
                Some(Overlay::TransactionExitConfirm { .. })
            ));
        } else {
            assert_reconnected(&commands, &app, connection);
        }
    }
}

#[test]
fn saved_profile_reconnect_preserves_uncommitted_relation_edits() {
    let (mut app, profile, connection) = connected_app();
    let mut relation = RelationTab::new("items");
    relation.descriptor.key.profile_id = profile.id;
    let mut edit = RelationEditSession::from_rows(vec![vec![]]);
    edit.rows[0].state = EditableRowState::Updated {
        changed_columns: Default::default(),
    };
    relation.edit = Some(edit);
    app.tabs.push(WorkspaceTab::Relation(relation));

    assert!(save_and_reconnect(&mut app, &profile, false).is_empty());

    assert_connection_preserved(&app, connection);
    assert!(
        matches!(app.tabs.last(), Some(WorkspaceTab::Relation(relation))
        if matches!(relation.edit.as_ref().unwrap().rows[0].state, EditableRowState::Updated { .. }))
    );
}
