use std::{collections::HashSet, panic::AssertUnwindSafe};

use futures_util::FutureExt;
use lazydb::{
    db::{
        DatabaseConnection, ErrorCategory,
        catalog::{
            CatalogCapabilities, CatalogCompleteness, CatalogCount, CatalogCursor, CatalogEntry,
            CatalogId, CatalogKind, CatalogMetadata, CatalogRequest, CatalogRequestKey,
            CatalogTarget, ColumnMetadata, ColumnMetadataCapabilities, ConstraintMembership,
            ConstraintMetadata, IndexMetadata, NamespaceModel, ObjectGroup, OptionalMetadata,
        },
        postgres::{self, PostgresAdapter},
        value::CellValue,
    },
    identity::ConnectionIdentity,
    profile::{CatalogScope, CatalogSelection, DatabaseScope, import_connection_url},
};
use uuid::Uuid;

#[test]
fn postgres_catalog_capabilities_are_truthful_before_lazy_pages() {
    assert_eq!(
        PostgresAdapter::catalog_capabilities(),
        CatalogCapabilities {
            namespace_model: NamespaceModel::DatabaseAndSchema,
            top_level_groups: vec![
                ObjectGroup::Tables,
                ObjectGroup::Views,
                ObjectGroup::MaterializedViews,
                ObjectGroup::Sequences,
                ObjectGroup::Functions,
                ObjectGroup::Procedures,
                ObjectGroup::Types,
            ],
            column_metadata: ColumnMetadataCapabilities {
                type_family: true,
                default_expression: true,
                identity: true,
                generated_expression: true,
                numeric_precision_and_scale: false,
                character_length: true,
                collation: true,
                comment: true,
                ..ColumnMetadataCapabilities::default()
            },
            supports_lazy_children: true,
        }
    );
}

#[test]
fn quotes_postgres_identifiers_and_uses_native_catalogs() {
    assert_eq!(postgres::quote_identifier("odd\"name"), "\"odd\"\"name\"");
    assert!(postgres::CATALOG_TABLES_SQL.contains("information_schema.tables"));
    assert!(postgres::CATALOG_INDEXES_SQL.contains("pg_indexes"));
    assert!(postgres::CATALOG_ROUTINES_SQL.contains("pg_proc"));
    assert!(postgres::CATALOG_ROUTINES_SQL.contains("prokind::text"));
    for sql in [
        postgres::CATALOG_TABLES_SQL,
        postgres::CATALOG_SCHEMAS_SQL,
        postgres::CATALOG_COLUMNS_SQL,
        postgres::CATALOG_INDEXES_SQL,
        postgres::CATALOG_ROUTINES_SQL,
    ] {
        assert!(sql.contains("NOT LIKE 'pg\\_%' ESCAPE '\\'"));
        assert!(!sql.contains("NOT LIKE 'pg_%'"));
    }
}

#[test]
fn postgres_version_support_starts_at_twelve() {
    assert!(!postgres::supports_server_version(119_999));
    assert!(postgres::supports_server_version(120_000));
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
    let version = database
        .execute("SELECT current_setting('server_version_num')::int")
        .await
        .unwrap();
    assert!(matches!(
        version.result_sets.last().unwrap().rows[0][0],
        CellValue::Integer(value) if value >= 120_000
    ));
    assert_eq!(
        database.catalog_capabilities(),
        PostgresAdapter::catalog_capabilities()
    );
    let discovery = database.discover_catalog_scope().await.unwrap();
    assert_eq!(discovery.databases.len(), 1);
    assert_eq!(discovery.databases[0].name, server.database);
    assert!(
        discovery.databases[0]
            .schemas
            .iter()
            .all(|schema| schema != "information_schema" && !schema.starts_with("pg_"))
    );
    assert!(
        discovery.databases[0]
            .schemas
            .windows(2)
            .all(|schemas| schemas[0] <= schemas[1])
    );
    let outcome = database
        .execute("SELECT 1::bigint AS n, true AS ok, 'Ada'::text AS name, NULL::text AS missing")
        .await
        .unwrap();
    let row = &outcome.result_sets.last().unwrap().rows[0];
    assert_eq!(row[0], CellValue::Integer(1));
    assert_eq!(row[1], CellValue::Boolean(true));
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
            "CREATE TEMP TABLE lazydb_task14_affected (value INTEGER); \
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
    database.close().await;
}

#[tokio::test]
async fn materialized_view_structure_reports_truthful_ddl_and_native_kind_when_configured() {
    let Ok(url) = std::env::var("LAZYDB_TEST_POSTGRES_URL") else {
        return;
    };
    let imported = import_connection_url(&url, Some("postgres-materialized-view")).unwrap();
    let profile_id = imported.profile.id;
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let database_name = database.probe().await.unwrap().database;
    let schema = format!("lazydb_mv_{}", Uuid::new_v4().simple());
    let name = "materialized_view";
    let qualified_schema = postgres::quote_identifier(&schema);
    database
        .execute(&format!(
            "CREATE SCHEMA {qualified_schema}; CREATE TABLE {qualified_schema}.source (id integer); CREATE MATERIALIZED VIEW {qualified_schema}.{name} AS SELECT id FROM {qualified_schema}.source;"
        ))
        .await
        .unwrap();
    let oid = match database
        .execute(&format!(
            "SELECT oid::bigint FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = '{schema}' AND c.relname = '{name}'"
        ))
        .await
        .unwrap()
        .result_sets
        .last()
        .unwrap()
        .rows[0][0]
        .clone()
    {
        CellValue::Integer(oid) => oid,
        value => panic!("unexpected materialized view oid: {value:?}"),
    };
    let relation = CatalogId::new(
        profile_id,
        CatalogKind::MaterializedView,
        [
            database_name.clone(),
            schema.clone(),
            name.to_owned(),
            oid.to_string(),
        ],
    );
    let mut profile = imported.profile;
    profile.catalog_scope = selected_scope(&database_name, &[&schema]);
    database.close().await;
    let database = DatabaseConnection::connect(&profile, imported.transient_password.as_ref())
        .await
        .unwrap();
    let structure = database.relation_structure(&relation).await.unwrap();
    assert_eq!(structure.relation.native_kind, "materialized_view");
    assert_eq!(
        structure.ddl.provenance,
        lazydb::db::catalog::DdlProvenance::AdapterGenerated
    );
    assert!(structure.ddl.sql.is_none());
    database
        .execute(&format!("DROP MATERIALIZED VIEW {qualified_schema}.{name}; DROP SCHEMA {qualified_schema} CASCADE;"))
        .await
        .unwrap();
    database.close().await;
}

fn selected_scope(database: &str, schemas: &[&str]) -> CatalogScope {
    CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: database.to_owned(),
            schemas: CatalogSelection::Selected(
                schemas.iter().map(|schema| (*schema).to_owned()).collect(),
            ),
        }]),
    }
}

fn catalog_request(
    profile_id: Uuid,
    target: CatalogTarget,
    scope: CatalogScope,
    page_size: usize,
    cursor: Option<CatalogCursor>,
    request_id: u64,
) -> CatalogRequest {
    CatalogRequest {
        key: CatalogRequestKey {
            connection: ConnectionIdentity {
                profile_id,
                generation: 7,
            },
            catalog_epoch: 3,
            request_id,
            target,
            cursor,
        },
        scope,
        page_size,
    }
}

#[tokio::test]
async fn catalog_page_exposes_scoped_postgres_objects_and_rich_metadata_when_configured() {
    let Ok(url) = std::env::var("LAZYDB_TEST_POSTGRES_URL") else {
        return;
    };
    let imported = import_connection_url(&url, Some("postgres-catalog-pages")).unwrap();
    let profile_id = imported.profile.id;
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let database_name = database.probe().await.unwrap().database;
    let suffix = Uuid::new_v4().simple().to_string();
    let schema = format!("LazyDbCase_{suffix}");
    let excluded_schema = format!("lazydb_excluded_{suffix}");
    let pgx_schema = format!("pgx_{suffix}");
    let quoted_schema = postgres::quote_identifier(&schema);
    let quoted_excluded = postgres::quote_identifier(&excluded_schema);
    let quoted_pgx = postgres::quote_identifier(&pgx_schema);

    let result = AssertUnwindSafe(async {
        database
        .execute(&format!(
            r#"
            CREATE SCHEMA {quoted_schema};
            CREATE SCHEMA {quoted_excluded};
            CREATE SCHEMA {quoted_pgx};
            CREATE TYPE {quoted_schema}.status_kind AS ENUM ('new', 'done');
            CREATE DOMAIN {quoted_schema}.positive_int AS integer CHECK (VALUE > 0);
            CREATE TABLE {quoted_schema}.parent (
                tenant_id integer NOT NULL,
                parent_id integer NOT NULL,
                code text NOT NULL,
                CONSTRAINT parent_pk PRIMARY KEY (tenant_id, parent_id)
            );
            CREATE TABLE {quoted_schema}.child (
                id bigint GENERATED BY DEFAULT AS IDENTITY,
                tenant_id integer NOT NULL,
                owner_id integer NOT NULL,
                code varchar(40) NOT NULL DEFAULT 'new',
                code_upper text GENERATED ALWAYS AS (upper(code)) STORED,
                CONSTRAINT child_pk PRIMARY KEY (tenant_id, id),
                CONSTRAINT child_tenant_code_key UNIQUE (tenant_id, code),
                CONSTRAINT child_parent_fk FOREIGN KEY (tenant_id, owner_id)
                    REFERENCES {quoted_schema}.parent (tenant_id, parent_id),
                CONSTRAINT child_owner_check CHECK (owner_id > 0)
            );
            COMMENT ON TABLE {quoted_schema}.child IS 'child table comment';
            COMMENT ON COLUMN {quoted_schema}.child.code IS 'code column comment';
            CREATE INDEX child_owner_code_idx ON {quoted_schema}.child (owner_id, code);
            CREATE VIEW {quoted_schema}.child_view AS SELECT tenant_id, id, code FROM {quoted_schema}.child;
            CREATE MATERIALIZED VIEW {quoted_schema}.child_snapshot AS SELECT tenant_id, id FROM {quoted_schema}.child;
            CREATE SEQUENCE {quoted_schema}.manual_sequence;
            CREATE FUNCTION {quoted_schema}.do_work(value integer) RETURNS integer
                LANGUAGE sql IMMUTABLE AS 'SELECT value + 1';
            CREATE FUNCTION {quoted_schema}.do_work(value text) RETURNS text
                LANGUAGE sql IMMUTABLE AS 'SELECT value';
            CREATE PROCEDURE {quoted_schema}.run_work() LANGUAGE sql AS 'SELECT 1';
            CREATE TABLE {quoted_excluded}.hidden_table (id integer PRIMARY KEY);
            "#
        ))
        .await
        .unwrap();
        let discovery = database.discover_catalog_scope().await.unwrap();
        assert!(discovery.databases[0].schemas.contains(&pgx_schema));
        let scope = selected_scope(&database_name, &[&schema]);
        let database_id = CatalogId::new(profile_id, CatalogKind::Database, [database_name.clone()]);
        let schema_id = CatalogId::new(
            profile_id,
            CatalogKind::Schema,
            [database_name.clone(), schema.clone()],
        );

        let databases = database
            .load_catalog_page(&catalog_request(
                profile_id,
                CatalogTarget::Databases,
                scope.clone(),
                10,
                None,
                1,
            ))
            .await
            .unwrap();
        assert_eq!(databases.total_count, CatalogCount::Exact(1));
        assert_eq!(databases.entries.len(), 1);
        assert_eq!(databases.entries[0].id, database_id);
        assert!(matches!(databases.entries[0].comment, OptionalMetadata::Supported(_)));

        let schemas = database
            .load_catalog_page(&catalog_request(
                profile_id,
                CatalogTarget::schemas(database_id.clone()).unwrap(),
                scope.clone(),
                10,
                None,
                2,
            ))
            .await
            .unwrap();
        assert_eq!(schemas.total_count, CatalogCount::Exact(1));
        assert_eq!(schemas.entries.len(), 1);
        assert_eq!(schemas.entries[0].id, schema_id);
        assert_ne!(schemas.entries[0].qualified_name.object, excluded_schema);

        let groups = database
            .load_catalog_page(&catalog_request(
                profile_id,
                CatalogTarget::groups(schema_id.clone()).unwrap(),
                scope.clone(),
                10,
                None,
                3,
            ))
            .await
            .unwrap();
        assert_eq!(groups.total_count, CatalogCount::Exact(7));
        let counts = groups
            .group_summaries
            .iter()
            .map(|summary| (summary.group, summary.object_count))
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(counts[&ObjectGroup::Tables], CatalogCount::Exact(2));
        assert_eq!(counts[&ObjectGroup::Views], CatalogCount::Exact(1));
        assert_eq!(counts[&ObjectGroup::MaterializedViews], CatalogCount::Exact(1));
        assert_eq!(counts[&ObjectGroup::Sequences], CatalogCount::Exact(2));
        assert_eq!(counts[&ObjectGroup::Functions], CatalogCount::Exact(2));
        assert_eq!(counts[&ObjectGroup::Procedures], CatalogCount::Exact(1));
        assert_eq!(counts[&ObjectGroup::Types], CatalogCount::Exact(2));

        let table_target = CatalogTarget::objects(schema_id.clone(), ObjectGroup::Tables).unwrap();
        let (first_names, first_ids, first_completeness) =
            collect_object_pages(&database, profile_id, &scope, &table_target, 10, 2).await;
        let (second_names, second_ids, _) =
            collect_object_pages(&database, profile_id, &scope, &table_target, 20, 2).await;
        assert_eq!(first_names, ["child", "parent"]);
        assert_eq!(first_names, second_names);
        assert_eq!(first_ids, second_ids);
        assert_eq!(first_ids.iter().collect::<HashSet<_>>().len(), 2);
        assert_eq!(first_completeness, [CatalogCompleteness::Partial, CatalogCompleteness::Complete]);
        assert!(first_ids.iter().all(|id| id.native_path.len() == 4));

        let function_target =
            CatalogTarget::objects(schema_id.clone(), ObjectGroup::Functions).unwrap();
        let (function_names, function_ids, function_completeness) = collect_object_pages(
            &database,
            profile_id,
            &scope,
            &function_target,
            25,
            2,
        )
        .await;
        let (repeated_function_names, repeated_function_ids, _) = collect_object_pages(
            &database,
            profile_id,
            &scope,
            &function_target,
            28,
            2,
        )
        .await;
        assert_eq!(function_names, ["do_work", "do_work"]);
        assert_eq!(function_names, repeated_function_names);
        assert_eq!(function_ids, repeated_function_ids);
        assert_eq!(function_ids.iter().collect::<HashSet<_>>().len(), 2);
        assert_eq!(
            function_completeness,
            [CatalogCompleteness::Partial, CatalogCompleteness::Complete]
        );

        for (group, expected_kind, expected_count) in [
            (ObjectGroup::Views, CatalogKind::View, 1),
            (ObjectGroup::MaterializedViews, CatalogKind::MaterializedView, 1),
            (ObjectGroup::Sequences, CatalogKind::Sequence, 2),
            (ObjectGroup::Functions, CatalogKind::Function, 2),
            (ObjectGroup::Procedures, CatalogKind::Procedure, 1),
            (ObjectGroup::Types, CatalogKind::Type, 2),
        ] {
            let page = database
                .load_catalog_page(&catalog_request(
                    profile_id,
                    CatalogTarget::objects(schema_id.clone(), group).unwrap(),
                    scope.clone(),
                    10,
                    None,
                    30 + expected_count,
                ))
                .await
                .unwrap();
            assert_eq!(page.total_count, CatalogCount::Exact(expected_count));
            assert!(page.entries.iter().all(|entry| entry.kind == expected_kind));
            assert!(page.entries.iter().all(|entry| entry.id.native_path.len() == 4));
        }

        let child = first_ids
            .iter()
            .find(|id| id.native_path[2] == "child")
            .unwrap()
            .clone();
        let children_target = CatalogTarget::relation_children(child.clone()).unwrap();
        let children = database
            .load_catalog_page(&catalog_request(
                profile_id,
                children_target.clone(),
                scope.clone(),
                100,
                None,
                50,
            ))
            .await
            .unwrap();
        assert_eq!(children.total_count, CatalogCount::Exact(12));
        assert_eq!(children.entries.len(), 12);
        assert!(children.entries.iter().all(|entry| entry.id.native_path.starts_with(&child.native_path)));
        let columns = children
            .entries
            .iter()
            .filter_map(|entry| match &entry.metadata {
                CatalogMetadata::Column(metadata) => Some((entry.qualified_name.object.as_str(), (entry, metadata))),
                _ => None,
            })
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(columns.len(), 5);
        let (code_entry, code) = columns["code"];
        assert_eq!(code.native_type, "character varying(40)");
        assert!(!code.nullable);
        let OptionalMetadata::Supported(Some(default_expression)) = &code.default_expression else {
            panic!("code default must be supported and present")
        };
        assert!(default_expression.contains("new"));
        assert_eq!(code.identity, OptionalMetadata::Supported(Some(false)));
        assert_eq!(code.generated_expression, OptionalMetadata::Supported(None));
        assert_eq!(code.numeric_precision, OptionalMetadata::Unsupported);
        assert_eq!(code.numeric_scale, OptionalMetadata::Unsupported);
        assert!(matches!(
            &code.collation,
            OptionalMetadata::Supported(Some(collation)) if !collation.is_empty() && collation.contains('.')
        ));
        assert_eq!(code_entry.comment, OptionalMetadata::Supported(Some("code column comment".to_owned())));
        let (_, generated) = columns["code_upper"];
        let OptionalMetadata::Supported(Some(generated_expression)) =
            &generated.generated_expression
        else {
            panic!("generated expression must be supported and present")
        };
        assert!(generated_expression.contains("upper"));
        assert!(generated_expression.contains("code"));
        assert_eq!(generated.default_expression, OptionalMetadata::Supported(None));
        let (_, identity) = columns["id"];
        assert_eq!(identity.identity, OptionalMetadata::Supported(Some(true)));

        let indexes = children.entries.iter().filter_map(|entry| match &entry.metadata {
            CatalogMetadata::Index(metadata) => Some((entry, metadata)),
            _ => None,
        }).collect::<Vec<_>>();
        assert_eq!(indexes.len(), 3, "one entry per native PostgreSQL index");
        let (_, composite_index) = indexes.iter().find(|(entry, _)| entry.qualified_name.object == "child_owner_code_idx").unwrap();
        assert_eq!(**composite_index, IndexMetadata { columns: vec!["owner_id".to_owned(), "code".to_owned()], unique: false });

        let primary = one_constraint(&children.entries, CatalogKind::PrimaryKey);
        let unique = one_constraint(&children.entries, CatalogKind::UniqueConstraint);
        let foreign = one_constraint(&children.entries, CatalogKind::ForeignKey);
        let check = one_constraint(&children.entries, CatalogKind::CheckConstraint);
        assert_eq!(primary.metadata, CatalogMetadata::Constraint(ConstraintMetadata::PrimaryKey { columns: vec!["tenant_id".to_owned(), "id".to_owned()] }));
        assert_eq!(unique.metadata, CatalogMetadata::Constraint(ConstraintMetadata::Unique { columns: vec!["tenant_id".to_owned(), "code".to_owned()] }));
        assert_eq!(foreign.metadata, CatalogMetadata::Constraint(ConstraintMetadata::ForeignKey {
            columns: vec!["tenant_id".to_owned(), "owner_id".to_owned()],
            referenced_relation: lazydb::db::catalog::QualifiedName { database: Some(database_name.clone()), schema: Some(schema.clone()), object: "parent".to_owned() },
            referenced_columns: vec!["tenant_id".to_owned(), "parent_id".to_owned()],
        }));
        assert!(matches!(&check.metadata, CatalogMetadata::Constraint(ConstraintMetadata::Check { expression }) if expression.contains("owner_id > 0")));
        assert_membership(columns["tenant_id"].1, &primary.id, 1);
        assert_membership(columns["tenant_id"].1, &unique.id, 1);
        assert_membership(columns["tenant_id"].1, &foreign.id, 1);
        assert_membership(columns["id"].1, &primary.id, 2);
        assert_membership(columns["code"].1, &unique.id, 2);
        assert_membership(columns["owner_id"].1, &foreign.id, 2);

        let repeated = database.load_catalog_page(&catalog_request(profile_id, children_target, scope.clone(), 100, None, 51)).await.unwrap();
        assert_eq!(children.entries.iter().map(|entry| &entry.id).collect::<Vec<_>>(), repeated.entries.iter().map(|entry| &entry.id).collect::<Vec<_>>());

        let wrong_case = schema.to_ascii_lowercase();
        let wrong_schema = CatalogId::new(profile_id, CatalogKind::Schema, [database_name.clone(), wrong_case.clone()]);
        let error = database.load_catalog_page(&catalog_request(
            profile_id,
            CatalogTarget::groups(wrong_schema).unwrap(),
            selected_scope(&database_name, &[&wrong_case]),
            10,
            None,
            60,
        )).await.unwrap_err();
        assert_eq!(error.code.as_deref(), Some("catalog_target_not_found"));

        let missing_relation = CatalogId::new(profile_id, CatalogKind::Table, [database_name.clone(), schema.clone(), "missing".to_owned(), "0".to_owned()]);
        let error = database.load_catalog_page(&catalog_request(profile_id, CatalogTarget::relation_children(missing_relation).unwrap(), scope.clone(), 10, None, 61)).await.unwrap_err();
        assert_eq!(error.code.as_deref(), Some("catalog_target_not_found"));

        let unsupported = database.load_catalog_page(&catalog_request(profile_id, CatalogTarget::objects(schema_id, ObjectGroup::Triggers).unwrap(), scope.clone(), 10, None, 62)).await.unwrap_err();
        assert_eq!(unsupported.category, ErrorCategory::Unsupported);
        let malformed = database.load_catalog_page(&catalog_request(profile_id, CatalogTarget::Databases, scope, 0, None, 63)).await.unwrap_err();
        assert_eq!(malformed.category, ErrorCategory::Configuration);
        assert_eq!(malformed.code.as_deref(), Some("invalid_catalog_request"));
    })
    .catch_unwind()
    .await;

    let mut cleanup_errors = Vec::new();
    for schema in [&quoted_schema, &quoted_excluded, &quoted_pgx] {
        if let Err(error) = database
            .execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
            .await
        {
            cleanup_errors.push(error);
        }
    }
    database.close().await;
    if let Err(panic) = result {
        for error in cleanup_errors {
            eprintln!("PostgreSQL fixture cleanup failed after body panic: {error}");
        }
        std::panic::resume_unwind(panic);
    }
    assert!(
        cleanup_errors.is_empty(),
        "PostgreSQL fixture cleanup failed: {cleanup_errors:?}"
    );
}

async fn collect_object_pages(
    database: &DatabaseConnection,
    profile_id: Uuid,
    scope: &CatalogScope,
    target: &CatalogTarget,
    first_request_id: u64,
    expected_total: u64,
) -> (Vec<String>, Vec<CatalogId>, Vec<CatalogCompleteness>) {
    let mut cursor = None;
    let mut names = Vec::new();
    let mut ids = Vec::new();
    let mut completeness = Vec::new();
    loop {
        let page = database
            .load_catalog_page(&catalog_request(
                profile_id,
                target.clone(),
                scope.clone(),
                1,
                cursor,
                first_request_id + names.len() as u64,
            ))
            .await
            .unwrap();
        assert_eq!(page.total_count, CatalogCount::Exact(expected_total));
        assert_eq!(page.entries.len(), 1);
        names.push(page.entries[0].qualified_name.object.clone());
        ids.push(page.entries[0].id.clone());
        completeness.push(page.completeness);
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    (names, ids, completeness)
}

fn one_constraint(entries: &[CatalogEntry], kind: CatalogKind) -> &CatalogEntry {
    let matches = entries
        .iter()
        .filter(|entry| entry.kind == kind)
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "expected one grouped {kind:?}");
    matches[0]
}

fn assert_membership(column: &ColumnMetadata, constraint_id: &CatalogId, ordinal_position: u32) {
    assert!(
        column
            .constraint_memberships
            .contains(&ConstraintMembership {
                constraint_id: constraint_id.clone(),
                ordinal_position,
            })
    );
}
