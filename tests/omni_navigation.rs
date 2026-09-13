use lazydb::{
    action::{Action, Command},
    app::App,
    commands::{CommandContext, CommandId},
    db::{
        ServerInfo,
        catalog::{CatalogEntry, CatalogId, CatalogKind, OptionalMetadata, QualifiedName},
    },
    model::{omni::OmniItemId, relation::RelationView, workspace::ConnectionStatus},
    profile::{DatabaseKind, import_connection_url},
};
use uuid::Uuid;

fn server(kind: DatabaseKind) -> ServerInfo {
    ServerInfo {
        kind,
        version: "test".into(),
        database: ":memory:".into(),
        current_user: None,
    }
}

#[test]
fn selecting_cross_profile_relation_resumes_only_after_matching_connection_success() {
    let first = import_connection_url("sqlite::memory:", Some("first"))
        .unwrap()
        .profile;
    let second = import_connection_url("sqlite:/tmp/second.db", Some("second"))
        .unwrap()
        .profile;
    let first_id = first.id;
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);

    let generation = match app.update(Action::RequestConnect(first_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ConnectionSucceeded {
        profile_id: first_id,
        generation,
        server: server(DatabaseKind::Sqlite),
        mutation_capabilities: Default::default(),
    });
    let other_table = add_table(&mut app, first_id, "other");
    let target_table = add_table(&mut app, second_id, "users");

    app.update(Action::OpenOmni);
    app.omni.as_mut().unwrap().selected = Some(OmniItemId::Catalog(target_table.clone()));
    let commands = app.update(Action::OmniConfirm);
    let next_generation = match commands.as_slice() {
        [
            Command::Connect {
                profile_id,
                generation,
                ..
            },
        ] if *profile_id == second_id => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    assert_eq!(app.connection.profile_id, Some(first_id));
    assert!(!app.tabs.iter().any(|tab| matches!(
        tab,
        lazydb::model::tab::WorkspaceTab::Relation(tab)
            if tab.descriptor.key.object_id == target_table
    )));

    app.update(Action::ConnectionSucceeded {
        profile_id: first_id,
        generation,
        server: server(DatabaseKind::Sqlite),
        mutation_capabilities: Default::default(),
    });
    assert_eq!(app.connection.profile_id, Some(first_id));

    app.update(Action::ConnectionSucceeded {
        profile_id: second_id,
        generation: next_generation,
        server: server(DatabaseKind::Sqlite),
        mutation_capabilities: Default::default(),
    });
    assert_eq!(app.connection.profile_id, Some(second_id));
    assert!(matches!(app.connection.status, ConnectionStatus::Connected));
    assert!(!app.tabs.iter().any(|tab| matches!(
        tab,
        lazydb::model::tab::WorkspaceTab::Relation(tab)
            if tab.descriptor.key.object_id == other_table
    )));
    assert!(app.tabs.iter().any(|tab| matches!(
        tab,
        lazydb::model::tab::WorkspaceTab::Relation(tab)
            if tab.descriptor.key.object_id == target_table
                && tab.view == RelationView::Data
    )));
}

#[test]
fn opening_another_console_records_and_restores_the_previous_location() {
    let mut app = App::new(Vec::new());
    let first = app.active_console().id;
    app.update(Action::NewConsoleNamed("analysis".into()));
    let second = app.active_console().id;

    app.update(Action::OpenOmni);
    app.omni.as_mut().unwrap().selected = Some(OmniItemId::Console {
        profile_id: None,
        console_id: first,
    });
    app.update(Action::OmniConfirm);
    assert_eq!(app.active_console().id, first);

    app.update(Action::ExecuteSemanticCommand {
        id: CommandId::ReturnToPreviousLocation,
        context: CommandContext::default(),
    });
    assert_eq!(app.active_console().id, second);

    app.update(Action::ExecuteSemanticCommand {
        id: CommandId::ReturnToPreviousLocation,
        context: CommandContext::default(),
    });
    assert_eq!(app.active_console().id, second);
    app.update(Action::ExecuteSemanticCommand {
        id: CommandId::ReturnToPreviousLocation,
        context: CommandContext::default(),
    });
    assert_eq!(app.active_console().id, second);
}

fn add_table(app: &mut App, profile_id: Uuid, name: &str) -> CatalogId {
    let database_id = CatalogId::new(profile_id, CatalogKind::Database, [":memory:"]);
    let schema_id = CatalogId::new(profile_id, CatalogKind::Schema, [":memory:", "main"]);
    let table_id = CatalogId::new(profile_id, CatalogKind::Table, [":memory:", "main", name]);
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
    let table = CatalogEntry::relation(
        table_id.clone(),
        schema_id,
        QualifiedName {
            database: Some(":memory:".into()),
            schema: Some("main".into()),
            object: name.into(),
        },
        "table",
        OptionalMetadata::Unsupported,
        true,
    )
    .unwrap();
    let tree = &mut app
        .explorer
        .normalized
        .profiles
        .get_mut(&profile_id)
        .unwrap()
        .catalog;
    tree.insert_subtree(vec![database, schema, table]).unwrap();
    table_id
}
