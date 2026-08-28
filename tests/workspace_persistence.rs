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
            open: true,
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
fn version_one_workspace_migrates_consoles_as_open() {
    let temp = TempDir::new().unwrap();
    let id = Uuid::new_v4();
    let manifest = temp.path().join("workspace.toml");
    std::fs::write(&manifest, format!("version = 1\nactive_console = \"{id}\"\n\n[[consoles]]\nid = \"{id}\"\nname = \"old\"\nsql_file = \"{id}.sql\"\ntransaction_mode = \"auto\"\n")).unwrap();
    let restored = WorkspaceStore::new(manifest, temp.path().join("sql"))
        .load()
        .unwrap()
        .unwrap();
    assert!(restored.consoles[0].open);
}

#[test]
fn deleting_sql_file_is_exact_and_missing_files_are_allowed() {
    let temp = TempDir::new().unwrap();
    let sql_dir = temp.path().join("sql");
    std::fs::create_dir_all(&sql_dir).unwrap();
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    std::fs::write(sql_dir.join(format!("{first}.sql")), "first").unwrap();
    std::fs::write(sql_dir.join(format!("{second}.sql")), "second").unwrap();
    let store = WorkspaceStore::new(temp.path().join("workspace.toml"), sql_dir.clone());

    store.delete_sql_file(first).unwrap();
    store.delete_sql_file(first).unwrap();

    assert!(!sql_dir.join(format!("{first}.sql")).exists());
    assert_eq!(
        std::fs::read_to_string(sql_dir.join(format!("{second}.sql"))).unwrap(),
        "second"
    );
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
