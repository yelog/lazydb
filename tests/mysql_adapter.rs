use lazydb::{
    db::{DatabaseConnection, mysql, value::CellValue},
    profile::import_connection_url,
};

#[test]
fn quotes_mysql_identifiers_and_uses_information_schema() {
    assert_eq!(mysql::quote_identifier("odd`name"), "`odd``name`");
    assert!(mysql::CATALOG_TABLES_SQL.contains("information_schema.tables"));
    assert!(mysql::CATALOG_INDEXES_SQL.contains("information_schema.statistics"));
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
    assert!(!database.load_catalog().await.unwrap().is_empty());
    database.close().await;
}
