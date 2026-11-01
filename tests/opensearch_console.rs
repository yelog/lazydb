use lazydb::{
    action::{Action, Command},
    app::App,
    db::ServerInfo,
    model::{execution_target::ExecutionTarget, workspace::Overlay},
    profile::{DatabaseKind, import_connection_url},
};

fn console(read_only: bool) -> (Box<App>, uuid::Uuid) {
    let mut profile = import_connection_url("opensearch://localhost:9200", Some("search"))
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
            .expect("OpenSearch is a console target"),
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
            kind: DatabaseKind::OpenSearch,
            version: "2.19.0".into(),
            database: "default".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    })
}

fn search_catalog_app(scheme: &str) -> (Box<App>, lazydb::db::catalog::CatalogId) {
    use lazydb::db::catalog::{
        CatalogEntry, CatalogId, CatalogKind, OptionalMetadata, QualifiedName,
    };

    let profile = import_connection_url(&format!("{scheme}://localhost:9200"), Some("search"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let kind = profile.kind;
    let mut app = Box::new(App::new(vec![profile]));
    app.update(Action::RequestConnect(profile_id));
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation: app.connection.pending_generation.unwrap(),
        server: ServerInfo {
            kind,
            version: "test".into(),
            database: "default".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });
    let database = CatalogId::new(profile_id, CatalogKind::Database, ["default"]);
    let schema = CatalogId::new(profile_id, CatalogKind::Schema, ["default", "default"]);
    let relation = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        ["default", "default", "books"],
    );
    let name = |schema: Option<&str>, object: &str| QualifiedName {
        database: Some("default".into()),
        schema: schema.map(str::to_owned),
        object: object.into(),
    };
    app.explorer
        .normalized
        .profiles
        .get_mut(&profile_id)
        .unwrap()
        .catalog
        .insert_subtree(vec![
            CatalogEntry::database(
                database.clone(),
                name(None, "default"),
                "database",
                OptionalMetadata::Unsupported,
                true,
            )
            .unwrap(),
            CatalogEntry::schema(
                schema.clone(),
                database,
                name(Some("default"), "default"),
                "schema",
                OptionalMetadata::Unsupported,
                true,
            )
            .unwrap(),
            CatalogEntry::relation(
                relation.clone(),
                schema,
                name(Some("default"), "books"),
                "index",
                OptionalMetadata::Unsupported,
                true,
            )
            .unwrap(),
        ])
        .unwrap();
    (app, relation)
}

#[test]
fn search_explorer_ddl_shortcut_opens_and_reuses_document_view() {
    use lazydb::model::{explorer::ExplorerNodeId, relation::RelationView, tab::WorkspaceTab};

    for scheme in ["opensearch", "elasticsearch"] {
        let (mut app, relation) = search_catalog_app(scheme);
        app.explorer.normalized.selected = Some(ExplorerNodeId::Catalog(relation.clone()));
        // Explorer's D shortcut dispatches this semantic action.
        for _ in 0..2 {
            let commands = app.update(Action::OpenSelectedRelation {
                view: RelationView::Ddl,
            });
            let WorkspaceTab::Relation(tab) = &app.tabs[app.active_tab] else {
                panic!("index tab expected");
            };
            assert_eq!(tab.view, RelationView::Data, "{scheme}");
            assert_eq!(tab.descriptor.key.object_id, relation);
            assert!(
                !commands
                    .iter()
                    .any(|command| matches!(command, Command::LoadRelationDdl(_)))
            );
            assert!(matches!(
                tab.data,
                lazydb::model::relation::RelationLoad::Loading { .. }
            ));
        }
        assert_eq!(
            app.tabs
                .iter()
                .filter(|tab| matches!(tab, WorkspaceTab::Relation(_)))
                .count(),
            1
        );
    }
}

#[test]
fn restored_search_ddl_tabs_use_document_view_and_load_preview() {
    use lazydb::{
        model::{relation::RelationView, tab::WorkspaceTab},
        persistence::workspace::PersistedTab,
    };

    for scheme in ["opensearch", "elasticsearch"] {
        let (mut app, relation) = search_catalog_app(scheme);
        app.update(Action::OpenCatalogRelation {
            id: relation.clone(),
            view: RelationView::Data,
        });
        let tab_id = app.tabs[app.active_tab].id();
        let mut snapshot = app.workspace_snapshot();
        for workspace in &mut snapshot.profiles {
            for tab in &mut workspace.tabs {
                if let PersistedTab::Relation(tab) = tab {
                    tab.view = RelationView::Ddl;
                }
            }
        }
        let profiles = app.profiles.clone();
        let profile_id = relation.profile_id();
        let kind = profiles[0].kind;
        let mut restored = Box::new(App::new(profiles));
        restored.restore_workspace(snapshot, Some(profile_id));
        let tab = restored.tabs.iter().find(|tab| tab.id() == tab_id).unwrap();
        assert!(
            matches!(tab, WorkspaceTab::Relation(tab) if tab.view == RelationView::Data),
            "{scheme}"
        );
        restored.update(Action::RequestConnect(profile_id));
        let mut commands = restored.update(Action::ConnectionSucceeded {
            profile_id,
            generation: restored.connection.pending_generation.unwrap(),
            server: ServerInfo {
                kind,
                version: "test".into(),
                database: "default".into(),
                current_user: None,
            },
            mutation_capabilities: Default::default(),
        });
        restored.active_tab = restored
            .tabs
            .iter()
            .position(|tab| tab.id() == tab_id)
            .unwrap();
        commands.extend(restored.update(Action::RefreshActiveRelation));
        assert!(commands.iter().any(|command| matches!(command, Command::LoadRelationPreview(request) if request.relation.object_id == relation)), "{scheme}: {commands:?}");
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, Command::LoadRelationDdl(_)))
        );
    }
}

#[test]
fn opensearch_lazy_console_executes_the_complete_native_request() {
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
    let profile = import_connection_url("opensearch://localhost:9200", Some("search"))
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
fn opensearch_write_requires_confirmation_and_read_only_blocks_it() {
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
fn opensearch_confirmation_rechecks_profile_read_only_and_defaults_to_cancel() {
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
fn opensearch_confirmation_describes_native_request_and_no_transaction() {
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
    assert!(text.contains("sending it to OpenSearch"));
    assert!(!text.contains("Elasticsearch"));
    assert!(text.contains("cannot be rolled back here"));
    assert!(!text.contains("Execute selected SQL?"));
}

#[test]
fn opensearch_response_failure_retains_result_and_shows_error_summary() {
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
    let message = "OpenSearch msearch: 1 of 2 search(es) failed; inspect response.responses.";
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
fn opensearch_agent_policy_uses_rest_classification() {
    use lazydb::agent::policy::{WritePolicy, authorize_query, authorize_write};
    let mut profile = import_connection_url("opensearch://localhost", None)
        .unwrap()
        .profile;
    assert!(authorize_query(&profile, "POST /books/_search\n{}").is_ok());
    assert!(authorize_query(&profile, "POST /books/_delete_by_query\n{}").is_err());
    assert!(authorize_write(&profile, WritePolicy::Deny, "DELETE /books").is_err());
    assert!(authorize_write(&profile, WritePolicy::NonProduction, "DELETE /books").is_ok());
    profile.read_only = true;
    assert!(authorize_write(&profile, WritePolicy::All, "DELETE /books").is_err());
}

#[test]
fn opensearch_confirmed_write_keeps_request_snapshot_and_owner() {
    let (mut app, profile_id) = console(false);
    let request = "PUT /books/_doc/1\n{\"title\":\"Rust; 2024\"}";
    app.update(Action::ReplaceEditor(request.into()));
    app.update(Action::RunActiveSql);
    if app.connection.pending_generation.is_some() {
        connected(&mut app, profile_id);
    }
    let draft = match app.overlay.as_ref().unwrap() {
        Overlay::ExecutionConfirm { draft, .. } => draft.clone(),
        other => panic!("{other:?}"),
    };
    app.update(Action::ToggleExecutionConfirmationFocus);
    let commands = app.update(Action::ConfirmExecution);
    assert!(commands.iter().any(|command| matches!(command,
        Command::RunQuery { sql, target, connection, console_id, .. }
        if sql == request && target == &draft.target && connection == &draft.connection && console_id == &draft.console_id
    )), "{commands:?}");
}

#[test]
fn opensearch_confirmation_rejects_document_and_target_changes() {
    for change_target in [false, true] {
        let (mut app, profile_id) = console(false);
        app.update(Action::ReplaceEditor("DELETE /books".into()));
        app.update(Action::RunActiveSql);
        if app.connection.pending_generation.is_some() {
            connected(&mut app, profile_id);
        }
        if change_target {
            app.active_console_mut().execution_target = None;
        } else {
            app.update(Action::ReplaceEditor("DELETE /other".into()));
        }
        app.update(Action::ToggleExecutionConfirmationFocus);
        let commands = app.update(Action::ConfirmExecution);
        assert!(!commands.iter().any(|command| matches!(
            command,
            Command::RunQuery { .. } | Command::RunQueryPage { .. }
        )));
    }
}

#[test]
fn opensearch_query_result_rejects_stale_generation_and_wrong_connection_owner() {
    use lazydb::{
        db::query::{QueryOutcome, QueryStats},
        model::workspace::QueryStatus,
    };
    let (mut app, profile_id) = console(false);
    app.update(Action::ReplaceEditor("GET /books/_search".into()));
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
    let outcome = || QueryOutcome {
        result_sets: Vec::new(),
        stats: QueryStats::new(std::time::Duration::ZERO, std::time::Duration::ZERO, 0),
    };
    let mut wrong_owner = connection;
    wrong_owner.profile_id = uuid::Uuid::new_v4();
    for (reply_generation, reply_owner) in [(generation + 1, connection), (generation, wrong_owner)]
    {
        app.update(Action::QueryFinished {
            tab_id,
            generation: reply_generation,
            connection: reply_owner,
            outcome: outcome(),
        });
        assert_eq!(app.active_console().query_status, QueryStatus::Running);
        assert!(app.active_console().outcome.is_none());
        app.update(Action::QueryFailed {
            tab_id,
            generation: reply_generation,
            connection: reply_owner,
            message: "stale failure".into(),
        });
        assert_eq!(app.active_console().query_status, QueryStatus::Running);
    }
    app.update(Action::QueryFinished {
        tab_id,
        generation,
        connection,
        outcome: outcome(),
    });
    assert_eq!(app.active_console().query_status, QueryStatus::Idle);
    assert!(app.active_console().outcome.is_some());
}

#[test]
fn opensearch_policy_fails_closed_for_sql_unknown_paths_and_production_writes() {
    use lazydb::{
        agent::policy::{PolicyError, WritePolicy, authorize_query, authorize_write},
        profile::Environment,
    };
    let mut profile = import_connection_url("opensearch://localhost", None)
        .unwrap()
        .profile;
    for text in [
        "SELECT 1",
        "GET /_search/../_refresh",
        "GET /books/%5fsearch",
        "GET /_unknown",
        "POST /books/_delete_by_query\n{}",
    ] {
        assert!(authorize_query(&profile, text).is_err(), "{text}");
    }
    assert!(authorize_write(&profile, WritePolicy::All, "SELECT 1").is_err());
    assert!(authorize_write(&profile, WritePolicy::All, "GET /books/_search").is_err());
    profile.environment = Environment::Production;
    assert_eq!(
        authorize_write(&profile, WritePolicy::NonProduction, "DELETE /books"),
        Err(PolicyError::ProductionWriteDisabled)
    );
    assert!(authorize_write(&profile, WritePolicy::All, "DELETE /books").is_ok());
}

#[test]
fn opensearch_rest_history_copies_the_complete_native_request() {
    use lazydb::model::{sql_history::*, sql_history_view::SqlHistoryState};
    let (mut app, profile_id) = console(false);
    let execution_id = uuid::Uuid::new_v4();
    let request = "POST /books/_search\n{\"query\":{\"query_string\":{\"query\":\"title:rust;edition:2024\"}}}";
    app.overlay = Some(Overlay::SqlHistory(SqlHistoryState {
        selected_execution: Some(execution_id),
        items: vec![ExecutionHistory {
            execution_id,
            operation_id: uuid::Uuid::new_v4(),
            transaction_id: None,
            sql: request.into(),
            status: HistoryExecutionStatus::Succeeded,
            certainty: HistoryResultCertainty::Confirmed,
            transaction_outcome: HistoryTransactionOutcome::NotApplicable,
            affected_rows: None,
            returned_rows: Some(1),
            requested_at: 0,
            elapsed_millis: Some(1),
            profile_id: Some(profile_id),
            database: Some("default".into()),
            schema: Some("default".into()),
            language: HistoryLanguage::Rest,
            mongo_command: None,
        }],
        ..Default::default()
    }));
    let commands = app.update(Action::SqlHistoryCopy);
    assert!(
        matches!(commands.as_slice(), [Command::WriteClipboard(payload)] if payload.text == request)
    );
    app.update(Action::SqlHistoryOpenDetail);
    assert!(
        matches!(app.overlay, Some(Overlay::SqlHistory(ref view)) if view.editor_session_id != uuid::Uuid::nil())
    );
}

#[test]
fn opensearch_cancelled_request_rejects_late_results_before_and_after_rerun() {
    use lazydb::{
        db::query::{QueryOutcome, QueryStats},
        model::{tab::ExecutionResult, workspace::QueryStatus},
    };
    let (mut app, profile_id) = console(false);
    app.update(Action::ReplaceEditor("GET /books/_search".into()));
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
    let cancel = app.update(Action::CancelActiveQuery);
    assert!(cancel.iter().any(|command| matches!(command, Command::CancelQuery { tab_id: id, generation: old } if *id == tab_id && *old == generation)));
    for rerun in [false, true] {
        if rerun {
            assert!(app.update(Action::RunActiveSql).iter().any(|command| matches!(command, Command::RunQuery { generation: new, .. } if *new > generation)));
        }
        let expected_status = if rerun {
            QueryStatus::Running
        } else {
            QueryStatus::Cancelled
        };
        app.update(Action::QueryFinished {
            tab_id,
            generation,
            connection,
            outcome: QueryOutcome {
                result_sets: Vec::new(),
                stats: QueryStats::new(std::time::Duration::ZERO, std::time::Duration::ZERO, 0),
            },
        });
        app.update(Action::QueryFailed {
            tab_id,
            generation,
            connection,
            message: "late cancelled response".into(),
        });
        assert_eq!(app.active_console().query_status, expected_status);
        assert!(app.active_console().outcome.is_none());
        assert_eq!(
            app.active_console().last_execution.as_ref().unwrap().result,
            if rerun {
                ExecutionResult::Dispatched
            } else {
                ExecutionResult::Cancelled
            }
        );
    }
}
