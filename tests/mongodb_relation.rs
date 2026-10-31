//! MongoDB collections open in the shared relation tab.
//!
//! T1 (S1): activating a collection in the explorer opens a `RelationTab` whose
//! DATA grid lists the collection's documents. BSON maps onto the existing
//! `CellValue` variants (S5) and paging reuses `FindSpec` skip/limit, so the
//! tab behaves like the SQL table tab it replaces.
//!
//! T4 (S3): the edit key opens the selected document as editable Canonical
//! EJSON and saving replaces exactly that document, bounded to its original
//! `_id`. This unit writes data, so every refusal below is a safety invariant:
//! each one must leave stored documents and emitted commands untouched.

mod support;

use lazydb::{
    action::{Action, Command},
    app::App,
    db::{
        ServerInfo,
        catalog::{CatalogEntry, CatalogId, CatalogKind, OptionalMetadata, QualifiedName},
        catalog_mutation::CatalogMutationCapabilities,
        mongodb::{
            read::{find_description, page_result_set, relation_find_spec},
            types::{
                FindSpec, MongoCollectionInfo, MongoCollectionStructure, MongoCollectionType,
                MongoDatabaseListing, MongoDocument, MongoDocumentPage, MongoExplorerNodeId,
                MongoIndexInfo, MongoNamespace,
            },
        },
        value::CellValue,
    },
    identity::ConnectionIdentity,
    model::{
        execution_target::ExecutionTarget,
        explorer::ExplorerNodeId,
        mongodb_structure::{MongoSchemaInference, MongoStructureSnapshot, infer_schema},
        pagination::{PageRequest, PageSize},
        relation::{RelationLoad, RelationRequest, RelationRequestKind, RelationView},
        tab::WorkspaceTab,
        workspace::Focus,
    },
    profile::{DatabaseKind, import_connection_url},
};
use uuid::Uuid;

const MONGO_DATABASE: &str = "speedbike_db";
const MONGO_COLLECTION: &str = "bikes";
const SQL_DATABASE: &str = "app";
const SQL_SCHEMA: &str = "public";
const SQL_TABLE: &str = "users";

fn document(bson: mongodb::bson::Document) -> MongoDocument {
    MongoDocument {
        raw: mongodb::bson::raw::RawDocumentBuf::from_bytes(
            mongodb::bson::to_vec(&bson).expect("encodes"),
        )
        .expect("raw document"),
        bson,
        editable: true,
    }
}

fn page(documents: Vec<MongoDocument>, next_offset: Option<u64>) -> MongoDocumentPage {
    MongoDocumentPage {
        documents,
        next_offset,
        truncated: false,
        oversized_document: false,
        bytes: 0,
    }
}

fn mongo_namespace() -> MongoNamespace {
    MongoNamespace::collection(MONGO_DATABASE, MONGO_COLLECTION)
}

// ---------------------------------------------------------------------------
// BSON -> CellValue (S5)
// ---------------------------------------------------------------------------

#[test]
fn bson_values_map_onto_the_existing_cell_value_variants() {
    let object_id = mongodb::bson::oid::ObjectId::new();
    let loaded = page(
        vec![document(mongodb::bson::doc! {
            "_id": object_id,
            "name": "Speedbike",
            "geometry": { "wheels": 2, "frame": "carbon" },
            "tags": ["road", "fast", "light"],
            "stock": 4i64,
            "shelf": 7i32,
            "active": true,
            "retired": mongodb::bson::Bson::Null,
        })],
        None,
    );

    let result = page_result_set(&loaded);

    assert_eq!(
        result
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "_id", "name", "geometry", "tags", "stock", "shelf", "active", "retired"
        ],
        "columns are the union of top-level fields with _id pinned left"
    );
    let row = &result.rows[0];
    assert_eq!(
        row[0],
        CellValue::Text(object_id.to_hex()),
        "an ObjectId cell is its hex text"
    );
    assert_eq!(row[1], CellValue::Text("Speedbike".into()));
    assert_eq!(
        row[2],
        CellValue::Text("{2 fields}".into()),
        "a nested document is a shape marker"
    );
    assert_eq!(
        row[3],
        CellValue::Text("[3]".into()),
        "an array is a shape marker"
    );
    assert_eq!(row[4], CellValue::Integer(4), "an Int64 cell is an Integer");
    assert_eq!(row[5], CellValue::Integer(7), "an Int32 cell is an Integer");
    assert_eq!(row[6], CellValue::Boolean(true), "a bool cell is a Boolean");
    assert_eq!(row[7], CellValue::Null);
    assert_eq!(result.rows.len(), 1);
}

#[test]
fn mongo_relation_preview_pages_through_the_find_spec_and_describes_it() {
    let request = PageRequest::at(PageSize::Ten, 20);

    let spec = relation_find_spec(&mongo_namespace(), request);

    assert_eq!(spec.namespace, mongo_namespace());
    assert_eq!(spec.skip, 20);
    assert_eq!(spec.page_size, PageSize::Ten.get() as u32);
    assert_eq!(
        spec.limit,
        Some(20 + PageSize::Ten.as_u64() + 1),
        "the find asks for one lookahead document so the page can advertise a next page"
    );
    let description = find_description(&spec);
    assert!(
        description.starts_with("db.bikes.find("),
        "the description names the collection it reads, got {description}"
    );
    assert!(
        description.contains(".skip(20)"),
        "the description states the offset it reads, got {description}"
    );
    assert!(
        description.contains(".limit(31)"),
        "the description states the bound it applies, got {description}"
    );
    assert!(
        description.contains(".sort("),
        "the description states the stable sort it applies, got {description}"
    );
}

#[test]
fn a_thousand_row_page_request_is_clamped_to_the_find_budget() {
    // The driver refuses pages above FindSpec::MAX_PAGE_SIZE, so the largest
    // grid page size has to be clamped instead of failing the load.
    let spec = relation_find_spec(
        &mongo_namespace(),
        PageRequest::first(PageSize::OneThousand),
    );

    assert_eq!(spec.page_size, FindSpec::MAX_PAGE_SIZE);
    assert!(spec.validate().is_ok());
}

// ---------------------------------------------------------------------------
// Reducer: activating a collection opens the relation tab
// ---------------------------------------------------------------------------

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
        database: MONGO_DATABASE.into(),
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
            databases: vec![MONGO_DATABASE.to_owned()],
            authorized_only: true,
            warning: None,
        },
    });
    app.update(Action::MongoCollectionsLoaded {
        profile_id,
        generation,
        database: MONGO_DATABASE.to_owned(),
        collections: vec![MongoCollectionInfo {
            namespace: mongo_namespace(),
            collection_type: MongoCollectionType::Collection,
            options_ejson: None,
        }],
    });
    app.explorer
        .normalized
        .expanded
        .insert(ExplorerNodeId::Profile(profile_id));
    app.explorer
        .normalized
        .expanded
        .insert(ExplorerNodeId::Mongo {
            profile_id,
            node: MongoExplorerNodeId::Database(MONGO_DATABASE.to_owned()),
        });
    app.focus = Focus::Explorer;
    (app, profile_id, generation)
}

fn collection_node(profile_id: Uuid) -> ExplorerNodeId {
    ExplorerNodeId::Mongo {
        profile_id,
        node: MongoExplorerNodeId::Collection(mongo_namespace()),
    }
}

#[test]
fn activating_a_mongodb_collection_opens_the_relation_tab_on_data() {
    let (mut app, profile_id, _generation) = mongo_app();
    assert!(app.explorer.select_id(collection_node(profile_id)));

    let commands = app.update(Action::ExplorerOpenSelected);

    let tab = match app.tabs.last() {
        Some(WorkspaceTab::Relation(tab)) => tab,
        Some(_) => panic!("activating a collection must open a relation tab"),
        None => panic!("activating a collection must open a tab"),
    };
    assert_eq!(app.active_tab, app.tabs.len() - 1);
    assert_eq!(app.focus, Focus::Results);
    assert_eq!(tab.view, RelationView::Data);
    assert_eq!(tab.descriptor.kind, CatalogKind::Table);
    assert_eq!(tab.descriptor.title, MONGO_COLLECTION);
    assert_eq!(
        tab.descriptor.qualified_name,
        QualifiedName {
            database: Some(MONGO_DATABASE.into()),
            schema: None,
            object: MONGO_COLLECTION.into(),
        }
    );
    assert_eq!(
        tab.descriptor.key.object_id,
        CatalogId::new(
            profile_id,
            CatalogKind::Table,
            [MONGO_DATABASE, MONGO_COLLECTION]
        ),
        "the collection is addressed as a table in its native namespace"
    );
    let tab_id = tab.id;
    let key = tab.descriptor.key.clone();
    let page_size = tab.pagination.page_size;
    assert!(
        commands.iter().any(|command| matches!(
            command,
            Command::LoadMongoRelationPreview(request)
                if request.tab_id == tab_id
                    && request.kind == RelationRequestKind::Preview
                    && request.relation == key
                    && request.page == PageRequest::first(page_size)
        )),
        "the first document page must be requested through the find budget, got {commands:?}"
    );
}

// ---------------------------------------------------------------------------
// Regression: SQL tables keep opening exactly as before
// ---------------------------------------------------------------------------

fn sql_app_with_table() -> (App, Uuid, CatalogId) {
    let profile = import_connection_url("postgres://localhost/app", Some("test"))
        .expect("postgres connection url")
        .profile;
    let profile_id = profile.id;
    let mut app = App::new(vec![profile.clone()]);
    app.explorer.normalized.add_profile_with_metadata(
        profile_id,
        "test".into(),
        DatabaseKind::Postgres,
        "localhost".into(),
        lazydb::model::explorer::ProfileProvenance::Saved,
    );
    let database = CatalogEntry::database(
        CatalogId::new(profile_id, CatalogKind::Database, [SQL_DATABASE]),
        QualifiedName {
            database: Some(SQL_DATABASE.into()),
            schema: None,
            object: SQL_DATABASE.into(),
        },
        "database",
        OptionalMetadata::Supported(None),
        true,
    )
    .expect("database entry");
    let schema = CatalogEntry::schema(
        CatalogId::new(profile_id, CatalogKind::Schema, [SQL_DATABASE, SQL_SCHEMA]),
        database.id.clone(),
        QualifiedName {
            database: Some(SQL_DATABASE.into()),
            schema: Some(SQL_SCHEMA.into()),
            object: SQL_SCHEMA.into(),
        },
        "schema",
        OptionalMetadata::Supported(None),
        true,
    )
    .expect("schema entry");
    let table = CatalogEntry::relation(
        CatalogId::new(
            profile_id,
            CatalogKind::Table,
            [SQL_DATABASE, SQL_SCHEMA, SQL_TABLE],
        ),
        schema.id.clone(),
        QualifiedName {
            database: Some(SQL_DATABASE.into()),
            schema: Some(SQL_SCHEMA.into()),
            object: SQL_TABLE.into(),
        },
        "table",
        OptionalMetadata::Supported(None),
        true,
    )
    .expect("table entry");
    let catalog = &mut app
        .explorer
        .normalized
        .profiles
        .get_mut(&profile_id)
        .expect("profile tree")
        .catalog;
    catalog.insert(database).expect("insert database");
    catalog.insert(schema).expect("insert schema");
    catalog.insert(table.clone()).expect("insert table");

    let target = ExecutionTarget {
        profile_id,
        database: SQL_DATABASE.to_owned(),
        schema: Some(SQL_SCHEMA.to_owned()),
    };
    let identity = app
        .sessions
        .start_attempt(target.clone())
        .expect("session attempt");
    let generation = identity.generation;
    let server = ServerInfo {
        kind: DatabaseKind::Postgres,
        version: "17".into(),
        database: SQL_DATABASE.into(),
        current_user: None,
    };
    assert!(
        app.sessions
            .accept_success(identity, server, CatalogMutationCapabilities::default())
    );
    app.connection.profile_id = Some(profile_id);
    app.connection.generation = generation;
    app.connection.status = lazydb::model::workspace::ConnectionStatus::Connected;
    app.connection.target = Some(target);
    app.explorer.normalized.selected = Some(ExplorerNodeId::Catalog(table.id.clone()));
    (app, profile_id, table.id)
}

#[test]
fn activating_a_sql_table_still_opens_the_relation_tab_with_a_sql_preview() {
    let (mut app, _profile_id, table_id) = sql_app_with_table();

    let commands = app.update(Action::ExplorerOpenSelected);

    let tab = match app.tabs.last() {
        Some(WorkspaceTab::Relation(tab)) => tab,
        Some(_) => panic!("activating a sql table must open a relation tab"),
        None => panic!("activating a sql table must open a tab"),
    };
    assert_eq!(tab.view, RelationView::Data);
    assert_eq!(tab.descriptor.key.object_id, table_id);
    let tab_id = tab.id;
    assert!(
        commands.iter().any(|command| matches!(
            command,
            Command::LoadRelationPreview(request)
                if request.tab_id == tab_id && request.relation.object_id == table_id
        )),
        "sql tables keep loading through the relation preview request, got {commands:?}"
    );
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::LoadMongoRelationPreview(_))),
        "sql tables never route through the mongodb loader, got {commands:?}"
    );
}

// ---------------------------------------------------------------------------
// T2 (S4): the status line names the field under the cursor
// ---------------------------------------------------------------------------

fn render_app(app: &App, width: u16, height: u16) -> String {
    use ratatui::{Terminal, backend::TestBackend};

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut state = lazydb::ui::UiState::new();
    terminal
        .draw(|frame| {
            lazydb::ui::render_with_state_using_icons(
                frame,
                app,
                &mut state,
                lazydb::ui::icons::IconSet::default(),
            )
        })
        .expect("render");
    let buffer = terminal.backend().buffer();
    let mut output = String::new();
    for y in 0..height {
        for x in 0..width {
            output.push_str(buffer[(x, y)].symbol());
        }
        output.push('\n');
    }
    output
}

fn loaded_mongo_relation(app: &mut App, profile_id: Uuid) {
    use lazydb::db::{
        RelationPreview,
        query::{QueryOutcome, QueryStats},
    };
    use lazydb::model::{
        pagination::ResultPagination,
        relation::{OwnedSnapshot, RelationLoad},
    };
    use lazydb::profile::CatalogScope;

    assert!(app.explorer.select_id(collection_node(profile_id)));
    app.update(Action::ExplorerOpenSelected);
    let Some(WorkspaceTab::Relation(tab)) = app.tabs.last_mut() else {
        panic!("activating a collection must open a relation tab");
    };
    let page_size = tab.pagination.page_size;
    tab.data = RelationLoad::Ready(OwnedSnapshot::new(
        RelationPreview {
            sql: find_description(&relation_find_spec(
                &mongo_namespace(),
                PageRequest::first(page_size),
            )),
            result: QueryOutcome {
                result_sets: vec![page_result_set(&page(
                    vec![document(mongodb::bson::doc! {
                        "_id": 1_i32,
                        "raceId": "race-1",
                        "position": 3_i64,
                    })],
                    None,
                ))],
                stats: QueryStats::new(Default::default(), Default::default(), 1),
            },
            pagination: ResultPagination::from_page(PageRequest::first(page_size), 1),
            row_versions: None,
        },
        ConnectionIdentity {
            profile_id,
            generation: 1,
        },
        CatalogScope::for_profile(DatabaseKind::MongoDb, MONGO_DATABASE, None),
    ));
}

#[test]
fn mongodb_relation_status_line_names_the_active_field() {
    let (mut app, profile_id, _generation) = mongo_app();
    loaded_mongo_relation(&mut app, profile_id);
    if let Some(WorkspaceTab::Relation(tab)) = app.tabs.last_mut() {
        tab.grid.selected_column = 1;
    }

    let output = render_app(&app, 220, 32);

    assert!(
        output.contains("Find:"),
        "a mongodb snapshot must not be labelled as SQL:\n{output}"
    );
    assert!(
        output
            .lines()
            .flat_map(|line| line.split('│'))
            .any(|pane| pane.contains("raceId") && pane.contains("field:")),
        "the status line must name the field under the cursor:\n{output}"
    );
}

// ---------------------------------------------------------------------------
// T3 (S2): the DDL view reports the collection structure
// ---------------------------------------------------------------------------

fn structure_metadata() -> MongoCollectionStructure {
    use mongodb::bson::Bson;

    MongoCollectionStructure {
        namespace: mongo_namespace(),
        collection_type: MongoCollectionType::Collection,
        options_ejson: None,
        validator_ejson: None,
        indexes: vec![
            MongoIndexInfo {
                name: "_id_".into(),
                keys: mongodb::bson::doc! { "_id": 1_i32 },
                unique: true,
                ttl_seconds: None,
                raw: mongodb::bson::doc! { "name": "_id_", "key": { "_id": 1_i32 } },
                key_description: vec![("_id".into(), Bson::Int32(1))],
            },
            MongoIndexInfo {
                name: "race_position".into(),
                keys: mongodb::bson::doc! { "raceId": 1_i32, "position": -1_i32 },
                unique: false,
                ttl_seconds: None,
                raw: mongodb::bson::doc! {
                    "name": "race_position",
                    "key": { "raceId": 1_i32, "position": -1_i32 },
                },
                key_description: vec![
                    ("raceId".into(), Bson::Int32(1)),
                    ("position".into(), Bson::Int32(-1)),
                ],
            },
        ],
        indexes_error: None,
    }
}

fn inferred_schema() -> MongoSchemaInference {
    infer_schema(&page(
        vec![
            document(mongodb::bson::doc! {
                "_id": 1_i32,
                "raceId": "race-1",
                "position": 3_i64,
            }),
            document(mongodb::bson::doc! {
                "_id": 2_i32,
                "raceId": "race-2",
            }),
        ],
        None,
    ))
}

fn structure_request(commands: &[Command]) -> RelationRequest {
    commands
        .iter()
        .find_map(|command| match command {
            Command::LoadMongoRelationStructure { request, .. } => Some(request.clone()),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!("the ddl view must request the collection structure, got {commands:?}")
        })
}

#[test]
fn opening_the_ddl_view_on_a_mongodb_relation_requests_the_structure() {
    let (mut app, profile_id, _generation) = mongo_app();
    loaded_mongo_relation(&mut app, profile_id);
    let expected_documents = vec![document(mongodb::bson::doc! {
        "_id": 1_i32,
        "raceId": "race-1",
        "position": 3_i64,
    })];
    if let Some(WorkspaceTab::Relation(tab)) = app.tabs.last_mut() {
        tab.mongodb_documents = Some(expected_documents.clone());
    }

    let commands = app.update(Action::SetRelationView(RelationView::Ddl));

    let Some(WorkspaceTab::Relation(tab)) = app.tabs.last() else {
        panic!("expected a relation tab");
    };
    let tab_id = tab.id;
    let key = tab.descriptor.key.clone();
    assert_eq!(tab.view, RelationView::Ddl);
    assert!(
        matches!(&tab.ddl, RelationLoad::Loading { .. }),
        "the ddl view must show the structure load in flight, got {:?}",
        tab.ddl
    );
    assert!(
        commands.iter().any(|command| matches!(
            command,
            Command::LoadMongoRelationStructure { request, documents: Some(documents) }
                if request.tab_id == tab_id
                    && request.kind == RelationRequestKind::Ddl
                    && request.relation == key
                    && *documents == expected_documents
        )),
        "the structure must be requested for the page the grid shows, got {commands:?}"
    );
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::LoadRelationDdl(_))),
        "a mongodb collection never asks for SQL DDL, got {commands:?}"
    );
}

#[test]
fn opening_the_ddl_view_without_a_loaded_page_asks_for_metadata_only() {
    let (mut app, profile_id, _generation) = mongo_app();
    assert!(app.explorer.select_id(collection_node(profile_id)));
    app.update(Action::ExplorerOpenSelected);

    let commands = app.update(Action::SetRelationView(RelationView::Ddl));

    assert!(
        commands.iter().any(|command| matches!(
            command,
            Command::LoadMongoRelationStructure {
                documents: None,
                ..
            }
        )),
        "with no loaded page the report carries metadata only, got {commands:?}"
    );
}

#[test]
fn the_structure_result_fills_the_ddl_view_with_indexes_and_inferred_fields() {
    let (mut app, profile_id, _generation) = mongo_app();
    loaded_mongo_relation(&mut app, profile_id);
    let commands = app.update(Action::SetRelationView(RelationView::Ddl));
    let request = structure_request(&commands);
    let ddl_editor_id = match app.tabs.last() {
        Some(WorkspaceTab::Relation(tab)) => tab.ddl_editor_id,
        _ => panic!("expected a relation tab"),
    };

    app.update(Action::MongoRelationStructureLoaded {
        request,
        result: Ok(Box::new(MongoStructureSnapshot {
            metadata: structure_metadata(),
            schema: Some(inferred_schema()),
        })),
    });

    let Some(WorkspaceTab::Relation(tab)) = app.tabs.last() else {
        panic!("expected a relation tab");
    };
    let RelationLoad::Ready(snapshot) = &tab.ddl else {
        panic!(
            "the structure result must fill the ddl view, got {:?}",
            tab.ddl
        );
    };
    let text = snapshot.value.sql.clone();
    for expected in [
        "validator: none",
        "_id_",
        "race_position",
        "\"position\": -1",
        "raceId",
        "position",
    ] {
        assert!(
            text.contains(expected),
            "the structure report must contain {expected:?}:\n{text}"
        );
    }
    let editor_text = app
        .editor_text(ddl_editor_id)
        .expect("the ddl editor session exists");
    assert!(
        editor_text.contains("race_position"),
        "the read-only DDL editor shows the report exactly like a SQL DDL:\n{editor_text}"
    );
}

// ---------------------------------------------------------------------------
// Regression: a SQL table's DDL view keeps loading through the SQL path
// ---------------------------------------------------------------------------

#[test]
fn opening_the_ddl_view_on_a_sql_table_still_loads_the_sql_ddl() {
    let (mut app, _profile_id, table_id) = sql_app_with_table();
    app.update(Action::ExplorerOpenSelected);

    let commands = app.update(Action::SetRelationView(RelationView::Ddl));

    assert!(
        commands.iter().any(|command| matches!(
            command,
            Command::LoadRelationDdl(request) if request.relation.object_id == table_id
        )),
        "sql tables must keep loading their DDL through the SQL request, got {commands:?}"
    );
    assert!(
        !commands
            .iter()
            .any(|command| matches!(command, Command::LoadMongoRelationStructure { .. })),
        "sql tables never route through the mongodb structure loader, got {commands:?}"
    );
}

// ---------------------------------------------------------------------------
// T4 (S3): the document edit modal, Canonical EJSON, and one bounded write
// ---------------------------------------------------------------------------

use lazydb::{
    db::{
        DatabaseConnection, RelationPreview,
        mongodb::mutation::MongoMutationOutcome,
        query::{ColumnMeta, QueryOutcome, QueryStats, ResultSet},
    },
    model::{
        cell_editor::JsonBuffer,
        relation::RelationTab,
        relation_edit::{
            DOCUMENT_ID_IMMUTABLE_MESSAGE, DocumentEditorState, DocumentEditorView,
            MONGODB_READ_ONLY_MESSAGE, RelationGridMode,
        },
    },
};

const RUEDAS_ID: &str = "6ae275e9e71e84bffceecaa8";
const FRENO_ID: &str = "6ae275e9e71e84bffceecaa9";
const CADENA_ID: &str = "6ae275e9e71e84bffceecaaa";

fn object_id(hex: &str) -> mongodb::bson::Bson {
    mongodb::bson::Bson::ObjectId(
        mongodb::bson::oid::ObjectId::parse_str(hex).expect("test object id"),
    )
}

/// The three documents the demo collection holds, as the grid loads them.
fn bike_documents() -> Vec<MongoDocument> {
    vec![
        document(mongodb::bson::doc! {
            "_id": object_id(RUEDAS_ID),
            "name": "Ruedas Pro",
            "price": 1200i64,
            "active": true,
        }),
        document(mongodb::bson::doc! {
            "_id": object_id(FRENO_ID),
            "name": "Freno XT",
            "price": 850i64,
            "active": true,
        }),
        document(mongodb::bson::doc! {
            "_id": object_id(CADENA_ID),
            "name": "Cadena 11v",
            "price": 340i64,
            "active": false,
        }),
    ]
}

fn mongo_preview_request(commands: &[Command]) -> RelationRequest {
    commands
        .iter()
        .find_map(|command| match command {
            Command::LoadMongoRelationPreview(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the tab must request the first page, got {commands:?}"))
}

fn active_relation_tab(app: &App) -> &RelationTab {
    match app.tabs.get(app.active_tab) {
        Some(WorkspaceTab::Relation(tab)) => tab,
        _ => panic!("the active tab must be a relation tab"),
    }
}

fn active_relation_tab_mut(app: &mut App) -> &mut RelationTab {
    match app.tabs.get_mut(app.active_tab) {
        Some(WorkspaceTab::Relation(tab)) => tab,
        _ => panic!("the active tab must be a relation tab"),
    }
}

/// Opens the collection tab and answers its preview request exactly as the
/// runtime does: one page of grid rows plus the BSON documents behind them.
fn mongo_app_with_page(documents: Vec<MongoDocument>) -> (App, RelationRequest) {
    let (mut app, profile_id, _generation) = mongo_app();
    assert!(app.explorer.select_id(collection_node(profile_id)));
    let commands = app.update(Action::ExplorerOpenSelected);
    let request = mongo_preview_request(&commands);
    let loaded = page(documents.clone(), None);
    let preview = RelationPreview {
        sql: find_description(&relation_find_spec(&mongo_namespace(), request.page)),
        result: QueryOutcome {
            result_sets: vec![page_result_set(&loaded)],
            stats: QueryStats::new(Default::default(), Default::default(), documents.len()),
        },
        pagination: lazydb::model::pagination::ResultPagination::from_page(
            request.page,
            documents.len(),
        ),
        row_versions: None,
    };
    let commands = app.update(Action::MongoRelationPreviewLoaded {
        request: request.clone(),
        preview: Box::new(preview),
        documents,
    });
    assert!(
        commands.is_empty(),
        "answering the preview writes nothing, got {commands:?}"
    );
    (app, request)
}

fn document_mode(app: &App) -> Option<RelationGridMode> {
    active_relation_tab(app)
        .edit
        .as_ref()
        .map(|edit| edit.mode.clone())
}

fn document_state(app: &App) -> DocumentEditorState {
    match document_mode(app) {
        Some(RelationGridMode::EditDocument(state)) => *state,
        other => panic!("the document modal must be open, got {other:?}"),
    }
}

/// Replaces the modal text the way a whole-document paste does.
fn set_document_text(app: &mut App, text: &str) {
    let tab = active_relation_tab_mut(app);
    let Some(RelationGridMode::EditDocument(state)) = tab.edit.as_mut().map(|edit| &mut edit.mode)
    else {
        panic!("the document modal must be open");
    };
    state.input = JsonBuffer::new(text);
}

fn replace_commands(commands: &[Command]) -> Vec<&Command> {
    commands
        .iter()
        .filter(|command| matches!(command, Command::ReplaceMongoRelationDocument { .. }))
        .collect()
}

#[derive(Debug)]
struct ReplaceCommand {
    tab_id: Uuid,
    tab_generation: u64,
    namespace: MongoNamespace,
    id: mongodb::bson::Bson,
    expected: mongodb::bson::Document,
    replacement: mongodb::bson::Document,
}

fn one_replace_command(commands: &[Command]) -> ReplaceCommand {
    let replaces = replace_commands(commands);
    assert_eq!(
        replaces.len(),
        1,
        "exactly one write must be emitted, got {commands:?}"
    );
    let Command::ReplaceMongoRelationDocument {
        tab_id,
        tab_generation,
        namespace,
        id,
        expected,
        replacement,
        ..
    } = replaces[0]
    else {
        unreachable!("filtered above")
    };
    ReplaceCommand {
        tab_id: *tab_id,
        tab_generation: *tab_generation,
        namespace: namespace.clone(),
        id: id.clone(),
        expected: expected.clone(),
        replacement: replacement.clone(),
    }
}

fn row_values(app: &App) -> Vec<Vec<CellValue>> {
    active_relation_tab(app)
        .edit
        .as_ref()
        .expect("the document grid session exists")
        .rows
        .iter()
        .map(|row| row.current.clone())
        .collect()
}

#[test]
fn the_edit_key_opens_the_selected_document_as_canonical_ejson() {
    let (mut app, _request) = mongo_app_with_page(bike_documents());

    let commands = app.update(Action::RelationEditCell);

    assert!(
        commands.is_empty(),
        "opening the modal writes nothing, got {commands:?}"
    );
    let state = document_state(&app);
    assert_eq!(state.row, 0, "the modal edits the row under the cursor");
    assert_eq!(state.collection, "speedbike_db.bikes");
    assert_eq!(
        state.id,
        object_id(RUEDAS_ID),
        "the modal keeps the original BSON _id value, not its rendering"
    );
    assert_eq!(
        state.input.value(),
        lazydb::model::relation_edit::canonical_document_ejson(&bike_documents()[0].bson),
        "the modal opens with the whole document in Canonical EJSON"
    );
    assert_eq!(state.error, None);
    assert!(!state.saving);
    assert!(
        state.title().contains("speedbike_db.bikes") && state.title().contains(RUEDAS_ID),
        "the title names the collection and the document, got {}",
        state.title()
    );
}

#[test]
fn the_modal_follows_the_row_under_the_cursor() {
    let (mut app, _request) = mongo_app_with_page(bike_documents());
    active_relation_tab_mut(&mut app).grid.selected_row = 2;

    app.update(Action::RelationEditCell);

    let state = document_state(&app);
    assert_eq!(state.row, 2);
    assert_eq!(state.id, object_id(CADENA_ID));
    assert_eq!(
        state.input.value(),
        lazydb::model::relation_edit::canonical_document_ejson(&bike_documents()[2].bson)
    );
}

/// Invariant 1: the write is bounded to the original `_id` value.
#[test]
fn saving_emits_one_replacement_bounded_to_the_original_id() {
    let (mut app, _request) = mongo_app_with_page(bike_documents());
    app.update(Action::RelationEditCell);
    let edited = format!(
        r#"{{"_id": {{"$oid": "{RUEDAS_ID}"}}, "name": "Ruedas Pro X", "price": {{"$numberLong": "1200"}}, "active": true}}"#
    );
    set_document_text(&mut app, &edited);

    let commands = app.update(Action::RelationEditConfirm);

    let replace = one_replace_command(&commands);
    assert_eq!(
        replace.id,
        object_id(RUEDAS_ID),
        "the filter carries the original _id value exactly"
    );
    assert_eq!(
        replace.expected,
        bike_documents()[0].bson,
        "the write carries the exact BSON document loaded when the editor opened"
    );
    assert_eq!(
        replace.replacement.get("_id"),
        Some(&object_id(RUEDAS_ID)),
        "the replacement keeps the original _id"
    );
    assert_eq!(
        replace.replacement.get_str("name"),
        Ok("Ruedas Pro X"),
        "the edited field is written"
    );
    assert_eq!(replace.namespace, mongo_namespace());
    assert_eq!(replace.tab_id, active_relation_tab(&app).id);
    assert_eq!(replace.tab_generation, active_relation_tab(&app).generation);
    assert_eq!(
        commands.len(),
        1,
        "the save must emit the write and nothing else, got {commands:?}"
    );
    let state = document_state(&app);
    assert!(
        state.saving,
        "a second save is refused while the write runs"
    );
    assert_eq!(state.error, None);

    assert!(
        app.update(Action::RelationEditConfirm).is_empty(),
        "one save emits one write"
    );
}

#[test]
fn saving_an_unchanged_document_closes_locally_without_a_write() {
    let (mut app, _request) = mongo_app_with_page(bike_documents());
    app.update(Action::RelationEditCell);
    app.update(Action::RelationEditDocumentToggleView);

    let commands = app.update(Action::RelationEditConfirm);

    assert!(commands.is_empty(), "an unchanged document writes nothing");
    assert_eq!(document_mode(&app), Some(RelationGridMode::Browse));
    assert!(
        app.notifications
            .history()
            .any(|notification| notification.body.contains("No document changes"))
    );
}

/// Invariant 2: a replaced or removed `_id` is rejected with no command.
#[test]
fn a_replacement_that_changes_or_drops_the_id_emits_no_command() {
    for edited in [
        format!(r#"{{"_id": {{"$oid": "{FRENO_ID}"}}, "name": "Ruedas Pro"}}"#),
        r#"{"name": "Ruedas Pro"}"#.to_owned(),
    ] {
        let (mut app, _request) = mongo_app_with_page(bike_documents());
        app.update(Action::RelationEditCell);
        set_document_text(&mut app, &edited);

        let commands = app.update(Action::RelationEditConfirm);

        assert!(
            commands.is_empty(),
            "a document that retargets or drops _id must not be written, got {commands:?}"
        );
        let state = document_state(&app);
        assert_eq!(
            state.error.as_deref(),
            Some("the replacement document must keep the original _id"),
            "the refusal is visible for {edited}"
        );
        assert!(!state.saving);
    }
}

/// Invariant 3: invalid or non-document EJSON is rejected with no command.
#[test]
fn invalid_canonical_ejson_emits_no_command() {
    for (edited, expected) in [
        (r#"{"_id": {"$oid": "#.to_owned(), "invalid Canonical EJSON"),
        (r#"[{"_id": 1}]"#.to_owned(), "requires a BSON document"),
        (r#"""#.to_owned(), "invalid Canonical EJSON"),
        (r#"{"_id": }"#.to_owned(), "invalid Canonical EJSON"),
    ] {
        let (mut app, _request) = mongo_app_with_page(bike_documents());
        let before = row_values(&app);
        app.update(Action::RelationEditCell);
        set_document_text(&mut app, &edited);

        let commands = app.update(Action::RelationEditConfirm);

        assert!(
            commands.is_empty(),
            "invalid input {edited:?} must not reach the database, got {commands:?}"
        );
        let error = document_state(&app).error.unwrap_or_default();
        assert!(
            error.contains(expected),
            "the refusal for {edited:?} must mention {expected:?}, got {error:?}"
        );
        assert_eq!(
            row_values(&app),
            before,
            "a refused edit leaves the loaded page untouched"
        );
    }
}

/// Invariant 4: a read-only profile refuses before any command.
#[test]
fn a_read_only_profile_refuses_to_edit_documents() {
    let (mut app, _request) = mongo_app_with_page(bike_documents());
    for profile in &mut app.profiles {
        profile.read_only = true;
    }

    let commands = app.update(Action::RelationEditCell);

    assert!(
        commands.is_empty(),
        "opening the modal on a read-only profile writes nothing, got {commands:?}"
    );
    assert_eq!(
        document_state(&app).error.as_deref(),
        Some(MONGODB_READ_ONLY_MESSAGE),
        "the modal reports that the profile is read-only"
    );

    set_document_text(
        &mut app,
        &format!(r#"{{"_id": {{"$oid": "{RUEDAS_ID}"}}, "name": "Nope"}}"#),
    );
    let commands = app.update(Action::RelationEditConfirm);

    assert!(
        commands.is_empty(),
        "a read-only profile must never emit a write, got {commands:?}"
    );
    let state = document_state(&app);
    assert_eq!(state.error.as_deref(), Some(MONGODB_READ_ONLY_MESSAGE));
    assert!(!state.saving);
}

/// Invariant 5: no write without a document at the cursor.
#[test]
fn no_write_is_emitted_without_a_document_at_the_cursor() {
    // An empty page has no row to edit, so the modal never opens.
    let (mut app, _request) = mongo_app_with_page(Vec::new());
    let commands = app.update(Action::RelationEditCell);
    assert!(
        commands.is_empty(),
        "an empty page emits no command, got {commands:?}"
    );
    assert_eq!(document_mode(&app), Some(RelationGridMode::Browse));

    // A cursor past the loaded page selects no document either.
    let (mut app, _request) = mongo_app_with_page(bike_documents());
    active_relation_tab_mut(&mut app).grid.selected_row = 7;
    let commands = app.update(Action::RelationEditCell);
    assert!(
        commands.is_empty(),
        "a cursor with no document emits no command, got {commands:?}"
    );
    assert_eq!(document_mode(&app), Some(RelationGridMode::Browse));
    assert!(
        app.notifications
            .history()
            .any(|notification| notification.body.contains("no document")),
        "the refusal is visible"
    );

    // A page that no longer holds the edited row refuses to write it.
    let (mut app, _request) = mongo_app_with_page(bike_documents());
    app.update(Action::RelationEditCell);
    active_relation_tab_mut(&mut app).mongodb_documents = Some(Vec::new());
    let commands = app.update(Action::RelationEditConfirm);
    assert!(
        commands.is_empty(),
        "a row that is no longer on the page emits no command, got {commands:?}"
    );
    assert!(document_state(&app).error.is_some());
}

/// Invariant 6 (success): the replacement is reported and the page reloads.
#[test]
fn a_replaced_document_is_reported_and_the_page_reloads() {
    let (mut app, request) = mongo_app_with_page(bike_documents());
    app.update(Action::RelationEditCell);
    set_document_text(
        &mut app,
        &format!(r#"{{"_id": {{"$oid": "{RUEDAS_ID}"}}, "name": "Ruedas Pro X"}}"#),
    );
    let commands = app.update(Action::RelationEditConfirm);
    let replace = one_replace_command(&commands);

    let commands = app.update(Action::MongoRelationDocumentReplaced {
        tab_id: replace.tab_id,
        tab_generation: replace.tab_generation,
        id: replace.id.clone(),
    });

    assert_eq!(
        document_mode(&app),
        Some(RelationGridMode::Browse),
        "a successful replacement closes the modal"
    );
    assert!(
        app.notifications.history().any(|notification| {
            notification.body.contains("replaced") && notification.body.contains("bikes")
        }),
        "the replacement is reported"
    );
    let reload = mongo_preview_request(&commands);
    assert_eq!(
        reload.page, request.page,
        "the page that was edited is the page that reloads"
    );
    assert_eq!(reload.tab_id, replace.tab_id);
}

/// Invariant 6 (failure): the adapter error is shown and the grid keeps its rows.
#[test]
fn a_failed_replacement_surfaces_the_error_and_keeps_the_page() {
    let (mut app, _request) = mongo_app_with_page(bike_documents());
    let before = row_values(&app);
    app.update(Action::RelationEditCell);
    set_document_text(
        &mut app,
        &format!(r#"{{"_id": {{"$oid": "{RUEDAS_ID}"}}, "name": "Ruedas Pro X"}}"#),
    );
    let commands = app.update(Action::RelationEditConfirm);
    let replace = one_replace_command(&commands);

    let commands = app.update(Action::MongoRelationDocumentReplaceFailed {
        tab_id: replace.tab_id,
        tab_generation: replace.tab_generation,
        id: replace.id.clone(),
        message: "MongoDB operation failed; inspect the error code and connection settings".into(),
    });

    assert!(
        commands.is_empty(),
        "a failed replacement reloads nothing, got {commands:?}"
    );
    let state = document_state(&app);
    assert_eq!(
        state.error.as_deref(),
        Some("MongoDB operation failed; inspect the error code and connection settings"),
        "the tab surfaces the adapter error"
    );
    assert!(!state.saving);
    assert_eq!(
        row_values(&app),
        before,
        "the grid keeps the values it had before the failed write"
    );
}

#[test]
fn a_compare_and_replace_conflict_keeps_the_document_draft_open() {
    let (mut app, _request) = mongo_app_with_page(bike_documents());
    let before = row_values(&app);
    app.update(Action::RelationEditCell);
    set_document_text(
        &mut app,
        &format!(r#"{{"_id": {{"$oid": "{RUEDAS_ID}"}}, "name": "Ruedas Pro X"}}"#),
    );
    let replace = one_replace_command(&app.update(Action::RelationEditConfirm));
    assert_eq!(replace.expected.get_str("name"), Ok("Ruedas Pro"));

    let commands = app.update(Action::MongoRelationDocumentReplaceFailed {
        tab_id: replace.tab_id,
        tab_generation: replace.tab_generation,
        id: replace.id,
        message: "the document changed or was deleted since it was loaded; reload before saving"
            .into(),
    });

    assert!(
        commands.is_empty(),
        "a conflict must not reload over the draft"
    );
    let state = document_state(&app);
    assert_eq!(
        state.error.as_deref(),
        Some("the document changed or was deleted since it was loaded; reload before saving")
    );
    assert!(!state.saving);
    assert!(state.input.value().contains("Ruedas Pro X"));
    assert_eq!(
        row_values(&app),
        before,
        "the loaded grid remains untouched"
    );
}

/// The generation guard: a stale result cannot touch a newer modal.
#[test]
fn a_stale_replacement_result_is_ignored() {
    let (mut app, _request) = mongo_app_with_page(bike_documents());
    app.update(Action::RelationEditCell);
    set_document_text(
        &mut app,
        &format!(r#"{{"_id": {{"$oid": "{RUEDAS_ID}"}}, "name": "Ruedas Pro X"}}"#),
    );
    let commands = app.update(Action::RelationEditConfirm);
    let replace = one_replace_command(&commands);

    let stale = app.update(Action::MongoRelationDocumentReplaced {
        tab_id: replace.tab_id,
        tab_generation: replace.tab_generation.saturating_add(1),
        id: replace.id.clone(),
    });

    assert!(
        stale.is_empty(),
        "a stale result reloads nothing, got {stale:?}"
    );
    assert!(
        matches!(document_mode(&app), Some(RelationGridMode::EditDocument(_))),
        "a stale result never closes a newer modal"
    );
}

/// Regression: the SQL cell editor opens and applies exactly as before.
#[test]
fn the_sql_cell_editor_still_opens_and_applies() {
    let (mut app, _profile_id, _table_id) = sql_app_with_table();
    app.update(Action::ExplorerOpenSelected);
    let table = match app.tabs.last_mut() {
        Some(WorkspaceTab::Relation(tab)) => tab,
        _ => panic!("activating a sql table must open a relation tab"),
    };
    table.data = RelationLoad::Ready(lazydb::model::relation::OwnedSnapshot {
        value: RelationPreview {
            sql: "select id, name from app.public.users".into(),
            result: QueryOutcome {
                result_sets: vec![ResultSet {
                    columns: vec![
                        ColumnMeta {
                            name: "id".into(),
                            type_name: "INTEGER".into(),
                        },
                        ColumnMeta {
                            name: "name".into(),
                            type_name: "TEXT".into(),
                        },
                    ],
                    rows: vec![vec![CellValue::Integer(1), CellValue::Text("old".into())]],
                    affected_rows: 0,
                }],
                stats: QueryStats::new(Default::default(), Default::default(), 1),
            },
            pagination: lazydb::model::pagination::ResultPagination::from_page(
                PageRequest::first(PageSize::Ten),
                1,
            ),
            row_versions: None,
        },
        attribution: lazydb::model::relation::SnapshotAttribution {
            connection: ConnectionIdentity {
                profile_id: _profile_id,
                generation: 1,
            },
            profile_id: _profile_id,
            scope: lazydb::profile::CatalogScope::for_profile(
                DatabaseKind::Postgres,
                SQL_DATABASE,
                Some(SQL_SCHEMA),
            ),
        },
    });
    table.edit = Some(
        lazydb::model::relation_edit::RelationEditSession::from_rows(vec![vec![
            CellValue::Integer(1),
            CellValue::Text("old".into()),
        ]]),
    );
    table.grid.selected_column = 1;

    let commands = app.update(Action::RelationEditCell);

    assert!(
        commands.is_empty(),
        "opening the sql cell editor emits no command, got {commands:?}"
    );
    assert!(
        matches!(document_mode(&app), Some(RelationGridMode::EditCell(_))),
        "a sql table still opens the cell editor, got {:?}",
        document_mode(&app)
    );
    app.update(Action::RelationEditDeleteToStart);
    for character in "new".chars() {
        app.update(Action::RelationEditInsert(character));
    }
    let commands = app.update(Action::RelationEditConfirm);

    assert!(
        commands.is_empty(),
        "applying a sql cell draft emits no command, got {commands:?}"
    );
    let edit = active_relation_tab(&app).edit.as_ref().expect("edit");
    assert_eq!(edit.mode, RelationGridMode::Browse);
    assert_eq!(edit.rows[0].original[1], CellValue::Text("old".into()));
    assert_eq!(edit.rows[0].current[1], CellValue::Text("new".into()));
}

/// The relation edit mode of the SQL tab a regression test just opened.
fn sql_relation_mode(app: &App) -> Option<RelationGridMode> {
    match app.tabs.last() {
        Some(WorkspaceTab::Relation(tab)) => tab.edit.as_ref().map(|edit| edit.mode.clone()),
        _ => None,
    }
}

/// Regression: the SQL JSON cell editor keeps its own keys and its own value.
///
/// The document modal gained a field-by-field view, but a `json` column still
/// opens the multi-line JSON cell editor: `Enter` inserts a newline, `Ctrl-F`
/// formats, `Ctrl-S` applies the text as the cell value, and invalid JSON is
/// still refused with the cell untouched.
#[test]
fn the_sql_json_cell_editor_still_opens_and_applies() {
    let mut app = sql_relation_app_with(
        vec![("id", "INTEGER"), ("payload", "json")],
        vec![CellValue::Integer(1), CellValue::Text(r#"{"a":1}"#.into())],
    );
    if let Some(WorkspaceTab::Relation(tab)) = app.tabs.last_mut() {
        tab.grid.selected_column = 1;
    }

    let commands = app.update(Action::RelationEditCell);

    assert!(
        commands.is_empty(),
        "opening the json cell editor emits no command, got {commands:?}"
    );
    assert!(
        matches!(sql_relation_mode(&app), Some(RelationGridMode::EditCell(_))),
        "a json column still opens the cell editor, got {:?}",
        sql_relation_mode(&app)
    );

    // Enter inserts a newline instead of applying, exactly as before.
    app.update(Action::RelationEditInsert('\n'));
    assert!(matches!(
        sql_relation_mode(&app),
        Some(RelationGridMode::EditCell(_))
    ));
    // Ctrl-F formats the JSON text in place.
    app.update(Action::RelationEditJsonFormat);
    let commands = app.update(Action::RelationEditConfirm);

    assert!(
        commands.is_empty(),
        "applying a json cell emits no command, got {commands:?}"
    );
    let applied = {
        let Some(WorkspaceTab::Relation(tab)) = app.tabs.last() else {
            panic!("relation tab");
        };
        let edit = tab.edit.as_ref().expect("edit session");
        assert_eq!(edit.mode, RelationGridMode::Browse);
        let CellValue::Text(applied) = &edit.rows[0].current[1] else {
            panic!(
                "the applied json cell is text, got {:?}",
                edit.rows[0].current[1]
            );
        };
        applied.clone()
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&applied)
            .expect("the applied value is valid JSON"),
        serde_json::json!({ "a": 1 }),
        "the formatted value is the same JSON, got {applied:?}"
    );
    assert!(
        applied.contains('\n'),
        "Ctrl-F formatted the text, got {applied:?}"
    );

    // Invalid JSON is still refused and leaves the cell untouched.
    let Some(WorkspaceTab::Relation(tab)) = app.tabs.last_mut() else {
        panic!("relation tab");
    };
    tab.grid.selected_column = 1;
    app.update(Action::RelationEditCell);
    app.update(Action::RelationEditDeleteToStart);
    app.update(Action::RelationEditInsert('{'));
    let commands = app.update(Action::RelationEditConfirm);
    assert!(
        commands.is_empty(),
        "an invalid json cell emits no command, got {commands:?}"
    );
    let current_after_refusal = {
        let Some(WorkspaceTab::Relation(tab)) = app.tabs.last() else {
            panic!("relation tab");
        };
        let edit = tab.edit.as_ref().expect("edit session");
        assert!(
            matches!(edit.mode, RelationGridMode::EditCell(_)),
            "an invalid value keeps the cell editor open"
        );
        edit.rows[0].current[1].clone()
    };
    assert_eq!(
        current_after_refusal,
        CellValue::Text(applied),
        "a refused value leaves the cell untouched"
    );
}

/// Opens a SQL relation tab loaded with `columns` and one row, the way the
/// runtime answers a relation preview.
fn sql_relation_app_with(columns: Vec<(&str, &str)>, values: Vec<CellValue>) -> App {
    let (mut app, profile_id, _table_id) = sql_app_with_table();
    app.update(Action::ExplorerOpenSelected);
    let table = match app.tabs.last_mut() {
        Some(WorkspaceTab::Relation(tab)) => tab,
        _ => panic!("activating a sql table must open a relation tab"),
    };
    table.data = RelationLoad::Ready(lazydb::model::relation::OwnedSnapshot {
        value: RelationPreview {
            sql: "select id, payload from app.public.users".into(),
            result: QueryOutcome {
                result_sets: vec![ResultSet {
                    columns: columns
                        .into_iter()
                        .map(|(name, type_name)| ColumnMeta {
                            name: name.into(),
                            type_name: type_name.into(),
                        })
                        .collect(),
                    rows: vec![values.clone()],
                    affected_rows: 0,
                }],
                stats: QueryStats::new(Default::default(), Default::default(), 1),
            },
            pagination: lazydb::model::pagination::ResultPagination::from_page(
                PageRequest::first(PageSize::Ten),
                1,
            ),
            row_versions: None,
        },
        attribution: lazydb::model::relation::SnapshotAttribution {
            connection: ConnectionIdentity {
                profile_id,
                generation: 1,
            },
            profile_id,
            scope: lazydb::profile::CatalogScope::for_profile(
                DatabaseKind::Postgres,
                SQL_DATABASE,
                Some(SQL_SCHEMA),
            ),
        },
    });
    table.edit = Some(lazydb::model::relation_edit::RelationEditSession::from_rows(vec![values]));
    app
}

/// The modal is rendered inside the relation tab with its title and hints.
#[test]
fn the_document_modal_renders_the_collection_id_and_save_hints() {
    let (mut app, _request) = mongo_app_with_page(bike_documents());
    app.update(Action::RelationEditCell);

    let output = render_app(&app, 200, 40);

    assert!(
        output.contains("EDIT DOCUMENT"),
        "the modal is titled, got:\n{output}"
    );
    assert!(
        output.contains("speedbike_db.bikes"),
        "the modal names the collection, got:\n{output}"
    );
    assert!(
        output.contains(RUEDAS_ID),
        "the modal names the _id it will write, got:\n{output}"
    );
    assert!(
        output.contains("Ctrl-S apply document") && output.contains("Esc close"),
        "the modal shows how to save and cancel, got:\n{output}"
    );
}

// ---------------------------------------------------------------------------
// T5 (S3): the field-by-field view inside the document modal
// ---------------------------------------------------------------------------

/// One document of every scalar BSON type the field editor parses, plus the
/// containers it edits as JSON text.
fn typed_documents() -> Vec<MongoDocument> {
    vec![document(mongodb::bson::doc! {
        "_id": object_id(RUEDAS_ID),
        "name": "Ruedas Pro",
        "stock": 4i32,
        "price": 1200i64,
        "weight": 1.5f64,
        "active": true,
        "retired": mongodb::bson::Bson::Null,
        "tags": ["road", "fast"],
        "geometry": { "wheels": 2i32, "frame": "carbon" },
        "spokes": object_id(FRENO_ID),
    })]
}

fn open_fields_view(app: &mut App) {
    app.update(Action::RelationEditCell);
    if document_state(app).view != DocumentEditorView::Fields {
        app.update(Action::RelationEditDocumentToggleView);
    }
    assert_eq!(document_state(app).view, DocumentEditorView::Fields);
}

fn field_names(app: &App) -> Vec<String> {
    document_state(app)
        .fields()
        .expect("the document text is valid")
        .into_iter()
        .map(|field| field.name)
        .collect()
}

fn field_value(app: &App, name: &str) -> Option<String> {
    document_state(app)
        .fields()
        .expect("the document text is valid")
        .into_iter()
        .find(|field| field.name == name)
        .map(|field| field.value)
}

/// Selects one field the way the list does: with relative cursor moves.
fn select_field(app: &mut App, name: &str) {
    let index = field_names(app)
        .iter()
        .position(|field| field == name)
        .unwrap_or_else(|| panic!("the document has no field {name}"));
    let current = document_state(app).selected_field;
    app.update(Action::RelationEditDocumentFieldMove(
        index as isize - current as isize,
    ));
    assert_eq!(
        document_state(app).selected_field,
        index,
        "the cursor must stop on {name}"
    );
}

/// Opens the field editor on `name` and replaces its text with `text`.
fn type_field_value(app: &mut App, name: &str, text: &str) {
    select_field(app, name);
    app.update(Action::RelationEditDocumentFieldEdit);
    assert!(
        document_state(app).field_edit.is_some(),
        "the {name} field editor must open"
    );
    app.update(Action::RelationEditDeleteToStart);
    for character in text.chars() {
        app.update(Action::RelationEditInsert(character));
    }
}

/// Applies the open field editor and returns the resulting document.
fn apply_field_value(app: &mut App) -> mongodb::bson::Document {
    let commands = app.update(Action::RelationEditDocumentFieldApply);
    assert!(
        commands.is_empty(),
        "applying a field writes nothing to the database, got {commands:?}"
    );
    let state = document_state(app);
    assert!(
        state.field_edit.is_none(),
        "applying closes the field editor"
    );
    assert_eq!(state.field_error, None, "the applied value is accepted");
    state
        .replacement_document()
        .expect("the document text is still a valid document")
}

#[test]
fn toggling_the_document_modal_lists_the_top_level_fields_with_their_types() {
    let (mut app, _request) = mongo_app_with_page(bike_documents());
    app.update(Action::RelationEditCell);
    let ejson = document_state(&app).input.value().to_owned();
    assert_eq!(
        document_state(&app).view,
        DocumentEditorView::Fields,
        "the modal opens on the field list"
    );

    let commands = app.update(Action::RelationEditDocumentToggleView);
    assert!(
        commands.is_empty(),
        "switching views writes nothing, got {commands:?}"
    );
    assert_eq!(
        document_state(&app).view,
        DocumentEditorView::Ejson,
        "the first toggle shows the EJSON text"
    );
    let commands = app.update(Action::RelationEditDocumentToggleView);
    assert!(
        commands.is_empty(),
        "switching back writes nothing, got {commands:?}"
    );
    let state = document_state(&app);
    assert_eq!(state.view, DocumentEditorView::Fields);
    assert_eq!(state.error, None);
    assert_eq!(state.field_error, None);
    assert_eq!(
        state
            .fields()
            .expect("the document text is valid")
            .iter()
            .map(|field| (field.name.as_str(), field.type_name, field.value.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("_id", "objectId", RUEDAS_ID),
            ("name", "string", "Ruedas Pro"),
            ("price", "int64", "1200"),
            ("active", "bool", "true"),
        ],
        "one row per top-level field, with its observed BSON type and compact value"
    );
    assert_eq!(
        state.selected_field, 0,
        "the list starts on the first field"
    );
    assert_eq!(
        state.input.value(),
        ejson,
        "the EJSON text is the source of truth and is not rewritten by the toggle"
    );

    let output = render_app(&app, 200, 40);
    assert!(
        output.contains("Ruedas Pro") && output.contains("int64") && output.contains("objectId"),
        "the field list renders names, types and values, got:\n{output}"
    );

    app.update(Action::RelationEditDocumentToggleView);
    let state = document_state(&app);
    assert_eq!(state.view, DocumentEditorView::Ejson);
    assert_eq!(state.input.value(), ejson);
}

#[test]
fn the_field_list_moves_between_fields_and_stops_at_both_ends() {
    let (mut app, _request) = mongo_app_with_page(bike_documents());
    open_fields_view(&mut app);

    let commands = app.update(Action::RelationEditDocumentFieldMove(1));

    assert!(
        commands.is_empty(),
        "moving the field cursor writes nothing, got {commands:?}"
    );
    assert_eq!(document_state(&app).selected_field, 1);
    app.update(Action::RelationEditDocumentFieldMove(1));
    app.update(Action::RelationEditDocumentFieldMove(1));
    assert_eq!(
        document_state(&app).selected_field,
        3,
        "the last field is reachable"
    );
    app.update(Action::RelationEditDocumentFieldMove(1));
    assert_eq!(
        document_state(&app).selected_field,
        3,
        "the cursor stops at the last field"
    );
    for _ in 0..9 {
        app.update(Action::RelationEditDocumentFieldMove(-1));
    }
    assert_eq!(
        document_state(&app).selected_field,
        0,
        "the cursor stops at the first field"
    );
}

#[test]
fn editing_a_scalar_field_applies_into_the_ejson_text_and_keeps_the_id() {
    let (mut app, _request) = mongo_app_with_page(bike_documents());
    open_fields_view(&mut app);
    let ejson = document_state(&app).input.value().to_owned();

    select_field(&mut app, "name");
    app.update(Action::RelationEditDocumentFieldEdit);
    let state = document_state(&app);
    let field = state
        .field_edit
        .clone()
        .expect("the name field editor opens");
    assert_eq!(field.name, "name");
    assert_eq!(
        field.input.value(),
        "Ruedas Pro",
        "the field editor starts on the value the row shows"
    );

    app.update(Action::RelationEditDeleteToStart);
    for character in "Ruedas Pro X".chars() {
        app.update(Action::RelationEditInsert(character));
    }
    assert_eq!(
        document_state(&app).input.value(),
        ejson,
        "typing into a field leaves the document text alone until it is applied"
    );

    let document = apply_field_value(&mut app);

    assert_eq!(document.get_str("name"), Ok("Ruedas Pro X"));
    assert_eq!(
        document.get("_id"),
        Some(&object_id(RUEDAS_ID)),
        "applying a field keeps the same _id"
    );
    let state = document_state(&app);
    assert_eq!(
        state.input.value(),
        &lazydb::model::relation_edit::canonical_document_ejson(&document),
        "the applied value is written into the EJSON text"
    );
    assert_eq!(
        state
            .fields()
            .expect("the document text is valid")
            .iter()
            .map(|field| field.value.clone())
            .collect::<Vec<_>>(),
        vec![
            RUEDAS_ID.to_owned(),
            "Ruedas Pro X".into(),
            "1200".into(),
            "true".into()
        ],
        "the list shows the applied value"
    );

    // Saving still runs the one bounded replacement the whole-document save uses.
    let replace = one_replace_command(&app.update(Action::RelationEditConfirm));
    assert_eq!(replace.id, object_id(RUEDAS_ID));
    assert_eq!(
        replace.replacement.get("_id"),
        Some(&object_id(RUEDAS_ID)),
        "the replacement is still bounded to the original _id"
    );
    assert_eq!(replace.replacement.get_str("name"), Ok("Ruedas Pro X"));
}

#[test]
fn leaving_a_field_edit_with_escape_keeps_the_document_text() {
    let (mut app, _request) = mongo_app_with_page(bike_documents());
    open_fields_view(&mut app);
    let ejson = document_state(&app).input.value().to_owned();
    type_field_value(&mut app, "name", "discarded");

    let commands = app.update(Action::RelationEditDocumentFieldCancel);

    assert!(
        commands.is_empty(),
        "leaving a field edit writes nothing, got {commands:?}"
    );
    let state = document_state(&app);
    assert!(state.field_edit.is_none());
    assert_eq!(state.field_error, None);
    assert_eq!(
        state.view,
        DocumentEditorView::Fields,
        "the list stays open"
    );
    assert_eq!(
        state.input.value(),
        ejson,
        "an abandoned field edit never touches the document text"
    );

    // The second escape is the modal's own cancel, exactly as before.
    app.update(Action::RelationEditCancel);
    assert_eq!(document_mode(&app), Some(RelationGridMode::Browse));
}

#[test]
fn typed_field_values_are_parsed_with_the_field_bson_type() {
    let (mut app, _request) = mongo_app_with_page(typed_documents());
    open_fields_view(&mut app);

    for (field, text, expected) in [
        ("stock", "9", mongodb::bson::Bson::Int32(9)),
        ("price", "950", mongodb::bson::Bson::Int64(950)),
        ("weight", "2.25", mongodb::bson::Bson::Double(2.25)),
        ("active", "false", mongodb::bson::Bson::Boolean(false)),
        ("retired", "null", mongodb::bson::Bson::Null),
        ("name", "true", mongodb::bson::Bson::String("true".into())),
        (
            "spokes",
            FRENO_ID,
            mongodb::bson::Bson::ObjectId(
                mongodb::bson::oid::ObjectId::parse_str(FRENO_ID).expect("object id"),
            ),
        ),
        (
            "tags",
            r#"["road"]"#,
            mongodb::bson::Bson::Array(vec![mongodb::bson::Bson::String("road".into())]),
        ),
        (
            "geometry",
            r#"{"wheels": 3}"#,
            mongodb::bson::Bson::Document(mongodb::bson::doc! { "wheels": 3i32 }),
        ),
    ] {
        type_field_value(&mut app, field, text);
        let document = apply_field_value(&mut app);
        assert_eq!(
            document.get(field),
            Some(&expected),
            "{field} must be parsed as its own BSON type from {text:?}"
        );
        assert_eq!(
            document.get("_id"),
            Some(&object_id(RUEDAS_ID)),
            "editing {field} keeps the same _id"
        );
    }
}

#[test]
fn invalid_typed_input_is_rejected_and_leaves_the_text_unchanged() {
    let (mut app, _request) = mongo_app_with_page(typed_documents());
    open_fields_view(&mut app);

    for (field, text, expected) in [
        ("stock", "7.5", "expected a 32-bit integer"),
        ("price", "12x", "expected a 64-bit integer"),
        ("weight", "heavy", "expected a double"),
        ("active", "yes", "expected true or false"),
        ("retired", "nil", "expected null"),
        ("spokes", "not-an-id", "expected a 24-character objectId"),
        ("tags", "road", "invalid Canonical EJSON"),
        ("geometry", "{", "invalid Canonical EJSON"),
    ] {
        let ejson = document_state(&app).input.value().to_owned();
        let value_before = field_value(&app, field);
        type_field_value(&mut app, field, text);

        let commands = app.update(Action::RelationEditDocumentFieldApply);

        assert!(
            commands.is_empty(),
            "an invalid {field} value must never reach the database, got {commands:?}"
        );
        let state = document_state(&app);
        let error = state.field_error.clone().unwrap_or_default();
        assert!(
            error.contains(expected),
            "the refusal for {field} {text:?} must mention {expected:?}, got {error:?}"
        );
        assert!(
            state.field_edit.is_some(),
            "a refused value keeps the field editor open so it can be corrected"
        );
        assert_eq!(
            state.input.value(),
            ejson,
            "a refused {field} value leaves the document text unchanged"
        );
        assert_eq!(
            field_value(&app, field),
            value_before,
            "the refused field keeps the value it had before the edit"
        );
        app.update(Action::RelationEditDocumentFieldCancel);
    }
}

#[test]
fn the_id_field_cannot_be_edited_from_the_field_list() {
    let (mut app, _request) = mongo_app_with_page(bike_documents());
    open_fields_view(&mut app);
    let ejson = document_state(&app).input.value().to_owned();
    assert_eq!(
        document_state(&app).selected_field,
        0,
        "the field list starts on _id"
    );
    assert_eq!(field_names(&app).first().map(String::as_str), Some("_id"));

    let commands = app.update(Action::RelationEditDocumentFieldEdit);

    assert!(
        commands.is_empty(),
        "attempting to edit _id writes nothing, got {commands:?}"
    );
    let state = document_state(&app);
    assert!(state.field_edit.is_none(), "_id never opens a field editor");
    assert_eq!(
        state.field_error.as_deref(),
        Some(DOCUMENT_ID_IMMUTABLE_MESSAGE),
        "the refusal reports that the identifier is immutable"
    );
    assert_eq!(state.input.value(), ejson);

    // Keys typed in the list reach neither the identifier nor the document.
    for character in "ffffffffffff".chars() {
        app.update(Action::RelationEditInsert(character));
    }
    let state = document_state(&app);
    assert_eq!(
        state.input.value(),
        ejson,
        "the identifier cannot be changed from the field list"
    );
    assert_eq!(
        state
            .replacement_document()
            .expect("the document is still valid")
            .get("_id"),
        Some(&object_id(RUEDAS_ID)),
        "the document keeps its original _id"
    );
    // Any other field of the same list is editable, so the refusal is about
    // _id and not about the view.
    app.update(Action::RelationEditCell);
    select_field(&mut app, "name");
    app.update(Action::RelationEditDocumentFieldEdit);
    assert!(
        document_state(&app).field_edit.is_some(),
        "a field other than _id still opens its editor"
    );
}

// ---------------------------------------------------------------------------
// T6 (issue #6): a container field is edited in its own multi-line modal
// ---------------------------------------------------------------------------

/// Opens the field editor on `name` and returns the text it starts with.
fn open_field_editor(app: &mut App, name: &str) -> String {
    select_field(app, name);
    app.update(Action::RelationEditDocumentFieldEdit);
    let state = document_state(app);
    let field = state
        .field_edit
        .clone()
        .unwrap_or_else(|| panic!("the {name} field editor must open"));
    assert_eq!(field.name, name);
    field.input.value().to_owned()
}

/// Replaces the open field editor's text the way typing does.
fn retype_field_editor(app: &mut App, text: &str) {
    app.update(Action::RelationEditDeleteToStart);
    for character in text.chars() {
        app.update(Action::RelationEditInsert(character));
    }
}

fn document_field_text(app: &App) -> String {
    document_state(app)
        .field_edit
        .as_ref()
        .map(|field| field.input.value().to_owned())
        .unwrap_or_default()
}

#[test]
fn a_container_field_opens_on_indented_ejson_in_its_own_modal() {
    let (mut app, _request) = mongo_app_with_page(typed_documents());
    open_fields_view(&mut app);

    let text = open_field_editor(&mut app, "geometry");

    assert_eq!(
        document_state(&app).view,
        DocumentEditorView::Fields,
        "the field list stays the surface underneath"
    );
    assert!(
        document_state(&app)
            .field_edit
            .as_ref()
            .is_some_and(|field| field.input.is_multiline()),
        "a document field opens the multi-line editor"
    );
    assert!(
        text.lines().count() > 1,
        "the value must be indented across lines, got {text:?}"
    );
    for expected in [
        "\n",
        "  \"wheels\": {",
        "    \"$numberInt\": \"2\"",
        "  \"frame\": \"carbon\"",
        "\n}",
    ] {
        assert!(
            text.contains(expected),
            "the indented EJSON must contain {expected:?}, got {text:?}"
        );
    }

    let output = render_app(&app, 200, 40);
    assert!(
        output.contains("EDIT FIELD geometry") && output.contains("document"),
        "the field modal names the field and its BSON type, got:\n{output}"
    );
    assert!(
        output.contains(RUEDAS_ID),
        "the document header naming the _id stays visible, got:\n{output}"
    );
    assert!(
        output.contains("Ctrl-F format") && output.contains("Ctrl-S apply"),
        "the field modal shows its own keys, got:\n{output}"
    );
    assert!(
        output
            .lines()
            .any(|line| line.contains("\"frame\": \"carbon\"")),
        "a nested document renders one property per line, got:\n{output}"
    );

    // The same modal renders an array with one element per line.
    app.update(Action::RelationEditDocumentFieldCancel);
    open_field_editor(&mut app, "tags");
    let output = render_app(&app, 200, 40);
    assert!(
        output.contains("EDIT FIELD tags · array"),
        "the field modal names the array type, got:\n{output}"
    );
    for element in ["\"road\",", "\"fast\""] {
        assert!(
            output.lines().any(|line| line.contains(element)),
            "every array element must render on its own line, missing {element}, got:\n{output}"
        );
    }
}

/// A document whose `tags` array is long enough that the field modal has to cap
/// its own height instead of growing over the document modal.
fn document_with_long_array() -> String {
    let items = (0..40)
        .map(|index| format!("\"item-{index:02}\""))
        .collect::<Vec<_>>()
        .join(",\n    ");
    format!("{{\n  \"_id\": {{\"$oid\": \"{RUEDAS_ID}\"}},\n  \"tags\": [\n    {items}\n  ]\n}}")
}

#[test]
fn every_field_opens_in_its_own_modal() {
    let (mut app, _request) = mongo_app_with_page(typed_documents());
    open_fields_view(&mut app);

    let text = open_field_editor(&mut app, "name");
    assert_eq!(
        text, "Ruedas Pro",
        "a scalar opens on the value the field list already shows"
    );
    assert!(
        !document_state(&app)
            .field_edit
            .as_ref()
            .is_some_and(|field| field.input.is_multiline()),
        "a scalar stays a one-line value inside the modal"
    );
    let output = render_app(&app, 200, 40);
    assert!(
        output.contains("EDIT FIELD name · string"),
        "the scalar field opens its own modal, got:\n{output}"
    );
    assert!(
        output
            .lines()
            .any(|line| line.contains("Enter apply field  Esc back to fields")),
        "the one-line field modal shows its own keys uncut, got:\n{output}"
    );

    // A scalar navigation key applies locally and opens the adjacent field.
    let index = field_names(&app)
        .iter()
        .position(|field| field == "name")
        .expect("name is a field");
    app.update(Action::RelationEditDocumentFieldMove(1));
    let state = document_state(&app);
    assert_eq!(
        state.field_edit.as_ref().map(|field| field.name.as_str()),
        Some("stock"),
        "moving opens the adjacent editable field"
    );
    assert_eq!(
        state.selected_field_index(field_names(&app).len()),
        index + 1,
        "the same key press moves the list cursor"
    );
}

#[test]
fn scalar_navigation_applies_locally_and_preserves_invalid_drafts() {
    let (mut app, _request) = mongo_app_with_page(typed_documents());
    open_fields_view(&mut app);
    let expected = document_state(&app).expected;
    type_field_value(&mut app, "name", "Updated name");
    assert!(
        app.update(Action::RelationEditDocumentFieldMove(1))
            .is_empty()
    );
    let state = document_state(&app);
    assert_eq!(
        state.replacement_document().unwrap().get_str("name"),
        Ok("Updated name")
    );
    assert_eq!(state.expected, expected);
    assert_eq!(state.field_edit.as_ref().unwrap().name, "stock");

    retype_field_editor(&mut app, "invalid integer");
    let before = document_state(&app).input.value().to_owned();
    assert!(
        app.update(Action::RelationEditDocumentFieldMove(1))
            .is_empty()
    );
    let state = document_state(&app);
    assert_eq!(state.field_edit.as_ref().unwrap().name, "stock");
    assert_eq!(
        state.field_edit.as_ref().unwrap().input.value(),
        "invalid integer"
    );
    assert!(
        state
            .field_error
            .as_deref()
            .unwrap()
            .contains("32-bit integer")
    );
    assert_eq!(state.input.value(), before);
}

#[test]
fn scalar_navigation_skips_id_and_stops_at_the_ends() {
    let (mut app, _request) = mongo_app_with_page(bike_documents());
    open_fields_view(&mut app);
    type_field_value(&mut app, "name", "Kept at start");
    app.update(Action::RelationEditDocumentFieldMove(-1));
    let state = document_state(&app);
    assert_eq!(state.field_edit.as_ref().unwrap().name, "name");
    assert_eq!(
        state.field_edit.as_ref().unwrap().input.value(),
        "Kept at start"
    );
    assert_eq!(
        state.replacement_document().unwrap().get_str("name"),
        Ok("Kept at start")
    );
    app.update(Action::RelationEditDocumentFieldCancel);
    type_field_value(&mut app, "active", "false");
    app.update(Action::RelationEditDocumentFieldMove(1));
    let state = document_state(&app);
    assert_eq!(state.field_edit.as_ref().unwrap().name, "active");
    assert_eq!(
        state.replacement_document().unwrap().get_bool("active"),
        Ok(false)
    );
}

/// The rows the field modal keeps blank around its value: the margin under the
/// document header and the margin over the footer. It also reports the value
/// rows, which must sit between the two margins.
#[derive(Debug)]
struct FieldModalLayout {
    above: String,
    below: String,
    value_rows: usize,
}

fn field_modal_layout(output: &str) -> FieldModalLayout {
    let lines = output.lines().collect::<Vec<_>>();
    let title_row = lines
        .iter()
        .position(|line| line.contains("EDIT FIELD "))
        .expect("the field modal is rendered");
    let title = lines[title_row].chars().collect::<Vec<_>>();
    let left = title
        .iter()
        .position(|cell| *cell == '╭')
        .expect("the field modal has a left border");
    let right = title
        .iter()
        .rposition(|cell| *cell == '╮')
        .expect("the field modal has a right border");
    let inside = |row: usize| {
        lines[row]
            .chars()
            .skip(left + 1)
            .take(right - left - 1)
            .collect::<String>()
    };
    let bottom_row = (title_row + 1..lines.len())
        .find(|row| lines[*row].chars().nth(left) == Some('╰'))
        .expect("the field modal has a bottom border");
    let rows = (title_row + 1..bottom_row)
        .filter(|row| !inside(*row).trim().is_empty())
        .collect::<Vec<_>>();
    assert!(
        rows.len() >= 3,
        "the modal holds a header, the value and a footer, got {rows:?}"
    );
    FieldModalLayout {
        above: inside(rows[0] + 1),
        below: inside(rows[rows.len() - 1] - 1),
        value_rows: rows.len() - 2,
    }
}

/// The BSON date of the issue screenshot: 2026-11-08T00:00:00Z.
const DATE_MILLIS: i64 = 1_794_096_000_000;
const DATE_ISO: &str = "2026-11-08T00:00:00Z";

/// A document with one BSON date field.
fn dated_documents() -> Vec<MongoDocument> {
    vec![document(mongodb::bson::doc! {
        "_id": object_id(RUEDAS_ID),
        "name": "Carrera 42",
        "date": mongodb::bson::DateTime::from_millis(DATE_MILLIS),
    })]
}

#[test]
fn a_date_field_shows_an_iso_timestamp_in_the_list_and_the_modal() {
    let (mut app, _request) = mongo_app_with_page(dated_documents());
    open_fields_view(&mut app);

    assert_eq!(
        field_value(&app, "date").as_deref(),
        Some(DATE_ISO),
        "the field list row shows the readable date instead of canonical EJSON"
    );
    let text = open_field_editor(&mut app, "date");
    assert_eq!(
        text, DATE_ISO,
        "the field modal opens on the ISO-8601 UTC form"
    );
    let output = render_app(&app, 200, 40);
    assert!(
        output.lines().any(|line| line.contains(DATE_ISO)),
        "the modal renders the readable date, got:\n{output}"
    );
    assert!(
        !output.contains("$date"),
        "neither the list nor the modal shows the raw canonical EJSON, got:\n{output}"
    );
}

#[test]
fn date_fields_preserve_precision_offsets_and_out_of_range_values() {
    use lazydb::model::relation_edit::{DocumentEditorState, DocumentFieldKind};
    use mongodb::bson::{Bson, DateTime, doc};
    let kind = DocumentFieldKind::of(&Bson::DateTime(DateTime::from_millis(0)));
    for text in [
        "2026-11-08T02:30:00.123+02:30",
        "2026-11-08 00:00:00.123",
        r#"{"$date":"2026-11-08T00:00:00.123Z"}"#,
    ] {
        assert_eq!(
            kind.parse(text).unwrap(),
            Bson::DateTime(DateTime::from_millis(DATE_MILLIS + 123))
        );
    }
    for millis in [i64::MIN, -1, 0, 123, i64::MAX, 253_402_300_800_000] {
        let original = doc! { "_id": 1, "date": DateTime::from_millis(millis) };
        let mut state =
            DocumentEditorState::new(0, &mongo_namespace(), &document(original.clone())).unwrap();
        state.selected_field = 1;
        state.begin_field_edit();
        let text = state.field_edit.as_ref().unwrap().input.value().to_owned();
        state.apply_field("date", &text).unwrap();
        assert_eq!(
            state.replacement_document().unwrap(),
            original,
            "round trip at {millis}"
        );
    }
    for text in [
        "2026-11-08T00:00:00.123456Z",
        "2026-11-08 00:00:00.000001",
        "2016-12-31T23:59:60Z",
        r#"{"$date":"2026-11-08T00:00:00.123456Z"}"#,
        r#"{"$date":"2016-12-31T23:59:60Z"}"#,
        r#"{"$numberLong":"1"}"#,
    ] {
        assert!(kind.parse(text).is_err(), "must refuse {text}");
    }
}

#[test]
fn a_date_field_applies_every_accepted_form_and_refuses_text_that_is_not_a_date() {
    let (mut app, _request) = mongo_app_with_page(dated_documents());
    open_fields_view(&mut app);

    for (typed, expected) in [
        (DATE_ISO, DATE_ISO),
        ("2026-11-08 15:30", "2026-11-08T15:30:00Z"),
        ("2026-11-08", "2026-11-08T00:00:00Z"),
        ("2026-11-08T00:00:00.250Z", "2026-11-08T00:00:00.250Z"),
        (
            r#"{"$date":{"$numberLong":"1794096000000"}}"#,
            "2026-11-08T00:00:00Z",
        ),
    ] {
        open_field_editor(&mut app, "date");
        retype_field_editor(&mut app, typed);
        app.update(Action::RelationEditDocumentFieldApply);
        assert_eq!(
            document_state(&app).field_error,
            None,
            "applying {typed:?} must be accepted"
        );
        assert_eq!(
            field_value(&app, "date").as_deref(),
            Some(expected),
            "after applying {typed:?}"
        );
    }

    // The document text stays canonical, so the write path and the EJSON view
    // keep seeing the same value they always did.
    let text = document_state(&app).input.value().to_owned();
    assert!(
        text.contains("\"$date\"") && text.contains("1794096000000"),
        "the document text keeps the canonical EJSON the write path stores, got {text:?}"
    );

    open_field_editor(&mut app, "date");
    retype_field_editor(&mut app, "not a date");
    app.update(Action::RelationEditDocumentFieldApply);
    assert!(
        document_state(&app).field_error.is_some(),
        "text that is not a date is refused"
    );
    assert!(
        document_state(&app).field_edit.is_some(),
        "the refused field keeps its modal open"
    );
    assert_eq!(
        field_value(&app, "date").as_deref(),
        Some("2026-11-08T00:00:00Z"),
        "a refused date leaves the document untouched"
    );
}

#[test]
fn the_field_modal_keeps_a_blank_row_around_the_value() {
    let (mut app, _request) = mongo_app_with_page(typed_documents());
    open_fields_view(&mut app);

    open_field_editor(&mut app, "name");
    let output = render_app(&app, 200, 40);
    let layout = field_modal_layout(&output);
    assert!(
        layout.value_rows >= 1,
        "the one-line value sits between both margins, got {layout:?}"
    );
    assert_eq!(
        layout.above.trim(),
        "",
        "a one-line field keeps a blank row above its value, got {:?}",
        layout.above
    );
    assert_eq!(
        layout.below.trim(),
        "",
        "a one-line field keeps a blank row below its value, got {:?}",
        layout.below
    );

    app.update(Action::RelationEditDocumentFieldCancel);
    open_field_editor(&mut app, "tags");
    let output = render_app(&app, 200, 40);
    let layout = field_modal_layout(&output);
    assert_eq!(
        layout.value_rows, 4,
        "the whole four-line array sits between both margins"
    );
    assert_eq!(
        layout.above.trim(),
        "",
        "a multi-line field keeps a blank row above its value, got {:?}",
        layout.above
    );
    assert_eq!(
        layout.below.trim(),
        "",
        "a multi-line field keeps a blank row below its value, got {:?}",
        layout.below
    );
}

#[test]
fn the_field_modal_stays_smaller_than_the_document_modal() {
    let (mut app, _request) = mongo_app_with_page(typed_documents());
    open_fields_view(&mut app);
    set_document_text(&mut app, &document_with_long_array());

    let text = open_field_editor(&mut app, "tags");
    assert_eq!(
        text.lines().count(),
        42,
        "the whole array is in the editor, one element per line"
    );
    let output = render_app(&app, 200, 40);
    assert!(
        output.contains("EDIT FIELD tags · array"),
        "the field modal names the field, got:\n{output}"
    );
    assert!(
        output.contains("EDIT DOCUMENT"),
        "the document modal keeps its own title, so the field modal never covers it, got:\n{output}"
    );
    assert!(
        output
            .lines()
            .any(|line| line.contains("Ctrl-F format") && line.contains("Esc back to fields")),
        "the field modal keeps its keys on one line, got:\n{output}"
    );
}

#[test]
fn applying_a_multiline_container_field_writes_the_whole_field() {
    let (mut app, _request) = mongo_app_with_page(typed_documents());
    open_fields_view(&mut app);
    let ejson = document_state(&app).input.value().to_owned();
    open_field_editor(&mut app, "tags");

    retype_field_editor(&mut app, "[\n  \"road\",\n  \"gravel\"\n]");
    let commands = app.update(Action::RelationEditDocumentFieldApply);

    assert!(
        commands.is_empty(),
        "applying a field writes nothing by itself, got {commands:?}"
    );
    let state = document_state(&app);
    assert!(
        state.field_edit.is_none(),
        "applying closes the field modal"
    );
    assert_eq!(state.field_error, None);
    assert_ne!(
        state.input.value(),
        ejson,
        "the document text was rewritten"
    );
    let document = state
        .replacement_document()
        .expect("the document is still valid");
    assert_eq!(
        document.get("tags"),
        Some(&mongodb::bson::Bson::Array(vec![
            mongodb::bson::Bson::String("road".into()),
            mongodb::bson::Bson::String("gravel".into()),
        ])),
        "the newline-separated array is parsed as one array value"
    );
    assert_eq!(
        document.get("_id"),
        Some(&object_id(RUEDAS_ID)),
        "editing a container keeps the same _id"
    );
    assert_eq!(
        field_value(&app, "geometry").as_deref(),
        Some(r#"{"wheels":{"$numberInt":"2"},"frame":"carbon"}"#),
        "the other container is untouched by this edit"
    );
}

#[test]
fn an_invalid_multiline_field_value_is_refused_and_keeps_the_modal_open() {
    let (mut app, _request) = mongo_app_with_page(typed_documents());
    open_fields_view(&mut app);
    let ejson = document_state(&app).input.value().to_owned();
    open_field_editor(&mut app, "tags");

    retype_field_editor(&mut app, "[\n  \"road\",\n");
    let commands = app.update(Action::RelationEditDocumentFieldApply);

    assert!(
        commands.is_empty(),
        "an invalid field value must never reach the database, got {commands:?}"
    );
    let state = document_state(&app);
    let error = state.field_error.clone().unwrap_or_default();
    assert!(
        error.contains("invalid Canonical EJSON"),
        "the refusal names the problem, got {error:?}"
    );
    assert!(
        state
            .field_edit
            .as_ref()
            .is_some_and(|field| field.input.is_multiline()),
        "a refused value keeps the multi-line editor open so it can be corrected"
    );
    assert_eq!(
        state.input.value(),
        ejson,
        "a refused value leaves the document text unchanged"
    );
}

#[test]
fn the_multiline_field_editor_formats_its_text_on_demand() {
    let (mut app, _request) = mongo_app_with_page(typed_documents());
    open_fields_view(&mut app);
    open_field_editor(&mut app, "tags");
    retype_field_editor(&mut app, r#"["road","gravel"]"#);

    let commands = app.update(Action::RelationEditDocumentFieldFormat);

    assert!(
        commands.is_empty(),
        "formatting writes nothing, got {commands:?}"
    );
    let state = document_state(&app);
    assert_eq!(state.field_error, None);
    assert_eq!(
        document_field_text(&app),
        "[\n  \"road\",\n  \"gravel\"\n]",
        "Ctrl-F re-indents the value in the field modal"
    );
}

#[test]
fn leaving_the_multiline_field_editor_with_escape_keeps_the_document_text() {
    let (mut app, _request) = mongo_app_with_page(typed_documents());
    open_fields_view(&mut app);
    let ejson = document_state(&app).input.value().to_owned();
    open_field_editor(&mut app, "tags");
    retype_field_editor(&mut app, "[\n  \"discarded\"\n]");

    let commands = app.update(Action::RelationEditDocumentFieldCancel);

    assert!(
        commands.is_empty(),
        "leaving the field modal writes nothing, got {commands:?}"
    );
    let state = document_state(&app);
    assert!(state.field_edit.is_none());
    assert_eq!(state.field_error, None);
    assert_eq!(
        state.view,
        DocumentEditorView::Fields,
        "the field list is the surface left behind"
    );
    assert_eq!(
        state.input.value(),
        ejson,
        "an abandoned field edit never touches the document text"
    );
}

#[test]
fn the_field_list_shows_a_shape_marker_for_container_fields() {
    let (mut app, _request) = mongo_app_with_page(typed_documents());
    open_fields_view(&mut app);

    let output = render_app(&app, 200, 40);

    assert!(
        output.contains("[2 items]") && output.contains("{2 fields}"),
        "containers collapse to a shape marker instead of clipped JSON, got:\n{output}"
    );
    assert!(
        !output.contains(r#"["road","fast"]"#),
        "the raw compact array is not shown in the list, got:\n{output}"
    );
    assert_eq!(
        field_value(&app, "tags").as_deref(),
        Some(r#"["road","fast"]"#),
        "the value the editors start from is still the compact EJSON"
    );
}

// ---------------------------------------------------------------------------
// Integration: isolated compare-and-replace against a local MongoDB
// ---------------------------------------------------------------------------

fn mongo_url_with_database(url: &str, database: &str) -> String {
    let (address, query) = url.split_once('?').unwrap_or((url, ""));
    let address = address.trim_end_matches('/');
    if query.is_empty() {
        format!("{address}/{database}")
    } else {
        format!("{address}/{database}?{query}")
    }
}

async fn load_documents(
    adapter: &lazydb::db::mongodb::MongoAdapter,
    namespace: &MongoNamespace,
) -> Result<Vec<MongoDocument>, String> {
    adapter
        .find_page(&FindSpec::new(namespace.clone()))
        .await
        .map(|page| page.documents)
        .map_err(|error| error.to_string())
}

#[tokio::test]
async fn relation_document_compare_and_replace_preserves_concurrent_updates() {
    let Some(server_url) = support::mongodb_standalone_url() else {
        return;
    };
    let database = format!("lazydb_relation_{}", Uuid::new_v4().simple());
    let collection = format!("documents_{}", Uuid::new_v4().simple());
    let connection_url = mongo_url_with_database(&server_url, &database);
    let imported = import_connection_url(&connection_url, Some("mongodb-relation-cas"))
        .expect("test MongoDB URL");
    let profile = imported.profile.clone();
    let connection = DatabaseConnection::connect(&profile, imported.transient_password.as_ref())
        .await
        .expect("connect to the standalone fixture");
    let DatabaseConnection::MongoDb(adapter) = connection else {
        panic!("expected a MongoDB adapter");
    };
    let fixture_client = mongodb::Client::with_uri_str(&server_url)
        .await
        .expect("fixture client");
    let namespace = MongoNamespace::collection(&database, &collection);
    let fixture_collection = fixture_client
        .database(&database)
        .collection::<mongodb::bson::Document>(&collection);
    let ids = (0..3)
        .map(|_| mongodb::bson::oid::ObjectId::new())
        .collect::<Vec<_>>();
    let seed = ids
        .iter()
        .enumerate()
        .map(|(index, id)| {
            mongodb::bson::doc! {
                "_id": id,
                "name": format!("fixture-{index}"),
                "stock": index as i32,
                "nested": { "stable": true },
            }
        })
        .collect::<Vec<_>>();

    let result = async {
        fixture_collection
            .insert_many(seed)
            .await
            .map_err(|error| error.to_string())?;
        let before = load_documents(&adapter, &namespace).await?;
        let original = before
            .first()
            .cloned()
            .ok_or_else(|| "fixture collection is empty".to_owned())?;
        let id = original
            .bson
            .get("_id")
            .cloned()
            .ok_or_else(|| "fixture document is missing _id".to_owned())?;
        let expected = original.bson.clone();
        let mut stale_replacement = expected.clone();
        stale_replacement.insert("name", "stale-editor-value");

        fixture_collection
            .update_one(
                mongodb::bson::doc! { "_id": id.clone() },
                mongodb::bson::doc! { "$set": { "stock": 99_i32, "remote": true } },
            )
            .await
            .map_err(|error| error.to_string())?;
        let stale_outcome = adapter
            .compare_and_replace_document_detailed(
                &namespace,
                id.clone(),
                &expected,
                stale_replacement,
            )
            .await
            .map_err(|error| error.to_string())?;
        if stale_outcome != (MongoMutationOutcome::Confirmed { affected: 0 }) {
            return Err(format!(
                "stale replacement was not rejected: {stale_outcome:?}"
            ));
        }

        let after_conflict = load_documents(&adapter, &namespace).await?;
        let current = after_conflict
            .iter()
            .find(|document| document.bson.get("_id") == Some(&id))
            .ok_or_else(|| "concurrently updated fixture disappeared".to_owned())?;
        if current.bson.get_i32("stock").ok() != Some(99)
            || current.bson.get_bool("remote").ok() != Some(true)
            || current.bson.get_str("name").ok() != expected.get_str("name").ok()
        {
            return Err(format!(
                "concurrent values were overwritten: {:?}",
                current.bson
            ));
        }
        let current_document = current.bson.clone();
        let mut replacement = current_document.clone();
        replacement.insert("name", "edited-by-relation");
        let saved = adapter
            .compare_and_replace_document_detailed(
                &namespace,
                id.clone(),
                &current_document,
                replacement,
            )
            .await
            .map_err(|error| error.to_string())?;
        if saved != (MongoMutationOutcome::Confirmed { affected: 1 }) {
            return Err(format!("unchanged document replacement failed: {saved:?}"));
        }

        let after = load_documents(&adapter, &namespace).await?;
        if after.len() != before.len() {
            return Err(format!(
                "unexpected document count after replacement: {}",
                after.len()
            ));
        }
        let edited = after
            .iter()
            .find(|document| document.bson.get("_id") == Some(&id))
            .ok_or_else(|| "edited fixture disappeared".to_owned())?;
        if edited.bson.get_str("name").ok() != Some("edited-by-relation")
            || edited.bson.get_i32("stock").ok() != Some(99)
            || edited.bson.get_bool("remote").ok() != Some(true)
        {
            return Err(format!(
                "replacement lost expected values: {:?}",
                edited.bson
            ));
        }
        let before_untouched = before
            .iter()
            .filter(|document| document.bson.get("_id") != Some(&id))
            .map(|document| {
                (
                    document.bson.get("_id").cloned().expect("fixture _id"),
                    document.raw.as_bytes().to_vec(),
                )
            })
            .collect::<Vec<_>>();
        let after_untouched = after
            .iter()
            .filter(|document| document.bson.get("_id") != Some(&id))
            .map(|document| {
                (
                    document.bson.get("_id").cloned().expect("fixture _id"),
                    document.raw.as_bytes().to_vec(),
                )
            })
            .collect::<Vec<_>>();
        if before_untouched != after_untouched {
            return Err("unrelated documents changed during replacement".to_owned());
        }
        Ok::<(), String>(())
    }
    .await;

    let cleanup = fixture_client.database(&database).drop().await;
    adapter.close().await;
    fixture_client.shutdown().await;
    cleanup.expect("drop the isolated MongoDB relation fixture");
    result.unwrap_or_else(|error| panic!("MongoDB relation CAS fixture failed: {error}"));
}

// ---------------------------------------------------------------------------
// S3 (user feedback): the modal opens on the fields, and EJSON is formatted
// ---------------------------------------------------------------------------

#[test]
fn the_document_modal_opens_on_the_field_view() {
    let (mut app, _request) = mongo_app_with_page(bike_documents());

    app.update(Action::RelationEditCell);

    let state = document_state(&app);
    assert_eq!(
        state.view,
        DocumentEditorView::Fields,
        "opening a document must land on the per-field view"
    );
    let names = field_names(&app);
    assert!(names.contains(&"_id".to_owned()), "{names:?}");
    assert!(names.contains(&"name".to_owned()), "{names:?}");
}

#[test]
fn the_document_modal_renders_indented_ejson() {
    let (mut app, _request) = mongo_app_with_page(bike_documents());
    app.update(Action::RelationEditCell);
    app.update(Action::RelationEditDocumentToggleView);

    let text = document_state(&app).input.value().to_owned();

    assert!(
        text.lines().count() > 1,
        "the document must be formatted, got {text:?}"
    );
    assert!(
        text.lines().any(|line| line.starts_with("  ")),
        "fields must be indented, got {text:?}"
    );
    assert!(
        text.starts_with("{\n"),
        "the object must open its own line, got {text:?}"
    );
}
