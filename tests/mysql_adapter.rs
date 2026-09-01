use std::{collections::HashSet, panic::AssertUnwindSafe};

use futures_util::FutureExt;
use lazydb::{
    db::{
        DatabaseConnection, ErrorCategory,
        catalog::{
            CatalogCapabilities, CatalogCompleteness, CatalogCount, CatalogCursor, CatalogEntry,
            CatalogId, CatalogKind, CatalogMetadata, CatalogRequest, CatalogRequestKey,
            CatalogSearchRequest, CatalogTarget, ColumnMetadata, ColumnMetadataCapabilities,
            ConstraintMembership, ConstraintMetadata, DdlProvenance, IndexMetadata, NamespaceModel,
            ObjectGroup, OptionalMetadata,
        },
        mysql::{self, MySqlAdapter},
        value::CellValue,
    },
    identity::ConnectionIdentity,
    profile::{CatalogScope, CatalogSelection, DatabaseScope, import_connection_url},
};
use uuid::Uuid;

#[test]
fn mysql_catalog_capabilities_are_truthful_before_lazy_pages() {
    assert_eq!(
        MySqlAdapter::catalog_capabilities(),
        CatalogCapabilities {
            namespace_model: NamespaceModel::DatabaseIsSchema,
            top_level_groups: vec![
                ObjectGroup::Tables,
                ObjectGroup::Views,
                ObjectGroup::Functions,
                ObjectGroup::Procedures,
                ObjectGroup::Triggers,
            ],
            column_metadata: ColumnMetadataCapabilities {
                type_family: true,
                default_expression: true,
                auto_increment: true,
                generated_expression: true,
                numeric_precision_and_scale: true,
                character_length: true,
                collation: true,
                character_set: true,
                comment: true,
                ..ColumnMetadataCapabilities::default()
            },
            supports_lazy_children: true,
        }
    );
}

#[test]
fn quotes_mysql_identifiers_and_uses_information_schema() {
    assert_eq!(mysql::quote_identifier("odd`name"), "`odd``name`");
    assert!(mysql::CATALOG_TABLES_SQL.contains("information_schema.tables"));
    assert!(mysql::CATALOG_INDEXES_SQL.contains("information_schema.statistics"));
    assert!(mysql::CATALOG_PAGE_INDEXES_SQL.contains("information_schema.statistics"));
    assert!(mysql::CATALOG_PAGE_INDEXES_SQL.contains("expression"));
    assert!(mysql::CATALOG_ROUTINES_SQL.contains("information_schema.routines"));
    for sql in [
        mysql::CATALOG_TABLES_SQL,
        mysql::CATALOG_INDEXES_SQL,
        mysql::CATALOG_ROUTINES_SQL,
    ] {
        assert!(!sql.contains("DATABASE()"));
    }
    assert_eq!(
        mysql::CATALOG_PAGE_BEGIN_SQL,
        "START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY"
    );
    assert!(mysql::CATALOG_PAGE_BEGIN_SQL.starts_with("START TRANSACTION "));
    assert!(
        mysql::CATALOG_PAGE_BEGIN_SQL.find("WITH CONSISTENT SNAPSHOT")
            < mysql::CATALOG_PAGE_BEGIN_SQL.find("READ ONLY")
    );
}

#[test]
fn mysql_catalog_search_sql_pushes_literal_matching_ranking_scope_and_bound() {
    let sql = mysql::CATALOG_SEARCH_CANDIDATES_SQL;
    assert!(sql.contains("information_schema.schemata"));
    assert!(sql.contains("information_schema.tables"));
    assert!(sql.contains("information_schema.routines"));
    assert!(sql.contains("information_schema.triggers"));
    assert!(sql.contains("information_schema.columns"));
    assert!(sql.contains("information_schema.statistics"));
    assert!(sql.contains("information_schema.table_constraints"));
    assert!(sql.contains("{scope_predicate}"));
    assert!(sql.contains("REGEXP_REPLACE(LOWER(object_name), '[^[:alnum:]]', '')"));
    assert!(sql.contains("REGEXP_REPLACE(LOWER(qualified_path), '[^[:alnum:]]', '')"));
    assert!(sql.contains("IF(?, normalized_name, LOWER(object_name)) AS search_name"));
    assert!(sql.contains("WHEN search_name=? THEN 0"));
    assert!(sql.contains("LIMIT 101"));
    assert!(!sql.to_ascii_lowercase().contains(" like "));
    for unsupported in ["materialized", "sequence", "check_constraint", "type'"] {
        assert!(!sql.to_ascii_lowercase().contains(unsupported));
    }
}

#[test]
fn mysql_catalog_requires_oracle_mysql_8_0_13() {
    assert!(!mysql::supports_catalog_version("5.7.44"));
    assert!(!mysql::supports_catalog_version("8.0.12"));
    assert!(mysql::supports_catalog_version("8.0.13"));
    assert!(mysql::supports_catalog_version("8.4.1-commercial"));
    assert!(!mysql::supports_catalog_version("10.1.48-MariaDB"));
    assert!(!mysql::supports_catalog_version("10.11.8-MariaDB"));
    assert!(!mysql::supports_catalog_version("5.5.5-10.11.8-MariaDB"));
}

#[test]
fn mysql_rejects_noncanonical_mirrored_schema_scope_without_io() {
    let scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: "app".to_owned(),
            schemas: CatalogSelection::Selected(vec!["app".to_owned()]),
        }]),
    };
    let error = mysql::validate_catalog_scope(&scope).unwrap_err();
    assert_eq!(error.category, ErrorCategory::Configuration);
    assert_eq!(error.code.as_deref(), Some("invalid_catalog_request"));
    assert!(error.message.contains("schemas must use All"));
}

#[test]
fn create_database_permission_detection_prefers_stable_mysql_codes() {
    for code in ["1044", "1045", "1142", "1227"] {
        assert!(is_create_database_permission_denial(
            &lazydb::db::DatabaseError {
                category: ErrorCategory::Sql,
                code: Some(code.to_owned()),
                message: "localized server message".to_owned(),
            }
        ));
    }
    assert!(!is_create_database_permission_denial(
        &lazydb::db::DatabaseError {
            category: ErrorCategory::Sql,
            code: Some("1064".to_owned()),
            message: "syntax error".to_owned(),
        }
    ));
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
    assert_eq!(
        database.catalog_capabilities(),
        MySqlAdapter::catalog_capabilities()
    );
    let discovery = database.discover_catalog_scope().await.unwrap();
    assert!(
        discovery
            .databases
            .iter()
            .all(|database| database.schemas == [database.name.clone()])
    );
    assert!(discovery.databases.iter().all(|database| !matches!(
        database.name.as_str(),
        "information_schema" | "mysql" | "performance_schema" | "sys"
    )));
    assert!(
        discovery
            .databases
            .windows(2)
            .all(|databases| databases[0].name <= databases[1].name)
    );
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
    database.close().await;
}

fn selected_scope(databases: &[&str]) -> CatalogScope {
    CatalogScope {
        databases: CatalogSelection::Selected(
            databases
                .iter()
                .map(|database| DatabaseScope {
                    name: (*database).to_owned(),
                    schemas: CatalogSelection::All,
                })
                .collect(),
        ),
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
async fn catalog_page_exposes_scoped_mysql_objects_and_rich_metadata_when_configured() {
    let Ok(url) = std::env::var("LAZYDB_TEST_MYSQL_URL") else {
        return;
    };
    let imported = import_connection_url(&url, Some("mysql-catalog-pages")).unwrap();
    let profile_id = imported.profile.id;
    let database =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let configured_database = database.probe().await.unwrap().database;
    let lower_case_table_names = mysql_integer(&database, "SELECT @@lower_case_table_names").await;
    assert!(
        !configured_database.is_empty(),
        "LAZYDB_TEST_MYSQL_URL must select a database"
    );
    let suffix = Uuid::new_v4().simple().to_string();
    let requested_database = format!("lazydb_{suffix}");
    let excluded_database = format!("lazydb_excluded_{suffix}");
    let database_fixture = create_database_fixture(
        &database,
        &requested_database,
        &excluded_database,
        &configured_database,
    )
    .await;
    let selected_database = database_fixture
        .selected_database(&requested_database)
        .to_owned();
    let prefix = format!("lazydb_{suffix}_");
    let parent = format!("{prefix}parent");
    let child = format!("{prefix}child");
    let second = format!("{prefix}literal%second");
    let view = format!("{prefix}child_view");
    let function = format!("{prefix}do_work");
    let procedure = format!("{prefix}run_work");
    let trigger = format!("{prefix}child_before_insert");
    let second_trigger = format!("{prefix}child_after_update");
    let excluded_object = format!("{prefix}hidden_table");
    let qdb = mysql::quote_identifier(&selected_database);
    let qparent = mysql::quote_identifier(&parent);
    let qchild = mysql::quote_identifier(&child);
    let qsecond = mysql::quote_identifier(&second);
    let qview = mysql::quote_identifier(&view);
    let qfunction = mysql::quote_identifier(&function);
    let qprocedure = mysql::quote_identifier(&procedure);
    let qtrigger = mysql::quote_identifier(&trigger);
    let qsecond_trigger = mysql::quote_identifier(&second_trigger);
    let qexcluded = mysql::quote_identifier(&excluded_object);
    let scope = selected_scope(&[&selected_database]);
    let database_id = CatalogId::new(
        profile_id,
        CatalogKind::Database,
        [selected_database.clone()],
    );
    let schema_id = CatalogId::new(
        profile_id,
        CatalogKind::Schema,
        [selected_database.clone(), selected_database.clone()],
    );

    let result = AssertUnwindSafe(async {
        let baseline_groups = database
            .load_catalog_page(&catalog_request(
                profile_id,
                CatalogTarget::groups(schema_id.clone()).unwrap(),
                scope.clone(),
                10,
                None,
                0,
            ))
            .await
            .unwrap();
        let baseline_counts = baseline_groups
            .group_summaries
            .iter()
            .map(|summary| (summary.group, summary.object_count))
            .collect::<std::collections::HashMap<_, _>>();
        if database_fixture.created_databases() {
            assert!(baseline_counts.values().all(|count| *count == CatalogCount::Exact(0)));
        }
        database
            .execute(&format!(
                "CREATE TABLE {qdb}.{qparent} (tenant_id INT NOT NULL, parent_id INT NOT NULL, CONSTRAINT {prefix}parent_pk PRIMARY KEY (tenant_id, parent_id)); \
                 CREATE TABLE {qdb}.{qchild} (id BIGINT NOT NULL AUTO_INCREMENT, tenant_id INT NOT NULL, owner_id INT NOT NULL, code VARCHAR(40) NOT NULL DEFAULT 'new' COMMENT 'code column comment', created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, code_upper VARCHAR(40) GENERATED ALWAYS AS (UPPER(code)) STORED, CONSTRAINT {prefix}child_pk PRIMARY KEY (id, tenant_id), CONSTRAINT {prefix}child_tenant_code_uq UNIQUE (tenant_id, code), CONSTRAINT {prefix}child_parent_fk FOREIGN KEY (tenant_id, owner_id) REFERENCES {qdb}.{qparent} (tenant_id, parent_id), INDEX {prefix}child_owner_code_idx (owner_id, code), INDEX {prefix}child_code_lower_idx ((LOWER(code)))) COMMENT='child table comment'; \
                 CREATE TABLE {qdb}.{qsecond} (id INT PRIMARY KEY); \
                 CREATE VIEW {qdb}.{qview} AS SELECT tenant_id, id, code FROM {qdb}.{qchild}; \
                 CREATE FUNCTION {qdb}.{qfunction}(value INT) RETURNS INT DETERMINISTIC RETURN value + 1; \
                 CREATE PROCEDURE {qdb}.{qprocedure}() SELECT 1; \
                 CREATE TRIGGER {qdb}.{qtrigger} BEFORE INSERT ON {qdb}.{qchild} FOR EACH ROW SET NEW.code = COALESCE(NEW.code, 'new'); \
                 CREATE TRIGGER {qdb}.{qsecond_trigger} AFTER UPDATE ON {qdb}.{qchild} FOR EACH ROW SET @lazydb_last_updated_id = NEW.id"
            ))
            .await
            .unwrap();
        if database_fixture.created_databases() {
            let excluded = mysql::quote_identifier(&excluded_database);
            database
                .execute(&format!("CREATE TABLE {excluded}.{qexcluded} (id INT PRIMARY KEY)"))
                .await
                .unwrap();
        } else {
            eprintln!(
                "skipping excluded-database object fixture: CREATE DATABASE permission denied"
            );
        }

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
        assert_eq!(databases.entries[0].id, database_id);
        if database_fixture.created_databases() {
            assert_ne!(databases.entries[0].qualified_name.object, excluded_database);
        } else {
            eprintln!(
                "skipping cross-database visibility assertions: CREATE DATABASE permission denied"
            );
        }

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
        assert_eq!(schemas.entries[0].id, schema_id);
        assert_eq!(schemas.entries[0].qualified_name.object, selected_database);

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
        let counts = groups
            .group_summaries
            .iter()
            .map(|summary| (summary.group, summary.object_count))
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(groups.total_count, CatalogCount::Exact(5));
        assert_count_delta(
            &baseline_counts,
            &counts,
            ObjectGroup::Tables,
            3,
        );
        assert_count_delta(&baseline_counts, &counts, ObjectGroup::Views, 1);
        assert_count_delta(&baseline_counts, &counts, ObjectGroup::Functions, 1);
        assert_count_delta(&baseline_counts, &counts, ObjectGroup::Procedures, 1);
        assert_count_delta(&baseline_counts, &counts, ObjectGroup::Triggers, 2);

        let table_target = CatalogTarget::objects(schema_id.clone(), ObjectGroup::Tables).unwrap();
        let (table_names, table_ids, completeness) = collect_object_pages(
            &database,
            profile_id,
            &scope,
            &table_target,
            10,
        )
        .await;
        assert!(table_names.contains(&parent));
        assert!(table_names.contains(&child));
        assert!(table_names.contains(&second));
        assert_eq!(table_ids.iter().collect::<HashSet<_>>().len(), table_ids.len());
        assert_eq!(completeness.last(), Some(&CatalogCompleteness::Complete));
        assert!(completeness[..completeness.len() - 1].iter().all(|value| *value == CatalogCompleteness::Partial));
        let (_, repeated_ids, _) = collect_object_pages(
            &database,
            profile_id,
            &scope,
            &table_target,
            30,
        )
        .await;
        assert_eq!(table_ids, repeated_ids);

        for (group, kind, expected_name) in [
            (ObjectGroup::Views, CatalogKind::View, view.as_str()),
            (
                ObjectGroup::Functions,
                CatalogKind::Function,
                function.as_str(),
            ),
            (
                ObjectGroup::Procedures,
                CatalogKind::Procedure,
                procedure.as_str(),
            ),
            (
                ObjectGroup::Triggers,
                CatalogKind::Trigger,
                trigger.as_str(),
            ),
        ] {
            let (entries, completeness) = collect_entry_pages(
                &database,
                profile_id,
                &scope,
                &CatalogTarget::objects(schema_id.clone(), group).unwrap(),
                50 + kind as u64,
            )
            .await;
            let (repeated_entries, repeated_completeness) = collect_entry_pages(
                &database,
                profile_id,
                &scope,
                &CatalogTarget::objects(schema_id.clone(), group).unwrap(),
                60 + kind as u64,
            )
            .await;
            assert_eq!(completeness.last(), Some(&CatalogCompleteness::Complete));
            assert!(
                completeness[..completeness.len() - 1]
                    .iter()
                    .all(|value| *value == CatalogCompleteness::Partial)
            );
            assert_eq!(completeness, repeated_completeness);
            assert_eq!(
                entries.iter().map(|entry| &entry.id).collect::<Vec<_>>(),
                repeated_entries
                    .iter()
                    .map(|entry| &entry.id)
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                entries.iter().map(|entry| &entry.id).collect::<HashSet<_>>().len(),
                entries.len()
            );
            let entry = entries
                .iter()
                .find(|entry| entry.qualified_name.object == expected_name)
                .unwrap_or_else(|| panic!("fixture {kind:?} was not returned"));
            assert_eq!(entry.kind, kind);
            if matches!(kind, CatalogKind::Function | CatalogKind::Procedure) {
                assert_eq!(entry.id.native_path.len(), 4);
            }
            if kind == CatalogKind::Trigger {
                assert_eq!(entry.parent_id.as_ref(), Some(&schema_id));
                assert_eq!(entry.relation_id.as_ref().unwrap().native_path[2], child);
            }
        }

        let child_id = table_ids
            .iter()
            .find(|id| id.native_path[2] == child)
            .unwrap()
            .clone();
        let children_target = CatalogTarget::relation_children(child_id.clone()).unwrap();
        let children = database
            .load_catalog_page(&catalog_request(
                profile_id,
                children_target.clone(),
                scope.clone(),
                100,
                None,
                70,
            ))
            .await
            .unwrap();
        assert_eq!(children.total_count, CatalogCount::Exact(14));
        assert_eq!(children.entries.len(), 14);
        let columns = children
            .entries
            .iter()
            .filter_map(|entry| match &entry.metadata {
                CatalogMetadata::Column(metadata) => {
                    Some((entry.qualified_name.object.as_str(), (entry, metadata)))
                }
                _ => None,
            })
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(columns.len(), 6);
        let (code_entry, code) = columns["code"];
        assert_eq!(code.native_type.to_ascii_lowercase(), "varchar(40)");
        assert!(!code.nullable);
        assert_eq!(code.default_expression, OptionalMetadata::Supported(Some("new".to_owned())));
        assert_eq!(code.auto_increment, OptionalMetadata::Supported(Some(false)));
        assert_eq!(code.generated_expression, OptionalMetadata::Supported(None));
        assert_eq!(code.character_maximum_length, OptionalMetadata::Supported(Some(40)));
        assert!(matches!(code.collation, OptionalMetadata::Supported(Some(_))));
        assert!(matches!(code.character_set, OptionalMetadata::Supported(Some(_))));
        assert_eq!(code_entry.comment, OptionalMetadata::Supported(Some("code column comment".to_owned())));
        assert_eq!(columns["id"].1.auto_increment, OptionalMetadata::Supported(Some(true)));
        assert!(matches!(columns["code_upper"].1.generated_expression, OptionalMetadata::Supported(Some(ref expression)) if expression.to_ascii_lowercase().contains("upper")));
        assert_eq!(columns["code_upper"].1.default_expression, OptionalMetadata::Supported(None));
        let created_at = columns["created_at"].1;
        assert!(matches!(
            &created_at.default_expression,
            OptionalMetadata::Supported(Some(value)) if value.eq_ignore_ascii_case("CURRENT_TIMESTAMP")
        ));
        assert_eq!(created_at.generated_expression, OptionalMetadata::Supported(None));

        let indexes = children
            .entries
            .iter()
            .filter_map(|entry| match &entry.metadata {
                CatalogMetadata::Index(metadata) => Some((entry, metadata)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(indexes.len(), 5, "one entry per native MySQL index");
        let (_, composite_index) = indexes
            .iter()
            .find(|(entry, _)| entry.qualified_name.object == format!("{prefix}child_owner_code_idx"))
            .unwrap();
        assert_eq!(
            **composite_index,
            IndexMetadata {
                columns: vec!["owner_id".to_owned(), "code".to_owned()],
                unique: false,
            }
        );
        let (_, functional_index) = indexes
            .iter()
            .find(|(entry, _)| entry.qualified_name.object == format!("{prefix}child_code_lower_idx"))
            .unwrap();
        assert_eq!(functional_index.columns.len(), 1);
        let expression = functional_index.columns[0].to_ascii_lowercase();
        assert!(expression.contains("lower"));
        assert!(expression.contains("code"));
        let primary = one_constraint(&children.entries, CatalogKind::PrimaryKey);
        let unique = one_constraint(&children.entries, CatalogKind::UniqueConstraint);
        let foreign = one_constraint(&children.entries, CatalogKind::ForeignKey);
        assert_eq!(primary.metadata, CatalogMetadata::Constraint(ConstraintMetadata::PrimaryKey { columns: vec!["id".to_owned(), "tenant_id".to_owned()] }));
        assert_eq!(unique.metadata, CatalogMetadata::Constraint(ConstraintMetadata::Unique { columns: vec!["tenant_id".to_owned(), "code".to_owned()] }));
        assert_eq!(foreign.metadata, CatalogMetadata::Constraint(ConstraintMetadata::ForeignKey {
            columns: vec!["tenant_id".to_owned(), "owner_id".to_owned()],
            referenced_relation: lazydb::db::catalog::QualifiedName { database: Some(selected_database.clone()), schema: Some(selected_database.clone()), object: parent.clone() },
            referenced_columns: vec!["tenant_id".to_owned(), "parent_id".to_owned()],
        }));
        assert_membership(columns["id"].1, &primary.id, 1);
        assert_membership(columns["tenant_id"].1, &primary.id, 2);
        assert_membership(columns["tenant_id"].1, &unique.id, 1);
        assert_membership(columns["tenant_id"].1, &foreign.id, 1);
        assert_membership(columns["code"].1, &unique.id, 2);
        assert_membership(columns["owner_id"].1, &foreign.id, 2);

        let repeated = database.load_catalog_page(&catalog_request(profile_id, children_target, scope.clone(), 100, None, 71)).await.unwrap();
        assert_eq!(children.entries.iter().map(|entry| &entry.id).collect::<Vec<_>>(), repeated.entries.iter().map(|entry| &entry.id).collect::<Vec<_>>());

        let ddl = database.relation_ddl(&child_id).await.unwrap();
        assert_eq!(ddl.children.entries, children.entries);
        assert_eq!(ddl.provenance, DdlProvenance::AdapterGenerated);
        assert!(ddl.sql.starts_with("-- Object\n\nCREATE TABLE"));
        assert_eq!(ddl.sql.matches("CREATE TABLE").count(), 1);
        assert_eq!(ddl.sql.matches(&format!("{prefix}child_owner_code_idx")).count(), 1);
        let triggers_section = ddl.sql.find("\n\n-- Triggers\n\n").unwrap();
        assert!(ddl.sql[..triggers_section].contains("COMMENT='child table comment'"));
        assert!(ddl.sql[triggers_section..].find(&second_trigger).unwrap()
            < ddl.sql[triggers_section..].find(&trigger).unwrap());
        assert_eq!(ddl.sql.matches("CREATE DEFINER=").count(), 2);
        assert!(!ddl.sql.contains("character_set_client"));
        assert!(!ddl.sql.contains("collation_connection"));

        let view_id = CatalogId::new(
            profile_id,
            CatalogKind::View,
            [selected_database.clone(), selected_database.clone(), view.clone()],
        );
        let view_ddl = database.relation_ddl(&view_id).await.unwrap();
        assert_eq!(view_ddl.provenance, DdlProvenance::NativeCatalog);
        assert!(view_ddl.sql.starts_with("-- Object\n\nCREATE"));
        assert!(!view_ddl.sql.contains("-- Triggers"));

        let DatabaseConnection::MySql(adapter) = &database else {
            unreachable!("MySQL fixture returned another adapter")
        };
        let search_request = CatalogSearchRequest {
            connection: ConnectionIdentity { profile_id, generation: 7 },
            session_id: 11,
            generation: 13,
            query: prefix.clone(),
            scope: scope.clone(),
            limit: 100,
        };
        let search = adapter.search_catalog(&search_request).await.unwrap();
        search.validate_for(&search_request).unwrap();
        assert_eq!(search.connection, search_request.connection);
        assert_eq!(search.session_id, 11);
        assert_eq!(search.generation, 13);
        assert_eq!(search.total_count, None);
        for kind in [
            CatalogKind::Table,
            CatalogKind::View,
            CatalogKind::Function,
            CatalogKind::Procedure,
            CatalogKind::Trigger,
            CatalogKind::Column,
            CatalogKind::Index,
            CatalogKind::PrimaryKey,
            CatalogKind::UniqueConstraint,
            CatalogKind::ForeignKey,
        ] {
            assert!(search.hits.iter().any(|hit| hit.entry.kind == kind), "missing search kind {kind:?}");
        }
        let trigger_hit = search.hits.iter().find(|hit| hit.entry.kind == CatalogKind::Trigger).unwrap();
        assert_eq!(trigger_hit.entry.parent_id.as_ref(), Some(&schema_id));
        assert_eq!(trigger_hit.ancestors.len(), 3);
        assert_eq!(trigger_hit.ancestors[2].id, *trigger_hit.entry.relation_id.as_ref().unwrap());
        let routine_hit = search.hits.iter().find(|hit| hit.entry.kind == CatalogKind::Function).unwrap();
        assert_eq!(routine_hit.entry.id.native_path.len(), 4);
        let child_hit = search.hits.iter().find(|hit| hit.entry.kind == CatalogKind::Column).unwrap();
        assert!(!matches!(child_hit.entry.metadata, CatalogMetadata::None));
        assert_eq!(child_hit.ancestors.len(), 3);

        let namespace_request = CatalogSearchRequest {
            query: selected_database.to_ascii_uppercase(),
            ..search_request.clone()
        };
        let namespaces = adapter.search_catalog(&namespace_request).await.unwrap();
        let namespace_hits = namespaces
            .hits
            .iter()
            .filter(|hit| matches!(hit.entry.kind, CatalogKind::Database | CatalogKind::Schema))
            .collect::<Vec<_>>();
        assert_eq!(namespace_hits.len(), 2);
        assert_eq!(namespace_hits[0].entry.kind, CatalogKind::Database);
        assert_eq!(namespace_hits[1].entry.kind, CatalogKind::Schema);
        assert_eq!(namespace_hits[0].qualified_path(), selected_database);
        assert_eq!(namespace_hits[1].qualified_path(), selected_database);
        assert_ne!(namespace_hits[0].entry.id, namespace_hits[1].entry.id);
        if database_fixture.created_databases() {
            let excluded_request = CatalogSearchRequest {
                query: excluded_database.clone(),
                ..search_request.clone()
            };
            assert!(adapter.search_catalog(&excluded_request).await.unwrap().hits.is_empty());
        }

        let literal_request = CatalogSearchRequest {
            query: "%second".to_owned(),
            limit: 100,
            ..search_request.clone()
        };
        let literal = adapter.search_catalog(&literal_request).await.unwrap();
        assert!(!literal.hits.is_empty());
        assert!(literal.hits.iter().all(|hit| hit.qualified_path().to_lowercase().contains("%second")));
        assert!(literal.hits.iter().any(|hit| hit.entry.qualified_name.object == second));

        let limited_request = CatalogSearchRequest { limit: 1, ..search_request.clone() };
        let limited = adapter.search_catalog(&limited_request).await.unwrap();
        assert_eq!(limited.hits.len(), 1);
        assert!(limited.truncated);
        assert_eq!(limited.hits[0].entry.kind, CatalogKind::Table);

        let wrong_profile = CatalogSearchRequest {
            connection: ConnectionIdentity { profile_id: Uuid::new_v4(), generation: 7 },
            ..search_request.clone()
        };
        let error = adapter.search_catalog(&wrong_profile).await.unwrap_err();
        assert_eq!(error.code.as_deref(), Some("invalid_catalog_request"));

        let wrong_case = selected_database.to_ascii_uppercase();
        // Mode 0 is the only mode where wrong-case rejection reflects server lookup truth.
        if lower_case_table_names == 0 && wrong_case != selected_database {
            let wrong_schema = CatalogId::new(profile_id, CatalogKind::Schema, [wrong_case.clone(), wrong_case.clone()]);
            let error = database.load_catalog_page(&catalog_request(profile_id, CatalogTarget::groups(wrong_schema).unwrap(), selected_scope(&[&wrong_case]), 10, None, 80)).await.unwrap_err();
            assert_eq!(error.code.as_deref(), Some("catalog_target_not_found"));
        }
        let wrong_kind = CatalogId::new(profile_id, CatalogKind::View, [selected_database.clone(), selected_database.clone(), child.clone()]);
        let error = database.load_catalog_page(&catalog_request(profile_id, CatalogTarget::relation_children(wrong_kind).unwrap(), scope.clone(), 10, None, 81)).await.unwrap_err();
        assert_eq!(error.code.as_deref(), Some("catalog_target_not_found"));
        let trailing_relation = CatalogId::new(
            profile_id,
            CatalogKind::Table,
            [
                selected_database.clone(),
                selected_database.clone(),
                child.clone(),
                "forged_suffix".to_owned(),
            ],
        );
        let error = database.load_catalog_page(&catalog_request(
            profile_id,
            CatalogTarget::relation_children(trailing_relation).unwrap(),
            scope.clone(),
            10,
            None,
            82,
        )).await.unwrap_err();
        assert_eq!(error.code.as_deref(), Some("catalog_target_not_found"));
        let unsupported = database.load_catalog_page(&catalog_request(profile_id, CatalogTarget::objects(schema_id, ObjectGroup::MaterializedViews).unwrap(), scope.clone(), 10, None, 82)).await.unwrap_err();
        assert_eq!(unsupported.category, ErrorCategory::Unsupported);
        let malformed = database.load_catalog_page(&catalog_request(profile_id, CatalogTarget::Databases, scope, 0, None, 83)).await.unwrap_err();
        assert_eq!(malformed.category, ErrorCategory::Configuration);
        assert_eq!(malformed.code.as_deref(), Some("invalid_catalog_request"));
    })
    .catch_unwind()
    .await;

    let mut cleanup_errors = Vec::new();
    for statement in [
        format!("DROP TRIGGER IF EXISTS {qdb}.{qsecond_trigger}"),
        format!("DROP TRIGGER IF EXISTS {qdb}.{qtrigger}"),
        format!("DROP PROCEDURE IF EXISTS {qdb}.{qprocedure}"),
        format!("DROP FUNCTION IF EXISTS {qdb}.{qfunction}"),
        format!("DROP VIEW IF EXISTS {qdb}.{qview}"),
        format!(
            "DROP TABLE IF EXISTS {}.{qexcluded}",
            mysql::quote_identifier(if database_fixture.created_databases() {
                &excluded_database
            } else {
                &selected_database
            })
        ),
        format!("DROP TABLE IF EXISTS {qdb}.{qchild}"),
        format!("DROP TABLE IF EXISTS {qdb}.{qsecond}"),
        format!("DROP TABLE IF EXISTS {qdb}.{qparent}"),
    ] {
        if let Err(error) = database.execute(&statement).await {
            cleanup_errors.push(error);
        }
    }
    if database_fixture.created_databases() {
        for name in [&requested_database, &excluded_database] {
            if let Err(error) = database
                .execute(&format!(
                    "DROP DATABASE IF EXISTS {}",
                    mysql::quote_identifier(name)
                ))
                .await
            {
                cleanup_errors.push(error);
            }
        }
    }
    database.close().await;
    if let Err(panic) = result {
        for error in cleanup_errors {
            eprintln!("MySQL fixture cleanup failed after body panic: {error}");
        }
        std::panic::resume_unwind(panic);
    }
    assert!(
        cleanup_errors.is_empty(),
        "MySQL fixture cleanup failed: {cleanup_errors:?}"
    );
}

enum DatabaseFixture {
    Created,
    Current(String),
}

impl DatabaseFixture {
    fn selected_database<'a>(&'a self, created: &'a str) -> &'a str {
        match self {
            Self::Created => created,
            Self::Current(current) => current,
        }
    }

    fn created_databases(&self) -> bool {
        matches!(self, Self::Created)
    }
}

async fn create_database_fixture(
    database: &DatabaseConnection,
    selected: &str,
    excluded: &str,
    current: &str,
) -> DatabaseFixture {
    let create_selected = format!("CREATE DATABASE {}", mysql::quote_identifier(selected));
    match database.execute(&create_selected).await {
        Ok(_) => {
            if let Err(error) = database
                .execute(&format!(
                    "CREATE DATABASE {}",
                    mysql::quote_identifier(excluded)
                ))
                .await
            {
                let cleanup = database
                    .execute(&format!(
                        "DROP DATABASE IF EXISTS {}",
                        mysql::quote_identifier(selected)
                    ))
                    .await;
                panic!(
                    "creating excluded fixture database failed: {error}; first database cleanup: {cleanup:?}"
                );
            }
            DatabaseFixture::Created
        }
        Err(error) if is_create_database_permission_denial(&error) => {
            eprintln!("CREATE DATABASE denied; using configured database fixture: {error}");
            DatabaseFixture::Current(current.to_owned())
        }
        Err(error) => panic!("CREATE DATABASE failed for a non-permission reason: {error}"),
    }
}

fn is_create_database_permission_denial(error: &lazydb::db::DatabaseError) -> bool {
    error.category == ErrorCategory::Permission
        || matches!(
            error.code.as_deref(),
            Some("1044" | "1045" | "1142" | "1227")
        )
        || error.message.to_ascii_lowercase().contains("access denied")
}

async fn collect_object_pages(
    database: &DatabaseConnection,
    profile_id: Uuid,
    scope: &CatalogScope,
    target: &CatalogTarget,
    first_request_id: u64,
) -> (Vec<String>, Vec<CatalogId>, Vec<CatalogCompleteness>) {
    let (entries, completeness) =
        collect_entry_pages(database, profile_id, scope, target, first_request_id).await;
    let names = entries
        .iter()
        .map(|entry| entry.qualified_name.object.clone())
        .collect();
    let ids = entries.iter().map(|entry| entry.id.clone()).collect();
    (names, ids, completeness)
}

async fn collect_entry_pages(
    database: &DatabaseConnection,
    profile_id: Uuid,
    scope: &CatalogScope,
    target: &CatalogTarget,
    first_request_id: u64,
) -> (Vec<CatalogEntry>, Vec<CatalogCompleteness>) {
    let mut cursor = None;
    let mut entries = Vec::new();
    let mut completeness = Vec::new();
    loop {
        let page = database
            .load_catalog_page(&catalog_request(
                profile_id,
                target.clone(),
                scope.clone(),
                1,
                cursor,
                first_request_id + entries.len() as u64,
            ))
            .await
            .unwrap();
        assert_eq!(page.entries.len(), 1);
        completeness.push(page.completeness);
        cursor = page.next_cursor;
        entries.extend(page.entries);
        if cursor.is_none() {
            break;
        }
    }
    (entries, completeness)
}

async fn mysql_integer(database: &DatabaseConnection, sql: &str) -> i64 {
    let outcome = database.execute(sql).await.unwrap();
    match outcome.result_sets.last().unwrap().rows[0][0] {
        CellValue::Integer(value) => value,
        CellValue::Unsigned(value) => i64::try_from(value).unwrap(),
        ref value => panic!("expected MySQL integer, found {value:?}"),
    }
}

fn one_constraint(entries: &[CatalogEntry], kind: CatalogKind) -> &CatalogEntry {
    let matches = entries
        .iter()
        .filter(|entry| entry.kind == kind)
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "expected one grouped {kind:?}");
    matches[0]
}

fn assert_count_delta(
    baseline: &std::collections::HashMap<ObjectGroup, CatalogCount>,
    actual: &std::collections::HashMap<ObjectGroup, CatalogCount>,
    group: ObjectGroup,
    expected_delta: u64,
) {
    let CatalogCount::Exact(baseline) = baseline[&group] else {
        panic!("baseline {group:?} count must be exact")
    };
    assert_eq!(
        actual[&group],
        CatalogCount::Exact(baseline + expected_delta)
    );
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
