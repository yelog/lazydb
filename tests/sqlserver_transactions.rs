use lazydb::{
    db::{
        DatabaseConnection,
        catalog::{CatalogId, CatalogKind},
        mutation::{
            DeleteRowMutation, InputValue, InsertRowMutation, MetadataFingerprint, MutationResult,
            RelationMutation, RelationMutationRequest, RowLocator, UpdateCellMutation,
        },
        transaction::TransactionBackend,
        value::CellValue,
    },
    identity::ConnectionIdentity,
    model::{execution_target::ExecutionTarget, relation::RelationKey},
    profile::import_connection_url,
};
use uuid::Uuid;

fn count(outcome: &lazydb::db::query::QueryOutcome) -> i64 {
    match &outcome.result_sets[0].rows[0][0] {
        lazydb::db::value::CellValue::Integer(value) => *value,
        lazydb::db::value::CellValue::Unsigned(value) => *value as i64,
        value => panic!("unexpected count value: {value:?}"),
    }
}

// The live suite is intentionally opt-in because it creates and drops objects in the
// configured SQL Server database. The backend-level lifecycle is exercised through the
// same public connection path used by the runtime.
#[tokio::test]
async fn sql_server_transactions_cover_isolation_commit_rollback_ddl_disconnect_and_pool_reuse() {
    let Ok(url) = std::env::var("LAZYDB_TEST_SQLSERVER_URL") else {
        return;
    };
    let imported = import_connection_url(&url, Some("sqlserver-transactions")).unwrap();
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap_or_else(|error| {
                panic!("SQL Server transaction fixture failed to connect: {error}")
            });

    let table = format!("[dbo].[lazydb_transaction_{}]", Uuid::new_v4().simple());
    database
        .execute(&format!("CREATE TABLE {table} ([id] int NOT NULL)"))
        .await
        .unwrap();

    let mut transaction = match &database {
        DatabaseConnection::SqlServer(adapter) => adapter.transaction_backend().await.unwrap(),
        _ => unreachable!("SQL Server URL produced a non-SQL Server connection"),
    };
    transaction.begin().await.unwrap();
    transaction
        .execute(&format!("INSERT INTO {table} ([id]) VALUES (1)"))
        .await
        .unwrap();
    let outside = database
        .execute(&format!("SELECT COUNT(*) FROM {table}"))
        .await
        .unwrap();
    assert_eq!(count(&outside), 0);
    transaction.commit().await.unwrap();
    assert_eq!(transaction.depth(), 0);

    let committed = database
        .execute(&format!("SELECT COUNT(*) FROM {table}"))
        .await
        .unwrap();
    assert_eq!(count(&committed), 1);

    transaction.begin().await.unwrap();
    transaction
        .execute(&format!("INSERT INTO {table} ([id]) VALUES (2)"))
        .await
        .unwrap();
    transaction.rollback().await.unwrap();
    let rolled_back = database
        .execute(&format!("SELECT COUNT(*) FROM {table}"))
        .await
        .unwrap();
    assert_eq!(count(&rolled_back), 1);

    transaction.begin().await.unwrap();
    transaction
        .execute(&format!(
            "CREATE TABLE [dbo].[lazydb_ddl_{}] ([id] int)",
            Uuid::new_v4().simple()
        ))
        .await
        .unwrap();
    transaction.rollback().await.unwrap();

    transaction.begin().await.unwrap();
    transaction
        .execute(&format!("INSERT INTO {table} ([id]) VALUES (3)"))
        .await
        .unwrap();
    transaction.force_close().await.unwrap();
    let after_disconnect = database
        .execute(&format!("SELECT COUNT(*) FROM {table}"))
        .await
        .unwrap();
    assert_eq!(count(&after_disconnect), 1);
    database.execute("SELECT 1").await.unwrap();
    database
        .execute(&format!("DROP TABLE {table}"))
        .await
        .unwrap();
    database.close().await;
}

#[tokio::test]
async fn sql_server_relation_mutations_use_output_and_atomic_optimistic_batches() {
    let Ok(url) = std::env::var("LAZYDB_TEST_SQLSERVER_URL") else {
        return;
    };
    let imported = import_connection_url(&url, Some("sqlserver-relation-mutations")).unwrap();
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let table = format!("[dbo].[lazydb_relation_{}]", Uuid::new_v4().simple());
    database.execute(&format!("CREATE TABLE {table} ([id] int IDENTITY(1,1) NOT NULL PRIMARY KEY, [value] nvarchar(80) NULL, [stamp] rowversion NOT NULL)")).await.unwrap();
    let mut backend = match &database {
        DatabaseConnection::SqlServer(adapter) => adapter.transaction_backend().await.unwrap(),
        _ => unreachable!(),
    };
    let profile_id = imported.profile.id;
    let table_name = table
        .trim_start_matches("[dbo].[")
        .trim_end_matches(']')
        .to_owned();
    let relation = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        [
            imported.profile.database.clone().unwrap(),
            "dbo".to_owned(),
            table_name,
            "1".to_owned(),
        ],
    );
    let metadata = MetadataFingerprint {
        relation: relation.native_path[2].clone(),
        columns: vec![
            ("id".into(), "int".into(), false),
            ("value".into(), "nvarchar".into(), true),
            ("stamp".into(), "rowversion".into(), false),
        ],
        primary_key: vec!["id".into()],
    };
    let request = |operation| RelationMutationRequest {
        tab_id: Uuid::nil(),
        tab_generation: 1,
        edit_generation: 1,
        row_id: lazydb::model::relation_edit::EditableRowId(1),
        connection: ConnectionIdentity {
            profile_id,
            generation: 1,
        },
        target: ExecutionTarget {
            profile_id,
            database: imported.profile.database.clone().unwrap(),
            schema: Some("dbo".into()),
        },
        relation: relation.clone(),
        relation_key: RelationKey {
            profile_id,
            object_id: relation.clone(),
        },
        scope: imported.profile.catalog_scope.clone(),
        metadata: metadata.clone(),
        operation,
    };
    backend.begin().await.unwrap();
    let inserted = backend
        .relation_mutation(request(RelationMutation::InsertRow(InsertRowMutation {
            columns: vec![1],
            values: vec![InputValue::Value(CellValue::Text("before".into()))],
        })))
        .await
        .unwrap();
    let CellValue::Integer(id) = inserted_row_value(&inserted, 0) else {
        panic!("identity was not returned")
    };
    let updated = backend
        .relation_mutation(request(RelationMutation::UpdateCell(UpdateCellMutation {
            row: RowLocator {
                columns: vec![0],
                values: vec![CellValue::Integer(id)],
            },
            column: 1,
            original: CellValue::Text("before".into()),
            value: InputValue::Value(CellValue::Text("after".into())),
        })))
        .await
        .unwrap();
    assert!(matches!(updated, MutationResult::Updated { .. }));
    let deleted = backend
        .relation_mutation(request(RelationMutation::DeleteRows(vec![
            DeleteRowMutation {
                row: RowLocator {
                    columns: vec![0],
                    values: vec![CellValue::Integer(id)],
                },
                original: vec![
                    CellValue::Integer(id),
                    CellValue::Text("after".into()),
                    updated_row_value(&updated, 2),
                ],
            },
        ])))
        .await
        .unwrap();
    assert_eq!(deleted, MutationResult::Deleted { rows: 1 });
    backend.commit().await.unwrap();
    database
        .execute(&format!("DROP TABLE {table}"))
        .await
        .unwrap();
    database.close().await;
}

fn inserted_row_value(result: &MutationResult, index: usize) -> CellValue {
    match result {
        MutationResult::Inserted { row } => row[index].clone(),
        _ => panic!("expected inserted row"),
    }
}

fn updated_row_value(result: &MutationResult, index: usize) -> CellValue {
    match result {
        MutationResult::Updated { row } => row[index].clone(),
        _ => panic!("expected updated row"),
    }
}
