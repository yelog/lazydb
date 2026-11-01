use std::panic::AssertUnwindSafe;

use futures_util::FutureExt;
use lazydb::{
    db::{
        DatabaseConnection, ErrorCategory,
        catalog::{
            CatalogCount, CatalogCursor, CatalogId, CatalogKind, CatalogMetadata, CatalogRequest,
            CatalogRequestKey, CatalogSearchObjectScope, CatalogSearchRequest, CatalogTarget,
            DdlProvenance, ObjectGroup,
        },
        catalog_drop::CatalogDropRequest,
        mysql::{self, MySqlAdapter, ServerCapabilities},
        value::CellValue,
    },
    identity::ConnectionIdentity,
    model::{
        pagination::{PageRequest, PageSize},
        relation::RelationPreviewOptions,
    },
    profile::{CatalogScope, CatalogSelection, DatabaseKind, DatabaseScope, import_connection_url},
};
use uuid::Uuid;

// Live tests remain optional locally and mandatory when CI requires database tests.
fn tidb_test_url() -> Option<String> {
    match std::env::var("LAZYDB_TEST_TIDB_URL") {
        Ok(url) if !url.trim().is_empty() => Some(url),
        Ok(_) | Err(std::env::VarError::NotPresent) => {
            if std::env::var_os("LAZYDB_REQUIRE_DATABASE_TESTS").is_some()
                || std::env::var_os("REQUIRE_DATABASE_TESTS").is_some()
            {
                panic!("LAZYDB_TEST_TIDB_URL is required for TiDB integration tests");
            }
            eprintln!("SKIP: LAZYDB_TEST_TIDB_URL is not set");
            None
        }
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("LAZYDB_TEST_TIDB_URL is not valid Unicode")
        }
    }
}

#[test]
fn product_version_gate_uses_stable_tidb_version_instead_of_mysql_prefix() {
    for version in [
        "5.7.25-TiDB-v8.5.0",
        "5.7.25-TiDB-v8.5.3",
        "8.0.11-TiDB-v8.5.4",
        "8.0.11-TiDB-v8.5.3-serverless",
        "8.0.11-TiDB-v9.0.0",
    ] {
        assert!(
            mysql::supports_catalog_version_for_kind(DatabaseKind::TiDb, version),
            "{version}"
        );
        for kind in [DatabaseKind::MySql, DatabaseKind::MariaDb] {
            assert!(
                !mysql::supports_catalog_version_for_kind(kind, version),
                "{kind:?}: {version}"
            );
        }
        assert_eq!(
            ServerCapabilities::for_kind(DatabaseKind::TiDb, version),
            ServerCapabilities {
                catalog: true,
                sequences: false,
                relation_edit: false,
            }
        );
    }
    for version in [
        "5.7.25-TiDB-v8.4.9",
        "8.0.11-TiDB-v7.5.3",
        "8.0.11-TiDB-v8.5.0-rc.1",
        "8.0.11-TiDB-v8.5.0-alpha",
        "8.0.11-TiDB-v8.5",
        "8.0.11-TiDB-vgarbage",
        "8.0.11-TiDB",
        "8.0.36",
        "11.4.2-MariaDB",
        "11.4.2-MariaDB-TiDB-v8.5.3",
        "",
    ] {
        assert!(
            !mysql::supports_catalog_version_for_kind(DatabaseKind::TiDb, version),
            "{version}"
        );
        assert!(
            !ServerCapabilities::for_kind(DatabaseKind::TiDb, version).catalog,
            "{version}"
        );
    }
    assert!(mysql::supports_catalog_version("8.0.36"));
    assert!(mysql::supports_catalog_version_for_kind(
        DatabaseKind::MariaDb,
        "11.4.2-MariaDB"
    ));
}

#[test]
fn tidb_catalog_advertises_only_native_supported_groups_and_metadata() {
    let capabilities = MySqlAdapter::tidb_catalog_capabilities();
    assert_eq!(
        capabilities.top_level_groups,
        [ObjectGroup::Tables, ObjectGroup::Views]
    );
    assert!(capabilities.supports_lazy_children);
    assert_eq!(
        capabilities.namespace_model,
        mysql::MySqlAdapter::catalog_capabilities().namespace_model
    );
    assert!(capabilities.column_metadata.type_family);
    assert!(capabilities.column_metadata.generated_expression);
    assert!(capabilities.column_metadata.auto_increment);
    assert!(!capabilities.column_metadata.identity);
    assert!(!capabilities.column_metadata.hidden);
}

fn identity(profile_id: Uuid) -> ConnectionIdentity {
    ConnectionIdentity {
        profile_id,
        generation: 7,
    }
}

fn scope(database: &str) -> CatalogScope {
    CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: database.to_owned(),
            schemas: CatalogSelection::All,
        }]),
    }
}

fn request(
    profile_id: Uuid,
    target: CatalogTarget,
    scope: CatalogScope,
    cursor: Option<CatalogCursor>,
) -> CatalogRequest {
    CatalogRequest {
        key: CatalogRequestKey {
            connection: identity(profile_id),
            catalog_epoch: 1,
            request_id: 1,
            target,
            cursor,
        },
        scope,
        page_size: 1,
    }
}

#[tokio::test]
async fn tidb_query_preserves_identity_empty_metadata_result_sets_and_native_values() {
    let Some(url) = tidb_test_url() else {
        return;
    };
    let imported = import_connection_url(&url, Some("TiDB query test")).unwrap();
    assert_eq!(imported.profile.kind, DatabaseKind::TiDb);
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    assert_eq!(database.kind(), DatabaseKind::TiDb);
    assert_eq!(database.probe().await.unwrap().kind, DatabaseKind::TiDb);
    assert_eq!(
        database.catalog_capabilities(),
        MySqlAdapter::tidb_catalog_capabilities()
    );
    let outcome = database
        .execute(
            "SELECT CAST(1 AS SIGNED) AS empty_number WHERE FALSE; \
         SELECT CAST(7 AS SIGNED) AS n, CAST(18446744073709551615 AS UNSIGNED) AS u, \
         CAST(12.34 AS DECIMAL(8,2)) AS dec_value, CAST('2025-01-02' AS DATE) AS date_value, \
         CAST('2025-01-02 03:04:05' AS DATETIME) AS datetime_value, \
         CAST('03:04:05' AS TIME) AS time_value, X'00ff' AS bytes_value, \
         JSON_OBJECT('a', 1) AS json_value, NULL AS missing; \
         SELECT 'last' AS final_value WHERE FALSE",
        )
        .await
        .unwrap();
    assert_eq!(outcome.result_sets.len(), 3);
    assert_eq!(outcome.result_sets[0].columns[0].name, "empty_number");
    assert!(!outcome.result_sets[0].columns[0].type_name.is_empty());
    assert!(outcome.result_sets[0].rows.is_empty());
    let row = &outcome.result_sets[1].rows[0];
    assert_eq!(row[0], CellValue::Integer(7));
    assert_eq!(row[1], CellValue::Unsigned(u64::MAX));
    assert_eq!(row[2], CellValue::Text("12.34".into()));
    assert!(matches!(row[3], CellValue::Date(_)));
    assert!(matches!(row[4], CellValue::DateTime(_)));
    assert!(matches!(row[5], CellValue::Time(_)));
    assert_eq!(row[6], CellValue::Bytes(vec![0, 255]));
    assert!(matches!(&row[7], CellValue::Text(json) if json.contains("\"a\"")));
    assert_eq!(row[8], CellValue::Null);
    assert_eq!(outcome.result_sets[2].columns[0].name, "final_value");
    assert!(outcome.result_sets[2].rows.is_empty());
    let escaped = database
        .execute(r#"SELECT "a\";b" AS value; SELECT 2 AS second_value"#)
        .await
        .unwrap();
    assert_eq!(escaped.result_sets.len(), 2);
    assert_eq!(escaped.result_sets[0].rows[0][0].clipboard_text(), "a\";b");
    assert_eq!(escaped.result_sets[1].rows[0][0], CellValue::Integer(2));
    assert_eq!(
        database
            .execute("SELECT * FROM lazydb_tidb_nonexistent_table")
            .await
            .unwrap_err()
            .category,
        ErrorCategory::Sql
    );
    database.close().await;

    for kind in [DatabaseKind::MySql, DatabaseKind::MariaDb] {
        let mut profile = imported.profile.clone();
        profile.kind = kind;
        let error = MySqlAdapter::connect(&profile, imported.transient_password.as_ref())
            .await
            .unwrap_err();
        assert_eq!(error.category, ErrorCategory::Configuration);
        assert!(error.message.contains("server product does not match"));
    }
}

#[tokio::test]
async fn tidb_query_refreshes_metadata_after_schema_changes() {
    let Some(url) = tidb_test_url() else {
        return;
    };
    let imported = import_connection_url(&url, Some("TiDB metadata test")).unwrap();
    let adapter = MySqlAdapter::connect(&imported.profile, imported.transient_password.as_ref())
        .await
        .unwrap();
    let table = mysql::quote_identifier(&format!("lazydb_metadata_{}", Uuid::new_v4().simple()));
    adapter
        .execute(&format!(
            "CREATE TABLE {table} (id INT PRIMARY KEY); INSERT INTO {table} VALUES (1)"
        ))
        .await
        .unwrap();
    let tested = AssertUnwindSafe(async {
        let result = adapter
            .execute(&format!(
                "SELECT * FROM {table}; ALTER TABLE {table} ADD COLUMN extra INT DEFAULT 7; \
             SELECT * FROM {table}; SELECT * FROM {table} WHERE FALSE; \
             ALTER TABLE {table} ADD COLUMN another INT; SELECT * FROM {table} WHERE FALSE"
            ))
            .await
            .unwrap();
        for (index, count) in [(0, 1), (2, 2), (3, 2), (5, 3)] {
            let set = &result.result_sets[index];
            assert_eq!(set.columns.len(), count);
            assert!(set.rows.iter().all(|row| row.len() == count));
        }
        assert!(result.result_sets[3].rows.is_empty());
        assert!(result.result_sets[5].rows.is_empty());
    })
    .catch_unwind()
    .await;
    adapter
        .execute(&format!("DROP TABLE {table}"))
        .await
        .unwrap();
    adapter.close().await;
    if let Err(panic) = tested {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
async fn tidb_query_uses_current_sql_mode_between_statements() {
    let Some(url) = tidb_test_url() else {
        return;
    };
    let imported = import_connection_url(&url, Some("TiDB SQL mode test")).unwrap();
    let adapter = MySqlAdapter::connect(&imported.profile, imported.transient_password.as_ref())
        .await
        .unwrap();
    let result = adapter
        .execute(
            r#"SET @lazydb_original_mode=@@SESSION.sql_mode;
           SET SESSION sql_mode='ANSI_QUOTES';
           SELECT 1 AS "ansi_value"; SELECT 2 AS second_value;
           SET SESSION sql_mode='NO_BACKSLASH_ESCAPES';
           SELECT 'ends_with_backslash\' AS value; SELECT 3 AS third_value;
           SET SESSION sql_mode=@lazydb_original_mode"#,
        )
        .await
        .unwrap();
    assert_eq!(result.result_sets.len(), 8);
    let conditional = adapter
        .execute("/*!40101 SELECT 42 AS conditional_value */")
        .await
        .unwrap();
    assert_eq!(
        conditional.result_sets[0].rows[0][0],
        CellValue::Integer(42)
    );
    assert_eq!(result.result_sets[2].columns[0].name, "ansi_value");
    assert_eq!(result.result_sets[3].rows[0][0], CellValue::Integer(2));
    assert_eq!(
        result.result_sets[5].rows[0][0].clipboard_text(),
        "ends_with_backslash\\"
    );
    assert_eq!(result.result_sets[6].rows[0][0], CellValue::Integer(3));
    adapter.close().await;
}

#[tokio::test]
async fn tidb_native_catalog_pages_search_preview_ddl_and_operation_boundaries() {
    let Some(url) = tidb_test_url() else {
        return;
    };
    let imported = import_connection_url(&url, Some("TiDB catalog test")).unwrap();
    let profile_id = imported.profile.id;
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let database_name = database.probe().await.unwrap().database;
    assert!(
        !database_name.is_empty(),
        "LAZYDB_TEST_TIDB_URL must select a writable test database"
    );
    let selected = scope(&database_name);
    let schema = CatalogId::new(
        profile_id,
        CatalogKind::Schema,
        [database_name.clone(), database_name.clone()],
    );
    let prefix = format!("lazydb_tidb_{}", Uuid::new_v4().simple());
    let table = format!("{prefix}_a");
    let second = format!("{prefix}_b");
    let view = format!("{prefix}_view");
    let qdb = mysql::quote_identifier(&database_name);
    let qt = format!("{qdb}.{}", mysql::quote_identifier(&table));
    let qb = format!("{qdb}.{}", mysql::quote_identifier(&second));
    let qv = format!("{qdb}.{}", mysql::quote_identifier(&view));
    let tested = AssertUnwindSafe(async {
        database.execute(&format!(
            "CREATE TABLE {qt} (id BIGINT PRIMARY KEY AUTO_INCREMENT, label VARCHAR(64) COMMENT 'native label', \
             amount DECIMAL(10,2) DEFAULT 1.25, generated_value DECIMAL(10,2) AS (amount + 1) VIRTUAL, KEY label_idx (label)) COMMENT='TiDB native table'; \
             CREATE TABLE {qb} (id BIGINT PRIMARY KEY); \
             CREATE VIEW {qv} AS SELECT id,label FROM {qt}; \
             INSERT INTO {qt} (label) VALUES ('Ada'),('Lin')"
        )).await.unwrap();
        let group_request = CatalogRequest { page_size: 10, ..request(profile_id, CatalogTarget::groups(schema.clone()).unwrap(), selected.clone(), None) };
        let groups = database.load_catalog_page(&group_request).await.unwrap();
        groups.validate_for(&group_request).unwrap();
        assert_eq!(groups.group_summaries.iter().map(|s| s.group).collect::<Vec<_>>(), [ObjectGroup::Tables, ObjectGroup::Views]);
        assert!(groups.group_summaries.iter().all(|summary| matches!(summary.object_count, CatalogCount::Exact(count) if count >= 1)));
        for group in [ObjectGroup::Functions, ObjectGroup::Procedures, ObjectGroup::Triggers, ObjectGroup::Sequences] {
            let forbidden = request(profile_id, CatalogTarget::objects(schema.clone(), group).unwrap(), selected.clone(), None);
            assert_eq!(database.load_catalog_page(&forbidden).await.unwrap_err().category, ErrorCategory::Unsupported);
        }
        let target = CatalogTarget::objects(schema.clone(), ObjectGroup::Tables).unwrap();
        let mut cursor = None;
        let mut names = Vec::new();
        loop {
            let page_request = request(profile_id, target.clone(), selected.clone(), cursor);
            let page = database.load_catalog_page(&page_request).await.unwrap();
            page.validate_for(&page_request).unwrap();
            assert!(page.entries.len() <= 1);
            names.extend(page.entries.iter().map(|entry| entry.qualified_name.object.clone()));
            cursor = page.next_cursor;
            if cursor.is_none() { break; }
        }
        assert!(names.contains(&table) && names.contains(&second));
        assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
        let search_request = CatalogSearchRequest {
            connection: identity(profile_id), session_id: 1, generation: 1, query: prefix.clone(),
            scope: selected.clone(), object_scope: CatalogSearchObjectScope::AllObjects, limit: 100,
        };
        let search = database.search_catalog(&search_request).await.unwrap();
        search.validate_for(&search_request).unwrap();
        assert!(search.hits.iter().any(|hit| hit.entry.kind == CatalogKind::Table && hit.entry.qualified_name.object == table));
        assert!(search.hits.iter().any(|hit| hit.entry.kind == CatalogKind::View && hit.entry.qualified_name.object == view));
        assert!(search.hits.iter().all(|hit| !matches!(hit.entry.kind, CatalogKind::Function | CatalogKind::Procedure | CatalogKind::Trigger | CatalogKind::Sequence)));
        let hidden_scope = CatalogScope { databases: CatalogSelection::Selected(vec![]) };
        assert!(database.search_catalog(&CatalogSearchRequest { scope: hidden_scope.clone(), ..search_request }).await.unwrap().hits.is_empty());
        let relation = CatalogId::new(profile_id, CatalogKind::Table, [database_name.clone(), database_name.clone(), table.clone()]);
        let children_request = CatalogRequest { page_size: 100, ..request(profile_id, CatalogTarget::relation_children(relation.clone()).unwrap(), selected.clone(), None) };
        let children = database.load_catalog_page(&children_request).await.unwrap();
        assert!(children.entries.iter().any(|entry| entry.kind == CatalogKind::Index));
        assert!(children.entries.iter().any(|entry| matches!(&entry.metadata, CatalogMetadata::Column(column) if column.ordinal_position == 1)));
        let preview = database.preview_relation_with_scope(&relation, &selected, &RelationPreviewOptions::default(), PageRequest::first(PageSize::Ten)).await.unwrap();
        assert_eq!(preview.result.result_sets[0].rows.len(), 2);
        assert!(database.preview_relation_with_scope(&relation, &hidden_scope, &RelationPreviewOptions::default(), PageRequest::first(PageSize::Ten)).await.is_err());
        for (kind, name, sql_prefix) in [(CatalogKind::Table, &table, "CREATE TABLE"), (CatalogKind::View, &view, "CREATE")] {
            let id = CatalogId::new(profile_id, kind, [database_name.clone(), database_name.clone(), name.clone()]);
            let ddl = database.relation_ddl_with_scope(&id, &selected).await.unwrap();
            let native_sql = database.object_ddl(kind, &database_name, name).await.unwrap().unwrap();
            assert!(native_sql.to_ascii_uppercase().starts_with(sql_prefix));
            assert!(ddl.sql.contains(native_sql.trim_end_matches(';')));
            assert!(ddl.sql.contains(name));
            assert_eq!(ddl.provenance, DdlProvenance::NativeCatalog);
            assert!(!ddl.sql.contains("-- Triggers"));
            assert!(database.relation_ddl_with_scope(&id, &hidden_scope).await.is_err());
        }
        let mut readonly_profile = imported.profile.clone();
        readonly_profile.read_only = true;
        let readonly = MySqlAdapter::connect(&readonly_profile, imported.transient_password.as_ref()).await.unwrap();
        assert!(readonly.discover_catalog_scope().await.unwrap().databases.iter().any(|entry| entry.name == database_name));
        assert!(readonly.load_catalog_page(&children_request).await.is_ok());
        assert!(readonly.relation_ddl_with_scope(&relation, &selected).await.is_ok());
        assert_eq!(readonly.preview_relation_with_scope(&relation, &selected, &RelationPreviewOptions::default(), PageRequest::first(PageSize::Ten)).await.unwrap().result.result_sets[0].rows.len(), 2);
        let empty_options = RelationPreviewOptions { where_clause: Some("1=0".into()), ..RelationPreviewOptions::default() };
        let empty_table = CatalogId::new(profile_id, CatalogKind::Table, [database_name.clone(), database_name.clone(), second.clone()]);
        let view_id = CatalogId::new(profile_id, CatalogKind::View, [database_name.clone(), database_name.clone(), view.clone()]);
        for (id, options, column_count) in [(empty_table, RelationPreviewOptions::default(), 1), (view_id, empty_options, 2)] {
            let preview = readonly.preview_relation_with_scope(&id, &selected, &options, PageRequest::first(PageSize::Ten)).await.unwrap();
            assert_eq!(preview.result.result_sets[0].columns.len(), column_count);
            assert!(preview.result.result_sets[0].rows.is_empty());
        }
        assert_eq!(readonly.load_monitor_snapshot().await.unwrap_err().category, ErrorCategory::Unsupported);
        assert_eq!(readonly.load_monitor_metadata().await.unwrap_err().category, ErrorCategory::Unsupported);
        assert_eq!(readonly.load_process_snapshot().await.unwrap_err().category, ErrorCategory::Unsupported);
        assert_eq!(readonly.list_principals().await.unwrap_err().category, ErrorCategory::Unsupported);
        readonly.close().await;
        assert!(database.list_principals().await.is_err());
        assert!(database.load_monitor_snapshot().await.is_err());
        assert!(database.load_monitor_metadata().await.is_err());
        assert!(database.load_process_snapshot().await.is_err());
        let entry = search.hits.iter().find(|hit| hit.entry.id == relation).unwrap().entry.clone();
        assert!(database.plan_catalog_drop(CatalogDropRequest::new(identity(profile_id), relation, 1), &entry).is_err());
    }).catch_unwind().await;
    database
        .execute(&format!(
            "DROP VIEW IF EXISTS {qv}; DROP TABLE IF EXISTS {qb}; DROP TABLE IF EXISTS {qt}"
        ))
        .await
        .unwrap();
    database.close().await;
    if let Err(panic) = tested {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
async fn tidb_read_only_batches_do_not_advance_sequences() {
    let Some(url) = tidb_test_url() else {
        return;
    };
    let imported = import_connection_url(&url, Some("TiDB sequence policy test")).unwrap();
    let writable = MySqlAdapter::connect(&imported.profile, imported.transient_password.as_ref())
        .await
        .unwrap();
    let suffix = Uuid::new_v4().simple().to_string();
    let sequence = mysql::quote_identifier(&format!("lazydb_tidb_seq_{suffix}"));
    let table_name = format!("lazydb_tidb_seq_rows_{suffix}");
    let table = mysql::quote_identifier(&table_name);
    writable
        .execute(&format!(
            "CREATE SEQUENCE {sequence} START WITH 1 NOCACHE; \
             CREATE TABLE {table} (id INT PRIMARY KEY); INSERT INTO {table} VALUES (1)"
        ))
        .await
        .unwrap();
    let tested = AssertUnwindSafe(async {
        let mut profile = imported.profile.clone();
        profile.read_only = true;
        let readonly = MySqlAdapter::connect(&profile, imported.transient_password.as_ref())
            .await
            .unwrap();
        for sql in [
            format!("SELECT NEXTVAL({sequence})"),
            format!("SELECT SETVAL({sequence}, 100)"),
            format!("SELECT NEXTVAL({sequence}); SELECT SETVAL({sequence}, 100)"),
            format!("SELECT (SELECT NEXTVAL({sequence}))"),
        ] {
            let result = readonly.execute(&sql).await;
            assert!(
                matches!(&result, Err(error) if error.category == ErrorCategory::Unsupported),
                "read-only sequence statement was not blocked: {result:?}"
            );
        }
        let database_name = writable.probe().await.unwrap().database;
        let relation = CatalogId::new(
            profile.id,
            CatalogKind::Table,
            [database_name.clone(), database_name, table_name.clone()],
        );
        for options in [
            RelationPreviewOptions {
                where_clause: Some(format!("id = NEXTVAL({sequence})")),
                ..RelationPreviewOptions::default()
            },
            RelationPreviewOptions {
                order_by_clause: Some(format!("SETVAL({sequence}, 100)")),
                ..RelationPreviewOptions::default()
            },
        ] {
            let result = readonly
                .preview_relation(&relation, &options, PageRequest::first(PageSize::Ten))
                .await;
            assert!(
                matches!(&result, Err(error) if error.category == ErrorCategory::Unsupported),
                "read-only preview sequence expression was not blocked: {result:?}"
            );
        }
        readonly.close().await;
        let next = writable
            .execute(&format!("SELECT NEXTVAL({sequence})"))
            .await
            .unwrap();
        assert_eq!(next.result_sets[0].rows[0][0].clipboard_text(), "1");
    })
    .catch_unwind()
    .await;
    writable
        .execute(&format!("DROP TABLE {table}; DROP SEQUENCE {sequence}"))
        .await
        .unwrap();
    writable.close().await;
    if let Err(panic) = tested {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
async fn tidb_read_only_profile_runs_queries_and_blocks_entire_write_batches() {
    let Some(url) = tidb_test_url() else {
        return;
    };
    let mut imported = import_connection_url(&url, Some("TiDB read-only test")).unwrap();
    imported.profile.read_only = true;
    let adapter = MySqlAdapter::connect(&imported.profile, imported.transient_password.as_ref())
        .await
        .unwrap();
    assert_eq!(
        adapter
            .execute("SELECT 1 AS value; SELECT 2 AS next_value")
            .await
            .unwrap()
            .result_sets
            .len(),
        2
    );
    for sql in [
        "DELETE FROM lazydb_nonexistent",
        "SELECT 1; DELETE FROM lazydb_nonexistent",
        "BEGIN",
        "SET @lazydb_value=1",
    ] {
        let error = adapter.execute(sql).await.unwrap_err();
        assert_eq!(error.category, ErrorCategory::Unsupported);
        assert!(error.message.contains("read-only"));
    }
    assert!(adapter.discover_catalog_scope().await.is_ok());
    adapter.close().await;
}
