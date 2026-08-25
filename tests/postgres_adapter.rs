use lazydb::{
    db::{DatabaseConnection, postgres, value::CellValue},
    profile::import_connection_url,
};

#[test]
fn quotes_postgres_identifiers_and_uses_native_catalogs() {
    assert_eq!(postgres::quote_identifier("odd\"name"), "\"odd\"\"name\"");
    assert!(postgres::CATALOG_TABLES_SQL.contains("information_schema.tables"));
    assert!(postgres::CATALOG_INDEXES_SQL.contains("pg_indexes"));
}

#[tokio::test]
async fn connects_and_decodes_common_postgres_values_when_configured() {
    let Ok(url) = std::env::var("LAZYDB_TEST_POSTGRES_URL") else {
        return;
    };
    let imported = import_connection_url(&url, Some("postgres-test")).unwrap();
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();

    let server = database.probe().await.unwrap();
    assert!(!server.version.is_empty());
    let outcome = database
        .execute("SELECT 1::bigint AS n, true AS ok, 'Ada'::text AS name, NULL::text AS missing")
        .await
        .unwrap();
    let row = &outcome.result_sets.last().unwrap().rows[0];
    assert_eq!(row[0], CellValue::Integer(1));
    assert_eq!(row[1], CellValue::Boolean(true));
    assert_eq!(row[2], CellValue::Text("Ada".into()));
    assert_eq!(row[3], CellValue::Null);
    assert!(!database.load_catalog().await.unwrap().is_empty());
    database.close().await;
}
