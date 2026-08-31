use lazydb::model::transaction::TransactionMode;
use lazydb::persistence::workspace::{
    PersistedConsole, PersistedProfileWorkspace, PersistedTab, WorkspaceSnapshot,
};
use lazydb::profile::import_connection_url;
use lazydb::{
    action::{Action, Command},
    app::App,
    db::catalog::{
        CatalogCount, CatalogCursor, CatalogEntry, CatalogGroupSummary, CatalogId, CatalogKind,
        CatalogPage, CatalogRequest, CatalogTarget, ObjectGroup, OptionalMetadata, QualifiedName,
    },
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
fn new_app_with_profiles_has_no_active_workspace_until_connected() {
    let profile = import_connection_url(":memory:", Some("saved"))
        .unwrap()
        .profile;
    let app = App::new(vec![profile]);

    assert_eq!(app.active_workspace_profile, None);
    assert!(app.tabs.is_empty());
    assert!(app.sql_editors.is_empty());
    assert!(app.active_console_opt().is_none());
}

#[test]
fn connection_workspace_preserves_mixed_tab_order() {
    let console = ConsoleTab::new("sql");
    let relation = RelationTab::new("users");
    let console_id = console.id;
    let relation_id = relation.id;
    let workspace = lazydb::model::workspace::ConnectionWorkspace {
        tabs: vec![WorkspaceTab::Relation(relation), WorkspaceTab::Sql(console)],
        sql_editors: Vec::new(),
        sql: Vec::new(),
        active_tab_id: Some(console_id),
    };

    assert_eq!(workspace.tabs[0].id(), relation_id);
    assert_eq!(workspace.tabs[1].id(), console_id);
    assert_eq!(workspace.active_tab_id, Some(console_id));
}

#[test]
fn connection_workspace_active_tab_is_an_id_not_an_index() {
    let first = ConsoleTab::new("first");
    let second = ConsoleTab::new("second");
    let second_id = second.id;
    let workspace = lazydb::model::workspace::ConnectionWorkspace {
        tabs: vec![WorkspaceTab::Sql(first), WorkspaceTab::Sql(second)],
        sql_editors: Vec::new(),
        sql: Vec::new(),
        active_tab_id: Some(second_id),
    };

    let reordered = [workspace.tabs[1].id(), workspace.tabs[0].id()];
    assert_eq!(reordered[0], workspace.active_tab_id.unwrap());
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
    let mut app = App::new(Vec::new());
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
    assert!(app.active_console().execution_target.is_none());
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
        active_profile: None,
        profiles: Vec::new(),
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
    let mut app = App::new(vec![profile.clone()]);
    assert!(app.active_console_opt().is_none());

    app.connection.profile_id = Some(profile.id);
    let expected = lazydb::model::execution_target::ExecutionTarget::from_profile(&profile);
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
        active_profile: Some(profile.id),
        profiles: Vec::new(),
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

#[test]
fn workspace_restore_caches_profiles_without_exposing_a_workspace() {
    let first = import_connection_url(":memory:", Some("first"))
        .unwrap()
        .profile;
    let second = import_connection_url(":memory:", Some("second"))
        .unwrap()
        .profile;
    let first_id = Uuid::new_v4();
    let second_id = Uuid::new_v4();
    let snapshot = WorkspaceSnapshot {
        active_profile: Some(second.id),
        profiles: vec![
            profile_workspace(first.id, first_id, None),
            profile_workspace(second.id, second_id, Some(second_id)),
        ],
        active_console: Uuid::nil(),
        consoles: Vec::new(),
        sql: vec![
            (first_id, "select first".into()),
            (second_id, "select second".into()),
        ],
    };
    let mut app = App::new(vec![first.clone(), second.clone()]);
    app.restore_workspace(snapshot, Some(first.id));

    assert!(app.tabs.is_empty());
    assert!(app.sql_editors.is_empty());
    assert_eq!(app.active_workspace_profile, None);
    let restored = app.workspace_snapshot();
    assert_eq!(
        restored
            .profiles
            .iter()
            .map(|profile| profile.profile_id)
            .collect::<Vec<_>>(),
        vec![first.id, second.id]
    );
    assert_eq!(restored.sql.len(), 2);
}

#[test]
fn workspace_restore_ignores_invalid_or_deleted_targets_and_uses_first_profile() {
    let profile = import_connection_url(":memory:", Some("startup"))
        .unwrap()
        .profile;
    let invalid_id = Uuid::new_v4();
    let deleted_profile = Uuid::new_v4();
    let snapshot = WorkspaceSnapshot {
        active_profile: None,
        profiles: vec![profile_workspace(
            deleted_profile,
            invalid_id,
            Some(invalid_id),
        )],
        active_console: Uuid::nil(),
        consoles: Vec::new(),
        sql: vec![(invalid_id, "select 1".into())],
    };
    let mut app = App::new(vec![profile.clone()]);
    app.restore_workspace(snapshot, None);

    assert!(app.tabs.is_empty());
    assert!(app.sql_editors.is_empty());
    assert_eq!(app.active_workspace_profile, None);
}

#[test]
fn workspace_restore_assigns_targetless_consoles_to_startup_then_first_profile() {
    let first = import_connection_url(":memory:", Some("first"))
        .unwrap()
        .profile;
    let second = import_connection_url(":memory:", Some("second"))
        .unwrap()
        .profile;
    let id = Uuid::new_v4();
    let snapshot = WorkspaceSnapshot {
        active_profile: None,
        profiles: vec![profile_workspace(Uuid::nil(), id, Some(id))],
        active_console: id,
        consoles: Vec::new(),
        sql: vec![(id, "select 1".into())],
    };
    let mut app = App::new(vec![first.clone(), second.clone()]);
    app.restore_workspace(snapshot.clone(), Some(second.id));
    assert!(app.tabs.is_empty());
    assert!(app.sql_editors.is_empty());

    let mut app = App::new(vec![first.clone(), second]);
    app.restore_workspace(snapshot, None);
    assert!(app.tabs.is_empty());
    assert!(app.sql_editors.is_empty());
}

#[test]
fn workspace_restore_rebuilds_all_profile_tabs_and_preserves_hidden_sql() {
    let first = import_connection_url(":memory:", Some("first"))
        .unwrap()
        .profile;
    let second = import_connection_url(":memory:", Some("second"))
        .unwrap()
        .profile;
    let console_id = Uuid::new_v4();
    let hidden_id = Uuid::new_v4();
    let relation_id = Uuid::new_v4();
    let other_console_id = Uuid::new_v4();
    let snapshot = WorkspaceSnapshot {
        active_profile: Some(first.id),
        profiles: vec![
            PersistedProfileWorkspace {
                profile_id: first.id,
                active_tab: Some(relation_id),
                consoles: vec![
                    persisted_console(console_id, "first", true),
                    persisted_console(hidden_id, "hidden", false),
                ],
                tabs: vec![
                    PersistedTab::Console { console_id },
                    PersistedTab::Relation(lazydb::persistence::workspace::PersistedRelationTab {
                        id: relation_id,
                        object_id: lazydb::db::catalog::CatalogId::new(
                            first.id,
                            lazydb::db::catalog::CatalogKind::Table,
                            ["users"],
                        ),
                        qualified_name: lazydb::db::catalog::QualifiedName {
                            database: None,
                            schema: Some("public".into()),
                            object: "users".into(),
                        },
                        catalog_kind: lazydb::db::catalog::CatalogKind::Table,
                        title: "users".into(),
                        view: lazydb::model::relation::RelationView::Ddl,
                    }),
                ],
            },
            profile_workspace(second.id, other_console_id, Some(other_console_id)),
        ],
        active_console: Uuid::nil(),
        consoles: Vec::new(),
        sql: vec![
            (console_id, "select first".into()),
            (hidden_id, "select hidden".into()),
            (other_console_id, "select second".into()),
        ],
    };
    let mut app = App::new(vec![first.clone(), second]);
    app.restore_workspace(snapshot.clone(), Some(first.id));

    assert!(app.tabs.is_empty());
    assert!(app.sql_editors.is_empty());
    assert_eq!(app.active_workspace_profile, None);

    let restored = app.workspace_snapshot();
    assert_eq!(restored.profiles.len(), 2);
    assert_eq!(restored.sql, snapshot.sql);
    assert!(matches!(
        restored.profiles[0].tabs[0],
        PersistedTab::Console { console_id: id } if id == console_id
    ));
    assert!(matches!(
        restored.profiles[0].tabs[1],
        PersistedTab::Relation(ref relation) if relation.id == relation_id
    ));
}

#[test]
fn restored_relation_tab_is_not_loaded_before_connection_installation() {
    let profile = import_connection_url(":memory:", Some("first"))
        .unwrap()
        .profile;
    let relation_id = Uuid::new_v4();
    let snapshot = WorkspaceSnapshot {
        active_profile: Some(profile.id),
        profiles: vec![PersistedProfileWorkspace {
            profile_id: profile.id,
            active_tab: Some(relation_id),
            consoles: Vec::new(),
            tabs: vec![PersistedTab::Relation(
                lazydb::persistence::workspace::PersistedRelationTab {
                    id: relation_id,
                    object_id: lazydb::db::catalog::CatalogId::new(
                        profile.id,
                        lazydb::db::catalog::CatalogKind::Table,
                        ["users"],
                    ),
                    qualified_name: lazydb::db::catalog::QualifiedName {
                        database: None,
                        schema: Some("main".into()),
                        object: "users".into(),
                    },
                    catalog_kind: lazydb::db::catalog::CatalogKind::Table,
                    title: "users".into(),
                    view: lazydb::model::relation::RelationView::Data,
                },
            )],
        }],
        active_console: Uuid::nil(),
        consoles: Vec::new(),
        sql: Vec::new(),
    };
    let mut app = App::new(vec![profile]);
    app.restore_workspace(snapshot, None);

    let restored = app.workspace_snapshot();
    assert!(matches!(
        &restored.profiles[0].tabs[0],
        PersistedTab::Relation(relation) if relation.id == relation_id
    ));
}

#[test]
fn restored_relation_waits_for_catalog_before_loading() {
    let profile = import_connection_url(":memory:", Some("first"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let first_id = Uuid::new_v4();
    let second_id = Uuid::new_v4();
    let database_id = CatalogId::new(profile_id, CatalogKind::Database, [":memory:"]);
    let schema_id = CatalogId::new(profile_id, CatalogKind::Schema, [":memory:", "main"]);
    let first_relation_id = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "first"],
    );
    let relation = |id: Uuid, title: &str| {
        PersistedTab::Relation(lazydb::persistence::workspace::PersistedRelationTab {
            id,
            object_id: CatalogId::new(profile_id, CatalogKind::Table, [":memory:", "main", title]),
            qualified_name: QualifiedName {
                database: Some(":memory:".into()),
                schema: Some("main".into()),
                object: title.into(),
            },
            catalog_kind: CatalogKind::Table,
            title: title.into(),
            view: lazydb::model::relation::RelationView::Data,
        })
    };
    let snapshot = WorkspaceSnapshot {
        active_profile: Some(profile_id),
        profiles: vec![PersistedProfileWorkspace {
            profile_id,
            active_tab: Some(first_id),
            consoles: Vec::new(),
            tabs: vec![relation(first_id, "first"), relation(second_id, "second")],
        }],
        active_console: Uuid::nil(),
        consoles: Vec::new(),
        sql: Vec::new(),
    };
    let mut app = App::new(vec![profile]);
    app.restore_workspace(snapshot, None);
    let generation = match app.update(Action::RequestConnect(profile_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };

    let commands = app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: lazydb::db::ServerInfo {
            kind: lazydb::profile::DatabaseKind::Sqlite,
            version: "3".into(),
            database: ":memory:".into(),
        },
    });
    assert_eq!(
        commands
            .iter()
            .filter(|command| matches!(command, Command::LoadCatalogPage(_)))
            .count(),
        1
    );
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::LoadRelationPreview(_)))
    );
    assert!(matches!(app.tabs[0], WorkspaceTab::Relation(ref tab)
        if matches!(tab.data, lazydb::model::relation::RelationLoad::Empty)));
    assert!(matches!(app.tabs[1], WorkspaceTab::Relation(ref tab)
        if matches!(tab.data, lazydb::model::relation::RelationLoad::Empty)));

    let database_request = commands
        .iter()
        .find_map(|command| match command {
            Command::LoadCatalogPage(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    let database = CatalogEntry::database(
        database_id.clone(),
        QualifiedName {
            database: Some(":memory:".into()),
            schema: None,
            object: ":memory:".into(),
        },
        "database",
        OptionalMetadata::Unsupported,
        true,
    )
    .unwrap();
    let commands = app.update(Action::CatalogPageLoaded(
        CatalogPage::new(
            &database_request,
            vec![database],
            CatalogCount::Exact(1),
            None,
        )
        .unwrap(),
    ));
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::LoadRelationPreview(_)))
    );

    let schema_target = CatalogTarget::schemas(database_id.clone()).unwrap();
    let schema_request = pending_catalog_request(&app, profile_id, &schema_target);
    let schema = CatalogEntry::schema(
        schema_id.clone(),
        database_id,
        QualifiedName {
            database: Some(":memory:".into()),
            schema: Some("main".into()),
            object: "main".into(),
        },
        "schema",
        OptionalMetadata::Unsupported,
        true,
    )
    .unwrap();
    let commands = app.update(Action::CatalogPageLoaded(
        CatalogPage::new(&schema_request, vec![schema], CatalogCount::Exact(1), None).unwrap(),
    ));
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::LoadRelationPreview(_)))
    );

    let groups_target = CatalogTarget::groups(schema_id.clone()).unwrap();
    let groups_request = pending_catalog_request(&app, profile_id, &groups_target);
    let commands = app.update(Action::CatalogPageLoaded(
        CatalogPage::groups(
            &groups_request,
            vec![CatalogGroupSummary {
                group: ObjectGroup::Tables,
                object_count: CatalogCount::Exact(1),
            }],
            CatalogCount::Exact(1),
            None,
        )
        .unwrap(),
    ));
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::LoadRelationPreview(_)))
    );

    let tables_target = CatalogTarget::objects(schema_id.clone(), ObjectGroup::Tables).unwrap();
    let tables_request = pending_catalog_request(&app, profile_id, &tables_target);
    let first_relation = CatalogEntry::relation(
        first_relation_id.clone(),
        schema_id,
        QualifiedName {
            database: Some(":memory:".into()),
            schema: Some("main".into()),
            object: "first".into(),
        },
        "table",
        OptionalMetadata::Unsupported,
        true,
    )
    .unwrap();
    let commands = app.update(Action::CatalogPageLoaded(
        CatalogPage::new(
            &tables_request,
            vec![first_relation],
            CatalogCount::Exact(1),
            None,
        )
        .unwrap(),
    ));
    assert!(matches!(
        commands.as_slice(),
        [Command::LoadRelationPreview(request)]
            if request.tab_id == first_id
                && request.relation.object_id == first_relation_id
    ));
    let commands = app.update(Action::ActivateTab(0));
    assert!(!commands.iter().any(|command| matches!(
        command,
        Command::LoadRelationPreview(request) if request.tab_id == first_id
    )));
}

fn pending_catalog_request(app: &App, profile_id: Uuid, target: &CatalogTarget) -> CatalogRequest {
    let owner = lazydb::model::explorer::owner_for_target(profile_id, target);
    app.explorer.normalized.profiles[&profile_id].pending_requests[&owner].clone()
}

#[test]
fn restored_ddl_relation_loads_after_catalog_identity_arrives() {
    let (mut app, profile_id, tab_id, relation_id, schema_id, tables_request) =
        restored_relation_at_tables_request(lazydb::model::relation::RelationView::Ddl);
    let relation = catalog_relation(relation_id.clone(), schema_id, "target");

    let commands = app.update(Action::CatalogPageLoaded(
        CatalogPage::new(
            &tables_request,
            vec![relation],
            CatalogCount::Exact(1),
            None,
        )
        .unwrap(),
    ));

    assert!(matches!(
        commands.as_slice(),
        [Command::LoadRelationDdl(request)]
            if request.tab_id == tab_id
                && request.connection.profile_id == profile_id
                && request.relation.object_id == relation_id
    ));
}

#[test]
fn restored_relation_waits_until_later_catalog_page_contains_identity() {
    let (mut app, profile_id, tab_id, relation_id, schema_id, mut tables_request) =
        restored_relation_at_tables_request(lazydb::model::relation::RelationView::Data);
    let other_id = CatalogId::new(
        relation_id.profile_id(),
        CatalogKind::Table,
        [":memory:", "main", "other"],
    );
    let cursor = CatalogCursor::from_keyset("other", "other").unwrap();
    tables_request.page_size = 1;
    let owner = lazydb::model::explorer::owner_for_target(profile_id, &tables_request.key.target);
    app.explorer
        .normalized
        .profiles
        .get_mut(&profile_id)
        .unwrap()
        .pending_requests
        .insert(owner, tables_request.clone());
    let commands = app.update(Action::CatalogPageLoaded(
        CatalogPage::new(
            &tables_request,
            vec![catalog_relation(other_id, schema_id.clone(), "other")],
            CatalogCount::Exact(2),
            Some(cursor),
        )
        .unwrap(),
    ));
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::LoadRelationPreview(_)))
    );
    let continuation = commands
        .iter()
        .find_map(|command| match command {
            Command::LoadCatalogPage(request) => Some(request.clone()),
            _ => None,
        })
        .expect("catalog continuation request");

    let commands = app.update(Action::CatalogPageLoaded(
        CatalogPage::new(
            &continuation,
            vec![catalog_relation(relation_id.clone(), schema_id, "target")],
            CatalogCount::Exact(2),
            None,
        )
        .unwrap(),
    ));

    assert!(matches!(
        commands.as_slice(),
        [Command::LoadRelationPreview(request)]
            if request.tab_id == tab_id && request.relation.object_id == relation_id
    ));
}

fn restored_relation_at_tables_request(
    view: lazydb::model::relation::RelationView,
) -> (App, Uuid, Uuid, CatalogId, CatalogId, CatalogRequest) {
    let profile = import_connection_url(":memory:", Some("restored"))
        .unwrap()
        .profile;
    let profile_id = profile.id;
    let tab_id = Uuid::new_v4();
    let database_id = CatalogId::new(profile_id, CatalogKind::Database, [":memory:"]);
    let schema_id = CatalogId::new(profile_id, CatalogKind::Schema, [":memory:", "main"]);
    let relation_id = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [":memory:", "main", "target"],
    );
    let snapshot = WorkspaceSnapshot {
        active_profile: Some(profile_id),
        profiles: vec![PersistedProfileWorkspace {
            profile_id,
            active_tab: Some(tab_id),
            consoles: Vec::new(),
            tabs: vec![PersistedTab::Relation(
                lazydb::persistence::workspace::PersistedRelationTab {
                    id: tab_id,
                    object_id: relation_id.clone(),
                    qualified_name: QualifiedName {
                        database: Some(":memory:".into()),
                        schema: Some("main".into()),
                        object: "target".into(),
                    },
                    catalog_kind: CatalogKind::Table,
                    title: "target".into(),
                    view,
                },
            )],
        }],
        active_console: Uuid::nil(),
        consoles: Vec::new(),
        sql: Vec::new(),
    };
    let mut app = App::new(vec![profile]);
    app.restore_workspace(snapshot, None);
    let generation = match app.update(Action::RequestConnect(profile_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    let commands = app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: lazydb::db::ServerInfo {
            kind: lazydb::profile::DatabaseKind::Sqlite,
            version: "3".into(),
            database: ":memory:".into(),
        },
    });
    assert!(!commands.iter().any(|command| matches!(
        command,
        Command::LoadRelationPreview(_) | Command::LoadRelationDdl(_)
    )));
    let database_request = commands
        .iter()
        .find_map(|command| match command {
            Command::LoadCatalogPage(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    let database = CatalogEntry::database(
        database_id.clone(),
        QualifiedName {
            database: Some(":memory:".into()),
            schema: None,
            object: ":memory:".into(),
        },
        "database",
        OptionalMetadata::Unsupported,
        true,
    )
    .unwrap();
    app.update(Action::CatalogPageLoaded(
        CatalogPage::new(
            &database_request,
            vec![database],
            CatalogCount::Exact(1),
            None,
        )
        .unwrap(),
    ));
    let schema_target = CatalogTarget::schemas(database_id.clone()).unwrap();
    let schema_request = pending_catalog_request(&app, profile_id, &schema_target);
    let schema = CatalogEntry::schema(
        schema_id.clone(),
        database_id,
        QualifiedName {
            database: Some(":memory:".into()),
            schema: Some("main".into()),
            object: "main".into(),
        },
        "schema",
        OptionalMetadata::Unsupported,
        true,
    )
    .unwrap();
    app.update(Action::CatalogPageLoaded(
        CatalogPage::new(&schema_request, vec![schema], CatalogCount::Exact(1), None).unwrap(),
    ));
    let groups_target = CatalogTarget::groups(schema_id.clone()).unwrap();
    let groups_request = pending_catalog_request(&app, profile_id, &groups_target);
    app.update(Action::CatalogPageLoaded(
        CatalogPage::groups(
            &groups_request,
            vec![CatalogGroupSummary {
                group: ObjectGroup::Tables,
                object_count: CatalogCount::Exact(1),
            }],
            CatalogCount::Exact(1),
            None,
        )
        .unwrap(),
    ));
    let tables_target = CatalogTarget::objects(schema_id.clone(), ObjectGroup::Tables).unwrap();
    let tables_request = pending_catalog_request(&app, profile_id, &tables_target);
    (
        app,
        profile_id,
        tab_id,
        relation_id,
        schema_id,
        tables_request,
    )
}

fn catalog_relation(id: CatalogId, schema: CatalogId, name: &str) -> CatalogEntry {
    CatalogEntry::relation(
        id,
        schema,
        QualifiedName {
            database: Some(":memory:".into()),
            schema: Some("main".into()),
            object: name.into(),
        },
        "table",
        OptionalMetadata::Unsupported,
        true,
    )
    .unwrap()
}

#[test]
fn console_lifecycle_requires_an_active_profile_workspace() {
    let profile = import_connection_url(":memory:", Some("saved"))
        .unwrap()
        .profile;
    let mut app = App::new(vec![profile]);

    app.update(Action::NewConsole);
    app.update(Action::OpenSqlEditorList);
    app.update(Action::ActivateSqlEditor(Uuid::new_v4()));
    app.update(Action::CloseActiveTab);

    assert!(app.tabs.is_empty());
    assert!(app.sql_editors.is_empty());
}

#[test]
fn restored_console_target_cannot_cross_profile_boundaries() {
    let first = import_connection_url(":memory:", Some("first"))
        .unwrap()
        .profile;
    let second = import_connection_url(":memory:", Some("second"))
        .unwrap()
        .profile;
    let console_id = Uuid::new_v4();
    let snapshot = WorkspaceSnapshot {
        active_profile: Some(first.id),
        profiles: vec![PersistedProfileWorkspace {
            profile_id: first.id,
            active_tab: Some(console_id),
            consoles: vec![PersistedConsole {
                id: console_id,
                name: "cross-profile".into(),
                sql_file: format!("{console_id}.sql").into(),
                target: Some(
                    lazydb::model::execution_target::ExecutionTarget::from_profile(&second),
                ),
                transaction_mode: TransactionMode::Auto,
                open: true,
            }],
            tabs: vec![PersistedTab::Console { console_id }],
        }],
        active_console: Uuid::nil(),
        consoles: Vec::new(),
        sql: vec![(console_id, "select 1".into())],
    };
    let mut app = App::new(vec![first.clone(), second]);
    app.connection.profile_id = Some(first.id);
    app.restore_workspace(snapshot, Some(first.id));

    assert_eq!(app.active_workspace_profile, Some(first.id));
    assert_eq!(
        app.active_console().execution_target,
        Some(lazydb::model::execution_target::ExecutionTarget::from_profile(&first))
    );
}

#[test]
fn console_numbering_is_collision_free_within_each_workspace() {
    let profile = import_connection_url(":memory:", Some("saved"))
        .unwrap()
        .profile;
    let first_id = Uuid::new_v4();
    let snapshot = WorkspaceSnapshot {
        active_profile: Some(profile.id),
        profiles: vec![PersistedProfileWorkspace {
            profile_id: profile.id,
            active_tab: Some(first_id),
            consoles: vec![PersistedConsole {
                id: first_id,
                name: "console_1".into(),
                sql_file: format!("{first_id}.sql").into(),
                target: None,
                transaction_mode: TransactionMode::Auto,
                open: true,
            }],
            tabs: vec![PersistedTab::Console {
                console_id: first_id,
            }],
        }],
        active_console: Uuid::nil(),
        consoles: Vec::new(),
        sql: vec![(first_id, String::new())],
    };
    let mut app = App::new(vec![profile.clone()]);
    app.connection.profile_id = Some(profile.id);
    app.restore_workspace(snapshot, Some(profile.id));
    app.update(Action::NewConsole);

    assert_eq!(app.active_console().name, "console_2");
    assert_eq!(app.sql_editors.len(), 2);
}

fn persisted_console(id: Uuid, name: &str, open: bool) -> PersistedConsole {
    PersistedConsole {
        id,
        name: name.into(),
        sql_file: format!("{id}.sql").into(),
        target: None,
        transaction_mode: TransactionMode::Auto,
        open,
    }
}

fn profile_workspace(
    profile_id: Uuid,
    console_id: Uuid,
    active_tab: Option<Uuid>,
) -> PersistedProfileWorkspace {
    PersistedProfileWorkspace {
        profile_id,
        active_tab,
        consoles: vec![PersistedConsole {
            id: console_id,
            name: "console".into(),
            sql_file: format!("{console_id}.sql").into(),
            target: None,
            transaction_mode: TransactionMode::Auto,
            open: true,
        }],
        tabs: vec![PersistedTab::Console { console_id }],
    }
}
