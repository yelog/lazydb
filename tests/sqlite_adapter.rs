use lazydb::{
    db::{DatabaseConnection, catalog::CatalogKind, value::CellValue},
    profile::import_connection_url,
};
use tempfile::TempDir;

#[tokio::test]
async fn probes_catalogs_queries_and_reads_ddl() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("catalog.db");
    let imported =
        import_connection_url(&format!("sqlite://{}", path.display()), Some("catalog")).unwrap();
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();

    database
        .execute(
            r#"
            PRAGMA foreign_keys = ON;
            CREATE TABLE teams (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
            CREATE TABLE users (
                id INTEGER PRIMARY KEY,
                team_id INTEGER REFERENCES teams(id),
                name TEXT NOT NULL,
                score REAL,
                payload BLOB
            );
            CREATE INDEX users_name_idx ON users(name);
            CREATE VIEW active_users AS SELECT id, name FROM users;
            CREATE TRIGGER users_name_guard BEFORE INSERT ON users
            WHEN NEW.name = '' BEGIN SELECT RAISE(ABORT, 'name required'); END;
            INSERT INTO teams VALUES (1, 'core');
            INSERT INTO users VALUES (1, 1, 'Ada', 9.5, X'0001FF');
            INSERT INTO users VALUES (2, NULL, 'Lin', NULL, NULL);
            "#,
        )
        .await
        .unwrap();

    let server = database.probe().await.unwrap();
    assert_eq!(
        server.database,
        ":memory:".replace(":memory:", &path.to_string_lossy())
    );
    assert!(server.version.chars().next().unwrap().is_ascii_digit());

    let catalog = database.load_catalog().await.unwrap();
    assert!(
        catalog
            .iter()
            .any(|node| node.kind == CatalogKind::Table && node.name == "users")
    );
    assert!(
        catalog
            .iter()
            .any(|node| node.kind == CatalogKind::View && node.name == "active_users")
    );
    assert!(
        catalog
            .iter()
            .any(|node| node.kind == CatalogKind::Column && node.name == "team_id")
    );
    assert!(
        catalog
            .iter()
            .any(|node| node.kind == CatalogKind::Index && node.name == "users_name_idx")
    );
    assert!(
        catalog
            .iter()
            .any(|node| node.kind == CatalogKind::ForeignKey)
    );
    assert!(
        catalog
            .iter()
            .any(|node| node.kind == CatalogKind::Trigger && node.name == "users_name_guard")
    );

    let outcome = database
        .execute("SELECT id, team_id, name, score, payload FROM users ORDER BY id")
        .await
        .unwrap();
    let result = outcome.result_sets.last().unwrap();
    assert_eq!(
        result
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["id", "team_id", "name", "score", "payload"]
    );
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], CellValue::Integer(1));
    assert_eq!(result.rows[0][2], CellValue::Text("Ada".into()));
    assert_eq!(result.rows[0][3], CellValue::Float(9.5));
    assert_eq!(result.rows[0][4], CellValue::Bytes(vec![0, 1, 255]));
    assert_eq!(result.rows[1][1], CellValue::Null);
    assert_eq!(result.rows[1][3], CellValue::Null);

    let ddl = database
        .object_ddl(CatalogKind::Table, "main", "users")
        .await
        .unwrap()
        .unwrap();
    assert!(ddl.contains("CREATE TABLE users"));
    database.close().await;
}

#[tokio::test]
async fn enforces_sqlite_read_only_at_connection_level() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("readonly.db");
    let imported =
        import_connection_url(&format!("sqlite://{}", path.display()), Some("readonly")).unwrap();
    let writable = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();
    writable
        .execute("CREATE TABLE records (id INTEGER PRIMARY KEY)")
        .await
        .unwrap();
    writable.close().await;

    let mut read_only_profile = imported.profile;
    read_only_profile.read_only = true;
    let read_only = DatabaseConnection::connect(&read_only_profile, None)
        .await
        .unwrap();

    let error = read_only
        .execute("INSERT INTO records VALUES (1)")
        .await
        .unwrap_err();
    assert!(error.to_string().to_ascii_lowercase().contains("readonly"));
    read_only.close().await;
}

#[tokio::test]
async fn preserves_result_sets_counts_timing_empty_values_and_errors() {
    let imported = import_connection_url("sqlite://:memory:", Some("results")).unwrap();
    let database = DatabaseConnection::connect(&imported.profile, None)
        .await
        .unwrap();

    let outcome = database
        .execute("SELECT '' AS empty_value, NULL AS missing_value; SELECT 7 AS second_result")
        .await
        .unwrap();
    assert_eq!(outcome.result_sets.len(), 2);
    assert_eq!(outcome.stats.row_count, 2);
    assert!(outcome.stats.total() >= outcome.stats.execution);
    assert_eq!(
        outcome.result_sets[0].rows[0][0],
        CellValue::Text(String::new())
    );
    assert_eq!(outcome.result_sets[0].rows[0][1], CellValue::Null);
    assert_eq!(outcome.result_sets[1].rows[0][0], CellValue::Integer(7));

    database
        .execute("CREATE TABLE affected (value TEXT); INSERT INTO affected VALUES ('x'), ('y')")
        .await
        .unwrap();
    let affected = database
        .execute("UPDATE affected SET value = value")
        .await
        .unwrap();
    assert_eq!(affected.result_sets.last().unwrap().affected_rows, 2);

    let error = database
        .execute("SELECT * FROM missing_table")
        .await
        .unwrap_err();
    assert_eq!(error.category, lazydb::db::ErrorCategory::Sql);
    database.close().await;
}
