use lazydb::{
    db::{DatabaseConnection, mysql, value::CellValue},
    profile::import_connection_url,
};

#[test]
fn quotes_mysql_identifiers_and_uses_information_schema() {
    assert_eq!(mysql::quote_identifier("odd`name"), "`odd``name`");
    assert!(mysql::CATALOG_TABLES_SQL.contains("information_schema.tables"));
    assert!(mysql::CATALOG_INDEXES_SQL.contains("information_schema.statistics"));
    assert!(mysql::CATALOG_ROUTINES_SQL.contains("information_schema.routines"));
}

#[tokio::test]
async fn connects_and_decodes_common_mysql_values_when_configured() {
    let Ok(url) = std::env::var("LAZYDB_TEST_MYSQL_URL") else {
        return;
    };
    let imported = import_connection_url(&url, Some("mysql-test")).unwrap();
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();

    let server = database.probe().await.unwrap();
    assert!(!server.version.is_empty());
    let outcome = database
        .execute("SELECT CAST(1 AS SIGNED) AS n, TRUE AS ok, 'Ada' AS name, NULL AS missing")
        .await
        .unwrap();
    let row = &outcome.result_sets.last().unwrap().rows[0];
    assert_eq!(row[0], CellValue::Integer(1));
    assert!(matches!(
        row[1],
        CellValue::Boolean(true) | CellValue::Integer(1)
    ));
    assert_eq!(row[2], CellValue::Text("Ada".into()));
    assert_eq!(row[3], CellValue::Null);
    let multiple = database
        .execute("SELECT 1 AS first; SELECT 2 AS second")
        .await
        .unwrap();
    assert_eq!(multiple.result_sets.len(), 2);
    assert!(multiple.stats.total() >= multiple.stats.execution);
    let affected = database
        .execute(
            "CREATE TEMPORARY TABLE lazydb_task14_affected (value INTEGER); \
             INSERT INTO lazydb_task14_affected VALUES (1), (2); \
             UPDATE lazydb_task14_affected SET value = value",
        )
        .await
        .unwrap();
    assert_eq!(affected.result_sets.last().unwrap().affected_rows, 2);
    let error = database
        .execute("SELECT * FROM missing_task14_table")
        .await
        .unwrap_err();
    assert_eq!(error.category, lazydb::db::ErrorCategory::Sql);
    assert!(!database.load_catalog().await.unwrap().is_empty());
    database.close().await;
}
