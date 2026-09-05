use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use lazydb::{
    action::{Action, Command},
    app::App,
    cli::{Cli, ConfirmationPolicy},
    db::ServerInfo,
    model::workspace::{ConnectionIdentity, Overlay},
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
    app.update(Action::ConnectionSucceeded {
        profile_id: identity.profile_id,
        generation: identity.generation,
        server: ServerInfo {
            kind: lazydb::profile::DatabaseKind::Sqlite,
            version: "3.50".into(),
            database: ":memory:".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });
    app.connection.target = app.active_console().execution_target.clone();
    app
}

fn dispatch_editor_key(app: &mut App, keymap: &mut lazydb::input::keymap::Keymap, event: KeyEvent) {
    if let Some(action) = keymap.map(event, app) {
        app.update(action);
    }
}

#[test]
fn current_run_does_not_fall_back_to_the_whole_buffer() {
    let mut app = connected_app(ConfirmationPolicy::RiskyOnly);
    app.update(Action::ReplaceEditor("SELECT 1; SELECT 2;".into()));

    let commands = app.update(Action::RunActiveSql);
    assert!(
        matches!(commands.as_slice(), [Command::RunQueryPage { source_sql, page, .. }] if source_sql == "SELECT 1;" && *page == lazydb::model::pagination::PageRequest::first(lazydb::model::pagination::PageSize::default()))
    );
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
    assert!(
        matches!(commands.as_slice(), [Command::RunQueryPage { source_sql, .. }] if source_sql == "SELECT 1;")
    );
}

#[test]
fn normal_r_runs_current_statement_through_keymap() {
    let mut app = connected_app(ConfirmationPolicy::RiskyOnly);
    app.update(Action::ReplaceEditor("SELECT 1; SELECT 2;".into()));
    let mut keymap = lazydb::input::keymap::Keymap::default();

    let commands = {
        let event = KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE);
        let action = keymap.map(event, &app).expect("R should route to editor");
        app.update(action)
    };
    assert!(
        matches!(commands.as_slice(), [Command::RunQueryPage { source_sql, .. }] if source_sql == "SELECT 1;")
    );
}

#[test]
fn visual_r_runs_exact_selection_through_keymap() {
    let mut app = connected_app(ConfirmationPolicy::RiskyOnly);
    app.update(Action::ReplaceEditor("SELECT 1; SELECT 2;".into()));
    let mut keymap = lazydb::input::keymap::Keymap::default();

    for event in [
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
    ] {
        dispatch_editor_key(&mut app, &mut keymap, event);
    }

    let commands = {
        let event = KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE);
        let action = keymap.map(event, &app).expect("R should route to editor");
        app.update(action)
    };
    assert!(
        matches!(
            commands.as_slice(),
        [Command::RunQueryPage { source_sql, .. }] if source_sql == "SELECT 1"
        ),
        "unexpected commands: {commands:?}"
    );
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
fn savepoint_execution_uses_current_scope_instead_of_full_buffer() {
    let mut app = connected_app(ConfirmationPolicy::RiskyOnly);
    app.update(Action::ReplaceEditor(
        "SAVEPOINT checkpoint; SELECT 99;".into(),
    ));
    app.active_console_mut().transaction_mode = lazydb::model::transaction::TransactionMode::Manual;
    app.active_console_mut().transaction_state =
        lazydb::model::transaction::TransactionState::Active;

    let commands = app.update(Action::RunActiveSql);
    assert!(matches!(
        commands.as_slice(),
        [Command::ManualExecute { sql, .. }] if sql == "SAVEPOINT checkpoint;"
    ));
}

#[test]
fn cli_exposes_risky_and_always_execution_confirmation() {
    let risky =
        <Cli as clap::Parser>::try_parse_from(["lazydb", "--confirm-execution", "risky"]).unwrap();
    let always =
        <Cli as clap::Parser>::try_parse_from(["lazydb", "--confirm-execution", "always"]).unwrap();
    assert_eq!(risky.confirm_execution, Some(ConfirmationPolicy::RiskyOnly));
    assert_eq!(always.confirm_execution, Some(ConfirmationPolicy::Always));
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
    assert!(app.notifications.history().any(|notification| {
        notification.level == lazydb::model::notification::NotificationLevel::Warning
            && notification.body.contains("Select an execution target")
    }));

    let profile_id = app.connection.profile_id.unwrap();
    app.active_console_mut().execution_target =
        Some(lazydb::model::execution_target::ExecutionTarget {
            profile_id,
            database: ":memory:".into(),
            schema: Some("other".into()),
        });
    assert!(app.update(Action::RunActiveSql).is_empty());
    assert!(app.notifications.history().any(|notification| {
        notification.level == lazydb::model::notification::NotificationLevel::Warning
            && notification.body.contains("target")
    }));
}

#[test]
fn target_mismatch_is_reported_before_query_or_transaction_dispatch() {
    let mut app = connected_app(ConfirmationPolicy::RiskyOnly);
    let profile_id = app.connection.profile_id.unwrap();
    app.active_console_mut().execution_target =
        Some(lazydb::model::execution_target::ExecutionTarget {
            profile_id,
            database: ":memory:".into(),
            schema: Some("other".into()),
        });
    app.update(Action::ReplaceEditor("SELECT 1".into()));

    let commands = app.update(Action::RunActiveSql);

    assert!(commands.is_empty());
    assert!(app.overlay.is_none());
    assert!(app.notifications.history().any(|notification| {
        notification.level == lazydb::model::notification::NotificationLevel::Warning
            && notification.body.contains("SQL was not executed")
            && notification.body.contains("Space d")
    }));

    app.update(Action::ReplaceEditor("BEGIN;".into()));
    let commands = app.update(Action::RunActiveSql);
    assert!(commands.is_empty());
    assert_eq!(
        app.active_console().transaction_state,
        lazydb::model::transaction::TransactionState::Idle
    );
}

#[test]
fn base_execution_resets_page_and_invalidates_total() {
    let mut app = connected_app(ConfirmationPolicy::RiskyOnly);
    app.update(Action::ReplaceEditor("SELECT 1".into()));
    app.active_console_mut().pagination.offset = 500;
    app.active_console_mut().pagination.total = lazydb::model::pagination::TotalRows::Exact(501);
    let commands = app.update(Action::RunActiveSql);
    assert!(
        matches!(commands.as_slice(), [Command::RunQueryPage { page, .. }] if page.offset == 0 && !page.resolve_total)
    );
    assert_eq!(app.active_console().pagination.offset, 0);
    assert_eq!(
        app.active_console().pagination.total,
        lazydb::model::pagination::TotalRows::LowerBound(0)
    );
}

#[test]
fn stale_page_response_preserves_result_and_result_view() {
    let mut app = connected_app(ConfirmationPolicy::RiskyOnly);
    app.update(Action::ReplaceEditor("SELECT 1".into()));
    let tab_id = app.active_console().id;
    let connection = app.connection.active_identity().unwrap();
    app.active_console_mut().result_view = lazydb::model::tab::ResultView::Data;
    let before = app.active_console().clone();
    app.update(Action::QueryPageFailed {
        tab_id,
        generation: app.active_console().generation.saturating_add(1),
        connection,
        message: "stale".into(),
    });
    assert_eq!(app.active_console(), &before);
}

#[test]
fn page_size_change_requests_a_new_page_without_recounting_exact_total() {
    let mut app = connected_app(ConfirmationPolicy::RiskyOnly);
    app.update(Action::ReplaceEditor("SELECT 1".into()));
    let commands = app.update(Action::RunActiveSql);
    let (tab_id, generation, connection) = match commands.as_slice() {
        [
            Command::RunQueryPage {
                tab_id,
                generation,
                connection,
                ..
            },
        ] => (*tab_id, *generation, *connection),
        other => panic!("unexpected commands: {other:?}"),
    };
    app.update(Action::QueryPageFinished {
        tab_id,
        generation,
        connection,
        outcome: lazydb::db::query::QueryOutcome {
            result_sets: Vec::new(),
            stats: lazydb::db::query::QueryStats::new(
                std::time::Duration::ZERO,
                std::time::Duration::ZERO,
                0,
            ),
        },
        pagination: lazydb::model::pagination::ResultPagination {
            page_size: lazydb::model::pagination::PageSize::FiveHundred,
            offset: 0,
            visible_rows: 500,
            has_next: false,
            total: lazydb::model::pagination::TotalRows::Exact(500),
        },
    });
    let commands = app.update(Action::SetResultPageSize(
        lazydb::model::pagination::PageSize::Ten,
    ));
    assert!(matches!(
        commands.as_slice(),
        [Command::RunQueryPage { page, .. }] if page.size == lazydb::model::pagination::PageSize::Ten && !page.resolve_total
    ));
}
