use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use lazydb::{
    action::{Action, Command},
    app::App,
    cli::{Cli, ConfirmationPolicy},
    model::workspace::{ConnectionIdentity, ConnectionStatus, Overlay},
    profile::import_connection_url,
    sql::{ExecutionDraft, ScopeKind, ScopeSource, SqlDialect, SqlRisk, TextRange},
};

fn connected_app(policy: ConfirmationPolicy) -> App {
    let profile = import_connection_url(":memory:", Some("test"))
        .unwrap()
        .profile;
    let identity = ConnectionIdentity {
        profile_id: profile.id,
        generation: 1,
    };
    let mut app = App::with_confirmation_policy(vec![profile], policy);
    app.connection.profile_id = Some(identity.profile_id);
    app.connection.generation = identity.generation;
    app.connection.status = ConnectionStatus::Connected;
    app.connection.target = app.active_console().execution_target.clone();
    app
}

#[test]
fn current_run_does_not_fall_back_to_the_whole_buffer() {
    let mut app = connected_app(ConfirmationPolicy::RiskyOnly);
    app.update(Action::ReplaceEditor("SELECT 1; SELECT 2;".into()));

    let commands = app.update(Action::RunActiveSql);
    assert!(matches!(commands.as_slice(), [Command::RunQuery { sql, .. }] if sql == "SELECT 1;"));
}

#[test]
fn current_run_executes_statement_when_cursor_is_on_internal_space() {
    let mut app = connected_app(ConfirmationPolicy::RiskyOnly);
    app.update(Action::ReplaceEditor("SELECT 1; SELECT 2;".into()));
    for _ in 0.."SELECT".len() {
        app.update(Action::EditorKey(KeyEvent::new(
            KeyCode::Char('l'),
            KeyModifiers::NONE,
        )));
    }

    let commands = app.update(Action::RunActiveSql);

    assert!(matches!(
        commands.as_slice(),
        [Command::RunQuery { sql, .. }] if sql == "SELECT 1;"
    ));
}

#[test]
fn full_run_is_explicit_and_starts_with_cancel_focused() {
    let mut app = connected_app(ConfirmationPolicy::RiskyOnly);
    app.update(Action::ReplaceEditor("SELECT 1; SELECT 2;".into()));

    assert!(app.update(Action::RunAllSql).is_empty());
    assert!(matches!(
        app.overlay,
        Some(Overlay::ExecutionConfirm { ref draft, focus: lazydb::model::workspace::ExecutionConfirmFocus::Cancel })
            if draft.scope == ScopeKind::FullBuffer && draft.sql == "SELECT 1; SELECT 2;"
    ));
}

#[test]
fn confirmation_uses_the_immutable_sql_snapshot() {
    let mut app = connected_app(ConfirmationPolicy::Always);
    app.update(Action::ReplaceEditor(
        "UPDATE users SET name = 'raw';".into(),
    ));
    app.update(Action::RunActiveSql);

    app.update(Action::ToggleExecutionConfirmationFocus);
    let commands = app.update(Action::ConfirmExecution);
    assert!(
        matches!(commands.as_slice(), [Command::RunQuery { sql, .. }] if sql == "UPDATE users SET name = 'raw';")
    );
    assert_eq!(
        app.active_console()
            .last_execution
            .as_ref()
            .unwrap()
            .draft
            .sql,
        "UPDATE users SET name = 'raw';"
    );
}

#[test]
fn bare_begin_enters_manual_without_pool_sql() {
    let mut app = connected_app(ConfirmationPolicy::RiskyOnly);
    app.update(Action::ReplaceEditor("BEGIN;".into()));
    let commands = app.update(Action::RunActiveSql);
    assert!(matches!(commands.as_slice(), [Command::ManualBegin { .. }]));
    assert_eq!(
        app.active_console().transaction_mode,
        lazydb::model::transaction::TransactionMode::Manual
    );
    assert_eq!(
        app.active_console().transaction_state,
        lazydb::model::transaction::TransactionState::Starting
    );
}

#[test]
fn cli_exposes_risky_and_always_execution_confirmation() {
    let risky =
        <Cli as clap::Parser>::try_parse_from(["lazydb", "--confirm-execution", "risky"]).unwrap();
    let always =
        <Cli as clap::Parser>::try_parse_from(["lazydb", "--confirm-execution", "always"]).unwrap();
    assert_eq!(risky.confirm_execution, ConfirmationPolicy::RiskyOnly);
    assert_eq!(always.confirm_execution, ConfirmationPolicy::Always);
}

#[test]
fn execution_draft_classifies_and_preserves_exact_sql() {
    let sql = "UPDATE t SET value = 'x';".to_owned();
    let draft = ExecutionDraft::new(
        uuid::Uuid::nil(),
        4,
        ConnectionIdentity {
            profile_id: uuid::Uuid::nil(),
            generation: 2,
        },
        lazydb::model::execution_target::ExecutionTarget {
            profile_id: uuid::Uuid::nil(),
            database: "db".into(),
            schema: Some("main".into()),
        },
        0,
        7,
        ScopeKind::CurrentStatement,
        ScopeSource::Contiguous(TextRange::new(0, sql.len())),
        sql.clone(),
        SqlDialect::Sqlite,
        Default::default(),
        Default::default(),
    );
    assert_eq!(draft.sql, sql);
    assert_eq!(draft.risks, vec![SqlRisk::Dml]);
    assert!(draft.requires_confirmation(false));
}

#[test]
fn confirmation_keymap_accepts_enter_execute_and_escape_cancel() {
    let mut app = connected_app(ConfirmationPolicy::Always);
    app.update(Action::ReplaceEditor("SELECT 1".into()));
    app.update(Action::RunActiveSql);
    let mut keymap = lazydb::input::keymap::Keymap::default();
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE), &app),
        Some(Action::ConfirmExecution)
    );
}

#[test]
fn execution_fails_closed_when_console_target_is_missing_or_stale() {
    let mut app = connected_app(ConfirmationPolicy::RiskyOnly);
    app.update(Action::ReplaceEditor("SELECT 1".into()));
    app.active_console_mut().execution_target = None;
    assert!(app.update(Action::RunActiveSql).is_empty());
    assert!(
        app.connection
            .error
            .as_deref()
            .unwrap()
            .contains("Select an execution target")
    );

    let profile_id = app.connection.profile_id.unwrap();
    app.active_console_mut().execution_target =
        Some(lazydb::model::execution_target::ExecutionTarget {
            profile_id,
            database: ":memory:".into(),
            schema: Some("other".into()),
        });
    assert!(app.update(Action::RunActiveSql).is_empty());
    assert!(app.connection.error.as_deref().unwrap().contains("target"));
}
