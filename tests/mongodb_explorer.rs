//! Explorer wiring for MongoDB namespaces.
//!
//! Expanding a MongoDB database has to load its collections, and expanding a
//! collection has to load its indexes, independently of whether the user
//! activates the row or expands it with the expand keybinding.

use lazydb::{
    action::{Action, Command},
    app::App,
    db::{
        ServerInfo,
        catalog_mutation::CatalogMutationCapabilities,
        mongodb::types::{
            MongoCollectionInfo, MongoCollectionType, MongoDatabaseListing, MongoExplorerNodeId,
            MongoNamespace,
        },
    },
    identity::ConnectionIdentity,
    model::{
        execution_target::ExecutionTarget, explorer::ExplorerNodeId, relation::RelationView,
        tab::WorkspaceTab, workspace::Focus,
    },
    profile::{DatabaseKind, import_connection_url},
};
use uuid::Uuid;

const DATABASE: &str = "speedbike_db";
const COLLECTION: &str = "bikes";

fn mongo_app() -> (App, Uuid, u64) {
    let profile = import_connection_url("mongodb://localhost:27017/speedbike_db", Some("mongo"))
        .expect("mongodb connection url")
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    let generation = match app.update(Action::RequestConnect(profile_id)).as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    let target = ExecutionTarget::from_profile(
        app.profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .expect("profile"),
    );
    let identity = ConnectionIdentity {
        profile_id,
        generation,
    };
    app.sessions.register_attempt(target, identity);
    let server = ServerInfo {
        kind: DatabaseKind::MongoDb,
        version: "8.0.34".into(),
        database: DATABASE.into(),
        current_user: None,
    };
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: server.clone(),
        mutation_capabilities: CatalogMutationCapabilities::default(),
    });
    assert!(
        app.sessions
            .accept_success(identity, server, CatalogMutationCapabilities::default())
    );
    app.update(Action::MongoDatabasesLoaded {
        profile_id,
        generation,
        listing: MongoDatabaseListing {
            databases: vec![DATABASE.to_owned()],
            authorized_only: true,
            warning: None,
        },
    });
    app.explorer
        .normalized
        .expanded
        .insert(ExplorerNodeId::Profile(profile_id));
    app.focus = Focus::Explorer;
    (app, profile_id, generation)
}

fn database_node(profile_id: Uuid) -> ExplorerNodeId {
    ExplorerNodeId::Mongo {
        profile_id,
        node: MongoExplorerNodeId::Database(DATABASE.to_owned()),
    }
}

fn collection_namespace() -> MongoNamespace {
    MongoNamespace::collection(DATABASE, COLLECTION)
}

fn collection_node(profile_id: Uuid) -> ExplorerNodeId {
    ExplorerNodeId::Mongo {
        profile_id,
        node: MongoExplorerNodeId::Collection(collection_namespace()),
    }
}

fn load_collections(app: &mut App, profile_id: Uuid, generation: u64) {
    app.update(Action::MongoCollectionsLoaded {
        profile_id,
        generation,
        database: DATABASE.to_owned(),
        collections: vec![MongoCollectionInfo {
            namespace: collection_namespace(),
            collection_type: MongoCollectionType::Collection,
            options_ejson: None,
        }],
    });
    app.explorer
        .normalized
        .expanded
        .insert(database_node(profile_id));
}

fn expect_collections_command(commands: &[Command], profile_id: Uuid, generation: u64) {
    let [
        Command::DiscoverMongoCollections {
            profile_id: requested_profile,
            generation: requested_generation,
            database,
        },
    ] = commands
    else {
        panic!("expected one mongodb collection discovery command, got {commands:?}");
    };
    assert_eq!(*requested_profile, profile_id);
    assert_eq!(*requested_generation, generation);
    assert_eq!(database, DATABASE);
}

#[test]
fn expanding_a_mongodb_database_loads_its_collections() {
    let (mut app, profile_id, generation) = mongo_app();
    assert!(app.explorer.select_id(database_node(profile_id)));

    let commands = app.update(Action::ExplorerExpand);

    expect_collections_command(&commands, profile_id, generation);
}

#[test]
fn expanding_a_mongodb_collection_loads_its_indexes() {
    let (mut app, profile_id, generation) = mongo_app();
    load_collections(&mut app, profile_id, generation);
    assert!(app.explorer.select_id(collection_node(profile_id)));

    let commands = app.update(Action::ExplorerExpand);

    let [
        Command::DiscoverMongoIndexes {
            profile_id: requested_profile,
            generation: requested_generation,
            namespace,
        },
    ] = commands.as_slice()
    else {
        panic!("expected one mongodb index discovery command, got {commands:?}");
    };
    assert_eq!(*requested_profile, profile_id);
    assert_eq!(*requested_generation, generation);
    assert_eq!(*namespace, collection_namespace());
}

#[test]
fn activating_a_mongodb_database_still_loads_its_collections() {
    let (mut app, profile_id, generation) = mongo_app();
    assert!(app.explorer.select_id(database_node(profile_id)));

    let commands = app.update(Action::ExplorerOpenSelected);

    expect_collections_command(&commands, profile_id, generation);
}

fn projected_ids(app: &App) -> Vec<ExplorerNodeId> {
    app.explorer
        .viewport(40)
        .rows
        .into_iter()
        .map(|row| row.id)
        .collect()
}

fn assert_database_children_projected(app: &App, context: &str) {
    let ids = projected_ids(app);
    assert!(
        !ids.iter().any(|id| matches!(
            id,
            ExplorerNodeId::Mongo {
                node: MongoExplorerNodeId::Monitor(_) | MongoExplorerNodeId::Users(_),
                ..
            }
        )),
        "the tree lists only databases and collections, no action rows ({context}), got {ids:?}"
    );
    assert!(
        ids.iter()
            .any(|id| matches!(id, ExplorerNodeId::Status { .. })),
        "an expanded mongodb database still shows its loading row ({context}), got {ids:?}"
    );
}

#[test]
fn toggling_a_mongodb_database_reveals_its_children() {
    let (mut app, profile_id, _generation) = mongo_app();
    let node = database_node(profile_id);
    assert!(app.explorer.select_id(node.clone()));

    app.update(Action::ExplorerToggleNode(node.clone()));

    assert!(
        app.explorer.normalized.expanded.contains(&node),
        "toggling a mongodb database must leave the node expanded"
    );
    assert_database_children_projected(&app, "after toggle");
}

#[test]
fn activating_a_mongodb_database_reveals_and_loads_its_children() {
    let (mut app, profile_id, generation) = mongo_app();
    let node = database_node(profile_id);
    assert!(app.explorer.select_id(node.clone()));

    let commands = app.update(Action::ExplorerOpenSelected);

    expect_collections_command(&commands, profile_id, generation);
    assert!(
        app.explorer.normalized.expanded.contains(&node),
        "activating a mongodb database must leave the node expanded"
    );
    assert_database_children_projected(&app, "after activation");
}

#[test]
fn expanding_a_mongodb_database_reveals_its_children() {
    let (mut app, profile_id, _generation) = mongo_app();
    let node = database_node(profile_id);
    assert!(app.explorer.select_id(node.clone()));

    app.update(Action::ExplorerExpand);

    assert!(app.explorer.normalized.expanded.contains(&node));
    assert_database_children_projected(&app, "after expand");
}

#[test]
fn activating_a_mongodb_collection_opens_its_rows_on_the_first_press() {
    let (mut app, profile_id, generation) = mongo_app();
    load_collections(&mut app, profile_id, generation);
    let node = collection_node(profile_id);
    assert!(app.explorer.select_id(node.clone()));

    let commands = app.update(Action::ExplorerOpenSelected);

    let Some(WorkspaceTab::Relation(tab)) = app.tabs.last() else {
        panic!(
            "one enter on a collection must open its rows, got {} tabs",
            app.tabs.len()
        );
    };
    assert_eq!(tab.view, RelationView::Data);
    assert_eq!(app.active_tab, app.tabs.len() - 1);
    assert_eq!(app.focus, Focus::Results);
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, Command::LoadMongoRelationPreview(_))),
        "opening the rows must request the first document page, got {commands:?}"
    );
    assert!(
        !app.explorer.normalized.expanded.contains(&node),
        "enter must not reveal the collection branch"
    );
    assert!(
        !projected_ids(&app).iter().any(|id| matches!(
            id,
            ExplorerNodeId::Mongo {
                node: MongoExplorerNodeId::Index { .. },
                ..
            }
        )),
        "enter must not reveal the collection's indexes, got {:?}",
        projected_ids(&app)
    );
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::DiscoverMongoIndexes { .. })),
        "enter must not load the collection's indexes, got {commands:?}"
    );
}

#[test]
fn toggling_a_mongodb_collection_only_toggles_expansion() {
    let (mut app, profile_id, generation) = mongo_app();
    load_collections(&mut app, profile_id, generation);
    let node = collection_node(profile_id);
    assert!(app.explorer.select_id(node.clone()));
    app.update(Action::ExplorerExpand);

    let commands = app.update(Action::ExplorerToggle);

    assert!(
        !app.explorer.normalized.expanded.contains(&node),
        "toggling an expanded collection must collapse it"
    );
    assert!(
        !app.tabs
            .iter()
            .any(|tab| matches!(tab, WorkspaceTab::Relation(_))),
        "collapsing a collection must not open its rows"
    );
    assert!(
        commands.is_empty(),
        "collapsing a collection must emit nothing, got {commands:?}"
    );
}

#[test]
fn activating_an_expanded_collection_preserves_the_tree_and_reuses_the_tab() {
    let (mut app, profile_id, generation) = mongo_app();
    load_collections(&mut app, profile_id, generation);
    let node = collection_node(profile_id);
    assert!(app.explorer.select_id(node.clone()));
    app.update(Action::ExplorerExpand);
    let expanded = app.explorer.normalized.expanded.clone();
    app.update(Action::ExplorerPrimary);
    let Some(WorkspaceTab::Relation(tab)) = app.tabs.get(app.active_tab) else {
        panic!("primary activation must open a relation");
    };
    let tab_id = tab.id;
    let count = app.tabs.len();
    assert_eq!(app.explorer.normalized.expanded, expanded);
    app.focus = Focus::Explorer;
    let commands = app.update(Action::ExplorerOpenSelected);
    assert_eq!(app.tabs.len(), count);
    assert!(
        matches!(app.tabs.get(app.active_tab), Some(WorkspaceTab::Relation(tab)) if tab.id == tab_id)
    );
    assert_eq!(app.explorer.normalized.expanded, expanded);
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::DiscoverMongoIndexes { .. }))
    );

    // The mouse chevron uses ToggleNode, and must only collapse the branch.
    let commands = app.update(Action::ExplorerToggleNode(node.clone()));
    assert!(commands.is_empty());
    assert_eq!(app.tabs.len(), count);
    assert_eq!(app.focus, Focus::Explorer);
    assert!(!app.explorer.normalized.expanded.contains(&node));
}

#[test]
fn monitor_and_users_open_from_the_database_row() {
    let (mut app, profile_id, _generation) = mongo_app();
    assert!(app.explorer.select_id(database_node(profile_id)));

    let commands = app.update(Action::OpenMongoNode(MongoExplorerNodeId::Monitor(
        DATABASE.to_owned(),
    )));
    let [Command::RefreshMongoMonitor { database, .. }] = commands.as_slice() else {
        panic!("monitor must open its dashboard, got {commands:?}");
    };
    assert_eq!(database, DATABASE);

    let commands = app.update(Action::OpenMongoNode(MongoExplorerNodeId::Users(
        DATABASE.to_owned(),
    )));
    let [Command::LoadMongoPrincipals { database, .. }] = commands.as_slice() else {
        panic!("users must open its dashboard, got {commands:?}");
    };
    assert_eq!(database, DATABASE);
}
