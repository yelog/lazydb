use futures_util::FutureExt;
use lazydb::{
    db::{
        DatabaseConnection, ErrorCategory,
        catalog::{
            CatalogCompleteness, CatalogCount, CatalogEntry, CatalogId, CatalogKind,
            CatalogMetadata, CatalogRequest, CatalogRequestKey, CatalogSearchRequest,
            CatalogTarget, DdlProvenance, IndexMetadata, OptionalMetadata, QualifiedName,
        },
        catalog_drop::CatalogDropRequest,
        mssql::{self, MsSqlAdapter},
        value::CellValue,
    },
    identity::ConnectionIdentity,
    profile::{
        CatalogScope, CatalogSelection, ConnectionProfile, ConnectionUrlFormat, CredentialPolicy,
        DatabaseKind, DatabaseScope, Environment, ProfileAccess, SslMode, import_connection_url,
    },
};
use secrecy::{ExposeSecret, SecretString};
use std::panic::AssertUnwindSafe;
use uuid::Uuid;

fn assert_adapter_traits<T: Clone + std::fmt::Debug + Send + Sync>() {}

#[test]
fn sql_server_adapter_is_clone_and_debug() {
    assert_adapter_traits::<MsSqlAdapter>();
    assert_adapter_traits::<DatabaseConnection>();
}

#[tokio::test]
async fn sql_server_connect_requires_a_password_before_network_io() {
    let profile = ConnectionProfile {
        id: Uuid::nil(),
        name: "SQL Server".to_owned(),
        access: ProfileAccess::Global,
        kind: DatabaseKind::SqlServer,
        url_format: ConnectionUrlFormat::SqlServer,
        host: Some("localhost".to_owned()),
        port: Some(1433),
        user: Some("sa".to_owned()),
        database: Some("app".to_owned()),
        default_schema: Some("dbo".to_owned()),
        sqlite_path: None,
        ssl_mode: SslMode::Prefer,
        credential_policy: CredentialPolicy::Prompt,
        read_only: false,
        environment: Environment::Development,
        catalog_scope: CatalogScope::for_profile(DatabaseKind::SqlServer, "app", Some("dbo")),
    };

    let error = DatabaseConnection::connect(&profile, None)
        .await
        .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Configuration);
    assert_eq!(error.code, None);
    assert_eq!(error.message, "SQL Server profile has no password");
}

#[test]
fn sql_server_support_starts_at_2012() {
    assert!(!mssql::supports_server_version("10.50.6000.34"));
    assert!(mssql::supports_server_version("11.0.2100.60"));
    assert!(mssql::supports_server_version("16.0.1000.6"));
    assert!(!mssql::supports_server_version("unknown"));
}

#[test]
fn sql_server_catalog_drop_quotes_identifier_edge_cases() {
    let profile_id = Uuid::new_v4();
    let id = CatalogId::new(
        profile_id,
        CatalogKind::Table,
        ["db.;\n", "schema].", "table;\r", "1"],
    );
    let entry = CatalogEntry {
        id: id.clone(),
        parent_id: Some(CatalogId::new(
            profile_id,
            CatalogKind::Schema,
            ["db.;\n", "schema]."],
        )),
        kind: CatalogKind::Table,
        native_kind: "table".into(),
        qualified_name: QualifiedName {
            database: Some("db.;\n".into()),
            schema: Some("schema].".into()),
            object: "table;\r".into(),
        },
        comment: OptionalMetadata::Unsupported,
        metadata: Default::default(),
        expandable: false,
        relation_id: None,
    };
    let plan = MsSqlAdapter::plan_catalog_drop(
        CatalogDropRequest::new(
            ConnectionIdentity {
                profile_id,
                generation: 1,
            },
            id,
            1,
        ),
        &entry,
    )
    .unwrap();
    assert_eq!(plan.sql(), "DROP TABLE [db.;\n].[schema]].].[table;\r]");
}

#[tokio::test]
async fn connects_probes_discovers_and_classifies_login_errors_when_configured() {
    let Ok(url) = std::env::var("LAZYDB_TEST_SQLSERVER_URL") else {
        return;
    };
    let imported = import_connection_url(&url, Some("sqlserver-test")).unwrap();
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap_or_else(|error| panic!("SQL Server connection failed: {error}"));

    let server = database.probe().await.unwrap();
    assert_eq!(server.kind, DatabaseKind::SqlServer);
    assert!(!server.version.is_empty());
    assert!(!server.database.is_empty());

    let discovery = database.discover_catalog_scope().await.unwrap();
    assert!(
        discovery
            .databases
            .windows(2)
            .all(|items| items[0].name <= items[1].name)
    );
    assert!(discovery.databases.iter().all(|database| {
        database
            .schemas
            .windows(2)
            .all(|schemas| schemas[0] <= schemas[1])
    }));
    assert!(discovery.databases.iter().all(|database| {
        database
            .schemas
            .iter()
            .all(|schema| !matches!(schema.as_str(), "guest" | "INFORMATION_SCHEMA" | "sys"))
    }));
    database.close().await;

    let wrong_password = SecretString::from("lazydb-intentionally-invalid-password".to_owned());
    let error = DatabaseConnection::connect(&imported.profile, Some(&wrong_password))
        .await
        .expect_err("invalid SQL Server credentials unexpectedly connected");
    assert_eq!(error.category, ErrorCategory::Authentication);
    assert_eq!(error.code.as_deref(), Some("18456"));
    assert!(!error.message.contains(wrong_password.expose_secret()));
}

#[tokio::test]
async fn executes_all_result_sets_and_decodes_sql_server_types_when_configured() {
    let Ok(url) = std::env::var("LAZYDB_TEST_SQLSERVER_URL") else {
        return;
    };
    let imported = import_connection_url(&url, Some("sqlserver-execute-test")).unwrap();
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let outcome = database
        .execute(
            "SELECT CAST(NULL AS int) AS [null_value], CAST(1 AS bit) AS [bit_value], CAST(255 AS tinyint) AS [tiny], CAST(-32768 AS smallint) AS [small], CAST(-2147483648 AS int) AS [regular], CAST(-9223372036854775807 AS bigint) AS [big], CAST(1.25 AS real) AS [real_value], CAST(2.5 AS float) AS [float_value], CAST(N'你好' AS nvarchar(8)) AS [unicode], CAST(0x0001FF AS varbinary(3)) AS [binary_value], CAST(12345678901234567890.123456789012345678 AS decimal(38,18)) AS [exact], CAST(-0.012 AS numeric(4,3)) AS [negative_exact];\nSELECT CAST('c97dbc01-fb45-4384-a194-e39a4560cf4a' AS uniqueidentifier) AS [id], CAST('2026-09-02' AS date) AS [date_value], CAST('12:34:56.1234567' AS time(7)) AS [time_value], CAST('2026-09-02T12:34:56.1234567' AS datetime2(7)) AS [datetime2_value], CAST('2026-09-02T12:34:56.123' AS datetime) AS [datetime_value], CAST('2026-09-02T12:34:00' AS smalldatetime) AS [small_datetime_value], CAST('2026-09-02T12:34:56.1234567+08:00' AS datetimeoffset(7)) AS [offset_value], CAST(N'<root />' AS xml) AS [xml_value], CAST(0.1 AS money) AS [money_value];",
        )
        .await
        .unwrap();

    assert_eq!(outcome.result_sets.len(), 2);
    assert_eq!(outcome.stats.row_count, 2);
    assert!(
        outcome
            .result_sets
            .iter()
            .all(|result| result.affected_rows == 0)
    );
    let scalars = &outcome.result_sets[0].rows[0];
    assert_eq!(scalars[0], CellValue::Null);
    assert_eq!(scalars[1], CellValue::Boolean(true));
    assert_eq!(scalars[2], CellValue::Unsigned(255));
    assert_eq!(scalars[3], CellValue::Integer(-32768));
    assert_eq!(scalars[4], CellValue::Integer(-2147483648));
    assert_eq!(scalars[5], CellValue::Integer(-9223372036854775807));
    assert_eq!(scalars[6], CellValue::Float(1.25));
    assert_eq!(scalars[7], CellValue::Float(2.5));
    assert_eq!(scalars[8], CellValue::Text("你好".to_owned()));
    assert_eq!(scalars[9], CellValue::Bytes(vec![0, 1, 255]));
    assert_eq!(
        scalars[10],
        CellValue::Text("12345678901234567890.123456789012345678".to_owned())
    );
    assert_eq!(scalars[11], CellValue::Text("-0.012".to_owned()));

    let temporal = &outcome.result_sets[1].rows[0];
    assert_eq!(
        temporal[0],
        CellValue::Text("c97dbc01-fb45-4384-a194-e39a4560cf4a".to_owned())
    );
    assert!(matches!(temporal[1], CellValue::Date(_)));
    assert!(matches!(temporal[2], CellValue::Time(_)));
    assert!(matches!(temporal[3], CellValue::DateTime(_)));
    assert!(matches!(temporal[4], CellValue::DateTime(_)));
    assert!(matches!(temporal[5], CellValue::DateTime(_)));
    assert!(matches!(temporal[6], CellValue::Timestamp(_)));
    assert_eq!(temporal[7], CellValue::Text("<root />".to_owned()));
    assert!(matches!(
        &temporal[8],
        CellValue::Unsupported { type_name, .. } if type_name == "money"
    ));

    let empty = database
        .execute("SELECT CAST(1 AS int) AS [value] WHERE 1 = 0")
        .await
        .unwrap();
    assert_eq!(empty.result_sets.len(), 1);
    assert_eq!(empty.result_sets[0].columns[0].name, "value");
    assert!(empty.result_sets[0].rows.is_empty());

    let batches = database
        .execute("SELECT 1 AS [value]\r\nGO\r\nSELECT 2 AS [value]\r\nGO 1")
        .await
        .unwrap();
    assert_eq!(batches.result_sets.len(), 2);
    assert_eq!(batches.stats.row_count, 2);
    assert_eq!(batches.result_sets[0].affected_rows, 0);
    assert_eq!(batches.result_sets[1].affected_rows, 0);

    let repeat_error = database
        .execute("SELECT 1\nGO 2\nSELECT 2")
        .await
        .unwrap_err();
    assert_eq!(
        repeat_error.code.as_deref(),
        Some("sql_server_go_count_unsupported")
    );

    let batch_error = database
        .execute("SELECT 1\nGO\nSELECT * FROM [lazydb_missing_task7_table]")
        .await
        .unwrap_err();
    assert!(
        batch_error
            .message
            .starts_with("SQL Server batch 2 failed:")
    );

    let table = format!("#lazydb_task6_{}", Uuid::new_v4().simple());
    let dml = database
        .execute(&format!(
            "CREATE TABLE {table} ([value] int); INSERT INTO {table} VALUES (1), (2), (3);"
        ))
        .await
        .unwrap();
    assert_eq!(dml.result_sets.last().unwrap().affected_rows, 3);

    let large = database
        .execute(
            "WITH [numbers] AS (SELECT 1 AS [value] UNION ALL SELECT [value] + 1 FROM [numbers] WHERE [value] < 10000) SELECT [value] FROM [numbers] OPTION (MAXRECURSION 10000)",
        )
        .await
        .unwrap();
    assert_eq!(large.stats.row_count, 10_000);
    assert_eq!(large.result_sets[0].rows.len(), 10_000);
    database.close().await;
}

#[tokio::test]
async fn previews_sql_server_relations_with_pagination_filters_order_and_escaped_names() {
    let Ok(url) = std::env::var("LAZYDB_TEST_SQLSERVER_URL") else {
        return;
    };
    let imported = import_connection_url(&url, Some("sqlserver-preview-test")).unwrap();
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let table = format!("lazydb_task8_{}]", Uuid::new_v4().simple());
    let quoted_table = mssql::quote_identifier(&table);
    let values = (1..=25)
        .map(|value| format!("({value}, N'row {value}')"))
        .collect::<Vec<_>>()
        .join(", ");
    database
        .execute(&format!(
            "CREATE TABLE [dbo].{quoted_table} ([id] int NOT NULL, [label] nvarchar(32) NOT NULL); INSERT INTO [dbo].{quoted_table} ([id], [label]) VALUES {values};"
        ))
        .await
        .unwrap();

    let relation = lazydb::db::catalog::CatalogId::new(
        imported.profile.id,
        lazydb::db::catalog::CatalogKind::Table,
        [
            imported.profile.database.as_deref().unwrap(),
            "dbo",
            table.as_str(),
        ],
    );
    let options = lazydb::model::relation::RelationPreviewOptions {
        where_clause: Some("[id] >= 1".to_owned()),
        order_by_clause: Some("[id] DESC".to_owned()),
    };

    let first = database
        .preview_relation(
            &relation,
            &options,
            lazydb::model::pagination::PageRequest::first(lazydb::model::pagination::PageSize::Ten),
        )
        .await
        .unwrap();
    assert_eq!(first.result.result_sets[0].rows.len(), 10);
    assert_eq!(
        first.result.result_sets[0].rows[0][0],
        CellValue::Integer(25)
    );
    assert!(first.pagination.has_next);

    let middle = database
        .preview_relation(
            &relation,
            &options,
            lazydb::model::pagination::PageRequest::at(
                lazydb::model::pagination::PageSize::Ten,
                10,
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        middle.result.result_sets[0].rows[0][0],
        CellValue::Integer(15)
    );

    let last = database
        .preview_relation(
            &relation,
            &options,
            lazydb::model::pagination::PageRequest::last(
                lazydb::model::pagination::PageSize::Ten,
                25,
            ),
        )
        .await
        .unwrap();
    assert_eq!(last.result.result_sets[0].rows.len(), 5);
    assert_eq!(last.result.result_sets[0].rows[0][0], CellValue::Integer(5));
    assert_eq!(
        last.pagination.total,
        lazydb::model::pagination::TotalRows::Exact(25)
    );

    database
        .execute(&format!("DROP TABLE [dbo].{quoted_table}"))
        .await
        .unwrap();
    database.close().await;
}

fn sqlserver_catalog_request(
    profile_id: Uuid,
    target: CatalogTarget,
    scope: CatalogScope,
    request_id: u64,
) -> CatalogRequest {
    CatalogRequest {
        key: CatalogRequestKey {
            connection: ConnectionIdentity {
                profile_id,
                generation: 1,
            },
            catalog_epoch: 1,
            request_id,
            target,
            cursor: None,
        },
        scope,
        page_size: 100,
    }
}

fn sqlserver_selected_scope(database: &str, schemas: &[&str]) -> CatalogScope {
    CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: database.to_owned(),
            schemas: CatalogSelection::Selected(
                schemas.iter().map(|schema| (*schema).to_owned()).collect(),
            ),
        }]),
    }
}

fn sqlserver_literal(value: &str) -> String {
    format!("N'{}'", value.replace('\'', "''"))
}

#[tokio::test]
async fn sql_server_catalog_search_matches_scoped_objects_when_configured() {
    let Ok(url) = std::env::var("LAZYDB_TEST_SQLSERVER_URL") else {
        return;
    };
    let imported = import_connection_url(&url, Some("sqlserver-search-test")).unwrap();
    let profile_id = imported.profile.id;
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let database_name = database.probe().await.unwrap().database;
    let table = format!("lazydb_search_{}", Uuid::new_v4().simple());
    let quoted = mssql::quote_identifier(&table);
    database
        .execute(&format!(
            "CREATE TABLE [dbo].{quoted} ([search_value] int NOT NULL);"
        ))
        .await
        .unwrap();
    let request = CatalogSearchRequest {
        connection: ConnectionIdentity {
            profile_id,
            generation: 1,
        },
        session_id: 1,
        generation: 1,
        query: table.clone(),
        scope: sqlserver_selected_scope(&database_name, &["dbo"]),
        limit: 10,
    };
    let result = database.search_catalog(&request).await.unwrap();
    assert!(result.hits.iter().any(|hit| {
        hit.entry.kind == CatalogKind::Table && hit.entry.qualified_name.object == table
    }));
    assert!(result.hits.iter().all(|hit| {
        hit.entry.id.profile_id() == profile_id
            && hit.entry.qualified_name.database.as_deref() == Some(database_name.as_str())
    }));
    database
        .execute(&format!("DROP TABLE [dbo].{quoted}"))
        .await
        .unwrap();
    database.close().await;
}

#[tokio::test]
async fn sql_server_relation_children_expose_task10_metadata_when_configured() {
    let Ok(url) = std::env::var("LAZYDB_TEST_SQLSERVER_URL") else {
        return;
    };
    let imported = import_connection_url(&url, Some("sqlserver-task10-catalog")).unwrap();
    let profile_id = imported.profile.id;
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let database_name = database.probe().await.unwrap().database;
    let suffix = Uuid::new_v4().simple().to_string();
    let schema_a = format!("lazydb_task10_a_{suffix}");
    let schema_b = format!("lazydb_task10_b_{suffix}");
    let parent = format!("parent_{suffix}");
    let child = format!("child_{suffix}");
    let qa = mssql::quote_identifier(&schema_a);
    let qb = mssql::quote_identifier(&schema_b);
    let qp = mssql::quote_identifier(&parent);
    let qc = mssql::quote_identifier(&child);
    let qdb = mssql::quote_identifier(&database_name);
    let setup = format!(
        "CREATE SCHEMA {qa}; CREATE SCHEMA {qb}; \
         CREATE TABLE {qb}.{qp} ([tenant_id] int NOT NULL, [parent_id] int NOT NULL, CONSTRAINT [{suffix}_parent_pk] PRIMARY KEY ([tenant_id], [parent_id])); \
         CREATE TABLE {qa}.{qc} ([id] int IDENTITY(1,1) NOT NULL, [tenant_id] int NOT NULL, [parent_id] int NOT NULL, [code] nvarchar(40) NOT NULL CONSTRAINT [{suffix}_code_default] DEFAULT N'new', [code_upper] AS (UPPER([code])) PERSISTED, [version] rowversion NOT NULL, CONSTRAINT [{suffix}_child_pk] PRIMARY KEY ([tenant_id], [id]), CONSTRAINT [{suffix}_child_uq] UNIQUE ([tenant_id], [code]), CONSTRAINT [{suffix}_child_fk] FOREIGN KEY ([tenant_id], [parent_id]) REFERENCES {qb}.{qp} ([tenant_id], [parent_id]), CONSTRAINT [{suffix}_child_check] CHECK ([id] >= 0)); \
         CREATE INDEX [{suffix}_child_idx] ON {qa}.{qc} ([code], [parent_id]); \
         CREATE TRIGGER [{suffix}_child_trigger] ON {qa}.{qc} AFTER INSERT AS BEGIN SET NOCOUNT ON; END; \
         EXEC {qdb}.sys.sp_addextendedproperty @name=N'MS_Description', @value=N'child table comment', @level0type=N'SCHEMA', @level0name={schema_a_literal}, @level1type=N'TABLE', @level1name={child_literal}; \
         EXEC {qdb}.sys.sp_addextendedproperty @name=N'MS_Description', @value=N'code column comment', @level0type=N'SCHEMA', @level0name={schema_a_literal}, @level1type=N'TABLE', @level1name={child_literal}, @level2type=N'COLUMN', @level2name=N'code';",
        schema_a_literal = sqlserver_literal(&schema_a),
        child_literal = sqlserver_literal(&child),
    );
    let cleanup = format!(
        "DROP TABLE IF EXISTS {qa}.{qc}; DROP TABLE IF EXISTS {qb}.{qp}; DROP SCHEMA {qa}; DROP SCHEMA {qb};"
    );
    database.execute(&setup).await.unwrap();

    let result = AssertUnwindSafe(async {
        let table_page = database
            .load_catalog_page(&sqlserver_catalog_request(
                profile_id,
                CatalogTarget::objects(
                    CatalogId::new(
                        profile_id,
                        CatalogKind::Schema,
                        [database_name.clone(), schema_a.clone()],
                    ),
                    lazydb::db::catalog::ObjectGroup::Tables,
                )
                .unwrap(),
                sqlserver_selected_scope(&database_name, &[&schema_a, &schema_b]),
                1,
            ))
            .await
            .unwrap();
        let relation = table_page
            .entries
            .iter()
            .find(|entry| entry.qualified_name.object == child)
            .unwrap()
            .id
            .clone();
        assert_eq!(table_page.total_count, CatalogCount::Exact(1));
        assert_eq!(table_page.completeness, CatalogCompleteness::Complete);
        assert_eq!(
            table_page.entries[0].comment,
            OptionalMetadata::Supported(Some("child table comment".to_owned()))
        );

        let page = database
            .load_catalog_page(&sqlserver_catalog_request(
                profile_id,
                CatalogTarget::relation_children(relation.clone()).unwrap(),
                sqlserver_selected_scope(&database_name, &[&schema_a, &schema_b]),
                2,
            ))
            .await
            .unwrap();
        assert_eq!(page.completeness, CatalogCompleteness::Complete);
        assert_eq!(
            page.entries
                .iter()
                .filter(|e| e.kind == CatalogKind::Column)
                .count(),
            6
        );
        assert_eq!(
            page.entries
                .iter()
                .filter(|e| e.kind == CatalogKind::Index)
                .count(),
            3
        );
        assert!(
            page.entries
                .iter()
                .any(|e| e.kind == CatalogKind::PrimaryKey)
        );
        assert!(
            page.entries
                .iter()
                .any(|e| e.kind == CatalogKind::UniqueConstraint)
        );
        assert!(
            page.entries
                .iter()
                .any(|e| e.kind == CatalogKind::ForeignKey)
        );
        assert!(
            page.entries
                .iter()
                .any(|e| e.kind == CatalogKind::CheckConstraint)
        );
        let trigger = page
            .entries
            .iter()
            .find(|e| e.kind == CatalogKind::Trigger)
            .unwrap();
        assert_eq!(trigger.relation_id.as_ref(), Some(&relation));
        assert_eq!(trigger.parent_id.as_ref(), Some(&relation));
        let names = page
            .entries
            .iter()
            .map(|e| e.id.clone())
            .collect::<Vec<_>>();
        let repeated = database
            .load_catalog_page(&sqlserver_catalog_request(
                profile_id,
                CatalogTarget::relation_children(relation).unwrap(),
                sqlserver_selected_scope(&database_name, &[&schema_a, &schema_b]),
                3,
            ))
            .await
            .unwrap();
        assert_eq!(
            names,
            repeated
                .entries
                .iter()
                .map(|e| e.id.clone())
                .collect::<Vec<_>>()
        );

        let columns = page
            .entries
            .iter()
            .filter_map(|entry| match &entry.metadata {
                CatalogMetadata::Column(metadata) => {
                    Some((entry.qualified_name.object.as_str(), (metadata, entry)))
                }
                _ => None,
            })
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(
            columns["id"].0.identity,
            OptionalMetadata::Supported(Some(true))
        );
        assert!(matches!(
            columns["code"].0.default_expression,
            OptionalMetadata::Supported(Some(_))
        ));
        assert!(matches!(
            columns["code_upper"].0.generated_expression,
            OptionalMetadata::Supported(Some(_))
        ));
        assert_eq!(
            columns["version"].0.hidden,
            OptionalMetadata::Supported(Some(true))
        );
        assert_eq!(
            columns["code"].1.comment,
            OptionalMetadata::Supported(Some("code column comment".to_owned()))
        );
        let index = page
            .entries
            .iter()
            .find(|entry| entry.qualified_name.object == format!("{suffix}_child_idx"))
            .unwrap();
        assert_eq!(
            index.metadata,
            CatalogMetadata::Index(IndexMetadata {
                columns: vec!["code".to_owned(), "parent_id".to_owned()],
                unique: false
            })
        );
    })
    .catch_unwind()
    .await;
    database.execute(&cleanup).await.unwrap();
    database.close().await;
    result.unwrap();
}

#[tokio::test]
async fn sql_server_task12_ddl_golden_objects_when_configured() {
    let Ok(url) = std::env::var("LAZYDB_TEST_SQLSERVER_URL") else {
        return;
    };
    let imported = import_connection_url(&url, Some("sqlserver-task12-ddl")).unwrap();
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let database_name = database.probe().await.unwrap().database;
    let suffix = Uuid::new_v4().simple().to_string();
    let table = format!("lazydb_task12_table_{suffix}");
    let view = format!("lazydb_task12_view_{suffix}");
    let function = format!("lazydb_task12_function_{suffix}");
    let procedure = format!("lazydb_task12_procedure_{suffix}");
    let trigger = format!("lazydb_task12_trigger_{suffix}");
    let qt = mssql::quote_identifier(&table);
    let qv = mssql::quote_identifier(&view);
    let qf = mssql::quote_identifier(&function);
    let qp = mssql::quote_identifier(&procedure);
    let qtr = mssql::quote_identifier(&trigger);
    let setup = format!(
        "CREATE TABLE [dbo].{qt} ([id] int NOT NULL, [label] nvarchar(32) NULL, CONSTRAINT {suffix}_pk PRIMARY KEY ([id])); CREATE INDEX {suffix}_idx ON [dbo].{qt} ([label]); CREATE VIEW [dbo].{qv} AS SELECT [id], [label] FROM [dbo].{qt}; GO CREATE FUNCTION [dbo].{qf} (@value int) RETURNS int AS BEGIN RETURN @value + 1; END; GO CREATE PROCEDURE [dbo].{qp} AS SELECT [id] FROM [dbo].{qt}; GO CREATE TRIGGER [dbo].{qtr} ON [dbo].{qt} AFTER INSERT AS BEGIN SET NOCOUNT ON; END;",
    );
    let cleanup = format!(
        "DROP TRIGGER IF EXISTS [dbo].{qtr}; DROP PROCEDURE IF EXISTS [dbo].{qp}; DROP FUNCTION IF EXISTS [dbo].{qf}; DROP VIEW IF EXISTS [dbo].{qv}; DROP TABLE IF EXISTS [dbo].{qt};"
    );
    database.execute(&setup).await.unwrap();
    let result = AssertUnwindSafe(async {
        for (kind, name, expected) in [
            (CatalogKind::View, view.as_str(), "CREATE VIEW"),
            (CatalogKind::Function, function.as_str(), "CREATE FUNCTION"),
            (
                CatalogKind::Procedure,
                procedure.as_str(),
                "CREATE PROCEDURE",
            ),
            (CatalogKind::Trigger, trigger.as_str(), "CREATE TRIGGER"),
        ] {
            let ddl = database
                .object_ddl(kind, "dbo", name)
                .await
                .unwrap()
                .unwrap();
            assert!(ddl.to_ascii_uppercase().contains(expected), "{ddl}");
        }
        let page = database
            .load_catalog_page(&sqlserver_catalog_request(
                imported.profile.id,
                CatalogTarget::objects(
                    CatalogId::new(
                        imported.profile.id,
                        CatalogKind::Schema,
                        [database_name.clone(), "dbo".to_owned()],
                    ),
                    lazydb::db::catalog::ObjectGroup::Tables,
                )
                .unwrap(),
                sqlserver_selected_scope(&database_name, &["dbo"]),
                12,
            ))
            .await
            .unwrap();
        let relation = page
            .entries
            .iter()
            .find(|entry| entry.qualified_name.object == table)
            .unwrap();
        let ddl = database.relation_ddl(&relation.id).await.unwrap();
        assert_eq!(ddl.provenance, DdlProvenance::AdapterGenerated);
        assert!(ddl.sql.contains("CREATE TABLE [dbo]"));
        assert!(ddl.sql.contains("CREATE INDEX"));
        assert!(ddl.sql.contains("CREATE TRIGGER"));
    })
    .catch_unwind()
    .await;
    database.execute(&cleanup).await.unwrap();
    database.close().await;
    result.unwrap();
}
