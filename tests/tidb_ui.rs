use std::time::Duration;

use lazydb::{
    action::{Action, Command},
    app::App,
    db::{
        RelationPreview,
        catalog::{CatalogId, CatalogKind, QualifiedName},
        query::{ColumnMeta, QueryOutcome, QueryStats, ResultSet},
        value::CellValue,
    },
    identity::ConnectionIdentity,
    model::{
        execution_target::ExecutionTarget,
        pagination::{PageRequest, PageSize, ResultPagination},
        relation::{
            OwnedSnapshot, RelationDescriptor, RelationKey, RelationLoad, RelationRequest,
            RelationRequestKind, RelationSnapshot, RelationTab, RelationView,
        },
        relation_edit::RelationEditSession,
        tab::WorkspaceTab,
        transaction::{TransactionMode, TransactionState},
        workspace::{ConnectionStatus, Focus, Overlay, TargetSelectorCandidate},
    },
    persistence::workspace::{
        PersistedConsole, PersistedProfileWorkspace, PersistedTab, WorkspaceSnapshot,
    },
    profile::{ConnectionProfile, import_connection_url},
};
use uuid::Uuid;

fn tidb_profile() -> ConnectionProfile {
    import_connection_url("tidb://root@localhost/app", None)
        .unwrap()
        .profile
}

fn connected_app(profile: &ConnectionProfile) -> App {
    let mut app = App::new(vec![profile.clone()]);
    app.update(Action::NewConsole);
    app.connection.profile_id = Some(profile.id);
    app.connection.generation = 1;
    app.connection.status = ConnectionStatus::Connected;
    app.connection.pending_profile_id = None;
    app.connection.pending_generation = None;
    app.connection.pending_target = None;
    let target = ExecutionTarget::from_profile(profile);
    app.connection.target = Some(target.clone());
    app.active_console_mut().execution_target = Some(target);
    app.active_workspace_profile = Some(profile.id);
    app.focus = Focus::Editor;
    app
}

fn assert_auto_query(commands: &[Command]) {
    assert!(
        commands.iter().any(|command| matches!(
            command,
            Command::RunQuery { .. } | Command::RunQueryPage { .. }
        )),
        "{commands:?}"
    );
    assert!(
        !commands.iter().any(|command| matches!(
            command,
            Command::ManualBegin { .. } | Command::ManualExecute { .. }
        )),
        "{commands:?}"
    );
}

#[test]
fn saving_read_only_tidb_profile_reconnects_instead_of_reusing_writable_sessions() {
    let profile = tidb_profile();
    let mut app = connected_app(&profile);
    let target = ExecutionTarget::from_profile(&profile);
    let identity = app.sessions.start_attempt(target.clone()).unwrap();
    assert!(app.sessions.accept_success(
        identity,
        lazydb::db::ServerInfo {
            kind: profile.kind,
            version: "8.0.11-TiDB-v8.5.3".into(),
            database: "app".into(),
            current_user: None,
        },
        Default::default(),
    ));
    app.connection.generation = identity.generation;
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
    saved.read_only = true;
    let commands = app.update(Action::ProfileSaved {
        request_id,
        profile: saved,
        warning: None,
        change: lazydb::model::profile_manager::ProfileChange {
            connection_settings_changed: true,
            catalog_scope_changed: false,
            display_only_changed: false,
            credentials_changed: false,
        },
        connect: true,
    });
    assert!(commands.iter().any(|command| matches!(command,
        Command::Disconnect { connection } if *connection == identity)));
    assert!(commands.iter().any(|command| matches!(command,
        Command::Connect { profile_id, generation, .. }
            if *profile_id == profile.id && *generation > identity.generation)));
    assert!(app.sessions.is_retired(identity));
    assert_eq!(app.connection.status, ConnectionStatus::Connecting);
    assert_ne!(app.sessions.get(&target).unwrap().identity, identity);
}

#[test]
fn transaction_mode_uses_console_target_instead_of_active_connection_kind() {
    let tidb = tidb_profile();
    let mysql = import_connection_url("mysql://root@localhost/app", None)
        .unwrap()
        .profile;
    let mut app = connected_app(&mysql);
    app.profiles.push(tidb.clone());
    app.active_console_mut().execution_target = Some(ExecutionTarget::from_profile(&tidb));
    assert!(
        app.update(Action::SetTransactionMode(TransactionMode::Manual))
            .is_empty()
    );
    assert_eq!(app.active_console().transaction_mode, TransactionMode::Auto);
    assert!(!app.transaction_menu_availability()[1].0);

    app.connection.profile_id = Some(tidb.id);
    app.active_console_mut().execution_target = Some(ExecutionTarget::from_profile(&mysql));
    app.update(Action::SetTransactionMode(TransactionMode::Manual));
    assert_eq!(
        app.active_console().transaction_mode,
        TransactionMode::Manual
    );
    assert!(app.transaction_menu_availability()[1].0);
}

#[test]
fn tidb_explicit_transaction_sql_is_rejected_without_changing_console_state() {
    for sql in [
        "BEGIN",
        "START TRANSACTION",
        "COMMIT",
        "ROLLBACK",
        "SAVEPOINT point",
    ] {
        let mut app = connected_app(&tidb_profile());
        app.update(Action::ReplaceEditor(sql.into()));
        let commands = app.update(Action::RunActiveSql);
        assert!(commands.is_empty(), "{sql}: {commands:?}");
        assert_eq!(app.active_console().transaction_mode, TransactionMode::Auto);
        assert_eq!(
            app.active_console().transaction_state,
            TransactionState::Idle
        );
        app.update(Action::ReplaceEditor("SELECT 1".into()));
        assert_auto_query(&app.update(Action::RunActiveSql));
    }
}

#[test]
fn retained_tidb_manual_mode_is_normalized_before_ordinary_sql() {
    let mut app = connected_app(&tidb_profile());
    app.active_console_mut().transaction_mode = TransactionMode::Manual;
    app.update(Action::ReplaceEditor("SELECT 1".into()));
    assert_auto_query(&app.update(Action::RunActiveSql));
    assert_eq!(app.active_console().transaction_mode, TransactionMode::Auto);
    assert_eq!(
        app.active_console().transaction_state,
        TransactionState::Idle
    );
}

#[test]
fn rebinding_an_idle_manual_console_to_tidb_resets_live_and_persisted_mode() {
    let tidb = tidb_profile();
    let mysql = import_connection_url("mysql://root@localhost/app", None)
        .unwrap()
        .profile;
    let mut app = connected_app(&mysql);
    app.profiles.push(tidb.clone());
    app.update(Action::SetTransactionMode(TransactionMode::Manual));
    let console_id = app.active_console().id;
    let target = ExecutionTarget::from_profile(&tidb);
    app.overlay = Some(Overlay::TargetSelector {
        candidates: vec![TargetSelectorCandidate::Target(target.clone())],
        selected: 0,
        console_id: Some(console_id),
        execution: None,
    });
    app.update(Action::ConfirmTargetSelector);
    assert_eq!(
        app.active_console().execution_target.as_ref(),
        Some(&target)
    );
    assert_eq!(app.active_console().transaction_mode, TransactionMode::Auto);
    assert_eq!(
        app.sql_editors
            .iter()
            .find(|record| record.id == console_id)
            .unwrap()
            .transaction_mode,
        TransactionMode::Auto
    );
}

#[test]
fn restoring_tidb_manual_consoles_uses_auto_in_legacy_and_profile_workspaces() {
    let profile = tidb_profile();
    let id = Uuid::new_v4();
    let console = PersistedConsole {
        id,
        name: "TiDB console".into(),
        sql_file: format!("{id}.sql").into(),
        target: Some(ExecutionTarget::from_profile(&profile)),
        transaction_mode: TransactionMode::Manual,
        open: true,
    };
    for profile_workspace in [false, true] {
        let mut app = connected_app(&profile);
        app.restore_workspace(
            WorkspaceSnapshot {
                active_profile: Some(profile.id),
                profiles: if profile_workspace {
                    vec![PersistedProfileWorkspace {
                        profile_id: profile.id,
                        active_tab: Some(id),
                        consoles: vec![console.clone()],
                        tabs: vec![PersistedTab::Console { console_id: id }],
                    }]
                } else {
                    Vec::new()
                },
                active_console: id,
                consoles: if profile_workspace {
                    Vec::new()
                } else {
                    vec![console.clone()]
                },
                tabs: if profile_workspace {
                    Vec::new()
                } else {
                    vec![PersistedTab::Console { console_id: id }]
                },
                sql: vec![(id, "SELECT 1".into())],
                recent_targets: Vec::new(),
            },
            Some(profile.id),
        );
        assert_eq!(app.active_console().transaction_mode, TransactionMode::Auto);
        assert_eq!(
            app.sql_editors
                .iter()
                .find(|record| record.id == id)
                .unwrap()
                .transaction_mode,
            TransactionMode::Auto
        );
        assert_auto_query(&app.update(Action::RunActiveSql));
    }
}

#[test]
fn tidb_monitoring_is_unavailable_before_connection_or_dashboard_creation() {
    let profile = tidb_profile();
    let mut app = connected_app(&profile);
    app.explorer
        .normalized
        .select(lazydb::model::explorer::ExplorerNodeId::Profile(profile.id));
    assert!(!app.dashboard_supported());
    assert!(app.update(Action::OpenDashboard).is_empty());
    assert!(
        !app.tabs
            .iter()
            .any(|tab| matches!(tab, WorkspaceTab::Dashboard(_)))
    );

    let mut offline = App::new(vec![profile.clone()]);
    offline
        .explorer
        .normalized
        .select(lazydb::model::explorer::ExplorerNodeId::Profile(profile.id));
    assert!(offline.update(Action::OpenDashboard).is_empty());
    assert!(
        !offline
            .tabs
            .iter()
            .any(|tab| matches!(tab, WorkspaceTab::Dashboard(_)))
    );
}

fn preview() -> RelationPreview {
    RelationPreview {
        sql: "SELECT id FROM app.items".into(),
        result: QueryOutcome {
            result_sets: vec![ResultSet {
                columns: vec![ColumnMeta {
                    name: "id".into(),
                    type_name: "BIGINT".into(),
                }],
                rows: vec![vec![CellValue::Integer(1)]],
                affected_rows: 0,
            }],
            stats: QueryStats::new(Duration::ZERO, Duration::ZERO, 1),
        },
        pagination: ResultPagination::from_page(PageRequest::first(PageSize::default()), 1),
        row_versions: None,
    }
}

fn relation_fixture(app: &App, profile: &ConnectionProfile) -> (RelationTab, RelationRequest) {
    let key = RelationKey {
        profile_id: profile.id,
        object_id: CatalogId::new(profile.id, CatalogKind::Table, ["app", "app", "items"]),
    };
    let tab = RelationTab::with_descriptor(
        RelationDescriptor {
            key: key.clone(),
            qualified_name: QualifiedName {
                database: Some("app".into()),
                schema: Some("app".into()),
                object: "items".into(),
            },
            kind: CatalogKind::Table,
            title: "items".into(),
        },
        RelationView::Data,
    );
    let request = RelationRequest {
        tab_id: tab.id,
        console_id: app.active_console().id,
        tab_generation: tab.generation,
        request_id: 1,
        connection: ConnectionIdentity {
            profile_id: profile.id,
            generation: 1,
        },
        relation: key,
        kind: RelationRequestKind::Preview,
        scope: profile.catalog_scope.clone(),
        options: Default::default(),
        page: PageRequest::first(PageSize::default()),
    };
    (tab, request)
}

#[test]
fn tidb_preview_acceptance_keeps_rows_without_an_edit_session() {
    let profile = tidb_profile();
    let mut app = connected_app(&profile);
    let (mut tab, request) = relation_fixture(&app, &profile);
    tab.data = RelationLoad::Loading {
        request: request.clone(),
        previous: None,
    };
    app.tabs.push(WorkspaceTab::Relation(tab));
    app.active_tab = app.tabs.len() - 1;
    app.update(Action::RelationSucceeded {
        request,
        snapshot: Box::new(RelationSnapshot::Preview(preview())),
    });
    let WorkspaceTab::Relation(tab) = &app.tabs[app.active_tab] else {
        unreachable!()
    };
    assert!(
        matches!(&tab.data, RelationLoad::Ready(snapshot) if snapshot.value.result.result_sets[0].rows.len() == 1)
    );
    assert!(tab.edit.is_none());
}

#[test]
fn tidb_grid_writes_are_blocked_before_metadata_and_use_the_relation_profile() {
    let profile = tidb_profile();
    let mysql = import_connection_url("mysql://root@localhost/app", None)
        .unwrap()
        .profile;
    for failed in [false, true] {
        let mut app = connected_app(&mysql);
        app.profiles.push(profile.clone());
        let (mut tab, mut request) = relation_fixture(&app, &profile);
        tab.data = RelationLoad::Ready(OwnedSnapshot::new(
            preview(),
            request.connection,
            profile.catalog_scope.clone(),
        ));
        let mut edit = RelationEditSession::from_rows(vec![vec![CellValue::Integer(1)]]);
        edit.yank_row(0);
        tab.edit = Some(edit);
        request.kind = RelationRequestKind::Ddl;
        tab.ddl = if failed {
            RelationLoad::Failed {
                message: "no DDL privileges".into(),
                previous: None,
            }
        } else {
            RelationLoad::Loading {
                request,
                previous: None,
            }
        };
        let before = tab.edit.clone();
        app.tabs.push(WorkspaceTab::Relation(tab));
        app.active_tab = app.tabs.len() - 1;
        app.focus = Focus::Results;
        for action in [
            Action::RelationEditCell,
            Action::RelationInsertRow,
            Action::RelationDeleteCurrent,
            Action::RelationPaste,
            Action::RelationCommit,
        ] {
            assert!(app.update(action).is_empty());
            let WorkspaceTab::Relation(tab) = &app.tabs[app.active_tab] else {
                unreachable!()
            };
            assert_eq!(tab.edit, before);
            assert_eq!(tab.transaction_state, TransactionState::Idle);
        }
        for query in ["edit selected cell", "insert row", "delete row"] {
            app.update(Action::ShowHelp);
            app.update(Action::HelpPaste(query.into()));
            assert_eq!(
                app.help_selected_id(),
                None,
                "unexpected write shortcut for {query}"
            );
            app.overlay = None;
        }
    }
}
