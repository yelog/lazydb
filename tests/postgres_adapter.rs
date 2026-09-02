use std::{collections::HashSet, panic::AssertUnwindSafe};

use futures_util::FutureExt;
use lazydb::{
    db::{
        DatabaseConnection, ErrorCategory,
        catalog::{
            CatalogCapabilities, CatalogCompleteness, CatalogCount, CatalogCursor, CatalogEntry,
            CatalogId, CatalogKind, CatalogMetadata, CatalogRequest, CatalogRequestKey,
            CatalogSearchRequest, CatalogTarget, ColumnMetadata, ColumnMetadataCapabilities,
            ConstraintMembership, ConstraintMetadata, IndexMetadata, NamespaceModel, ObjectGroup,
            OptionalMetadata,
        },
        catalog_mutation::{
            CatalogMutationAnchor, CatalogMutationAvailability, CatalogMutationMode,
            CatalogMutationRequest, CatalogObjectDefinition, CatalogObjectDefinitionRequest,
            CatalogObjectType, ConstraintDefinitionKind, SequenceBound,
        },
        postgres::{self, PostgresAdapter},
        value::CellValue,
    },
    identity::ConnectionIdentity,
    model::dashboard::MetricKey,
    model::{
        catalog_editor::{CatalogDraft, ColumnDraft, DraftRowState},
        execution_target::ExecutionTarget,
    },
    profile::{CatalogScope, CatalogSelection, DatabaseScope, import_connection_url},
};
use uuid::Uuid;

#[test]
fn monitoring_sql_aggregates_database_and_activity_stats_separately() {
    let status = postgres::PostgresAdapter::MONITOR_STATUS_SQL;
    assert!(status.contains("WITH db_stats AS"));
    assert!(status.contains("activity_stats AS"));
    assert!(status.contains("pg_stat_database"));
    assert!(status.contains("pg_stat_activity"));
    assert!(status.contains("pg_backend_pid()"));
    assert!(status.contains("pg_postmaster_start_time()"));
    assert!(status.contains("pg_wal_lsn_diff(pg_current_wal_lsn(), '0/0')"));
    assert!(status.contains("AS wal_bytes"));
    assert!(status.contains("AS server_uptime"));
    assert!(status.contains("sum(tup_returned)"));
    assert!(status.contains(
        "floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint AS server_time_millis"
    ));
    assert!(status.contains(
        "floor(extract(epoch FROM pg_postmaster_start_time()) * 1000)::bigint AS server_generation"
    ));

    let process = postgres::PostgresAdapter::PROCESS_LIST_SQL;
    for field in [
        "pid",
        "user_name",
        "database_name",
        "client",
        "application_name",
        "state",
        "wait",
        "elapsed_seconds",
        "query",
    ] {
        assert!(process.contains(field), "missing {field}");
    }
    assert!(process.contains("LIMIT 2001"));
    assert!(process.contains("AS elapsed_seconds"));
    assert!(process.contains("::double precision AS elapsed_seconds"));
}

#[tokio::test]
async fn monitoring_snapshot_decodes_integer_timestamps_when_configured() {
    let Ok(url) = std::env::var("LAZYDB_TEST_POSTGRES_URL") else {
        return;
    };
    let imported = import_connection_url(&url, Some("postgres-monitoring")).unwrap();
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();

    let snapshot = database.load_monitor_snapshot().await.unwrap();
    assert!(snapshot.server_time_millis > 0);
    assert!(snapshot.server_generation > 0);
    assert!(snapshot.server_time_millis >= snapshot.server_generation);
    assert!(snapshot.values.contains_key(&MetricKey::Transactions));
    assert!(snapshot.values.contains_key(&MetricKey::Connections));

    let metadata = database.load_monitor_metadata().await.unwrap();
    assert!(metadata.version.is_some());
    assert!(metadata.max_connections.is_some());

    database.close().await;
}

#[test]
fn index_mutation_capability_is_advertised_for_relations() {
    let capabilities = lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities();
    assert!(capabilities.create.iter().any(|option| {
        option.object_type
            == lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
                lazydb::db::catalog::CatalogKind::Index,
            )
    }));
    assert!(capabilities.edit.iter().any(|option| {
        option.object_type
            == lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
                lazydb::db::catalog::CatalogKind::Index,
            )
    }));
}

#[test]
fn materialized_view_mutation_create_and_native_safe_edit_never_replaces_definition() {
    use lazydb::{
        db::catalog_mutation::{
            CatalogMutationAnchor, CatalogMutationMode, CatalogMutationRequest,
            CatalogObjectDefinition, MaterializedViewDefinition,
        },
        model::catalog_editor::{CatalogDraft, MaterializedViewDraft},
    };
    let profile = Uuid::new_v4();
    let connection = ConnectionIdentity {
        profile_id: profile,
        generation: 1,
    };
    let schema = CatalogId::new(profile, CatalogKind::Schema, ["app", "public"]);
    let mut create = MaterializedViewDraft {
        name: "mv".into(),
        schema: "public".into(),
        owner: "alice".into(),
        comment: "note".into(),
        query: "SELECT 1".into(),
        tablespace: "fast".into(),
        with_data: false,
        selected_field: 0,
        query_editable: true,
    };
    let request = CatalogMutationRequest::new(
        connection,
        1,
        1,
        CatalogMutationMode::Create,
        CatalogMutationAnchor::Catalog(schema.clone()),
        CatalogObjectType::Catalog(CatalogKind::MaterializedView),
    )
    .unwrap();
    let plan = PostgresAdapter::plan_catalog_mutation(
        request,
        CatalogDraft::MaterializedView(create.clone()),
        None,
    )
    .unwrap();
    assert!(
        plan.sql()
            .contains("CREATE MATERIALIZED VIEW \"public\".\"mv\"")
    );
    assert!(plan.sql().contains("WITH NO DATA"));
    create.name = "renamed".into();
    create.query = "SELECT 999".into();
    let object = CatalogId::new(
        profile,
        CatalogKind::MaterializedView,
        ["app", "public", "mv", "42"],
    );
    let request = CatalogMutationRequest::new(
        connection,
        2,
        1,
        CatalogMutationMode::Edit,
        CatalogMutationAnchor::Catalog(object),
        CatalogObjectType::Catalog(CatalogKind::MaterializedView),
    )
    .unwrap();
    let baseline = CatalogObjectDefinition::MaterializedView(MaterializedViewDefinition {
        database: "app".into(),
        schema: "public".into(),
        name: "mv".into(),
        owner: "alice".into(),
        comment: OptionalMetadata::Supported(Some("note".into())),
        query: "SELECT 1".into(),
        tablespace: OptionalMetadata::Supported(Some("fast".into())),
        populated: false,
        baseline_fingerprint: "sha256:x".into(),
    });
    let plan = PostgresAdapter::plan_catalog_mutation(
        request,
        CatalogDraft::MaterializedView(create),
        Some(baseline),
    )
    .unwrap();
    assert!(plan.sql().contains("RENAME TO \"renamed\""));
    assert!(!plan.sql().contains("SELECT 999"));
    assert!(!plan.sql().contains("CREATE MATERIALIZED VIEW"));
}

#[test]
fn postgres_constraint_definition_uses_authoritative_constraint_attributes() {
    let source = include_str!("../src/db/postgres.rs");
    assert!(source.contains("con.condeferrable"));
    assert!(source.contains("con.convalidated"));
    assert!(source.contains("con.connoinherit"));
    assert!(source.contains("confmatchtype"));
    assert!(source.contains("confupdtype"));
    assert!(source.contains("confdeltype"));
    assert!(source.contains("VALIDATE CONSTRAINT"));
    assert!(postgres::CATALOG_CONSTRAINT_DEFINITION_SQL.contains("confmatchtype"));
    assert!(postgres::CATALOG_CONSTRAINT_DEFINITION_SQL.contains("connoinherit"));
}

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
fn postgres_mutation_capabilities_expose_only_schema_slice() {
    let capabilities = PostgresAdapter::catalog_mutation_capabilities();
    assert_eq!(
        capabilities.create_availability(CatalogObjectType::Catalog(CatalogKind::Schema)),
        Some(CatalogMutationAvailability::Available)
    );
    assert_eq!(
        capabilities.edit_availability(CatalogObjectType::Catalog(CatalogKind::Schema)),
        Some(CatalogMutationAvailability::Available)
    );
    assert_eq!(
        capabilities.create_availability(CatalogObjectType::Catalog(CatalogKind::Database)),
        None
    );
    assert_eq!(
        capabilities
            .profile_create
            .iter()
            .find(|option| option.object_type == CatalogObjectType::Catalog(CatalogKind::Database))
            .map(|option| option.availability),
        Some(CatalogMutationAvailability::Available)
    );
    assert_eq!(
        capabilities
            .profile_create
            .iter()
            .find(|option| option.object_type == CatalogObjectType::LoginRole)
            .map(|option| option.availability),
        Some(CatalogMutationAvailability::Available)
    );
    assert_eq!(
        capabilities
            .profile_create
            .iter()
            .find(|option| option.object_type == CatalogObjectType::Role)
            .map(|option| option.availability),
        Some(CatalogMutationAvailability::Available)
    );
    assert!(
        capabilities
            .create_options(
                &lazydb::db::catalog_mutation::CatalogMutationAnchor::Profile {
                    profile_id: Uuid::new_v4()
                },
                None
            )
            .unwrap()
            == vec![
                CatalogObjectType::Catalog(CatalogKind::Database),
                CatalogObjectType::LoginRole,
                CatalogObjectType::Role,
            ]
    );
    assert_eq!(
        capabilities.create_availability(CatalogObjectType::Catalog(CatalogKind::View)),
        Some(CatalogMutationAvailability::Available)
    );
    assert_eq!(
        capabilities.edit_availability(CatalogObjectType::Catalog(CatalogKind::View)),
        Some(CatalogMutationAvailability::Available)
    );
}

#[test]
fn schema_definition_shape_includes_authoritative_baseline() {
    let definition = lazydb::db::catalog_mutation::SchemaDefinition {
        database: "app".to_owned(),
        name: "public".to_owned(),
        owner: "alice".to_owned(),
        comment: OptionalMetadata::Supported(Some("owned schema".to_owned())),
        baseline_fingerprint: "sha256:baseline".to_owned(),
    };
    assert_eq!(definition.database, "app");
    assert_eq!(definition.name, "public");
    assert_eq!(definition.owner, "alice");
    assert_eq!(definition.baseline_fingerprint, "sha256:baseline");
}

#[test]
fn table_definition_shape_includes_ordered_column_attributes_and_placeholders() {
    let definition = lazydb::db::catalog_mutation::TableDefinition {
        database: "app".into(),
        schema: "public".into(),
        name: "events".into(),
        owner: "alice".into(),
        comment: OptionalMetadata::Supported(Some("event log".into())),
        columns: vec![],
        indexes: vec![],
        constraints: vec![],
        baseline_fingerprint: "sha256:table".into(),
    };
    assert_eq!(definition.schema, "public");
    assert!(definition.indexes.is_empty());
    assert!(definition.constraints.is_empty());
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
fn postgres_catalog_search_sql_pushes_literal_matching_scope_order_and_limit() {
    let sql = postgres::SEARCH_CATALOG_SQL;
    assert!(sql.contains("current_database()"));
    assert!(sql.contains("$2::text[] IS NULL OR n.nspname = ANY($2)"));
    assert!(sql.contains("regexp_replace(lower(object_name), '[^[:alnum:]]', '', 'g')"));
    assert!(sql.contains("regexp_replace(lower(qualified_path), '[^[:alnum:]]', '', 'g')"));
    assert!(sql.contains("strpos(search_name, $1)"));
    assert!(!sql.contains("ILIKE"));
    assert!(sql.contains("ORDER BY relevance, lower(qualified_path) COLLATE \"C\""));
    assert!(sql.contains("LIMIT $4"));
    for kind in [
        "database",
        "schema",
        "table",
        "view",
        "materialized_view",
        "sequence",
        "function",
        "procedure",
        "type",
        "column",
        "index",
        "primary_key",
        "unique_constraint",
        "foreign_key",
        "check_constraint",
    ] {
        assert!(sql.contains(kind), "search SQL omits {kind}");
    }
    assert!(!sql.contains("'trigger'"));
}

#[test]
fn postgres_version_support_starts_at_twelve() {
    assert!(!postgres::supports_server_version(119_999));
    assert!(postgres::supports_server_version(120_000));
}

#[test]
fn postgres_sequence_mutation_is_advertised_without_sequence_children() {
    let profile = uuid::Uuid::new_v4();
    let caps = lazydb::db::postgres::PostgresAdapter::catalog_mutation_capabilities();
    assert!(
        caps.create_availability(lazydb::db::catalog_mutation::CatalogObjectType::Catalog(
            lazydb::db::catalog::CatalogKind::Sequence
        ))
        .is_some()
    );
    assert!(
        caps.create_options(
            &lazydb::db::catalog_mutation::CatalogMutationAnchor::Catalog(
                lazydb::db::catalog::CatalogId::new(
                    profile,
                    lazydb::db::catalog::CatalogKind::Sequence,
                    ["app", "public", "seq", "1"]
                )
            ),
            None
        )
        .unwrap()
        .is_empty()
    );
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
async fn privileged_role_mutation_is_gated_by_environment_and_createrole() {
    let Ok(url) = std::env::var("LAZYDB_TEST_POSTGRES_URL") else {
        eprintln!("skipping role mutation: LAZYDB_TEST_POSTGRES_URL is not set");
        return;
    };
    let imported = import_connection_url(&url, Some("postgres-role-test")).unwrap();
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let privilege = database
        .execute("SELECT rolcreaterole FROM pg_roles WHERE rolname = current_user")
        .await
        .unwrap();
    let can_create = matches!(
        privilege
            .result_sets
            .last()
            .and_then(|set| set.rows.first())
            .and_then(|row| row.first()),
        Some(CellValue::Boolean(true))
    );
    if !can_create {
        eprintln!("skipping role mutation: current PostgreSQL role lacks CREATEROLE");
        return;
    }
    let name = format!("lazydb_role_test_{}", Uuid::new_v4().simple());
    database
        .execute(&format!("CREATE ROLE \"{name}\" NOLOGIN"))
        .await
        .unwrap();
    database
        .execute(&format!("DROP ROLE \"{name}\""))
        .await
        .unwrap();
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
            "CREATE SCHEMA {qualified_schema}; \
             CREATE TABLE {qualified_schema}.source (id integer); \
             CREATE MATERIALIZED VIEW {qualified_schema}.{name} AS SELECT id FROM {qualified_schema}.source; \
             CREATE UNIQUE INDEX materialized_view_id_idx ON {qualified_schema}.{name} (id); \
             COMMENT ON MATERIALIZED VIEW {qualified_schema}.{name} IS 'materialized owner''s note';"
        ))
        .await
        .unwrap();
    let oid = match database
        .execute(&format!(
            "SELECT c.oid::bigint FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = '{schema}' AND c.relname = '{name}'"
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
    let ddl = database.relation_ddl(&relation).await.unwrap();
    assert_eq!(ddl.relation.native_kind, "materialized_view");
    assert_eq!(
        ddl.provenance,
        lazydb::db::catalog::DdlProvenance::AdapterGenerated
    );
    assert!(ddl.sql.contains("-- View\n\nCREATE MATERIALIZED VIEW"));
    assert!(ddl.sql.contains("SELECT"));
    assert!(ddl.sql.contains("source"));
    assert!(ddl.sql.contains("id"));
    assert!(ddl.sql.contains("-- Comments"));
    assert!(ddl.sql.contains("materialized owner''s note"));
    assert!(ddl.sql.contains("-- Indexes"));
    assert!(
        ddl.sql
            .contains("CREATE UNIQUE INDEX materialized_view_id_idx")
    );
    assert!(!ddl.sql.contains("-- Triggers"));
    database
        .execute(&format!("DROP MATERIALIZED VIEW {qualified_schema}.{name}; DROP SCHEMA {qualified_schema} CASCADE;"))
        .await
        .unwrap();
    database.close().await;
}

#[tokio::test]
async fn serialized_postgres_catalog_mutations_round_trip_catalog_definitions() {
    let Ok(url) = std::env::var("LAZYDB_TEST_POSTGRES_URL") else {
        eprintln!("skipping serialized catalog mutations: LAZYDB_TEST_POSTGRES_URL is not set");
        return;
    };
    let imported = import_connection_url(&url, Some("postgres-catalog-mutations")).unwrap();
    let profile_id = imported.profile.id;
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let database_name = database.probe().await.unwrap().database;
    let suffix = Uuid::new_v4().simple().to_string();
    let schema_name = format!("lazydb_mutation_{suffix}");
    let table_name = format!("events_{suffix}");
    let parent_name = format!("parents_{suffix}");
    let index_name = format!("events_value_idx_{suffix}");
    let primary_name = format!("events_pk_{suffix}");
    let unique_name = format!("events_value_key_{suffix}");
    let foreign_name = format!("events_parent_fk_{suffix}");
    let check_name = format!("events_value_check_{suffix}");
    let view_name = format!("events_view_{suffix}");
    let materialized_name = format!("events_snapshot_{suffix}");
    let sequence_name = format!("events_sequence_{suffix}");
    let qschema = postgres::quote_identifier(&schema_name);
    let mut cleanup = vec![
        format!("DROP SCHEMA IF EXISTS {qschema} CASCADE"),
        format!("DROP SEQUENCE IF EXISTS {qschema}.{sequence_name}"),
        format!("DROP TABLE IF EXISTS {qschema}.{parent_name} CASCADE"),
        format!("DROP TABLE IF EXISTS {qschema}.{table_name} CASCADE"),
        format!("DROP VIEW IF EXISTS {qschema}.{view_name}"),
        format!("DROP MATERIALIZED VIEW IF EXISTS {qschema}.{materialized_name}"),
    ];

    let result = AssertUnwindSafe(async {
        let owner = scalar_text(&database, "SELECT current_user").await;
        let schema_anchor = CatalogId::new(
            profile_id,
            CatalogKind::Schema,
            [&database_name, &schema_name],
        );
        let database_anchor = CatalogId::new(profile_id, CatalogKind::Database, [&database_name]);
        apply_plan(
            &database,
            mutation_request(
                profile_id,
                CatalogMutationMode::Create,
                CatalogMutationAnchor::Catalog(database_anchor.clone()),
                CatalogObjectType::Catalog(CatalogKind::Schema),
            ),
            CatalogDraft::Schema(lazydb::model::catalog_editor::SchemaDraft {
                name: schema_name.clone().into(),
                owner: owner.clone().into(),
                comment: "created".into(),
            }),
        )
        .await;
        let schema_definition =
            load_definition(&database, profile_id, schema_anchor.clone(), &database_name).await;
        assert!(matches!(
            schema_definition,
            CatalogObjectDefinition::Schema(_)
        ));

        let parent_anchor = CatalogId::new(
            profile_id,
            CatalogKind::Table,
            [&database_name, &schema_name, &parent_name, ""],
        );
        let table_anchor = CatalogId::new(
            profile_id,
            CatalogKind::Table,
            [&database_name, &schema_name, &table_name, ""],
        );
        for (name, anchor) in [
            (&parent_name, parent_anchor.clone()),
            (&table_name, table_anchor.clone()),
        ] {
            apply_plan(
                &database,
                mutation_request(
                    profile_id,
                    CatalogMutationMode::Create,
                    CatalogMutationAnchor::Group {
                        schema: schema_anchor.clone(),
                        group: ObjectGroup::Tables,
                    },
                    CatalogObjectType::Catalog(CatalogKind::Table),
                ),
                CatalogDraft::Table(lazydb::model::catalog_editor::TableDraft {
                    name: name.clone().into(),
                    schema: schema_name.clone().into(),
                    owner: owner.clone().into(),
                    comment: "table".into(),
                    columns: vec![
                        added_column("id", 1, "integer", false),
                        added_column("value", 2, "text", true),
                    ],
                    selected_section: lazydb::model::catalog_editor::CatalogEditorSection::Columns,
                    selected_column: 0,
                    indexes: vec![],
                    constraints: vec![],
                }),
            )
            .await;
            assert!(matches!(
                load_definition(&database, profile_id, anchor, &database_name).await,
                CatalogObjectDefinition::Table(_)
            ));
        }
        let table = find_entry(
            &database,
            profile_id,
            &table_name,
            CatalogKind::Table,
            &database_name,
            &schema_name,
        )
        .await;
        let parent = find_entry(
            &database,
            profile_id,
            &parent_name,
            CatalogKind::Table,
            &database_name,
            &schema_name,
        )
        .await;
        apply_plan(
            &database,
            mutation_request(
                profile_id,
                CatalogMutationMode::Create,
                CatalogMutationAnchor::Catalog(parent.id.clone()),
                CatalogObjectType::Catalog(CatalogKind::PrimaryKey),
            ),
            CatalogDraft::Constraint(constraint_draft(
                &database_name,
                &schema_name,
                &parent_name,
                ConstraintDefinitionKind::PrimaryKey {
                    columns: vec!["id".into()],
                },
                &format!("{parent_name}_pk"),
            )),
        )
        .await;
        let CatalogObjectDefinition::Table(mut table_definition) =
            load_definition(&database, profile_id, table.id.clone(), &database_name).await
        else {
            panic!("table definition expected")
        };
        table_definition.comment = OptionalMetadata::Supported(Some("edited table".into()));
        let mut table_draft =
            lazydb::model::catalog_editor::TableDraft::from_definition(&table_definition);
        table_draft.comment = "edited table".into();
        apply_plan(
            &database,
            mutation_request(
                profile_id,
                CatalogMutationMode::Edit,
                CatalogMutationAnchor::Catalog(table.id.clone()),
                CatalogObjectType::Catalog(CatalogKind::Table),
            ),
            CatalogDraft::Table(table_draft),
        )
        .await;
        assert!(matches!(
            load_definition(&database, profile_id, table.id.clone(), &database_name).await,
            CatalogObjectDefinition::Table(_)
        ));

        let CatalogObjectDefinition::Table(column_table) =
            load_definition(&database, profile_id, table.id.clone(), &database_name).await
        else {
            panic!("table definition expected")
        };
        let mut column_draft =
            lazydb::model::catalog_editor::TableDraft::from_definition(&column_table);
        column_draft
            .columns
            .push(added_column("extra", 3, "integer", true));
        apply_plan(
            &database,
            mutation_request(
                profile_id,
                CatalogMutationMode::Edit,
                CatalogMutationAnchor::Catalog(table.id.clone()),
                CatalogObjectType::Catalog(CatalogKind::Table),
            ),
            CatalogDraft::Table(column_draft),
        )
        .await;
        let table = find_entry(
            &database,
            profile_id,
            &table_name,
            CatalogKind::Table,
            &database_name,
            &schema_name,
        )
        .await;
        let column = find_child(
            &database,
            profile_id,
            &table.id,
            "extra",
            CatalogKind::Column,
            &database_name,
            &schema_name,
        )
        .await;
        assert!(matches!(
            load_definition(&database, profile_id, column.id.clone(), &database_name).await,
            CatalogObjectDefinition::Table(_)
        ));

        let index = apply_index(
            &database,
            profile_id,
            &database_name,
            &schema_name,
            &table,
            &index_name,
            false,
        )
        .await;
        assert!(matches!(
            load_definition(&database, profile_id, index.id.clone(), &database_name).await,
            CatalogObjectDefinition::Index(_)
        ));
        let mut index_definition =
            match load_definition(&database, profile_id, index.id.clone(), &database_name).await {
                CatalogObjectDefinition::Index(v) => v,
                _ => unreachable!(),
            };
        index_definition.name = format!("{index_name}_renamed");
        let index_draft =
            lazydb::model::catalog_editor::IndexDraft::from_definition(&index_definition);
        apply_plan(
            &database,
            mutation_request(
                profile_id,
                CatalogMutationMode::Edit,
                CatalogMutationAnchor::Catalog(index.id.clone()),
                CatalogObjectType::Catalog(CatalogKind::Index),
            ),
            CatalogDraft::Index(index_draft),
        )
        .await;

        for (name, kind, draft) in [
            (
                primary_name.clone(),
                CatalogKind::PrimaryKey,
                constraint_draft(
                    &database_name,
                    &schema_name,
                    &table_name,
                    ConstraintDefinitionKind::PrimaryKey {
                        columns: vec!["id".into()],
                    },
                    &primary_name,
                ),
            ),
            (
                unique_name.clone(),
                CatalogKind::UniqueConstraint,
                constraint_draft(
                    &database_name,
                    &schema_name,
                    &table_name,
                    ConstraintDefinitionKind::Unique {
                        columns: vec!["value".into()],
                    },
                    &unique_name,
                ),
            ),
            (
                foreign_name.clone(),
                CatalogKind::ForeignKey,
                foreign_constraint(
                    &database_name,
                    &schema_name,
                    &table_name,
                    &parent_name,
                    &foreign_name,
                ),
            ),
            (
                check_name.clone(),
                CatalogKind::CheckConstraint,
                constraint_draft(
                    &database_name,
                    &schema_name,
                    &table_name,
                    ConstraintDefinitionKind::Check {
                        expression: "value IS NOT NULL".into(),
                        no_inherit: false,
                    },
                    &check_name,
                ),
            ),
        ] {
            apply_plan(
                &database,
                mutation_request(
                    profile_id,
                    CatalogMutationMode::Create,
                    CatalogMutationAnchor::Catalog(table.id.clone()),
                    CatalogObjectType::Catalog(kind),
                ),
                CatalogDraft::Constraint(draft),
            )
            .await;
            let entry = find_entry(
                &database,
                profile_id,
                &name,
                kind,
                &database_name,
                &schema_name,
            )
            .await;
            assert!(matches!(
                load_definition(&database, profile_id, entry.id.clone(), &database_name).await,
                CatalogObjectDefinition::Constraint(_)
            ));
            cleanup.push(format!(
                "ALTER TABLE {qschema}.{table_name} DROP CONSTRAINT IF EXISTS {}",
                postgres::quote_identifier(&name)
            ));
        }
        let view = apply_view(
            &database,
            profile_id,
            &database_name,
            &schema_name,
            &table_name,
            &view_name,
            false,
        )
        .await;
        assert!(matches!(
            load_definition(&database, profile_id, view.id.clone(), &database_name).await,
            CatalogObjectDefinition::View(_)
        ));
        let materialized = apply_materialized(
            &database,
            profile_id,
            &database_name,
            &schema_name,
            &table_name,
            &materialized_name,
        )
        .await;
        assert!(matches!(
            load_definition(
                &database,
                profile_id,
                materialized.id.clone(),
                &database_name
            )
            .await,
            CatalogObjectDefinition::MaterializedView(_)
        ));
        let sequence = apply_sequence(
            &database,
            profile_id,
            &database_name,
            &schema_name,
            &sequence_name,
        )
        .await;
        assert!(matches!(
            load_definition(&database, profile_id, sequence.id.clone(), &database_name).await,
            CatalogObjectDefinition::Sequence(_)
        ));
    })
    .catch_unwind()
    .await;
    let mut cleanup_errors = Vec::new();
    while let Some(sql) = cleanup.pop() {
        if let Err(error) = database.execute(&sql).await {
            cleanup_errors.push(error);
        }
    }
    database.close().await;
    if let Err(panic) = result {
        eprintln!("mutation cleanup errors after panic: {cleanup_errors:?}");
        std::panic::resume_unwind(panic);
    }
    assert!(
        cleanup_errors.is_empty(),
        "mutation cleanup failed: {cleanup_errors:?}"
    );
}

async fn apply_plan(
    database: &DatabaseConnection,
    request: CatalogMutationRequest,
    draft: CatalogDraft,
) {
    let baseline = if request.mode == CatalogMutationMode::Edit
        || request.object_type == CatalogObjectType::Catalog(CatalogKind::Column)
    {
        let CatalogMutationAnchor::Catalog(id) = &request.anchor else {
            panic!("catalog anchor expected")
        };
        let object = if request.object_type == CatalogObjectType::Catalog(CatalogKind::Column) {
            CatalogId::new(
                request.connection.profile_id,
                CatalogKind::Table,
                id.native_path[..4].to_vec(),
            )
        } else {
            id.clone()
        };
        let database_name = match &request.anchor {
            CatalogMutationAnchor::Catalog(id) => {
                id.native_path.first().cloned().unwrap_or_default()
            }
            CatalogMutationAnchor::Group { schema, .. } => {
                schema.native_path.first().cloned().unwrap_or_default()
            }
            CatalogMutationAnchor::Profile { .. } => String::new(),
        };
        Some(
            load_definition(
                database,
                request.connection.profile_id,
                object,
                &database_name,
            )
            .await,
        )
    } else {
        None
    };
    let plan = PostgresAdapter::plan_catalog_mutation(request, draft, baseline).unwrap();
    database.execute_catalog_mutation(&plan).await.unwrap();
}

async fn apply_index(
    database: &DatabaseConnection,
    profile_id: Uuid,
    db: &str,
    schema: &str,
    table: &CatalogEntry,
    name: &str,
    unique: bool,
) -> CatalogEntry {
    let draft = lazydb::model::catalog_editor::IndexDraft {
        name: name.into(),
        schema: schema.into(),
        relation: table.qualified_name.object.clone().into(),
        unique,
        access_method: "btree".into(),
        columns: vec![lazydb::model::catalog_editor::IndexColumnDraft {
            expression: "value".into(),
            descending: false,
            nulls_first: false,
            is_expression: false,
        }],
        include_columns: "".into(),
        predicate: "".into(),
        tablespace: "".into(),
    };
    apply_plan(
        database,
        mutation_request(
            profile_id,
            CatalogMutationMode::Create,
            CatalogMutationAnchor::Catalog(table.id.clone()),
            CatalogObjectType::Catalog(CatalogKind::Index),
        ),
        CatalogDraft::Index(draft),
    )
    .await;
    find_entry(database, profile_id, name, CatalogKind::Index, db, schema).await
}

async fn apply_view(
    database: &DatabaseConnection,
    profile_id: Uuid,
    db: &str,
    schema: &str,
    table: &str,
    name: &str,
    _replace: bool,
) -> CatalogEntry {
    let draft = lazydb::model::catalog_editor::ViewDraft {
        name: name.into(),
        schema: schema.into(),
        owner: "".into(),
        comment: "view".into(),
        query: format!(
            "SELECT id, value FROM {}.{}",
            postgres::quote_identifier(schema),
            postgres::quote_identifier(table)
        )
        .into(),
        output_columns: "".into(),
        security_barrier: lazydb::db::catalog_mutation::ViewOption::available(None),
        security_invoker: lazydb::db::catalog_mutation::ViewOption::available(None),
        check_option: lazydb::db::catalog_mutation::ViewOption::available(None),
        selected_field: 0,
    };
    apply_plan(
        database,
        mutation_request(
            profile_id,
            CatalogMutationMode::Create,
            CatalogMutationAnchor::Catalog(CatalogId::new(
                profile_id,
                CatalogKind::Schema,
                [db, schema],
            )),
            CatalogObjectType::Catalog(CatalogKind::View),
        ),
        CatalogDraft::View(draft),
    )
    .await;
    find_entry(database, profile_id, name, CatalogKind::View, db, schema).await
}

async fn apply_materialized(
    database: &DatabaseConnection,
    profile_id: Uuid,
    db: &str,
    schema: &str,
    table: &str,
    name: &str,
) -> CatalogEntry {
    let draft = lazydb::model::catalog_editor::MaterializedViewDraft {
        name: name.into(),
        schema: schema.into(),
        owner: "".into(),
        comment: "snapshot".into(),
        query: format!(
            "SELECT id, value FROM {}.{}",
            postgres::quote_identifier(schema),
            postgres::quote_identifier(table)
        )
        .into(),
        tablespace: "".into(),
        with_data: true,
        selected_field: 0,
        query_editable: true,
    };
    apply_plan(
        database,
        mutation_request(
            profile_id,
            CatalogMutationMode::Create,
            CatalogMutationAnchor::Catalog(CatalogId::new(
                profile_id,
                CatalogKind::Schema,
                [db, schema],
            )),
            CatalogObjectType::Catalog(CatalogKind::MaterializedView),
        ),
        CatalogDraft::MaterializedView(draft),
    )
    .await;
    find_entry(
        database,
        profile_id,
        name,
        CatalogKind::MaterializedView,
        db,
        schema,
    )
    .await
}

async fn apply_sequence(
    database: &DatabaseConnection,
    profile_id: Uuid,
    db: &str,
    schema: &str,
    name: &str,
) -> CatalogEntry {
    let draft = lazydb::model::catalog_editor::SequenceDraft {
        name: name.into(),
        schema: schema.into(),
        owner: "".into(),
        comment: "sequence".into(),
        data_type: "bigint".into(),
        increment: "1".into(),
        min_value: SequenceBound::Unset,
        max_value: SequenceBound::Unset,
        start_value: "1".into(),
        restart_value: "".into(),
        cache: "1".into(),
        cycle: false,
        owned_by: "NONE".into(),
        selected_field: 0,
    };
    apply_plan(
        database,
        mutation_request(
            profile_id,
            CatalogMutationMode::Create,
            CatalogMutationAnchor::Catalog(CatalogId::new(
                profile_id,
                CatalogKind::Schema,
                [db, schema],
            )),
            CatalogObjectType::Catalog(CatalogKind::Sequence),
        ),
        CatalogDraft::Sequence(draft),
    )
    .await;
    find_entry(
        database,
        profile_id,
        name,
        CatalogKind::Sequence,
        db,
        schema,
    )
    .await
}

async fn load_definition(
    database: &DatabaseConnection,
    profile_id: Uuid,
    object: CatalogId,
    database_name: &str,
) -> CatalogObjectDefinition {
    database
        .load_catalog_object_definition(&CatalogObjectDefinitionRequest {
            connection: ConnectionIdentity {
                profile_id,
                generation: 7,
            },
            request_id: 1,
            catalog_epoch: 1,
            object,
            target: ExecutionTarget {
                profile_id,
                database: database_name.to_owned(),
                schema: None,
            },
        })
        .await
        .unwrap()
}

fn mutation_request(
    profile_id: Uuid,
    mode: CatalogMutationMode,
    anchor: CatalogMutationAnchor,
    object_type: CatalogObjectType,
) -> CatalogMutationRequest {
    CatalogMutationRequest::new(
        ConnectionIdentity {
            profile_id,
            generation: 7,
        },
        1,
        1,
        mode,
        anchor,
        object_type,
    )
    .unwrap()
    .with_current_database("postgres")
}

async fn scalar_text(database: &DatabaseConnection, sql: &str) -> String {
    match &database
        .execute(sql)
        .await
        .unwrap()
        .result_sets
        .last()
        .unwrap()
        .rows[0][0]
    {
        CellValue::Text(value) => value.clone(),
        value => panic!("expected text, got {value:?}"),
    }
}

fn added_column(
    name: &str,
    ordinal_position: u32,
    native_type: &str,
    nullable: bool,
) -> ColumnDraft {
    ColumnDraft {
        row_id: Uuid::new_v4(),
        ordinal_position,
        existing_name: None,
        name: name.into(),
        native_type: native_type.into(),
        nullable,
        default_expression: "".into(),
        identity: false,
        generated_expression: "".into(),
        collation: "".into(),
        comment: "".into(),
        state: DraftRowState::Added,
    }
}

async fn find_entry(
    database: &DatabaseConnection,
    profile_id: Uuid,
    name: &str,
    kind: CatalogKind,
    database_name: &str,
    schema: &str,
) -> CatalogEntry {
    database
        .search_catalog(&catalog_search_request(
            profile_id,
            name,
            selected_scope(database_name, &[schema]),
            20,
        ))
        .await
        .unwrap()
        .hits
        .into_iter()
        .find(|hit| hit.entry.kind == kind && hit.entry.qualified_name.object == name)
        .unwrap()
        .entry
}

async fn find_child(
    database: &DatabaseConnection,
    profile_id: Uuid,
    relation: &CatalogId,
    name: &str,
    kind: CatalogKind,
    database_name: &str,
    schema: &str,
) -> CatalogEntry {
    database
        .load_catalog_page(&catalog_request(
            profile_id,
            CatalogTarget::relation_children(relation.clone()).unwrap(),
            selected_scope(database_name, &[schema]),
            100,
            None,
            99,
        ))
        .await
        .unwrap()
        .entries
        .into_iter()
        .find(|entry| entry.kind == kind && entry.qualified_name.object == name)
        .unwrap()
}

fn constraint_draft(
    database: &str,
    schema: &str,
    relation: &str,
    kind: ConstraintDefinitionKind,
    name: &str,
) -> lazydb::model::catalog_editor::ConstraintDraft {
    let mut draft = lazydb::model::catalog_editor::ConstraintDraft::new(kind, schema, relation);
    draft.database = database.into();
    draft.name = name.into();
    draft.columns = match &draft.kind {
        ConstraintDefinitionKind::PrimaryKey { columns }
        | ConstraintDefinitionKind::Unique { columns } => columns.join(", ").into(),
        _ => "".into(),
    };
    draft
}

fn foreign_constraint(
    database: &str,
    schema: &str,
    relation: &str,
    parent: &str,
    name: &str,
) -> lazydb::model::catalog_editor::ConstraintDraft {
    let mut draft = constraint_draft(
        database,
        schema,
        relation,
        ConstraintDefinitionKind::ForeignKey {
            columns: vec!["id".into()],
            referenced_schema: schema.into(),
            referenced_relation: parent.into(),
            referenced_columns: vec!["id".into()],
            match_type: "SIMPLE".into(),
            on_update: "NO ACTION".into(),
            on_delete: "NO ACTION".into(),
        },
        name,
    );
    draft.columns = "id".into();
    draft.referenced_schema = schema.into();
    draft.referenced_relation = parent.into();
    draft.referenced_columns = "id".into();
    draft
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

fn catalog_search_request(
    profile_id: Uuid,
    query: impl Into<String>,
    scope: CatalogScope,
    limit: usize,
) -> CatalogSearchRequest {
    CatalogSearchRequest {
        connection: ConnectionIdentity {
            profile_id,
            generation: 7,
        },
        session_id: 11,
        generation: 13,
        query: query.into(),
        scope,
        limit,
    }
}

#[tokio::test]
async fn native_catalog_search_covers_postgres_contract_when_configured() {
    let Ok(url) = std::env::var("LAZYDB_TEST_POSTGRES_URL") else {
        return;
    };
    let imported = import_connection_url(&url, Some("postgres-catalog-search")).unwrap();
    let profile_id = imported.profile.id;
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let database_name = database.probe().await.unwrap().database;
    let suffix = Uuid::new_v4().simple().to_string();
    let schema = format!("lazydb_search_{suffix}");
    let excluded_schema = format!("lazydb_search_excluded_{suffix}");
    let quoted_schema = postgres::quote_identifier(&schema);
    let quoted_excluded = postgres::quote_identifier(&excluded_schema);
    let token = format!("lazyfind_{suffix}");
    let exact = token.clone();
    let prefix = format!("{token}_tail");
    let contains = format!("x_{token}");

    let result = AssertUnwindSafe(async {
        database
            .execute(&format!(
                r#"
                CREATE SCHEMA {quoted_schema};
                CREATE SCHEMA {quoted_excluded};
                CREATE TYPE {quoted_schema}.{token}_type AS ENUM ('one', 'two');
                CREATE TABLE {quoted_schema}.parent_{token} (id integer PRIMARY KEY);
                CREATE TABLE {quoted_schema}.{exact} (
                    id bigint GENERATED BY DEFAULT AS IDENTITY,
                    parent_id integer NOT NULL,
                    code varchar(20) DEFAULT 'one',
                    CONSTRAINT {token}_pk PRIMARY KEY (id),
                    CONSTRAINT {token}_uq UNIQUE (code),
                    CONSTRAINT {token}_fk FOREIGN KEY (parent_id)
                        REFERENCES {quoted_schema}.parent_{token}(id),
                    CONSTRAINT {token}_check CHECK (parent_id > 0)
                );
                COMMENT ON TABLE {quoted_schema}.{exact} IS 'search table comment';
                COMMENT ON COLUMN {quoted_schema}.{exact}.code IS 'search column comment';
                CREATE INDEX {token}_idx ON {quoted_schema}.{exact} (parent_id, code);
                CREATE TABLE {quoted_schema}.{prefix} (id integer);
                CREATE TABLE {quoted_schema}.{contains} (id integer);
                CREATE TABLE {quoted_schema}."literal%_name" (id integer);
                CREATE VIEW {quoted_schema}.{token}_view AS SELECT id FROM {quoted_schema}.{exact};
                CREATE MATERIALIZED VIEW {quoted_schema}.{token}_mv AS SELECT id FROM {quoted_schema}.{exact};
                CREATE SEQUENCE {quoted_schema}.{token}_seq;
                CREATE FUNCTION {quoted_schema}.{token}_routine(value integer) RETURNS integer
                    LANGUAGE sql IMMUTABLE AS 'SELECT value';
                CREATE FUNCTION {quoted_schema}.{token}_routine(value text) RETURNS text
                    LANGUAGE sql IMMUTABLE AS 'SELECT value';
                CREATE PROCEDURE {quoted_schema}.{token}_procedure() LANGUAGE sql AS 'SELECT 1';
                CREATE TABLE {quoted_excluded}.{token}_hidden (id integer);
                "#
            ))
            .await
            .unwrap();

        let mut scoped_profile = imported.profile.clone();
        let scope = selected_scope(&database_name, &[&schema]);
        scoped_profile.catalog_scope = scope.clone();
        let adapter = PostgresAdapter::connect(
            &scoped_profile,
            imported.transient_password.as_ref(),
        )
        .await
        .unwrap();

        let malformed = adapter
            .search_catalog(&catalog_search_request(profile_id, " ", scope.clone(), 10))
            .await
            .unwrap_err();
        assert_eq!(malformed.code.as_deref(), Some("invalid_catalog_request"));
        let wrong_profile = adapter
            .search_catalog(&catalog_search_request(
                Uuid::new_v4(),
                &token,
                scope.clone(),
                10,
            ))
            .await
            .unwrap_err();
        assert_eq!(wrong_profile.code.as_deref(), Some("invalid_catalog_request"));

        let expected_kinds = [
            (&database_name, CatalogKind::Database),
            (&schema, CatalogKind::Schema),
            (&exact, CatalogKind::Table),
            (&format!("{token}_view"), CatalogKind::View),
            (&format!("{token}_mv"), CatalogKind::MaterializedView),
            (&format!("{token}_seq"), CatalogKind::Sequence),
            (&format!("{token}_routine"), CatalogKind::Function),
            (&format!("{token}_procedure"), CatalogKind::Procedure),
            (&format!("{token}_type"), CatalogKind::Type),
            (&format!("{token}_idx"), CatalogKind::Index),
            (&format!("{token}_pk"), CatalogKind::PrimaryKey),
            (&format!("{token}_uq"), CatalogKind::UniqueConstraint),
            (&format!("{token}_fk"), CatalogKind::ForeignKey),
            (&format!("{token}_check"), CatalogKind::CheckConstraint),
        ];
        for (name, kind) in expected_kinds {
            let page = adapter
                .search_catalog(&catalog_search_request(
                    profile_id,
                    name.clone(),
                    scope.clone(),
                    20,
                ))
                .await
                .unwrap();
            assert!(
                page.hits
                    .iter()
                    .any(|hit| hit.entry.kind == kind && hit.entry.qualified_name.object == *name),
                "missing {kind:?} {name} in {page:?}"
            );
            assert!(page.hits.iter().all(|hit| hit.entry.kind != CatalogKind::Trigger));
        }

        let routines = adapter
            .search_catalog(&catalog_search_request(
                profile_id,
                format!("{token}_routine"),
                scope.clone(),
                10,
            ))
            .await
            .unwrap();
        let routine_ids = routines
            .hits
            .iter()
            .filter(|hit| hit.entry.kind == CatalogKind::Function)
            .map(|hit| hit.entry.id.clone())
            .collect::<HashSet<_>>();
        assert_eq!(routine_ids.len(), 2, "overloaded routine OIDs must remain distinct");
        assert!(routine_ids.iter().all(|id| id.native_path.len() == 4));

        let index = adapter
            .search_catalog(&catalog_search_request(
                profile_id,
                format!("{token}_idx"),
                scope.clone(),
                10,
            ))
            .await
            .unwrap();
        let index = index
            .hits
            .iter()
            .find(|hit| hit.entry.kind == CatalogKind::Index)
            .unwrap();
        assert_eq!(
            index.entry.metadata,
            CatalogMetadata::Index(IndexMetadata {
                columns: vec!["parent_id".to_owned(), "code".to_owned()],
                unique: false,
            })
        );
        let foreign_key = adapter
            .search_catalog(&catalog_search_request(
                profile_id,
                format!("{token}_fk"),
                scope.clone(),
                10,
            ))
            .await
            .unwrap();
        assert!(matches!(
            &foreign_key
                .hits
                .iter()
                .find(|hit| hit.entry.kind == CatalogKind::ForeignKey)
                .unwrap()
                .entry
                .metadata,
            CatalogMetadata::Constraint(ConstraintMetadata::ForeignKey {
                columns,
                referenced_relation,
                referenced_columns,
            }) if columns == &["parent_id"]
                && referenced_relation.schema.as_deref() == Some(schema.as_str())
                && referenced_relation.object == format!("parent_{token}")
                && referenced_columns == &["id"]
        ));

        let literal = adapter
            .search_catalog(&catalog_search_request(profile_id, "%_", scope.clone(), 10))
            .await
            .unwrap();
        assert!(literal.hits.iter().any(|hit| hit.entry.qualified_name.object == "literal%_name"));
        assert!(literal.hits.iter().filter(|hit| hit.entry.kind == CatalogKind::Table).all(
            |hit| hit.entry.qualified_name.object.contains("%_")
        ));

        let ordered = adapter
            .search_catalog(&catalog_search_request(profile_id, &token, scope.clone(), 100))
            .await
            .unwrap();
        let table_names = ordered
            .hits
            .iter()
            .filter(|hit| hit.entry.kind == CatalogKind::Table)
            .map(|hit| hit.entry.qualified_name.object.as_str())
            .collect::<Vec<_>>();
        assert!(table_names.iter().position(|name| *name == exact).unwrap()
            < table_names.iter().position(|name| *name == prefix).unwrap());
        assert!(table_names.iter().position(|name| *name == prefix).unwrap()
            < table_names.iter().position(|name| *name == contains).unwrap());
        let repeated = adapter
            .search_catalog(&catalog_search_request(profile_id, &token, scope.clone(), 100))
            .await
            .unwrap();
        assert_eq!(
            ordered.hits.iter().map(|hit| &hit.entry.id).collect::<Vec<_>>(),
            repeated.hits.iter().map(|hit| &hit.entry.id).collect::<Vec<_>>()
        );

        let limited = adapter
            .search_catalog(&catalog_search_request(profile_id, &token, scope.clone(), 1))
            .await
            .unwrap();
        assert_eq!(limited.hits.len(), 1);
        assert!(limited.truncated);
        assert_eq!(limited.total_count, None);
        assert!(ordered.hits.iter().all(|hit| {
            hit.entry.qualified_name.schema.as_deref() != Some(excluded_schema.as_str())
        }));
        adapter.close().await;
    })
    .catch_unwind()
    .await;

    let mut cleanup_errors = Vec::new();
    for schema in [&quoted_schema, &quoted_excluded] {
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
            eprintln!("PostgreSQL search fixture cleanup failed after body panic: {error}");
        }
        std::panic::resume_unwind(panic);
    }
    assert!(
        cleanup_errors.is_empty(),
        "PostgreSQL search fixture cleanup failed: {cleanup_errors:?}"
    );
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
            CREATE FUNCTION {quoted_schema}.child_audit() RETURNS trigger LANGUAGE plpgsql AS 'BEGIN RETURN NEW; END';
            CREATE TRIGGER child_audit BEFORE UPDATE ON {quoted_schema}.child
                FOR EACH ROW EXECUTE FUNCTION {quoted_schema}.child_audit();
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

        let ddl = database.relation_ddl(&child).await.unwrap();
        assert_eq!(ddl.children.entries, children.entries);
        assert_eq!(ddl.provenance, lazydb::db::catalog::DdlProvenance::AdapterGenerated);
        assert!(ddl.sql.contains("-- Table\n\nCREATE TABLE"));
        assert!(ddl.sql.contains("\"id\" bigint GENERATED BY DEFAULT AS IDENTITY"));
        assert!(ddl.sql.contains("\"code\" character varying(40) DEFAULT 'new'::character varying NOT NULL"));
        assert!(ddl.sql.contains("\"code_upper\" text GENERATED ALWAYS AS"));
        assert!(ddl.sql.contains("upper"));
        assert!(ddl.sql.contains("code"));
        assert!(ddl.sql.contains("CONSTRAINT \"child_pk\" PRIMARY KEY"));
        assert!(ddl.sql.contains("CONSTRAINT \"child_parent_fk\" FOREIGN KEY"));
        assert!(ddl.sql.contains("-- Comments"));
        assert!(ddl.sql.contains("COMMENT ON TABLE"));
        assert!(ddl.sql.contains("COMMENT ON COLUMN"));
        assert!(ddl.sql.contains("-- Indexes"));
        assert!(ddl.sql.contains("CREATE INDEX child_owner_code_idx"));
        assert!(!ddl.sql.contains("CREATE UNIQUE INDEX child_pk"));
        assert!(!ddl.sql.contains("CREATE UNIQUE INDEX child_tenant_code_key"));
        assert!(ddl.sql.contains("-- Triggers"));
        assert!(ddl.sql.contains("CREATE TRIGGER child_audit"));

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
