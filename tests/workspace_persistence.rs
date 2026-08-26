use tempfile::TempDir;
use uuid::Uuid;

use lazydb::{
    model::execution_target::ExecutionTarget,
    model::transaction::TransactionMode,
    persistence::workspace::{PersistedConsole, WorkspaceSnapshot, WorkspaceStore},
};

#[test]
fn workspace_round_trip_restores_sql_targets_and_mode_preference() {
    let temp = TempDir::new().unwrap();
    let store = WorkspaceStore::new(
        temp.path().join("state/workspace.toml"),
        temp.path().join("state/sql"),
    );
    let id = Uuid::new_v4();
    let snapshot = WorkspaceSnapshot {
        active_console: id,
        consoles: vec![PersistedConsole {
            id,
            name: "console_1".into(),
            sql_file: format!("{id}.sql").into(),
            target: Some(ExecutionTarget {
                profile_id: Uuid::new_v4(),
                database: "app".into(),
                schema: Some("public".into()),
            }),
            transaction_mode: TransactionMode::Manual,
        }],
        sql: vec![(id, "select 数据;".into())],
    };
    store.save(&snapshot).unwrap();
    let restored = store.load().unwrap().unwrap();
    assert_eq!(restored.active_console, id);
    assert_eq!(restored.sql, snapshot.sql);
    assert_eq!(restored.consoles[0].target, snapshot.consoles[0].target);
    assert_eq!(
        restored.consoles[0].transaction_mode,
        TransactionMode::Manual
    );
    assert!(
        !std::fs::read_to_string(temp.path().join("state/workspace.toml"))
            .unwrap()
            .contains("password")
    );
}

#[test]
fn missing_workspace_is_empty_and_unsupported_version_is_rejected() {
    let temp = TempDir::new().unwrap();
    let manifest = temp.path().join("workspace.toml");
    let store = WorkspaceStore::new(manifest.clone(), temp.path().join("sql"));
    assert!(store.load().unwrap().is_none());
    std::fs::write(
        &manifest,
        "version = 99\nactive_console = \"00000000-0000-0000-0000-000000000000\"\nconsoles = []\n",
    )
    .unwrap();
    assert!(matches!(
        store.load(),
        Err(lazydb::persistence::workspace::WorkspaceError::UnsupportedVersion { .. })
    ));
}

#[test]
fn workspace_lock_allows_one_writer_and_releases_on_drop() {
    let temp = TempDir::new().unwrap();
    let store = WorkspaceStore::new(temp.path().join("workspace.toml"), temp.path().join("sql"));
    let lock = store.lock().unwrap();
    assert!(matches!(
        store.lock(),
        Err(lazydb::persistence::workspace::WorkspaceError::Locked)
    ));
    drop(lock);
    assert!(store.lock().is_ok());
}
