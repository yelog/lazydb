use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use lazydb::{
    action::{Action, Command},
    agent::policy::WritePolicy,
    app::App,
    commands::{CommandAvailability, CommandContext, CommandId, matching_commands},
    data_ops::{
        DataEndpoint, DataJobPlan, DataJobRecord, DataJobSpec, DataJobStatus, DataJobStore,
        DataOperation, NativeOutcome, NativePlan, NativeToolPaths,
    },
    db::catalog::{CatalogId, CatalogKind},
    input::{
        keymap::{Keymap, map_paste},
        mouse::map_mouse,
    },
    model::{
        data_ops::{DataField, DataOperationAction as A},
        execution_target::ExecutionTarget,
        explorer::ExplorerNodeId,
        workspace::{Focus, Overlay},
    },
    persistence::{profiles::ProfileStore, secrets::NativeSecretStore},
    profile::{ConnectionProfile, import_connection_url},
    runtime::Runtime,
    ui::{self, HitTarget, UiState, theme::Theme},
};
use ratatui::{Terminal, backend::TestBackend};
use sqlx::{Connection, Executor};
use uuid::Uuid;

fn profile(url: &str) -> ConnectionProfile {
    import_connection_url(url, None).unwrap().profile
}

fn context(profile: &ConnectionProfile, database: &str) -> CommandContext {
    CommandContext {
        profile_id: Some(profile.id),
        target: Some(ExecutionTarget {
            profile_id: profile.id,
            database: database.into(),
            schema: None,
        }),
        ..Default::default()
    }
}

fn send(app: &mut App, action: A) -> Vec<Command> {
    app.update(Action::DataOperation(action))
}

fn open(app: &mut App, operation: DataOperation, profile: &ConnectionProfile) {
    let commands = send(
        app,
        A::Open {
            operation,
            context: context(profile, "source_db"),
        },
    );
    assert!(commands.is_empty());
    assert_eq!(app.overlay, Some(Overlay::DataOperation));
}

fn fill(app: &mut App, field: DataField, value: &str) {
    send(app, A::FocusField(field));
    send(
        app,
        A::Edit(lazydb::model::text_input::TextInputEdit::Clear),
    );
    send(app, A::Paste(value.into()));
}

fn request_plan(app: &mut App) -> (Uuid, DataJobSpec) {
    let commands = send(app, A::Review);
    match commands.as_slice() {
        [
            Command::PlanDataOperation {
                request_id, spec, ..
            },
        ] => (*request_id, spec.clone()),
        _ => panic!("Expected a plan request: {commands:?}"),
    }
}

fn plan(spec: DataJobSpec) -> DataJobPlan {
    DataJobPlan {
        id: Uuid::new_v4(),
        spec,
        native: NativePlan {
            engine: lazydb::profile::DatabaseKind::Postgres,
            steps: vec![
                "Snapshot the full source database".into(),
                "Verify archive readability".into(),
            ],
            warnings: vec!["Archive readability is not a data-parity check".into()],
            tools: Vec::new(),
        },
        profile_fingerprint: "frozen-profile-fingerprint".into(),
        created_at: Utc::now(),
    }
}

fn accept_plan(app: &mut App) -> DataJobPlan {
    let (request_id, spec) = request_plan(app);
    let plan = plan(spec);
    assert!(
        send(
            app,
            A::PlanReady {
                request_id,
                result: Ok(Box::new(plan.clone()))
            }
        )
        .is_empty()
    );
    plan
}

fn record(plan: DataJobPlan, status: DataJobStatus) -> DataJobRecord {
    let now = Utc::now();
    DataJobRecord {
        schema_version: 1,
        id: plan.id,
        plan,
        status,
        created_at: now,
        updated_at: now,
        message: format!("{status:?}"),
        bytes: Some(42),
        log: vec!["Full database operation".into()],
        outcome: None,
        error: None,
        target_may_be_partial: false,
    }
}

#[test]
fn omni_exposes_all_data_commands_and_exact_engine_availability() {
    for (query, id) in [
        ("backup database", CommandId::BackupDatabase),
        ("restore database", CommandId::RestoreDatabase),
        ("copy database", CommandId::CopyDatabase),
        ("data tasks", CommandId::DataTasks),
    ] {
        assert!(
            matching_commands(query)
                .iter()
                .any(|command| command.id == id)
        );
    }
    for url in [
        "postgres://user@localhost/source_db",
        "mysql://user@localhost/source_db",
        "mariadb://user@localhost/source_db",
        "sqlite:///tmp/source.db",
    ] {
        let profile = profile(url);
        let app = App::new(vec![profile.clone()]);
        assert_eq!(
            app.data_command_availability(
                CommandId::BackupDatabase,
                &context(&profile, "source_db")
            ),
            Some(CommandAvailability::Ready)
        );
    }
    for url in [
        "cockroachdb://user@localhost/source_db",
        "tidb://user@localhost/source_db",
    ] {
        let profile = profile(url);
        let app = App::new(vec![profile.clone()]);
        assert!(matches!(
            app.data_command_availability(
                CommandId::BackupDatabase,
                &context(&profile, "source_db")
            ),
            Some(CommandAvailability::Disabled(_))
        ));
    }
    let source = profile("postgres://user@localhost/source_db");
    let mut app = App::new(vec![source]);
    app.update(Action::OpenOmni);
    let omni = app.omni.as_ref().unwrap();
    for id in [
        CommandId::BackupDatabase,
        CommandId::RestoreDatabase,
        CommandId::CopyDatabase,
        CommandId::DataTasks,
    ] {
        assert!(
            omni.items
                .iter()
                .any(|item| item.id == lazydb::model::omni::OmniItemId::Command(id))
        );
    }
}

#[test]
fn form_requires_review_then_runs_the_frozen_plan_once() {
    let source = profile("postgres://user@localhost/source_db");
    let other = profile("postgres://user@other/other_db");
    let mut app = App::new(vec![source.clone(), other.clone()]);
    open(&mut app, DataOperation::Copy, &source);
    assert!(send(&mut app, A::Execute).is_empty());
    fill(&mut app, DataField::TargetDatabase, "new_database");
    fill(&mut app, DataField::Artifact, "/tmp/backup.dump");
    let reviewed = accept_plan(&mut app);
    app.connection.profile_id = Some(other.id);
    app.connection.target = Some(ExecutionTarget::from_profile(&other));
    app.explorer.normalized.selected = Some(ExplorerNodeId::Profile(other.id));
    send(&mut app, A::Paste("changed".into()));
    let commands = send(&mut app, A::Execute);
    assert!(
        matches!(commands.as_slice(), [Command::RunDataOperation { plan, .. }] if plan == &reviewed)
    );
    assert_eq!(app.overlay, Some(Overlay::DataTasks));
    assert!(send(&mut app, A::Execute).is_empty());
    assert_eq!(reviewed.spec.source.unwrap().profile_id, source.id);
}

#[test]
fn source_context_comes_from_explorer_and_never_from_an_unrelated_editor() {
    let explorer = profile("postgres://user@explorer/default_db");
    let editor = profile("postgres://user@editor/editor_db");
    let mut app = App::new(vec![explorer.clone(), editor.clone()]);
    app.update(Action::NewConsole);
    app.active_console_mut().execution_target = Some(ExecutionTarget::from_profile(&editor));
    let database = CatalogId::new(explorer.id, CatalogKind::Database, ["selected_db"]);
    app.explorer.normalized.selected = Some(ExplorerNodeId::Catalog(database));
    app.focus = Focus::Explorer;
    app.update(Action::OpenOmni);
    let item = app
        .omni
        .as_ref()
        .unwrap()
        .items
        .iter()
        .find(|item| item.id == lazydb::model::omni::OmniItemId::Command(CommandId::BackupDatabase))
        .unwrap()
        .clone();
    assert_eq!(item.context.profile_id, Some(explorer.id));
    app.update(Action::OmniDismiss);
    app.update(Action::ExecuteSemanticCommand {
        id: CommandId::BackupDatabase,
        context: item.context,
    });
    let draft = app.data_operations.draft.as_ref().unwrap();
    assert_eq!(draft.source_profile_id, Some(explorer.id));
    assert_eq!(draft.source_database.value(), "selected_db");
    send(&mut app, A::Close);
    app.focus = Focus::Editor;
    app.update(Action::OpenOmni);
    let item = app
        .omni
        .as_ref()
        .unwrap()
        .items
        .iter()
        .find(|item| item.id == lazydb::model::omni::OmniItemId::Command(CommandId::BackupDatabase))
        .unwrap();
    assert_eq!(item.context.profile_id, Some(editor.id));
    assert!(item.context.catalog_id.is_none());
}

#[test]
fn editing_or_closing_a_pending_form_rejects_late_plan_results() {
    let source = profile("postgres://user@localhost/source_db");
    let mut app = App::new(vec![source.clone()]);
    open(&mut app, DataOperation::Backup, &source);
    fill(&mut app, DataField::Artifact, "/tmp/backup.dump");
    let (first_request, first_spec) = request_plan(&mut app);
    let cancelled = send(&mut app, A::Paste("-new".into()));
    assert!(
        matches!(cancelled.as_slice(), [Command::CancelDataOperation { request_id }] if *request_id == first_request)
    );
    let (second_request, second_spec) = request_plan(&mut app);
    send(
        &mut app,
        A::PlanReady {
            request_id: first_request,
            result: Ok(Box::new(plan(first_spec))),
        },
    );
    assert!(app.data_operations.draft.as_ref().unwrap().plan.is_none());
    send(
        &mut app,
        A::PlanReady {
            request_id: second_request,
            result: Ok(Box::new(plan(second_spec))),
        },
    );
    assert!(app.data_operations.draft.as_ref().unwrap().plan.is_some());
    let commands = send(&mut app, A::Close);
    assert!(
        matches!(commands.as_slice(), [Command::CancelDataOperation { request_id }] if *request_id == second_request)
    );
    send(
        &mut app,
        A::PlanReady {
            request_id: second_request,
            result: Err("Late failure".into()),
        },
    );
    assert!(app.overlay.is_none());
    assert!(app.data_operations.draft.is_none());
}

#[test]
fn service_path_normalization_is_reviewed_without_rejecting_the_request() {
    let source = profile("sqlite:///tmp/source.db");
    let mut app = App::new(vec![source.clone()]);
    open(&mut app, DataOperation::Backup, &source);
    fill(&mut app, DataField::Artifact, "backup.sqlite");
    let (request_id, mut spec) = request_plan(&mut app);
    spec.artifact = PathBuf::from("/resolved/backup.sqlite");
    spec.source.as_mut().unwrap().database = "/resolved/source.db".into();
    let normalized = plan(spec);
    send(
        &mut app,
        A::PlanReady {
            request_id,
            result: Ok(Box::new(normalized.clone())),
        },
    );
    assert_eq!(
        app.data_operations.draft.as_ref().unwrap().plan,
        Some(normalized)
    );
}

#[test]
fn readonly_source_allows_backup_and_copy_while_restore_to_it_is_disabled() {
    let mut source = profile("postgres://user@localhost/source_db");
    source.read_only = true;
    let mut app = App::new(vec![source.clone()]);
    assert_eq!(
        app.data_command_availability(CommandId::BackupDatabase, &context(&source, "source_db")),
        Some(CommandAvailability::Ready)
    );
    assert_eq!(
        app.data_command_availability(CommandId::CopyDatabase, &context(&source, "source_db")),
        Some(CommandAvailability::Ready)
    );
    assert!(matches!(
        app.data_command_availability(CommandId::RestoreDatabase, &context(&source, "source_db")),
        Some(CommandAvailability::Disabled(_))
    ));
    open(&mut app, DataOperation::Backup, &source);
    fill(&mut app, DataField::Artifact, "/tmp/backup.dump");
    assert!(matches!(
        request_plan(&mut app).1.operation,
        DataOperation::Backup
    ));
    send(&mut app, A::Close);
}

#[test]
fn copy_from_readonly_source_requires_an_eligible_writable_target() {
    let mut source = profile("postgres://user@localhost/source_db");
    source.read_only = true;
    let mut readonly_target = profile("postgres://user@readonly/target_db");
    readonly_target.read_only = true;
    let wrong_engine = profile("mysql://user@localhost/target_db");
    let target = profile("postgres://user@writable/target_db");
    let mut app = App::new(vec![
        source.clone(),
        readonly_target.clone(),
        wrong_engine,
        target.clone(),
    ]);
    open(&mut app, DataOperation::Copy, &source);
    let draft = app.data_operations.draft.as_ref().unwrap();
    assert_eq!(draft.source_profile_id, Some(source.id));
    assert!(draft.target_profile_id.is_none());
    assert!(draft.target_database.value().is_empty());
    assert_eq!(draft.focus, DataField::TargetProfile);
    fill(&mut app, DataField::TargetDatabase, "new_database");
    fill(&mut app, DataField::Artifact, "/tmp/backup.dump");
    assert!(send(&mut app, A::Review).is_empty());
    assert!(
        app.data_operations
            .draft
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("target profile")
    );
    app.data_operations
        .draft
        .as_mut()
        .unwrap()
        .target_profile_id = Some(readonly_target.id);
    assert!(send(&mut app, A::Review).is_empty());
    assert!(
        app.data_operations
            .draft
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("Read-only")
    );
    send(&mut app, A::FocusField(DataField::TargetProfile));
    send(&mut app, A::CycleProfile(1));
    assert_eq!(
        app.data_operations
            .draft
            .as_ref()
            .unwrap()
            .target_profile_id,
        Some(target.id)
    );
    let reviewed = accept_plan(&mut app);
    assert_eq!(reviewed.spec.source.as_ref().unwrap().profile_id, source.id);
    assert_eq!(
        reviewed.spec.destination.as_ref().unwrap().profile_id,
        target.id
    );
    app.set_data_operation_write_policy(WritePolicy::Deny);
    assert!(send(&mut app, A::Execute).is_empty());
    app.set_data_operation_write_policy(WritePolicy::All);
    assert!(matches!(
        send(&mut app, A::Execute).as_slice(),
        [Command::RunDataOperation { plan, .. }] if plan == &reviewed
    ));
}

#[test]
fn global_policy_and_latest_profile_policy_are_rechecked_before_execution() {
    let source = profile("postgres://user@localhost/source_db");
    let mut app = App::new(vec![source.clone()]);
    open(&mut app, DataOperation::Restore, &source);
    fill(&mut app, DataField::TargetDatabase, "new_database");
    fill(&mut app, DataField::Artifact, "/tmp/backup.dump");
    accept_plan(&mut app);
    app.profiles[0].read_only = true;
    assert!(send(&mut app, A::Execute).is_empty());
    assert!(
        app.data_operations
            .draft
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("Read-only")
    );
    app.profiles[0].read_only = false;
    app.set_data_operation_write_policy(WritePolicy::Deny);
    assert!(send(&mut app, A::Execute).is_empty());
    for id in [CommandId::RestoreDatabase, CommandId::CopyDatabase] {
        assert!(matches!(
            app.data_command_availability(id, &context(&source, "source_db")),
            Some(CommandAvailability::Disabled(_))
        ));
    }
    assert_eq!(
        app.data_command_availability(CommandId::BackupDatabase, &context(&source, "source_db")),
        Some(CommandAvailability::Ready)
    );
}

#[test]
fn history_rerun_creates_an_editable_draft_and_requires_a_new_plan() {
    let source = profile("postgres://user@localhost/source_db");
    let mut app = App::new(vec![source.clone()]);
    let spec = DataJobSpec {
        operation: DataOperation::Restore,
        source: None,
        destination: Some(DataEndpoint {
            profile_id: source.id,
            database: "old_new_db".into(),
        }),
        artifact: PathBuf::from("/tmp/input.dump"),
        tools: NativeToolPaths {
            dump: None,
            restore: Some(PathBuf::from("/usr/local/bin/pg_restore")),
        },
    };
    let original = plan(spec);
    let old_id = original.id;
    let commands = send(&mut app, A::OpenTasks);
    let [Command::LoadDataTasks { request_id }] = commands.as_slice() else {
        panic!()
    };
    send(
        &mut app,
        A::TasksLoaded {
            request_id: *request_id,
            result: Ok(vec![record(original, DataJobStatus::Failed)]),
        },
    );
    assert!(send(&mut app, A::RerunTask).is_empty());
    assert!(app.data_operations.draft.as_ref().unwrap().plan.is_none());
    assert!(send(&mut app, A::Execute).is_empty());
    fill(&mut app, DataField::TargetDatabase, "different_new_db");
    let new_plan = accept_plan(&mut app);
    assert_ne!(new_plan.id, old_id);
    assert_eq!(
        new_plan.spec.destination.unwrap().database,
        "different_new_db"
    );
}

#[test]
fn copy_rejects_cross_engine_and_readonly_targets_without_runtime_io() {
    let source = profile("mysql://user@localhost/source_db");
    let target = profile("mariadb://user@localhost/target_db");
    let mut app = App::new(vec![source.clone(), target.clone()]);
    open(&mut app, DataOperation::Copy, &source);
    app.data_operations
        .draft
        .as_mut()
        .unwrap()
        .target_profile_id = Some(target.id);
    fill(&mut app, DataField::TargetDatabase, "new_database");
    fill(&mut app, DataField::Artifact, "/tmp/backup.sql");
    assert!(send(&mut app, A::Review).is_empty());
    assert!(
        app.data_operations
            .draft
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("exact same")
    );
    app.profiles[1].kind = lazydb::profile::DatabaseKind::MySql;
    app.profiles[1].read_only = true;
    assert!(send(&mut app, A::Review).is_empty());
    assert!(
        app.data_operations
            .draft
            .as_ref()
            .unwrap()
            .error
            .as_deref()
            .unwrap()
            .contains("Read-only")
    );
}

#[test]
fn stale_history_requests_cannot_replace_newer_results() {
    let source = profile("postgres://user@localhost/source_db");
    let mut app = App::new(vec![source]);
    let first = send(&mut app, A::OpenTasks);
    let [
        Command::LoadDataTasks {
            request_id: first_id,
        },
    ] = first.as_slice()
    else {
        panic!()
    };
    let second = send(&mut app, A::RefreshTasks);
    let [
        Command::LoadDataTasks {
            request_id: second_id,
        },
    ] = second.as_slice()
    else {
        panic!()
    };
    send(
        &mut app,
        A::TasksLoaded {
            request_id: *first_id,
            result: Err("stale".into()),
        },
    );
    assert!(app.data_operations.history_error.is_none());
    assert_eq!(app.data_operations.history_request, Some(*second_id));
    send(
        &mut app,
        A::TasksLoaded {
            request_id: *second_id,
            result: Err("current".into()),
        },
    );
    assert_eq!(
        app.data_operations.history_error.as_deref(),
        Some("current")
    );
}

#[test]
fn progress_stays_bound_to_its_request_and_keeps_the_workspace_usable() {
    let source = profile("postgres://user@localhost/source_db");
    let mut app = App::new(vec![source.clone()]);
    open(&mut app, DataOperation::Backup, &source);
    fill(&mut app, DataField::Artifact, "/tmp/backup.dump");
    let reviewed = accept_plan(&mut app);
    let commands = send(&mut app, A::Execute);
    let [Command::RunDataOperation { request_id, .. }] = commands.as_slice() else {
        panic!()
    };
    let request_id = *request_id;
    send(
        &mut app,
        A::JobUpdated {
            request_id: Uuid::new_v4(),
            record: Box::new(record(reviewed.clone(), DataJobStatus::Succeeded)),
        },
    );
    assert_eq!(
        app.data_operations.selected_record().unwrap().status,
        DataJobStatus::Queued
    );
    let cancel = send(&mut app, A::CancelTask);
    assert!(
        matches!(cancel.as_slice(), [Command::CancelDataOperation { request_id: id }] if *id == request_id)
    );
    send(&mut app, A::Close);
    app.update(Action::NewConsole);
    let active = app.active_console().id;
    let mut final_record = record(reviewed.clone(), DataJobStatus::Cancelled);
    final_record.target_may_be_partial = false;
    send(
        &mut app,
        A::JobUpdated {
            request_id,
            record: Box::new(final_record),
        },
    );
    assert!(app.overlay.is_none());
    assert_eq!(app.active_console().id, active);
    assert!(app.data_operations.runs.is_empty());
    send(
        &mut app,
        A::JobUpdated {
            request_id,
            record: Box::new(record(reviewed, DataJobStatus::Running)),
        },
    );
    assert_eq!(
        app.data_operations.selected_record().unwrap().status,
        DataJobStatus::Cancelled
    );
}

#[test]
fn normal_quit_opens_local_active_tasks_without_cancelling_or_saving_for_exit() {
    for status in [
        DataJobStatus::Queued,
        DataJobStatus::Preflight,
        DataJobStatus::Running,
        DataJobStatus::Verifying,
    ] {
        let source = profile("postgres://user@localhost/source_db");
        let mut app = App::new(vec![source.clone()]);
        open(&mut app, DataOperation::Backup, &source);
        fill(&mut app, DataField::Artifact, "/tmp/backup.dump");
        let reviewed = accept_plan(&mut app);
        let commands = send(&mut app, A::Execute);
        let [Command::RunDataOperation { request_id, .. }] = commands.as_slice() else {
            panic!("Expected a data task: {commands:?}");
        };
        let request_id = *request_id;
        send(
            &mut app,
            A::JobUpdated {
                request_id,
                record: Box::new(record(reviewed.clone(), status)),
            },
        );
        let previous = record(plan(reviewed.spec.clone()), DataJobStatus::Succeeded);
        let previous_id = previous.id;
        app.data_operations.merge_record(previous);
        app.data_operations.selected = app
            .data_operations
            .records
            .iter()
            .position(|record| record.id == previous_id)
            .unwrap();
        send(&mut app, A::Close);
        app.update(Action::OpenOmni);
        let commands = app.update(Action::Quit);
        assert!(matches!(
            commands.as_slice(),
            [Command::LoadDataTasks { .. }]
        ));
        assert!(!app.should_quit);
        assert!(app.omni.is_none());
        assert_eq!(app.overlay, Some(Overlay::DataTasks));
        assert_eq!(
            app.data_operations.selected_record().unwrap().id,
            reviewed.id
        );
        assert_eq!(app.data_operations.runs.len(), 1);
        assert!(app.data_operations.cancelling.is_empty());
        assert!(app.notifications.history().any(|notification| {
            notification.body.contains("press c")
                && notification.body.contains("wait for cancellation")
        }));
        app.notifications.dismiss_all_live();
        let mut terminal = Terminal::new(TestBackend::new(90, 28)).unwrap();
        terminal.draw(|frame| ui::render(frame, &app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(
            text.contains("wait for tasks to finish, or cancel with c"),
            "{text}"
        );
        let cancel = Keymap::default()
            .map(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE), &app)
            .unwrap();
        let commands = app.update(cancel);
        assert!(matches!(
            commands.as_slice(),
            [Command::CancelDataOperation { request_id: cancelled }] if *cancelled == request_id
        ));
        assert!(app.data_operations.cancelling.contains(&reviewed.id));
        assert!(matches!(
            app.update(Action::Quit).as_slice(),
            [Command::LoadDataTasks { .. }]
        ));
        assert!(!app.should_quit);
        assert_eq!(app.data_operations.runs.len(), 1);
    }
}

#[test]
fn terminal_local_tasks_allow_existing_quit_flow_even_with_other_process_history() {
    for status in [
        DataJobStatus::Succeeded,
        DataJobStatus::SucceededWithWarnings,
        DataJobStatus::Failed,
        DataJobStatus::Cancelled,
        DataJobStatus::Interrupted,
    ] {
        let source = profile("postgres://user@localhost/source_db");
        let mut app = App::new(vec![source.clone()]);
        open(&mut app, DataOperation::Backup, &source);
        fill(&mut app, DataField::Artifact, "/tmp/backup.dump");
        let reviewed = accept_plan(&mut app);
        let commands = send(&mut app, A::Execute);
        let [Command::RunDataOperation { request_id, .. }] = commands.as_slice() else {
            panic!("Expected a data task: {commands:?}");
        };
        let request_id = *request_id;
        if status == DataJobStatus::Cancelled {
            send(&mut app, A::CancelTask);
        }
        send(
            &mut app,
            A::JobUpdated {
                request_id,
                record: Box::new(record(reviewed.clone(), status)),
            },
        );
        assert!(app.data_operations.runs.is_empty());
        assert!(app.data_operations.cancelling.is_empty());
        app.data_operations
            .merge_record(record(plan(reviewed.spec), DataJobStatus::Running));
        send(&mut app, A::Close);
        let commands = app.update(Action::Quit);
        let [
            Command::PersistWorkspace { revision, .. },
            Command::FlushWorkspace { revision: flushed },
        ] = commands.as_slice()
        else {
            panic!("Expected the existing workspace quit flow: {commands:?}");
        };
        let revision = *revision;
        assert_eq!(revision, *flushed);
        assert!(!app.should_quit);
        app.update(Action::WorkspaceSaveSucceeded { revision });
        assert!(matches!(
            app.update(Action::WorkspaceSaveFlushed { revision })
                .as_slice(),
            [Command::Quit]
        ));
        assert!(app.should_quit);
    }
}

#[test]
fn keyboard_enter_only_executes_after_review_and_escape_returns_to_editing() {
    let source = profile("postgres://user@localhost/source_db");
    let mut app = App::new(vec![source.clone()]);
    open(&mut app, DataOperation::Backup, &source);
    fill(&mut app, DataField::Artifact, "/tmp/backup.dump");
    let mut keymap = Keymap::default();
    let action = keymap
        .map(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &app)
        .unwrap();
    assert_eq!(action, Action::DataOperation(A::Review));
    let (request_id, spec) = request_plan(&mut app);
    send(
        &mut app,
        A::PlanReady {
            request_id,
            result: Ok(Box::new(plan(spec))),
        },
    );
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &app),
        Some(Action::DataOperation(A::Execute))
    );
    let escape = keymap
        .map(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &app)
        .unwrap();
    app.update(escape);
    assert!(app.data_operations.draft.as_ref().unwrap().plan.is_none());
    assert_eq!(
        keymap.map(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &app),
        Some(Action::DataOperation(A::Review))
    );
    assert_eq!(
        map_paste("new".into(), &app),
        vec![Action::DataOperation(A::Paste("new".into()))]
    );
}

#[test]
fn dialogs_render_at_compact_sizes_and_disabled_actions_are_not_clickable() {
    let mut source = profile("postgres://user@localhost/source_db");
    source.name = "数据来源".into();
    let mut app = App::new(vec![source.clone()]);
    open(&mut app, DataOperation::Backup, &source);
    fill(&mut app, DataField::Artifact, "/tmp/备份.dump");
    let (request_id, spec) = request_plan(&mut app);
    for (width, height) in [(120, 36), (80, 24), (56, 16)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut state = UiState::new();
        terminal
            .draw(|frame| ui::render_with_state(frame, &app, &mut state))
            .unwrap();
        assert!(
            !state
                .hit_regions
                .iter()
                .any(|region| region.target == HitTarget::DataOperation(Box::new(A::Review)))
        );
        assert!(
            state
                .hit_regions
                .iter()
                .any(|region| region.target == HitTarget::DataOperation(Box::new(A::Close)))
        );
        for region in state
            .hit_regions
            .iter()
            .filter(|region| matches!(region.target, HitTarget::DataOperation(_)))
        {
            assert!(region.area.right() <= width);
            assert!(region.area.bottom() <= height);
        }
    }
    send(
        &mut app,
        A::PlanReady {
            request_id,
            result: Ok(Box::new(plan(spec))),
        },
    );
    for theme in [
        Theme::default(),
        Theme::for_color_mode(lazydb::cli::ColorMode::Never),
    ] {
        let mut terminal = Terminal::new(TestBackend::new(90, 28)).unwrap();
        let mut state = UiState::new();
        terminal
            .draw(|frame| {
                ui::render_with_state_using_icons_sequence_and_theme(
                    frame,
                    &app,
                    &mut state,
                    Default::default(),
                    None,
                    theme,
                )
            })
            .unwrap();
        let region = state
            .hit_regions
            .iter()
            .find(|region| region.target == HitTarget::DataOperation(Box::new(A::Execute)))
            .unwrap();
        let event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: region.area.x,
            row: region.area.y,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(
            map_mouse(event, &state, &app),
            Some(Action::DataOperation(A::Execute))
        );
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Overwrite refused"));
    }
}

async fn sqlite_source(path: &Path) {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let mut connection = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    connection
        .execute("CREATE TABLE data(value INTEGER); INSERT INTO data VALUES (42);")
        .await
        .unwrap();
    connection.close().await.unwrap();
}

#[tokio::test]
async fn sqlite_runtime_roundtrip_uses_custom_job_store_and_publishes_updates() {
    let temporary = tempfile::tempdir().unwrap();
    let source_path = temporary.path().join("source.sqlite");
    let archive_path = temporary.path().join("backup.sqlite");
    sqlite_source(&source_path).await;
    let source = profile(&format!("sqlite://{}", source_path.display()));
    let store = DataJobStore::new(temporary.path().join("jobs"));
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut runtime = Runtime::new(
        vec![source.clone()],
        HashSet::new(),
        HashMap::new(),
        None,
        ProfileStore::new(temporary.path().join("connections.toml")),
        Arc::new(NativeSecretStore),
        sender,
    );
    runtime.set_data_job_store(store.clone());
    let mut app = App::new(vec![source.clone()]);
    open(&mut app, DataOperation::Backup, &source);
    fill(
        &mut app,
        DataField::Artifact,
        archive_path.to_str().unwrap(),
    );
    for command in send(&mut app, A::Review) {
        runtime.dispatch(command);
    }
    let event = tokio::time::timeout(Duration::from_secs(15), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            &event,
            Action::DataOperation(A::PlanReady { result: Ok(_), .. })
        ),
        "{event:?}"
    );
    app.update(event);
    for command in send(&mut app, A::Execute) {
        runtime.dispatch(command);
    }
    send(&mut app, A::Close);
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let event = receiver.recv().await.unwrap();
            let terminal = matches!(&event, Action::DataOperation(A::JobUpdated { record, .. }) if record.status.is_terminal());
            assert!(!matches!(&event, Action::DataOperation(A::JobFailed { .. })), "{event:?}");
            app.update(event);
            if terminal { break; }
        }
    }).await.unwrap();
    let result = app.data_operations.records.first().unwrap();
    assert!(result.status.is_success(), "{result:?}");
    assert!(app.overlay.is_none());
    assert_eq!(store.list().unwrap().len(), 1);
    assert!(archive_path.exists());
    assert!(
        result
            .outcome
            .as_ref()
            .is_some_and(|outcome: &NativeOutcome| !outcome.verification.is_empty())
    );
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(archive_path)
        .read_only(true);
    let mut restored = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    let value: i64 = sqlx::query_scalar("SELECT value FROM data")
        .fetch_one(&mut restored)
        .await
        .unwrap();
    assert_eq!(value, 42);
    restored.close().await.unwrap();
    Box::pin(runtime.shutdown()).await;
}

#[tokio::test]
async fn runtime_rejects_source_scope_changes_after_review() {
    let temporary = tempfile::tempdir().unwrap();
    let source_path = temporary.path().join("source.sqlite");
    let archive_path = temporary.path().join("backup.sqlite");
    sqlite_source(&source_path).await;
    let source = profile(&format!("sqlite://{}", source_path.display()));
    let profiles = ProfileStore::new(temporary.path().join("connections.toml"));
    profiles.save(vec![source.clone()]).unwrap();
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut runtime = Runtime::new(
        vec![source.clone()],
        HashSet::from([source.id]),
        HashMap::new(),
        None,
        profiles,
        Arc::new(NativeSecretStore),
        sender,
    );
    runtime.set_data_job_store(DataJobStore::new(temporary.path().join("jobs")));
    let mut app = App::new(vec![source.clone()]);
    open(&mut app, DataOperation::Backup, &source);
    fill(
        &mut app,
        DataField::Artifact,
        archive_path.to_str().unwrap(),
    );
    for command in send(&mut app, A::Review) {
        runtime.dispatch(command);
    }
    let event = tokio::time::timeout(Duration::from_secs(15), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            &event,
            Action::DataOperation(A::PlanReady { result: Ok(_), .. })
        ),
        "{event:?}"
    );
    app.update(event);
    let mut changed = source.clone();
    let selected =
        lazydb::profile::CatalogSelection::Selected(vec![lazydb::profile::DatabaseScope {
            name: "main".into(),
            schemas: lazydb::profile::CatalogSelection::All,
        }]);
    changed.catalog_scope.databases = if changed.catalog_scope.databases == selected {
        lazydb::profile::CatalogSelection::All
    } else {
        selected
    };
    runtime.dispatch(Command::SaveProfile {
        request_id: 17,
        submission: lazydb::model::profile_manager::ProfileSubmission::new(
            changed,
            lazydb::model::profile_manager::CredentialUpdate::Preserve,
            0,
        ),
        connect: false,
    });
    let saved = tokio::time::timeout(Duration::from_secs(15), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(saved, Action::ProfileSaved { request_id: 17, .. }),
        "{saved:?}"
    );
    for command in send(&mut app, A::Execute) {
        runtime.dispatch(command);
    }
    let failed = tokio::time::timeout(Duration::from_secs(15), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(&failed, Action::DataOperation(A::JobFailed { message, .. })
        if message.contains("changed after review")),
        "{failed:?}"
    );
    app.update(failed);
    assert_eq!(app.data_operations.records[0].status, DataJobStatus::Failed);
    assert!(!archive_path.exists());
    Box::pin(runtime.shutdown()).await;
}

#[tokio::test]
async fn runtime_history_filters_other_projects_missing_profiles_and_mixed_endpoints() {
    let temporary = tempfile::tempdir().unwrap();
    let project = lazydb::project::ProjectContext::resolve_from(temporary.path()).unwrap();
    let global = profile("postgres://user@localhost/global");
    let mut current = profile("postgres://user@localhost/current");
    let mut other = profile("postgres://user@localhost/other");
    current.access = lazydb::profile::ProfileAccess::Projects {
        roots: vec![project.root.clone()],
    };
    other.access = lazydb::profile::ProfileAccess::Projects {
        roots: vec![project.root.join("other")],
    };
    let profiles = vec![global.clone(), current.clone(), other.clone()];
    let persisted = profiles
        .iter()
        .map(|profile| profile.id)
        .collect::<HashSet<_>>();
    let store = DataJobStore::new(temporary.path().join("jobs"));
    let task_for = |profile_id| {
        record(
            plan(DataJobSpec {
                operation: DataOperation::Backup,
                source: Some(DataEndpoint {
                    profile_id,
                    database: "full_database".into(),
                }),
                destination: None,
                artifact: PathBuf::from("/tmp/history.dump"),
                tools: NativeToolPaths::default(),
            }),
            DataJobStatus::Succeeded,
        )
    };
    let global_record = task_for(global.id);
    let current_record = task_for(current.id);
    let other_record = task_for(other.id);
    let missing_record = task_for(Uuid::new_v4());
    let mut mixed = task_for(global.id);
    mixed.plan.spec.operation = DataOperation::Copy;
    mixed.plan.spec.destination = Some(DataEndpoint {
        profile_id: other.id,
        database: "new_target".into(),
    });
    for record in [
        &global_record,
        &current_record,
        &other_record,
        &missing_record,
        &mixed,
    ] {
        store.create(record).unwrap();
    }
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut runtime = Runtime::new(
        profiles.clone(),
        persisted.clone(),
        HashMap::new(),
        None,
        ProfileStore::new(temporary.path().join("connections.toml")),
        Arc::new(NativeSecretStore),
        sender,
    );
    runtime.set_data_job_store(store);
    runtime.set_data_operation_project_root(project.root.clone());
    let mut app = App::with_startup_project(
        profiles,
        persisted,
        lazydb::cli::ConfirmationPolicy::RiskyOnly,
        project,
    );
    for command in send(&mut app, A::OpenTasks) {
        runtime.dispatch(command);
    }
    let event = tokio::time::timeout(Duration::from_secs(15), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(&event, Action::DataOperation(A::TasksLoaded { result: Ok(records), .. })
        if records.len() == 2 && records.iter().all(|record| record.id == global_record.id || record.id == current_record.id)),
        "{event:?}"
    );
    app.update(event);
    assert_eq!(app.data_operations.records.len(), 2);
    let commands = send(&mut app, A::RefreshTasks);
    let [Command::LoadDataTasks { request_id }] = commands.as_slice() else {
        panic!()
    };
    send(
        &mut app,
        A::TasksLoaded {
            request_id: *request_id,
            result: Ok(vec![other_record, missing_record, mixed]),
        },
    );
    assert_eq!(app.data_operations.records.len(), 2);
    Box::pin(runtime.shutdown()).await;
}

#[tokio::test]
async fn runtime_shutdown_cancels_and_awaits_a_queued_job() {
    let temporary = tempfile::tempdir().unwrap();
    let source_path = temporary.path().join("source.sqlite");
    let archive_path = temporary.path().join("backup.sqlite");
    sqlite_source(&source_path).await;
    let source = profile(&format!("sqlite://{}", source_path.display()));
    let store = DataJobStore::new(temporary.path().join("jobs"));
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut runtime = Runtime::new(
        vec![source.clone()],
        HashSet::new(),
        HashMap::new(),
        None,
        ProfileStore::new(temporary.path().join("connections.toml")),
        Arc::new(NativeSecretStore),
        sender,
    );
    runtime.set_data_job_store(store.clone());
    runtime.dispatch(Command::PlanDataOperation {
        request_id: Uuid::new_v4(),
        spec: DataJobSpec {
            operation: DataOperation::Backup,
            source: Some(DataEndpoint::from_profile(&source)),
            destination: None,
            artifact: archive_path.clone(),
            tools: NativeToolPaths::default(),
        },
        policy: WritePolicy::Deny,
    });
    let event = tokio::time::timeout(Duration::from_secs(15), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let Action::DataOperation(A::PlanReady {
        result: Ok(plan), ..
    }) = event
    else {
        panic!("Expected a prepared SQLite plan: {event:?}");
    };
    runtime.dispatch(Command::RunDataOperation {
        request_id: Uuid::new_v4(),
        plan: *plan,
        policy: WritePolicy::Deny,
    });
    tokio::time::timeout(Duration::from_secs(10), Box::pin(runtime.shutdown()))
        .await
        .unwrap();
    let records = store.list().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, DataJobStatus::Cancelled);
    assert!(!archive_path.exists());
}
