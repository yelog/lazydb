use lazydb::{
    action::{Action, Command},
    app::App,
    db::ServerInfo,
    model::{execution_target::ExecutionTarget, workspace::Overlay},
    profile::{DatabaseKind, import_connection_url},
};

fn console(read_only: bool) -> (Box<App>, uuid::Uuid) {
    let mut profile = import_connection_url("elasticsearch://localhost:9200", Some("search"))
        .unwrap()
        .profile;
    profile.read_only = read_only;
    let id = profile.id;
    let mut app = Box::new(App::new(vec![profile]));
    app.update(Action::NewUnboundConsoleNamed("REST".into()));
    let console_id = app.active_console().id;
    let target = ExecutionTarget::from_profile(app.profiles.first().unwrap());
    app.update(Action::OpenConsoleTargetSelector { console_id });
    let index = match app.overlay.as_ref().unwrap() {
        Overlay::TargetSelector { candidates, .. } => candidates
            .iter()
            .position(|candidate| candidate.target() == Some(&target))
            .expect("Elasticsearch is a console target"),
        other => panic!("{other:?}"),
    };
    app.update(Action::SelectTargetSelector(index));
    (app, id)
}

fn connected(app: &mut App, profile_id: uuid::Uuid) -> Vec<Command> {
    let generation = app
        .connection
        .pending_generation
        .expect("connect requested");
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: ServerInfo {
            kind: DatabaseKind::Elasticsearch,
            version: "8.17.0".into(),
            database: "default".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    })
}

#[test]
fn elasticsearch_lazy_console_executes_the_complete_native_request() {
    let (mut app, profile_id) = console(false);
    let request = "POST /books/_search\n{\"query\":{\"query_string\":{\"query\":\"title:rust;edition:2024\"}}}";
    app.update(Action::ReplaceEditor(request.into()));
    let mut commands = app.update(Action::RunActiveSql);
    if app.connection.pending_generation.is_some() {
        commands.extend(connected(&mut app, profile_id));
    }
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, Command::RunQuery {sql,..} if sql == request)),
        "{commands:?}"
    );
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::RunQueryPage { .. }))
    );
    assert!(!matches!(
        app.overlay,
        Some(Overlay::ExecutionConfirm { .. })
    ));
}

#[test]
fn unbound_console_resolves_the_full_rest_buffer_after_target_selection() {
    let profile = import_connection_url("elasticsearch://localhost:9200", Some("search"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let target = ExecutionTarget::from_profile(&profile);
    let mut app = Box::new(App::new(vec![profile]));
    app.update(Action::NewUnboundConsoleNamed("REST".into()));
    let request = "POST /books/_search?q=title:rust;edition:2024\n{\"query\":{\"match_all\":{}}}";
    app.update(Action::ReplaceEditor(request.into()));
    let mut commands = app.update(Action::RunActiveSql);
    let index = match app.overlay.as_ref().unwrap() {
        Overlay::TargetSelector { candidates, .. } => candidates
            .iter()
            .position(|candidate| candidate.target() == Some(&target))
            .unwrap(),
        other => panic!("{other:?}"),
    };
    commands.extend(app.update(Action::SelectTargetSelector(index)));
    if app.connection.pending_generation.is_some() {
        commands.extend(connected(&mut app, profile_id));
    }
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, Command::RunQuery { sql, .. } if sql == request)),
        "{commands:?}"
    );
}

#[test]
fn elasticsearch_write_requires_confirmation_and_read_only_blocks_it() {
    for read_only in [false, true] {
        let (mut app, profile_id) = console(read_only);
        app.update(Action::ReplaceEditor(
            "PUT /books/_doc/1\n{\"title\":\"Rust\"}".into(),
        ));
        let mut commands = app.update(Action::RunActiveSql);
        if app.connection.pending_generation.is_some() {
            commands.extend(connected(&mut app, profile_id));
        }
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, Command::RunQuery { .. }))
        );
        assert_eq!(
            matches!(app.overlay, Some(Overlay::ExecutionConfirm { .. })),
            !read_only
        );
    }
}

#[test]
fn elasticsearch_confirmation_rechecks_profile_read_only_and_defaults_to_cancel() {
    use lazydb::model::workspace::ExecutionConfirmFocus;
    let (mut app, profile_id) = console(false);
    app.update(Action::ReplaceEditor("DELETE /books".into()));
    app.update(Action::RunActiveSql);
    if app.connection.pending_generation.is_some() {
        connected(&mut app, profile_id);
    }
    assert!(matches!(
        app.overlay,
        Some(Overlay::ExecutionConfirm {
            focus: ExecutionConfirmFocus::Cancel,
            ..
        })
    ));
    app.profiles
        .iter_mut()
        .find(|profile| profile.id == profile_id)
        .unwrap()
        .read_only = true;
    app.update(Action::ToggleExecutionConfirmationFocus);
    let commands = app.update(Action::ConfirmExecution);
    assert!(!commands.iter().any(|command| matches!(
        command,
        Command::RunQuery { .. } | Command::RunQueryPage { .. }
    )));
    assert!(app.active_console().last_execution.is_none());
}

#[test]
fn elasticsearch_confirmation_describes_native_request_and_no_transaction() {
    use lazydb::ui::{self, UiState};
    use ratatui::{Terminal, backend::TestBackend};

    let (mut app, profile_id) = console(false);
    app.update(Action::ReplaceEditor(
        "PUT /books/_doc/1\n{\"title\":\"Rust\"}".into(),
    ));
    app.update(Action::RunActiveSql);
    if app.connection.pending_generation.is_some() {
        connected(&mut app, profile_id);
    }
    let mut terminal = Terminal::new(TestBackend::new(130, 45)).unwrap();
    let mut state = UiState::default();
    terminal
        .draw(|frame| ui::render_with_state(frame, &app, &mut state))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Execute REST request?"));
    assert!(text.contains("One complete REST request"));
    assert!(text.contains("REST preview"));
    assert!(text.contains("cannot be rolled back here"));
    assert!(!text.contains("Execute selected SQL?"));
}

#[test]
fn elasticsearch_response_failure_retains_result_and_shows_error_summary() {
    use lazydb::{
        db::query::{ColumnMeta, QueryOutcome, QueryStats, ResultSet},
        db::value::CellValue,
        model::{
            tab::{ExecutionResult, OutputKind, ResultView},
            workspace::QueryStatus,
        },
    };
    let (mut app, profile_id) = console(false);
    app.update(Action::ReplaceEditor("POST /_msearch\n{}\n{}".into()));
    let mut commands = app.update(Action::RunActiveSql);
    if app.connection.pending_generation.is_some() {
        commands.extend(connected(&mut app, profile_id));
    }
    let (tab_id, generation, connection) = commands
        .iter()
        .find_map(|command| match command {
            Command::RunQuery {
                tab_id,
                generation,
                connection,
                ..
            } => Some((*tab_id, *generation, *connection)),
            _ => None,
        })
        .unwrap();
    let message = "Elasticsearch msearch: 1 of 2 search(es) failed; inspect response.responses.";
    let mut stats = QueryStats::new(std::time::Duration::ZERO, std::time::Duration::ZERO, 1);
    stats.response_error = Some(message.into());
    app.update(Action::QueryFinished {
        tab_id,
        generation,
        connection,
        outcome: QueryOutcome {
            stats,
            result_sets: vec![ResultSet {
                columns: vec![ColumnMeta {
                    name: "response".into(),
                    type_name: "JSON".into(),
                }],
                rows: vec![vec![CellValue::Text(
                    "{\"responses\":[{}, {\"error\":{}}]}".into(),
                )]],
                affected_rows: 0,
            }],
        },
    });
    let tab = app.active_console();
    assert_eq!(tab.query_status, QueryStatus::Failed);
    assert_eq!(
        tab.last_execution.as_ref().unwrap().result,
        ExecutionResult::Failed
    );
    assert_eq!(tab.result_view, ResultView::Output);
    assert_eq!(tab.outcome.as_ref().unwrap().result_sets[0].rows.len(), 1);
    assert!(
        tab.output
            .iter()
            .any(|entry| entry.kind == OutputKind::Error && entry.message.contains(message))
    );
}

#[test]
fn elasticsearch_agent_policy_uses_rest_classification() {
    use lazydb::agent::policy::{WritePolicy, authorize_query, authorize_write};
    let mut profile = import_connection_url("elasticsearch://localhost", None)
        .unwrap()
        .profile;
    assert!(authorize_query(&profile, "POST /books/_search\n{}").is_ok());
    assert!(authorize_query(&profile, "POST /books/_delete_by_query\n{}").is_err());
    assert!(authorize_write(&profile, WritePolicy::Deny, "DELETE /books").is_err());
    assert!(authorize_write(&profile, WritePolicy::NonProduction, "DELETE /books").is_ok());
    profile.read_only = true;
    assert!(authorize_write(&profile, WritePolicy::All, "DELETE /books").is_err());
}
