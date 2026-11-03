mod support;

use futures_util::TryStreamExt;
use lazydb::{
    db::{
        DatabaseConnection,
        mongodb::{MongoAdapter, mutation::MongoMutationOutcome, types::MongoNamespace},
    },
    model::{
        mongodb_browser::{MongoBrowserTab, MongoMutationKind},
        tab::WorkspaceTab,
    },
    profile::import_connection_url,
};
use mongodb::{
    Client,
    bson::{Bson, Document, doc},
};
use uuid::Uuid;

async fn mutation_fixture() -> Option<(MongoAdapter, Client, String)> {
    let url = support::mongodb_test_url()?;
    let imported = import_connection_url(&url, Some("mutation-test")).unwrap();
    let connection =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let DatabaseConnection::MongoDb(adapter) = connection else {
        panic!("expected MongoDB adapter")
    };
    let client = Client::with_uri_str(&url).await.unwrap();
    let database = format!("lazydb_mutation_{}", Uuid::new_v4().simple());
    Some((adapter, client, database))
}

#[tokio::test]
async fn ordinary_collection_delete_is_bounded_to_loaded_bson_snapshots() {
    let Some((adapter, client, database)) = mutation_fixture().await else {
        return;
    };
    let namespace = MongoNamespace::collection(&database, "items");
    let collection = client.database(&database).collection::<Document>("items");
    let ids = vec![
        Bson::ObjectId(mongodb::bson::oid::ObjectId::new()),
        Bson::String("7".into()),
        Bson::Document(doc! { "tenant": "test", "key": 1 }),
    ];
    let mut documents = ids
        .iter()
        .cloned()
        .map(|id| doc! { "_id": id, "kind": "same" })
        .collect::<Vec<_>>();
    documents.push(doc! { "_id": 7, "kind": "same" });
    documents.push(doc! { "_id": "unmarked", "kind": "same" });
    collection.insert_many(documents).await.unwrap();
    assert!(
        collection
            .insert_one(doc! { "_id": ids[0].clone() })
            .await
            .is_err(),
        "an ordinary collection must reject a second document with the same _id"
    );

    assert!(
        adapter
            .delete_documents_detailed(&namespace, Vec::new())
            .await
            .is_err()
    );
    assert_eq!(
        collection.count_documents(doc! {}).await.unwrap(),
        5,
        "an empty request must leave every document intact"
    );
    assert!(
        adapter
            .delete_documents_detailed(
                &namespace,
                vec![doc! { "_id": Bson::RegularExpression(mongodb::bson::Regex {
                    pattern: ".*".into(),
                    options: String::new(),
                }) }],
            )
            .await
            .is_err(),
        "a regex must never turn an id into a broader query"
    );
    assert_eq!(collection.count_documents(doc! {}).await.unwrap(), 5);
    let originals = collection
        .find(doc! { "_id": { "$in": ids } })
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(
        adapter
            .delete_documents_detailed(&namespace, originals)
            .await
            .unwrap(),
        MongoMutationOutcome::Confirmed { affected: 3 }
    );
    assert!(
        collection
            .find_one(doc! { "_id": 7 })
            .await
            .unwrap()
            .is_some(),
        "the numeric id must survive a delete addressed to the string id"
    );
    assert!(
        collection
            .find_one(doc! { "_id": "unmarked" })
            .await
            .unwrap()
            .is_some(),
        "matching non-id fields must never include an unmarked document"
    );
    assert_eq!(collection.count_documents(doc! {}).await.unwrap(), 2);

    let remaining = collection
        .find(doc! {})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    collection
        .delete_one(doc! { "_id": "unmarked" })
        .await
        .unwrap();
    assert_eq!(
        adapter
            .delete_documents_detailed(&namespace, remaining)
            .await
            .unwrap(),
        MongoMutationOutcome::Partial {
            affected: 1,
            requested: 2
        }
    );
    assert_eq!(collection.count_documents(doc! {}).await.unwrap(), 0);
    client.database(&database).drop().await.unwrap();
    client.shutdown().await;
    adapter.close().await;
}

#[tokio::test]
async fn changed_documents_survive_a_staged_snapshot_delete() {
    let Some((adapter, client, database)) = mutation_fixture().await else {
        return;
    };
    let namespace = MongoNamespace::collection(&database, "items");
    let collection = client.database(&database).collection::<Document>("items");
    collection
        .insert_many([
            doc! { "_id": 1, "value": { "$gt": 1 } },
            doc! { "_id": 2, "value": "unchanged" },
            doc! { "_id": 3, "value": "unmarked" },
        ])
        .await
        .unwrap();
    let originals = collection
        .find(doc! { "_id": { "$in": [1, 2] } })
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    collection
        .update_one(doc! { "_id": 1 }, doc! { "$set": { "value": "changed" } })
        .await
        .unwrap();

    assert_eq!(
        adapter
            .delete_documents_detailed(&namespace, originals)
            .await
            .unwrap(),
        MongoMutationOutcome::Partial {
            affected: 1,
            requested: 2
        }
    );
    assert_eq!(
        collection
            .find_one(doc! { "_id": 1 })
            .await
            .unwrap()
            .unwrap()
            .get_str("value"),
        Ok("changed"),
        "a changed document must survive its stale snapshot"
    );
    assert!(
        collection
            .find_one(doc! { "_id": 2 })
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        collection
            .find_one(doc! { "_id": 3 })
            .await
            .unwrap()
            .is_some()
    );
    client.database(&database).drop().await.unwrap();
    client.shutdown().await;
    adapter.close().await;
}

#[tokio::test]
async fn timeseries_duplicate_id_delete_is_refused_without_removing_measurements() {
    let Some((adapter, client, database)) = mutation_fixture().await else {
        return;
    };
    let db = client.database(&database);
    db.create_collection("measurements")
        .timeseries(
            mongodb::options::TimeseriesOptions::builder()
                .time_field("at")
                .build(),
        )
        .await
        .unwrap();
    let namespace = MongoNamespace::collection(&database, "measurements");
    let collection = db.collection::<Document>("measurements");
    let id = Bson::ObjectId(mongodb::bson::oid::ObjectId::new());
    let at = mongodb::bson::DateTime::now();
    collection
        .insert_many([
            doc! { "_id": id.clone(), "at": at, "value": 1 },
            doc! { "_id": id.clone(), "at": at, "value": 2 },
            doc! { "_id": "unmarked", "at": at, "value": 3 },
        ])
        .await
        .unwrap();
    assert_eq!(
        collection
            .count_documents(doc! { "_id": id.clone() })
            .await
            .unwrap(),
        2,
        "the fixture must reproduce multiple measurements sharing one _id"
    );

    let original = collection
        .find_one(doc! { "_id": id.clone(), "value": 1 })
        .await
        .unwrap()
        .unwrap();
    let error = adapter
        .delete_documents_detailed(&namespace, vec![original])
        .await
        .expect_err("time-series collections must be refused before the delete command");
    assert!(error.message.contains("ordinary collection"), "{error}");
    assert_eq!(
        collection
            .count_documents(doc! { "_id": id })
            .await
            .unwrap(),
        2,
        "both measurements with the marked id must survive"
    );
    assert_eq!(
        collection.count_documents(doc! {}).await.unwrap(),
        3,
        "refusal must not delete any measurement"
    );
    db.drop().await.unwrap();
    client.shutdown().await;
    adapter.close().await;
}

#[tokio::test]
async fn nonordinary_and_missing_collections_refuse_staged_deletion() {
    let Some((adapter, client, database)) = mutation_fixture().await else {
        return;
    };
    let db = client.database(&database);
    let collection = db.collection::<Document>("items");
    collection.insert_one(doc! { "_id": "kept" }).await.unwrap();
    db.create_collection("item_view")
        .view_on("items")
        .await
        .unwrap();
    db.create_collection("capped_items")
        .capped(true)
        .size(4096_u64)
        .await
        .unwrap();
    let capped = db.collection::<Document>("capped_items");
    capped.insert_one(doc! { "_id": "kept" }).await.unwrap();
    for name in ["item_view", "missing", "capped_items"] {
        assert!(
            adapter
                .delete_documents_detailed(
                    &MongoNamespace::collection(&database, name),
                    vec![doc! { "_id": "kept" }]
                )
                .await
                .is_err(),
            "{name} must be refused"
        );
    }
    assert_eq!(collection.count_documents(doc! {}).await.unwrap(), 1);
    assert_eq!(capped.count_documents(doc! {}).await.unwrap(), 1);
    db.drop().await.unwrap();
    client.shutdown().await;
    adapter.close().await;
}

#[tokio::test]
async fn snapshot_comparison_uses_simple_collation_for_ids_and_document_contents() {
    let Some((adapter, client, database)) = mutation_fixture().await else {
        return;
    };
    let db = client.database(&database);
    db.create_collection("items")
        .collation(
            mongodb::options::Collation::builder()
                .locale("en")
                .strength(mongodb::options::CollationStrength::Secondary)
                .build(),
        )
        .await
        .unwrap();
    let collection = db.collection::<Document>("items");
    collection
        .insert_many([
            doc! { "_id": "A", "kind": "id" },
            doc! { "_id": "payload", "value": "A" },
        ])
        .await
        .unwrap();
    let originals = collection
        .find(doc! {})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    collection.delete_one(doc! { "_id": "A" }).await.unwrap();
    collection
        .insert_one(doc! { "_id": "a", "kind": "id" })
        .await
        .unwrap();
    collection
        .update_one(doc! { "_id": "payload" }, doc! { "$set": { "value": "a" } })
        .await
        .unwrap();
    assert_eq!(
        adapter
            .delete_documents_detailed(&MongoNamespace::collection(&database, "items"), originals)
            .await
            .unwrap(),
        MongoMutationOutcome::Partial {
            affected: 0,
            requested: 2
        }
    );
    assert_eq!(
        collection.count_documents(doc! {}).await.unwrap(),
        2,
        "case-insensitive defaults must not retarget an id or hide changed content"
    );
    let current = collection
        .find(doc! {})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(
        adapter
            .delete_documents_detailed(&MongoNamespace::collection(&database, "items"), current)
            .await
            .unwrap(),
        MongoMutationOutcome::Confirmed { affected: 2 }
    );
    db.drop().await.unwrap();
    client.shutdown().await;
    adapter.close().await;
}

/// Optional authenticated fixture: the URI user has readWrite on exactly its
/// selected database, with no cluster/admin privileges.
#[tokio::test]
async fn snapshot_delete_works_with_only_database_read_write_privileges() {
    let url = match std::env::var("LAZYDB_TEST_MONGODB_READWRITE_URL") {
        Ok(url) if !url.trim().is_empty() => url,
        Ok(_) | Err(std::env::VarError::NotPresent) => {
            eprintln!("SKIP: LAZYDB_TEST_MONGODB_READWRITE_URL is not set");
            return;
        }
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("LAZYDB_TEST_MONGODB_READWRITE_URL is not valid Unicode")
        }
    };
    let imported = import_connection_url(&url, Some("mutation-readwrite-test")).unwrap();
    let database = imported
        .profile
        .database
        .clone()
        .expect("the readWrite fixture URI must select its authorized database");
    let connection =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let DatabaseConnection::MongoDb(adapter) = connection else {
        panic!("expected MongoDB adapter")
    };
    let client = Client::with_uri_str(&url).await.unwrap();
    let denied = client
        .database("admin")
        .run_command(doc! { "shardingState": 1 })
        .await
        .expect_err("the fixture must not grant cluster privileges");
    assert!(
        matches!(denied.kind.as_ref(), mongodb::error::ErrorKind::Command(error) if error.code == 13)
    );
    let name = format!("mutation_{}", Uuid::new_v4().simple());
    let namespace = MongoNamespace::collection(&database, &name);
    let collection = client.database(&database).collection::<Document>(&name);
    collection
        .insert_one(doc! { "_id": "selected", "value": 1 })
        .await
        .unwrap();
    let original = collection
        .find_one(doc! { "_id": "selected" })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        adapter
            .delete_documents_detailed(&namespace, vec![original])
            .await
            .unwrap(),
        MongoMutationOutcome::Confirmed { affected: 1 }
    );
    assert_eq!(collection.count_documents(doc! {}).await.unwrap(), 0);
    collection.drop().await.unwrap();
    client.shutdown().await;
    adapter.close().await;
}

#[test]
fn mutation_outcomes_preserve_confirmed_partial_and_unknown_certainty() {
    assert!(MongoMutationOutcome::Confirmed { affected: 4 }.is_fully_confirmed());
    assert!(
        !MongoMutationOutcome::Partial {
            affected: 2,
            requested: 5
        }
        .is_fully_confirmed()
    );
    assert!(!MongoMutationOutcome::Unknown { requested: 5 }.is_fully_confirmed());
}

#[test]
fn reducer_rejects_stale_mutation_and_does_not_refresh_uncertain_outcomes() {
    let profile_id = Uuid::new_v4();
    let tab = MongoBrowserTab::new(profile_id, MongoNamespace::collection("app", "items"));
    let tab_id = tab.id;
    let mut app = lazydb::app::App::new(Vec::new());
    app.tabs.push(WorkspaceTab::MongoBrowser(tab));
    app.active_tab = app.tabs.len() - 1;
    let first = match &mut app.tabs[app.active_tab] {
        WorkspaceTab::MongoBrowser(tab) => tab.begin_mutation(),
        _ => unreachable!(),
    };
    let second = match &mut app.tabs[app.active_tab] {
        WorkspaceTab::MongoBrowser(tab) => tab.begin_mutation(),
        _ => unreachable!(),
    };
    let commands = app.update(lazydb::action::Action::MongoMutationOutcome {
        tab_id,
        generation: 1,
        mutation_generation: first,
        operation: MongoMutationKind::Insert,
        outcome: MongoMutationOutcome::Partial {
            affected: 2,
            requested: 5,
        },
    });
    assert!(commands.is_empty());
    let commands = app.update(lazydb::action::Action::MongoMutationOutcome {
        tab_id,
        generation: 1,
        mutation_generation: second,
        operation: MongoMutationKind::Insert,
        outcome: MongoMutationOutcome::Unknown { requested: 5 },
    });
    assert!(commands.is_empty());
    let Some(WorkspaceTab::MongoBrowser(tab)) = app.tabs.last() else {
        unreachable!()
    };
    assert!(
        tab.mutation_message
            .as_deref()
            .unwrap()
            .contains("Outcome unknown")
    );
}
