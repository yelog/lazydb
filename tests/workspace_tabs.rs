use lazydb::model::transaction::TransactionMode;
use lazydb::persistence::workspace::{PersistedConsole, WorkspaceSnapshot};
use lazydb::profile::import_connection_url;
use lazydb::{
    action::{Action, Command},
    app::App,
    model::relation::RelationTab,
    model::tab::{ConsoleTab, TabKind, WorkspaceTab},
    model::workspace::Focus,
};
use uuid::Uuid;

#[test]
fn workspace_tabs_expose_common_identity() {
    let console = ConsoleTab::new("sql");
    let console_id = console.id;
    let relation = RelationTab::new("users");
    let relation_id = relation.id;

    let tabs = [WorkspaceTab::Sql(console), WorkspaceTab::Relation(relation)];
    assert_eq!(tabs[0].id(), console_id);
    assert_eq!(tabs[0].title(), "sql");
    assert_eq!(tabs[0].kind(), TabKind::Sql);
    assert_eq!(tabs[1].id(), relation_id);
    assert_eq!(tabs[1].title(), "users");
    assert_eq!(tabs[1].kind(), TabKind::Relation);
}

#[test]
fn relation_tabs_have_no_console_accessor() {
    let mut tab = WorkspaceTab::Relation(RelationTab::new("users"));
    assert!(tab.as_console().is_none());
    assert!(tab.as_console_mut().is_none());
}

#[test]
fn mixed_tabs_cycle_and_activate_without_sql_assumptions() {
    let mut app = App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));

    app.update(Action::NextTab);
    assert_eq!(app.active_tab, 1);
    app.update(Action::NextTab);
    assert_eq!(app.active_tab, 0);
    app.update(Action::PreviousTab);
    assert_eq!(app.active_tab, 1);
}

#[test]
fn cycling_relation_tabs_normalizes_editor_focus() {
    let mut app = App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.focus = Focus::Editor;

    app.update(Action::NextTab);

    assert_eq!(app.active_tab, 1);
    assert_eq!(app.focus, Focus::Results);

    app.focus = Focus::Editor;
    app.update(Action::PreviousTab);
    assert_eq!(app.active_tab, 0);
    assert_eq!(app.focus, Focus::Editor);
}

#[test]
fn closing_relation_tab_bypasses_transaction_exit() {
    let mut app = App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;

    let commands = app.update(Action::CloseActiveTab);

    assert!(
        commands.is_empty()
            || commands
                .iter()
                .any(|command| { matches!(command, lazydb::action::Command::PersistWorkspace(_)) })
    );
    assert_eq!(app.tabs.len(), 1);
}

#[test]
fn activating_relation_normalizes_editor_focus() {
    let mut app = App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.focus = Focus::Editor;

    app.update(Action::ActivateTab(1));

    assert_eq!(app.active_tab, 1);
    assert_ne!(app.focus, Focus::Editor);
}

#[test]
fn closing_final_sql_console_creates_a_replacement_editor() {
    let mut app = App::new(Vec::new());
    let commands = app.update(Action::CloseActiveTab);

    assert_eq!(app.tabs.len(), 1);
    assert!(app.tabs.iter().any(|tab| tab.kind() == TabKind::Sql));
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, Command::PersistWorkspace(_)))
    );
}

#[test]
fn closing_and_reopening_sql_editor_preserves_persisted_text_and_target() {
    let profile = import_connection_url(":memory:", Some("active"))
        .unwrap()
        .profile;
    let expected = lazydb::model::execution_target::ExecutionTarget::from_profile(&profile);
    let mut app = App::new(vec![profile.clone()]);
    let editor_id = app.active_console().id;
    app.update(Action::ReplaceEditor("select 42".into()));
    app.update(Action::CloseActiveTab);

    assert!(
        !app.sql_editors
            .iter()
            .find(|record| record.id == editor_id)
            .unwrap()
            .open
    );
    assert!(app.tabs.iter().all(|tab| tab.id() != editor_id));
    assert_eq!(app.editor_text(editor_id).unwrap(), "select 42");

    app.update(Action::ActivateSqlEditor(editor_id));

    assert_eq!(app.active_console().id, editor_id);
    assert_eq!(app.active_editor_text().unwrap(), "select 42");
    assert_eq!(
        app.active_console().execution_target.as_ref(),
        Some(&expected)
    );
}

#[test]
fn deleting_sql_editor_requires_confirmation_and_removes_record() {
    let mut app = App::new(Vec::new());
    let editor_id = app.active_console().id;

    assert!(app.update(Action::RequestDeleteActiveConsole).is_empty());
    assert!(matches!(
        app.overlay,
        Some(lazydb::model::workspace::Overlay::DeleteConsole { console_id })
            if console_id == editor_id
    ));

    app.update(Action::CancelDeleteConsole);
    assert!(app.sql_editors.iter().any(|record| record.id == editor_id));

    app.update(Action::RequestDeleteActiveConsole);
    app.update(Action::ConfirmDeleteConsole);
    assert!(!app.sql_editors.iter().any(|record| record.id == editor_id));
    assert!(app.tabs.iter().all(|tab| tab.id() != editor_id));
    assert!(app.active_console_opt().is_some());
}

#[test]
fn sql_editor_list_reopens_a_hidden_editor() {
    let mut app = App::new(Vec::new());
    let editor_id = app.active_console().id;
    app.update(Action::ReplaceEditor("select 7".into()));
    app.update(Action::CloseActiveTab);

    app.update(Action::OpenSqlEditorList);
    app.update(Action::ActivateSqlEditor(editor_id));

    assert_eq!(app.active_console().id, editor_id);
    assert_eq!(app.active_editor_text().unwrap(), "select 7");
}

#[test]
fn workspace_restore_keeps_hidden_editors_hidden() {
    let id = Uuid::new_v4();
    let snapshot = WorkspaceSnapshot {
        active_console: Uuid::nil(),
        consoles: vec![PersistedConsole {
            id,
            name: "closed".into(),
            sql_file: format!("{id}.sql").into(),
            target: None,
            transaction_mode: TransactionMode::Auto,
            open: false,
        }],
        sql: vec![(id, "select 9".into())],
    };
    let mut app = App::new(Vec::new());
    app.restore_workspace(snapshot, None);

    assert!(
        app.sql_editors
            .iter()
            .any(|record| record.id == id && !record.open)
    );
    assert!(app.tabs.iter().all(|tab| tab.id() != id));
}

#[test]
fn sql_only_actions_are_noops_on_relation_tabs() {
    let mut app = App::new(Vec::new());
    app.tabs
        .push(WorkspaceTab::Relation(RelationTab::new("users")));
    app.active_tab = 1;

    for action in [
        Action::CancelActiveQuery,
        Action::SetTransactionMode(lazydb::model::transaction::TransactionMode::Manual),
        Action::CommitTransaction,
        Action::RollbackTransaction,
        Action::ClearTransactionOutcome,
        Action::GridMove {
            rows: 1,
            columns: 1,
        },
        Action::GridSelect { row: 1, column: 1 },
        Action::ToggleResultView,
        Action::CompletionExplicit,
        Action::CompletionNext,
        Action::CompletionPrevious,
        Action::CompletionAccept,
        Action::CompletionDismiss,
        Action::RunActiveSql,
        Action::RunAllSql,
        Action::ReplaceEditor("select 1".to_owned()),
    ] {
        assert!(app.update(action).is_empty());
    }
}

#[test]
fn initial_and_new_consoles_use_the_active_profile_target() {
    let profile = import_connection_url(":memory:", Some("active"))
        .unwrap()
        .profile;
    let expected = lazydb::model::execution_target::ExecutionTarget::from_profile(&profile);
    let mut app = App::new(vec![profile.clone()]);
    assert_eq!(
        app.active_console().execution_target.as_ref(),
        Some(&expected)
    );

    app.connection.profile_id = Some(profile.id);
    app.update(Action::NewConsole);
    assert_eq!(
        app.active_console().execution_target.as_ref(),
        Some(&expected)
    );
}

#[test]
fn workspace_restore_preserves_valid_targets_and_defaults_missing_targets() {
    let profile = import_connection_url(":memory:", Some("active"))
        .unwrap()
        .profile;
    let expected = lazydb::model::execution_target::ExecutionTarget::from_profile(&profile);
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let snapshot = WorkspaceSnapshot {
        active_console: second,
        consoles: vec![
            PersistedConsole {
                id: first,
                name: "saved".into(),
                sql_file: format!("{first}.sql").into(),
                target: Some(expected.clone()),
                transaction_mode: TransactionMode::Auto,
                open: true,
            },
            PersistedConsole {
                id: second,
                name: "missing".into(),
                sql_file: format!("{second}.sql").into(),
                target: None,
                transaction_mode: TransactionMode::Manual,
                open: true,
            },
        ],
        sql: vec![(first, "select 1".into()), (second, "select 2".into())],
    };
    let mut app = App::new(vec![profile.clone()]);
    app.restore_workspace(snapshot, Some(profile.id));

    assert_eq!(app.active_console().id, second);
    assert_eq!(
        app.active_console().execution_target.as_ref(),
        Some(&expected)
    );
    assert_eq!(
        app.active_console().transaction_mode,
        TransactionMode::Manual
    );
    assert_eq!(app.active_editor_text().unwrap(), "select 2");
    assert_eq!(
        app.tabs[0].as_console().unwrap().execution_target.as_ref(),
        Some(&expected)
    );
}
