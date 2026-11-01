use lazydb::{
    action::{Action, Command},
    app::App,
    db::ServerInfo,
    model::workspace::Overlay,
    persistence::workspace::WorkspaceStore,
    profile::import_connection_url,
};
use tempfile::TempDir;

#[test]
fn unbound_console_can_be_saved_restored_bound_and_executed_lazily() {
    let profile = import_connection_url(":memory:", Some("lazy-flow"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let temp = TempDir::new().unwrap();
    let store = WorkspaceStore::new(temp.path().join("workspace.toml"), temp.path().join("sql"));

    let console_id = save_unbound_console(&store, profile.clone());

    let mut restored = App::new(vec![profile]);
    restored.restore_workspace(store.load().unwrap().unwrap(), None);
    assert_eq!(restored.editor_text(console_id).unwrap(), "SELECT 1");
    assert!(restored.active_console().execution_target.is_none());

    let target = lazydb::model::execution_target::ExecutionTarget::from_profile(
        restored.profiles.first().unwrap(),
    );
    restored.update(Action::OpenConsoleTargetSelector { console_id });
    let target_index = match restored.overlay.as_ref().unwrap() {
        Overlay::TargetSelector { candidates, .. } => candidates
            .iter()
            .position(|candidate| candidate.target() == Some(&target))
            .unwrap(),
        overlay => panic!("unexpected overlay: {overlay:?}"),
    };
    restored.update(Action::SelectTargetSelector(target_index));
    assert_eq!(
        restored.active_console().execution_target.as_ref(),
        Some(&target)
    );
    assert!(
        restored
            .update(Action::RunActiveSql)
            .iter()
            .any(|command| matches!(command, Command::Connect { .. }))
    );

    let generation = restored.connection.pending_generation.unwrap();
    let commands = restored.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: ServerInfo {
            kind: lazydb::profile::DatabaseKind::Sqlite,
            version: "3.50".into(),
            database: ":memory:".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, Command::RunQueryPage { .. }))
    );
    assert!(!matches!(
        restored.overlay,
        Some(Overlay::ExecutionConfirm { .. })
    ));
}

fn save_unbound_console(
    store: &WorkspaceStore,
    profile: lazydb::profile::ConnectionProfile,
) -> uuid::Uuid {
    let mut app = App::new(vec![profile]);
    app.reveal_startup_profile(None);
    app.update(Action::NewUnboundConsoleNamed("scratch".into()));
    app.update(Action::ReplaceEditor("SELECT 1".into()));
    let console_id = app.active_console().id;
    assert!(app.active_console().execution_target.is_none());
    store.save(&app.workspace_snapshot()).unwrap();
    console_id
}

#[test]
fn unbound_execution_selects_target_connects_and_resumes_once() {
    let profile = import_connection_url(":memory:", Some("run-unbound"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.update(Action::NewUnboundConsoleNamed("scratch".into()));
    app.update(Action::ReplaceEditor("SELECT 1".into()));
    let console_id = app.active_console().id;

    assert!(
        app.update(Action::RunActiveSql)
            .iter()
            .all(|command| !matches!(command, Command::Connect { .. }))
    );
    let target = lazydb::model::execution_target::ExecutionTarget::from_profile(
        app.profiles.first().unwrap(),
    );
    let target_index = match app.overlay.as_ref().unwrap() {
        Overlay::TargetSelector { candidates, .. } => candidates
            .iter()
            .position(|candidate| candidate.target() == Some(&target))
            .unwrap(),
        overlay => panic!("unexpected overlay: {overlay:?}"),
    };
    let commands = app.update(Action::SelectTargetSelector(target_index));
    let generation = commands
        .iter()
        .find_map(|command| match command {
            Command::Connect { generation, .. } => Some(*generation),
            _ => None,
        })
        .expect("selecting a target should connect");
    assert_eq!(
        app.active_console().execution_target.as_ref(),
        Some(&target)
    );

    let commands = app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: ServerInfo {
            kind: lazydb::profile::DatabaseKind::Sqlite,
            version: "3.50".into(),
            database: ":memory:".into(),
            current_user: None,
        },
        mutation_capabilities: Default::default(),
    });
    assert_eq!(app.active_console().id, console_id);
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, Command::RunQueryPage { .. }))
    );
}

#[test]
fn unbound_execution_is_discarded_when_document_changes_before_selection() {
    let profile = import_connection_url(":memory:", Some("stale-run"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile]);
    app.update(Action::NewUnboundConsoleNamed("scratch".into()));
    app.update(Action::ReplaceEditor("SELECT 1".into()));
    app.update(Action::RunActiveSql);
    app.update(Action::ReplaceEditor("SELECT 2".into()));

    let target = lazydb::model::execution_target::ExecutionTarget::from_profile(
        app.profiles.first().unwrap(),
    );
    let target_index = match app.overlay.as_ref().unwrap() {
        Overlay::TargetSelector { candidates, .. } => candidates
            .iter()
            .position(|candidate| candidate.target() == Some(&target))
            .unwrap(),
        overlay => panic!("unexpected overlay: {overlay:?}"),
    };
    let commands = app.update(Action::SelectTargetSelector(target_index));
    assert!(commands.is_empty());
    assert!(app.active_console().execution_connection.is_none());
}
