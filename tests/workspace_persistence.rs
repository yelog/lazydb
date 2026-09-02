use tempfile::TempDir;
use uuid::Uuid;

use lazydb::{
    db::catalog::{CatalogId, CatalogKind, QualifiedName},
    model::execution_target::ExecutionTarget,
    model::relation::RelationView,
    model::transaction::TransactionMode,
    persistence::workspace::{
        PersistedConsole, PersistedProfileWorkspace, PersistedRelationTab, PersistedTab,
        WorkspaceError, WorkspaceSnapshot, WorkspaceStore,
    },
};

#[test]
fn workspace_v3_round_trip_restores_two_profile_workspaces_and_durable_state() {
    let temp = TempDir::new().unwrap();
    let store = WorkspaceStore::new(
        temp.path().join("state/workspace.toml"),
        temp.path().join("state/sql"),
    );
    let profile_a = Uuid::new_v4();
    let profile_b = Uuid::new_v4();
    let console_a = Uuid::new_v4();
    let hidden_console_a = Uuid::new_v4();
    let relation_b = Uuid::new_v4();
    let console_b = Uuid::new_v4();
    let snapshot = WorkspaceSnapshot {
        active_profile: Some(profile_b),
        profiles: vec![
            PersistedProfileWorkspace {
                profile_id: profile_a,
                active_tab: Some(relation_b),
                consoles: vec![
                    console(profile_a, console_a, "a", true),
                    console(profile_a, hidden_console_a, "hidden", false),
                ],
                tabs: vec![
                    PersistedTab::Console {
                        console_id: console_a,
                    },
                    PersistedTab::Relation(PersistedRelationTab {
                        id: relation_b,
                        object_id: CatalogId::new(
                            profile_a,
                            CatalogKind::Table,
                            ["public", "users"],
                        ),
                        qualified_name: QualifiedName {
                            database: Some("app".into()),
                            schema: Some("public".into()),
                            object: "users".into(),
                        },
                        catalog_kind: CatalogKind::Table,
                        title: "users".into(),
                        view: RelationView::Ddl,
                    }),
                ],
            },
            PersistedProfileWorkspace {
                profile_id: profile_b,
                active_tab: Some(console_b),
                consoles: vec![console(profile_b, console_b, "b", true)],
                tabs: vec![PersistedTab::Console {
                    console_id: console_b,
                }],
            },
        ],
        sql: vec![
            (console_a, "select from_a;".into()),
            (hidden_console_a, "select hidden;".into()),
            (console_b, "select from_b;".into()),
        ],
        active_console: Uuid::nil(),
        consoles: Vec::new(),
    };
    store.save(&snapshot).unwrap();
    let restored = store.load().unwrap().unwrap();
    assert_eq!(restored.active_profile, Some(profile_b));
    assert_eq!(restored.profiles.len(), 2);
    assert_eq!(restored.profiles[0].tabs[1], snapshot.profiles[0].tabs[1]);
    assert!(!restored.profiles[0].consoles[1].open);
    assert_eq!(restored.sql, snapshot.sql);
    let manifest = std::fs::read_to_string(temp.path().join("state/workspace.toml")).unwrap();
    for transient in ["outcome", "transaction_state", "rows", "edit", "generation"] {
        assert!(
            !manifest.contains(transient),
            "found transient field {transient}"
        );
    }
}

#[test]
fn workspace_round_trip_preserves_open_and_closed_console_sql_and_names() {
    let temp = TempDir::new().unwrap();
    let store = WorkspaceStore::new(temp.path().join("workspace.toml"), temp.path().join("sql"));
    let profile_id = Uuid::new_v4();
    let open_id = Uuid::new_v4();
    let closed_id = Uuid::new_v4();
    let snapshot = WorkspaceSnapshot {
        active_profile: Some(profile_id),
        profiles: vec![PersistedProfileWorkspace {
            profile_id,
            active_tab: Some(open_id),
            consoles: vec![
                console(profile_id, open_id, "renamed open", true),
                console(profile_id, closed_id, "renamed closed", false),
            ],
            tabs: vec![PersistedTab::Console {
                console_id: open_id,
            }],
        }],
        sql: vec![
            (open_id, "select open;".into()),
            (closed_id, "select closed;".into()),
        ],
        active_console: Uuid::nil(),
        consoles: Vec::new(),
    };

    store.save(&snapshot).unwrap();
    let restored = store.load().unwrap().unwrap();
    let consoles = &restored.profiles[0].consoles;

    assert_eq!(consoles[0].name, "renamed open");
    assert!(consoles[0].open);
    assert_eq!(consoles[1].name, "renamed closed");
    assert!(!consoles[1].open);
    assert_eq!(restored.sql, snapshot.sql);
}

#[test]
fn missing_workspace_is_empty_and_unsupported_version_is_rejected() {
    let temp = TempDir::new().unwrap();
    let manifest = temp.path().join("workspace.toml");
    let store = WorkspaceStore::new(manifest.clone(), temp.path().join("sql"));
    assert!(store.load().unwrap().is_none());
    std::fs::write(
        &manifest,
        "version = 99\nactive_profile = \"00000000-0000-0000-0000-000000000000\"\nprofiles = []\n",
    )
    .unwrap();
    assert!(matches!(
        store.load(),
        Err(lazydb::persistence::workspace::WorkspaceError::UnsupportedVersion { .. })
    ));
}

#[test]
fn deleting_a_sql_file_is_idempotent_and_only_removes_that_console() {
    let temp = TempDir::new().unwrap();
    let store = WorkspaceStore::new(temp.path().join("workspace.toml"), temp.path().join("sql"));
    let deleted = Uuid::new_v4();
    let retained = Uuid::new_v4();
    std::fs::create_dir_all(temp.path().join("sql")).unwrap();
    std::fs::write(
        temp.path().join("sql").join(format!("{deleted}.sql")),
        "delete",
    )
    .unwrap();
    std::fs::write(
        temp.path().join("sql").join(format!("{retained}.sql")),
        "retain",
    )
    .unwrap();

    store.delete_sql_file(deleted).unwrap();
    store.delete_sql_file(deleted).unwrap();
    assert!(
        !temp
            .path()
            .join("sql")
            .join(format!("{deleted}.sql"))
            .exists()
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("sql").join(format!("{retained}.sql"))).unwrap(),
        "retain"
    );
}

#[test]
fn workspace_v1_migrates_all_consoles_open_and_preserves_root_active_console() {
    let temp = TempDir::new().unwrap();
    let active = Uuid::new_v4();
    let hidden = Uuid::new_v4();
    let profile = Uuid::new_v4();
    let store = WorkspaceStore::new(temp.path().join("workspace.toml"), temp.path().join("sql"));
    std::fs::write(
        temp.path().join("workspace.toml"),
        format!(
            "version = 1\nactive_console = \"{active}\"\n\n[[consoles]]\nid = \"{active}\"\nname = \"active\"\nsql_file = \"{active}.sql\"\ntarget = {{ profile_id = \"{profile}\", database = \"app\", schema = \"public\" }}\ntransaction_mode = \"auto\"\n\n[[consoles]]\nid = \"{hidden}\"\nname = \"hidden\"\nsql_file = \"{hidden}.sql\"\ntarget = {{ profile_id = \"{profile}\", database = \"app\", schema = \"public\" }}\ntransaction_mode = \"auto\"\n"
        ),
    )
    .unwrap();

    let snapshot = store.load().unwrap().unwrap();
    assert_eq!(snapshot.active_console, active);
    assert_eq!(snapshot.profiles.len(), 1);
    assert_eq!(snapshot.profiles[0].active_tab, Some(active));
    assert!(
        snapshot.profiles[0]
            .consoles
            .iter()
            .all(|console| console.open)
    );
    assert_eq!(snapshot.profiles[0].tabs.len(), 2);
}

#[test]
fn workspace_v2_migrates_targeted_consoles_into_their_profiles() {
    let temp = TempDir::new().unwrap();
    let first_profile = Uuid::new_v4();
    let second_profile = Uuid::new_v4();
    let first_console = Uuid::new_v4();
    let second_console = Uuid::new_v4();
    let store = WorkspaceStore::new(temp.path().join("workspace.toml"), temp.path().join("sql"));
    std::fs::write(
        temp.path().join("workspace.toml"),
        format!(
            "version = 2\nactive_console = \"{second_console}\"\n\n[[consoles]]\nid = \"{first_console}\"\nname = \"first\"\nsql_file = \"{first_console}.sql\"\ntarget = {{ profile_id = \"{first_profile}\", database = \"one\", schema = \"public\" }}\ntransaction_mode = \"auto\"\nopen = true\n\n[[consoles]]\nid = \"{second_console}\"\nname = \"second\"\nsql_file = \"{second_console}.sql\"\ntarget = {{ profile_id = \"{second_profile}\", database = \"two\", schema = \"public\" }}\ntransaction_mode = \"manual\"\nopen = false\n"
        ),
    )
    .unwrap();

    let snapshot = store.load().unwrap().unwrap();
    assert_eq!(snapshot.active_console, second_console);
    assert_eq!(snapshot.profiles.len(), 2);
    assert_eq!(snapshot.profiles[0].profile_id, first_profile);
    assert_eq!(snapshot.profiles[1].profile_id, second_profile);
    assert_eq!(snapshot.profiles[1].active_tab, None);
    assert_eq!(snapshot.profiles[1].tabs.len(), 0);
}

fn console(profile_id: Uuid, id: Uuid, name: &str, open: bool) -> PersistedConsole {
    PersistedConsole {
        id,
        name: name.into(),
        sql_file: format!("{id}.sql").into(),
        target: Some(ExecutionTarget {
            profile_id,
            database: "app".into(),
            schema: Some("public".into()),
        }),
        transaction_mode: TransactionMode::Manual,
        open,
    }
}

fn valid_snapshot() -> WorkspaceSnapshot {
    let profile_id = Uuid::new_v4();
    let console_id = Uuid::new_v4();
    WorkspaceSnapshot {
        active_profile: Some(profile_id),
        profiles: vec![PersistedProfileWorkspace {
            profile_id,
            active_tab: Some(console_id),
            consoles: vec![console(profile_id, console_id, "console", true)],
            tabs: vec![PersistedTab::Console { console_id }],
        }],
        sql: vec![(console_id, "select 1".into())],
        active_console: Uuid::nil(),
        consoles: Vec::new(),
    }
}

#[test]
fn workspace_v3_validation_rejects_duplicate_and_cross_profile_references() {
    let temp = TempDir::new().unwrap();
    let store = WorkspaceStore::new(temp.path().join("workspace.toml"), temp.path().join("sql"));

    let mut duplicate_profiles = valid_snapshot();
    duplicate_profiles
        .profiles
        .push(duplicate_profiles.profiles[0].clone());
    assert!(matches!(
        store.save(&duplicate_profiles),
        Err(WorkspaceError::Invalid(_))
    ));

    let mut missing_console = valid_snapshot();
    missing_console.profiles[0].tabs = vec![PersistedTab::Console {
        console_id: Uuid::new_v4(),
    }];
    assert!(matches!(
        store.save(&missing_console),
        Err(WorkspaceError::Invalid(_))
    ));

    let mut wrong_profile_relation = valid_snapshot();
    let other_profile = Uuid::new_v4();
    wrong_profile_relation.profiles[0].tabs = vec![PersistedTab::Relation(PersistedRelationTab {
        id: Uuid::new_v4(),
        object_id: CatalogId::new(other_profile, CatalogKind::Table, ["users"]),
        qualified_name: QualifiedName {
            database: None,
            schema: None,
            object: "users".into(),
        },
        catalog_kind: CatalogKind::Table,
        title: "users".into(),
        view: RelationView::Data,
    })];
    assert!(matches!(
        store.save(&wrong_profile_relation),
        Err(WorkspaceError::Invalid(_))
    ));
}

#[test]
fn workspace_v3_validation_rejects_duplicate_tab_ids_and_incomplete_sql() {
    let temp = TempDir::new().unwrap();
    let store = WorkspaceStore::new(temp.path().join("workspace.toml"), temp.path().join("sql"));
    let mut duplicate_tabs = valid_snapshot();
    let console_id = duplicate_tabs.profiles[0].consoles[0].id;
    duplicate_tabs.profiles[0]
        .tabs
        .push(PersistedTab::Console { console_id });
    assert!(matches!(
        store.save(&duplicate_tabs),
        Err(WorkspaceError::Invalid(_))
    ));

    let mut incomplete_sql = valid_snapshot();
    incomplete_sql.sql.clear();
    assert!(matches!(
        store.save(&incomplete_sql),
        Err(WorkspaceError::Invalid(_))
    ));
}

#[test]
fn workspace_v3_validation_rejects_console_tab_from_another_profile() {
    let temp = TempDir::new().unwrap();
    let store = WorkspaceStore::new(temp.path().join("workspace.toml"), temp.path().join("sql"));
    let mut snapshot = valid_snapshot();
    let first_profile = snapshot.profiles[0].clone();
    let second_profile_id = Uuid::new_v4();
    let second_console_id = Uuid::new_v4();
    snapshot.profiles.push(PersistedProfileWorkspace {
        profile_id: second_profile_id,
        active_tab: Some(first_profile.consoles[0].id),
        consoles: vec![console(
            second_profile_id,
            second_console_id,
            "second",
            true,
        )],
        tabs: vec![PersistedTab::Console {
            console_id: first_profile.consoles[0].id,
        }],
    });
    snapshot.sql.push((second_console_id, "select 2".into()));

    assert!(matches!(
        store.save(&snapshot),
        Err(WorkspaceError::Invalid(message))
            if message == "console tab references a missing console"
    ));
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
