use lazydb::{
    db::{
        catalog::*, cockroach::CockroachAdapter, query::QueryBudget,
        transaction::TransactionBackend, value::CellValue,
    },
    identity::ConnectionIdentity,
    model::{
        pagination::{PageRequest, PageSize},
        relation::RelationPreviewOptions,
    },
    profile::{CatalogScope, CatalogSelection, DatabaseKind, DatabaseScope, import_connection_url},
};
use uuid::Uuid;

fn test_url() -> Option<String> {
    match std::env::var("LAZYDB_TEST_COCKROACHDB_URL") {
        Ok(url) if !url.trim().is_empty() => Some(url),
        _ if std::env::var_os("LAZYDB_REQUIRE_DATABASE_TESTS").is_some() => {
            panic!("LAZYDB_TEST_COCKROACHDB_URL is required for CockroachDB integration tests")
        }
        _ => {
            eprintln!("SKIP: LAZYDB_TEST_COCKROACHDB_URL is not set");
            None
        }
    }
}

fn request(
    profile: Uuid,
    scope: &CatalogScope,
    target: CatalogTarget,
    size: usize,
    cursor: Option<CatalogCursor>,
) -> CatalogRequest {
    CatalogRequest {
        key: CatalogRequestKey {
            connection: ConnectionIdentity {
                profile_id: profile,
                generation: 1,
            },
            catalog_epoch: 1,
            request_id: 1,
            target,
            cursor,
        },
        scope: scope.clone(),
        page_size: size,
    }
}

#[test]
fn cockroach_catalog_advertises_only_supported_groups() {
    let capabilities = CockroachAdapter::catalog_capabilities();
    assert_eq!(
        capabilities.namespace_model,
        NamespaceModel::DatabaseAndSchema
    );
    assert_eq!(
        capabilities.top_level_groups,
        vec![ObjectGroup::Tables, ObjectGroup::Views]
    );
    assert!(capabilities.supports_lazy_children);
}

#[tokio::test]
async fn cockroach_root_catalog_lists_accessible_databases_and_applies_scope() {
    let Some(url) = test_url() else {
        return;
    };
    let imported = import_connection_url(&url, None).unwrap();
    let adapter =
        CockroachAdapter::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let expected = adapter.discoverable_databases().await.unwrap();
    let mut actual = Vec::new();
    let mut cursor = None;
    loop {
        let page = adapter
            .load_catalog_page(&request(
                imported.profile.id,
                &CatalogScope {
                    databases: CatalogSelection::All,
                },
                CatalogTarget::Databases,
                1,
                cursor,
            ))
            .await
            .unwrap();
        actual.extend(
            page.entries
                .iter()
                .map(|entry| entry.qualified_name.object.clone()),
        );
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(actual, expected);
    let selected = expected.last().unwrap();
    let page = adapter
        .load_catalog_page(&request(
            imported.profile.id,
            &CatalogScope {
                databases: CatalogSelection::Selected(vec![DatabaseScope {
                    name: selected.clone(),
                    schemas: CatalogSelection::All,
                }]),
            },
            CatalogTarget::Databases,
            10,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        page.entries
            .iter()
            .map(|entry| &entry.qualified_name.object)
            .collect::<Vec<_>>(),
        vec![selected]
    );
    adapter.close().await;
}

#[tokio::test]
async fn cockroach_relation_search_limits_follow_qualified_path_order() {
    let Some(url) = test_url() else {
        return;
    };
    let imported = import_connection_url(&url, None).unwrap();
    let profile_id = imported.profile.id;
    let adapter =
        CockroachAdapter::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let database = adapter.probe().await.unwrap().database;
    let suffix = Uuid::new_v4().simple().to_string();
    let schemas = [format!("lazydb_a_{suffix}"), format!("lazydb_z_{suffix}")];
    adapter.execute(&format!("CREATE SCHEMA {}; CREATE SCHEMA {}; CREATE TABLE {}.zmatch(id INT); CREATE TABLE {}.amatch(id INT); CREATE TABLE {}.bmatch(id INT)", schemas[0], schemas[1], schemas[0], schemas[1], schemas[1])).await.unwrap();
    let scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: database,
            schemas: CatalogSelection::Selected(schemas.to_vec()),
        }]),
    };
    for limit in [1, 2, 3] {
        let page = adapter
            .search_catalog(&CatalogSearchRequest {
                connection: ConnectionIdentity {
                    profile_id,
                    generation: 1,
                },
                session_id: 1,
                generation: 1,
                query: "match".into(),
                scope: scope.clone(),
                object_scope: CatalogSearchObjectScope::RelationsOnly,
                limit,
            })
            .await
            .unwrap();
        let names: Vec<_> = page
            .hits
            .iter()
            .map(|hit| hit.entry.qualified_name.object.as_str())
            .collect();
        assert_eq!(names, ["zmatch", "amatch", "bmatch"][..limit]);
    }
    adapter
        .execute(&format!(
            "DROP SCHEMA {} CASCADE; DROP SCHEMA {} CASCADE",
            schemas[0], schemas[1]
        ))
        .await
        .unwrap();
    adapter.close().await;
}

#[tokio::test]
async fn cockroach_catalog_preview_native_ddl_and_stable_identity() {
    let Some(url) = test_url() else {
        return;
    };
    let mut imported = import_connection_url(&url, Some("cockroach-adapter-test")).unwrap();
    imported.profile.kind = DatabaseKind::CockroachDb;
    let profile_id = imported.profile.id;
    let adapter =
        CockroachAdapter::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let info = adapter.probe().await.unwrap();
    assert_eq!(info.kind, DatabaseKind::CockroachDb);
    assert!(info.version.contains("CockroachDB"));
    let database = info.database;
    let schema = format!("lazydb_crdb_{}", Uuid::new_v4().simple());
    let scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: database.clone(),
            schemas: CatalogSelection::Selected(vec![schema.clone()]),
        }]),
    };
    adapter.execute(&format!("CREATE SCHEMA {schema}; CREATE TABLE {schema}.items (id INT PRIMARY KEY, label STRING NOT NULL, amount DECIMAL DEFAULT 1.5); CREATE INDEX items_label ON {schema}.items(label); CREATE TABLE {schema}.other (id INT PRIMARY KEY); CREATE VIEW {schema}.item_view AS SELECT id, label FROM {schema}.items; INSERT INTO {schema}.items SELECT i, 'row-' || i::STRING, i FROM generate_series(1,25) AS g(i)")).await.unwrap();

    let database_id = CatalogId::new(profile_id, CatalogKind::Database, [database.clone()]);
    let schema_id = CatalogId::new(
        profile_id,
        CatalogKind::Schema,
        [database.clone(), schema.clone()],
    );
    let schemas = adapter
        .load_catalog_page(&request(
            profile_id,
            &scope,
            CatalogTarget::Schemas {
                database: database_id,
            },
            10,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(schemas.entries.len(), 1);
    let target = CatalogTarget::Objects {
        schema: schema_id.clone(),
        group: ObjectGroup::Tables,
    };
    let first = adapter
        .load_catalog_page(&request(profile_id, &scope, target.clone(), 1, None))
        .await
        .unwrap();
    assert_eq!(first.entries[0].qualified_name.object, "items");
    let second = adapter
        .load_catalog_page(&request(
            profile_id,
            &scope,
            target,
            1,
            first.next_cursor.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(second.entries[0].qualified_name.object, "other");
    assert!(second.next_cursor.is_none());
    let relation = first.entries[0].id.clone();
    let children = adapter
        .load_catalog_page(&request(
            profile_id,
            &scope,
            CatalogTarget::RelationChildren {
                relation: relation.clone(),
            },
            100,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        children
            .entries
            .iter()
            .filter(|entry| entry.kind == CatalogKind::Column)
            .count(),
        3
    );
    assert!(children.entries.iter().any(|entry| matches!(&entry.metadata, CatalogMetadata::Constraint(ConstraintMetadata::PrimaryKey { columns }) if columns == &["id"])));
    assert!(children.entries.iter().any(|entry| matches!(&entry.metadata, CatalogMetadata::Index(index) if index.columns == ["label"])));
    let preview = adapter
        .preview_relation_with_scope(
            &relation,
            &scope,
            &RelationPreviewOptions {
                where_clause: Some("id > 5".into()),
                order_by_clause: Some("id DESC".into()),
            },
            PageRequest::first(PageSize::Ten),
        )
        .await
        .unwrap();
    assert_eq!(preview.result.result_sets[0].rows.len(), 10);
    assert_eq!(
        preview.result.result_sets[0].rows[0][0],
        CellValue::Integer(25)
    );
    assert!(preview.pagination.has_next);
    assert!(preview.row_versions.is_none());
    assert!(!preview.sql.contains("xmin"));
    let last = adapter
        .preview_relation_with_scope(
            &relation,
            &scope,
            &RelationPreviewOptions::default(),
            PageRequest::last(PageSize::Ten, 0),
        )
        .await
        .unwrap();
    assert_eq!(last.pagination.offset, 20);
    assert_eq!(last.result.result_sets[0].rows.len(), 5);
    let ddl = adapter
        .relation_ddl_with_scope(&relation, &scope)
        .await
        .unwrap();
    assert_eq!(ddl.provenance, DdlProvenance::NativeCatalog);
    assert!(ddl.sql.contains("CREATE TABLE"));
    assert!(ddl.sql.contains("PRIMARY KEY"));
    let view_page = adapter
        .load_catalog_page(&request(
            profile_id,
            &scope,
            CatalogTarget::Objects {
                schema: schema_id,
                group: ObjectGroup::Views,
            },
            10,
            None,
        ))
        .await
        .unwrap();
    assert!(
        adapter
            .relation_ddl_with_scope(&view_page.entries[0].id, &scope)
            .await
            .unwrap()
            .sql
            .contains("CREATE VIEW")
    );
    let search = CatalogSearchRequest {
        connection: ConnectionIdentity {
            profile_id,
            generation: 1,
        },
        session_id: 1,
        generation: 1,
        query: "items".into(),
        scope: scope.clone(),
        object_scope: CatalogSearchObjectScope::RelationsOnly,
        limit: 10,
    };
    let results = adapter.search_catalog(&search).await.unwrap();
    assert!(results.hits.iter().any(|hit| hit.entry.id == relation));
    let child_search = CatalogSearchRequest {
        query: "label".into(),
        object_scope: CatalogSearchObjectScope::AllObjects,
        ..search.clone()
    };
    let results = adapter.search_catalog(&child_search).await.unwrap();
    assert!(
        results
            .hits
            .iter()
            .any(|hit| hit.entry.kind == CatalogKind::Column)
    );
    assert!(
        results
            .hits
            .iter()
            .any(|hit| hit.entry.kind == CatalogKind::Index)
    );

    let excluded = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: database.clone(),
            schemas: CatalogSelection::Selected(vec!["excluded_schema".into()]),
        }]),
    };
    assert!(
        adapter
            .preview_relation_with_scope(
                &relation,
                &excluded,
                &RelationPreviewOptions::default(),
                PageRequest::first(PageSize::Ten)
            )
            .await
            .is_err()
    );
    assert!(
        adapter
            .relation_ddl_with_scope(&relation, &excluded)
            .await
            .is_err()
    );
    assert!(
        adapter
            .resolve_relation_identity_with_scope(&relation, &excluded)
            .await
            .unwrap()
            .is_none()
    );
    let mut foreign = relation.clone();
    foreign.native_path[0] = "other_database".into();
    assert!(
        adapter
            .resolve_relation_identity_with_scope(&foreign, &scope)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        adapter
            .preview_relation_with_scope(
                &foreign,
                &scope,
                &RelationPreviewOptions::default(),
                PageRequest::first(PageSize::Ten)
            )
            .await
            .is_err()
    );
    let mut wrong_profile = relation.clone();
    wrong_profile.connection_id = Uuid::new_v4();
    assert!(
        adapter
            .resolve_relation_identity_with_scope(&wrong_profile, &scope)
            .await
            .unwrap()
            .is_none()
    );
    adapter
        .execute(&format!("DROP VIEW {schema}.item_view"))
        .await
        .unwrap();
    adapter
        .execute(&format!(
            "ALTER TABLE {schema}.items RENAME TO renamed_items"
        ))
        .await
        .unwrap();
    let renamed = adapter
        .resolve_relation_identity_with_scope(&relation, &scope)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(renamed.qualified_name.object, "renamed_items");
    assert_eq!(renamed.id.native_path[3], relation.native_path[3]);
    assert!(
        adapter
            .preview_relation_with_scope(
                &relation,
                &scope,
                &RelationPreviewOptions::default(),
                PageRequest::first(PageSize::Ten)
            )
            .await
            .is_err()
    );
    adapter.execute(&format!("DROP TABLE {schema}.renamed_items; CREATE TABLE {schema}.renamed_items (id INT PRIMARY KEY)")).await.unwrap();
    assert!(
        adapter
            .resolve_relation_identity_with_scope(&renamed.id, &scope)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        adapter
            .relation_ddl_with_scope(&renamed.id, &scope)
            .await
            .is_err()
    );
    adapter
        .execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .unwrap();
    adapter.close().await;
}

#[tokio::test]
async fn cockroach_connection_dispatch_values_read_only_and_agent_service() {
    use async_trait::async_trait;
    use lazydb::{
        agent::{context::AgentProjectContext, service::AgentService},
        db::{DatabaseConnection, principal::PrincipalCapability},
        persistence::{
            credentials::CredentialResolver,
            local_credentials::LocalCredentialStore,
            secrets::{SecretStore, SecretStoreError},
        },
    };
    use secrecy::SecretString;
    use std::sync::Arc;
    struct EmptySecrets;
    #[async_trait]
    impl SecretStore for EmptySecrets {
        async fn available(&self) -> Result<(), SecretStoreError> {
            Ok(())
        }
        async fn get(&self, _: Uuid) -> Result<Option<SecretString>, SecretStoreError> {
            Ok(None)
        }
        async fn set(&self, _: Uuid, _: &SecretString) -> Result<(), SecretStoreError> {
            Ok(())
        }
        async fn delete(&self, _: Uuid) -> Result<(), SecretStoreError> {
            Ok(())
        }
    }
    let Some(url) = test_url() else {
        return;
    };
    let mut imported = import_connection_url(&url, Some("cockroach-service")).unwrap();
    imported.profile.kind = DatabaseKind::CockroachDb;
    let connection =
        DatabaseConnection::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    assert_eq!(connection.kind(), DatabaseKind::CockroachDb);
    assert_eq!(
        connection.probe().await.unwrap().kind,
        DatabaseKind::CockroachDb
    );
    assert!(!connection.capabilities().relation_edit);
    assert!(!connection.capabilities().monitoring);
    assert_eq!(
        connection.principal_capability(),
        PrincipalCapability::Unsupported
    );
    assert!(connection.catalog_mutation_capabilities().create.is_empty());
    assert!(
        connection
            .discoverable_pgwire_databases()
            .await
            .unwrap()
            .unwrap()
            .contains(&"defaultdb".into())
    );
    let values = connection.execute("SELECT 9223372036854775807::INT8, 12345678901234567890.123456789::DECIMAL, '{\"x\":1}'::JSONB, 'f47ac10b-58cc-4372-a567-0e02b2c3d479'::UUID, '2026-01-02'::DATE, '2026-01-02 03:04:05+00'::TIMESTAMPTZ, decode('00ff','hex'), NULL::INT8, true").await.unwrap();
    let row = &values.result_sets[0].rows[0];
    assert_eq!(row[0], CellValue::Integer(i64::MAX));
    assert_eq!(
        row[1],
        CellValue::Text("12345678901234567890.123456789".into())
    );
    assert!(matches!(&row[2], CellValue::Text(value) if value.contains("\"x\"")));
    assert_eq!(
        row[3],
        CellValue::Text("f47ac10b-58cc-4372-a567-0e02b2c3d479".into())
    );
    assert!(matches!(row[4], CellValue::Date(_)));
    assert!(matches!(row[5], CellValue::Timestamp(_)));
    assert_eq!(row[6], CellValue::Bytes(vec![0, 255]));
    assert_eq!(row[7], CellValue::Null);
    assert_eq!(row[8], CellValue::Boolean(true));
    let schema = format!("lazydb_crdb_service_{}", Uuid::new_v4().simple());
    connection
        .execute(&format!("CREATE SCHEMA {schema}"))
        .await
        .unwrap();
    connection
        .execute(&format!(
            "CREATE TABLE {schema}.service_items (id INT PRIMARY KEY)"
        ))
        .await
        .unwrap();
    let mut profile = imported.profile.clone();
    let database = connection.probe().await.unwrap().database;
    profile.catalog_scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: database,
            schemas: CatalogSelection::Selected(vec![schema.clone()]),
        }]),
    };
    profile.read_only = true;
    let readonly = DatabaseConnection::connect(&profile, imported.transient_password.as_ref())
        .await
        .unwrap();
    assert!(
        readonly
            .execute(&format!("INSERT INTO {schema}.service_items VALUES (1)"))
            .await
            .is_err()
    );
    readonly.close().await;
    let temp = tempfile::TempDir::new().unwrap();
    let local_store = LocalCredentialStore::new(temp.path().join("key"), "test");
    if let Some(password) = imported.transient_password.as_ref() {
        profile.credential_policy = lazydb::profile::CredentialPolicy::LocalEncrypted(
            local_store.encrypt(profile.id, password).unwrap(),
        );
    }
    let service = AgentService::new(
        AgentProjectContext::resolve(Some(temp.path())).unwrap(),
        vec![profile],
        CredentialResolver::new(Arc::new(EmptySecrets), local_store),
    )
    .with_limits(2, 1024 * 1024);
    assert_eq!(service.connections()[0].kind, DatabaseKind::CockroachDb);
    let result = service
        .query(None, "SELECT i FROM generate_series(1,5) AS g(i)")
        .await
        .unwrap();
    assert_eq!(result.target.connection.kind, DatabaseKind::CockroachDb);
    assert_eq!(result.outcome.row_count, 2);
    assert!(result.outcome.truncated);
    let result = service
        .search_schema(None, "service_items".into(), 10)
        .await
        .unwrap();
    assert_eq!(result.target.connection.kind, DatabaseKind::CockroachDb);
    assert!(result.hits.iter().any(|hit| hit.name == "service_items"));
    assert!(
        service
            .query(None, "DELETE FROM service_items")
            .await
            .is_err()
    );
    connection
        .execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .unwrap();
    connection.close().await;
}

#[tokio::test]
async fn cockroach_query_budgets_transactions_sqlstate_and_cancellation() {
    let Some(url) = test_url() else {
        return;
    };
    let mut imported = import_connection_url(&url, Some("cockroach-transaction-test")).unwrap();
    imported.profile.kind = DatabaseKind::CockroachDb;
    let adapter =
        CockroachAdapter::connect(&imported.profile, imported.transient_password.as_ref())
            .await
            .unwrap();
    let budget = adapter
        .execute_pool_with_budget(
            "SELECT i FROM generate_series(1,20) AS g(i); SELECT 42",
            QueryBudget {
                max_rows: 3,
                max_bytes: usize::MAX,
            },
        )
        .await
        .unwrap();
    assert_eq!(budget.stats.row_count, 3);
    assert_eq!(budget.stats.fetched_row_count, 21);
    assert!(budget.stats.truncated);
    assert_eq!(budget.result_sets.len(), 2);
    let bytes = adapter
        .execute_pool_with_budget(
            "SELECT repeat('x',100) FROM generate_series(1,10)",
            QueryBudget {
                max_rows: 100,
                max_bytes: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(bytes.stats.row_count, 0);
    assert!(bytes.stats.truncated);
    let table = format!("public.lazydb_crdb_tx_{}", Uuid::new_v4().simple());
    adapter
        .execute(&format!("CREATE TABLE {table} (id INT PRIMARY KEY)"))
        .await
        .unwrap();
    let mut backend = adapter.transaction_backend().await.unwrap();
    backend.begin().await.unwrap();
    assert_eq!(backend.depth(), 1);
    backend
        .execute(&format!("INSERT INTO {table} VALUES (1)"))
        .await
        .unwrap();
    backend.rollback().await.unwrap();
    assert_eq!(backend.depth(), 0);
    backend.begin().await.unwrap();
    backend
        .execute(&format!("INSERT INTO {table} VALUES (2)"))
        .await
        .unwrap();
    backend.commit().await.unwrap();
    assert_eq!(backend.depth(), 0);
    backend.begin().await.unwrap();
    let error = backend
        .execute(&format!("INSERT INTO {table} VALUES (2)"))
        .await
        .unwrap_err();
    assert_eq!(error.history_snapshot().code.as_deref(), Some("23505"));
    backend.rollback().await.unwrap();
    backend.begin().await.unwrap();
    // Return the old value before a competing transaction changes it, preventing
    // transparent replay. This works with Cloud users without internal-function privileges.
    backend
        .execute(&format!("SELECT id FROM {table} WHERE id = 2"))
        .await
        .unwrap();
    adapter
        .execute(&format!("UPDATE {table} SET id = 20 WHERE id = 2"))
        .await
        .unwrap();
    let retry = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        backend.execute(&format!("UPDATE {table} SET id = 21 WHERE id = 2")),
    )
    .await
    .expect("retry error must reach the client")
    .unwrap_err();
    assert_eq!(retry.history_snapshot().code.as_deref(), Some("40001"));
    assert!(
        retry
            .relation_diagnostic()
            .unwrap()
            .message
            .contains("ROLLBACK and restart")
    );
    backend.rollback().await.unwrap();
    backend.begin().await.unwrap();
    backend
        .execute(&format!("INSERT INTO {table} VALUES (3)"))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), backend.cancel())
        .await
        .expect("cancellation must close the session without waiting for TLS shutdown")
        .unwrap();
    assert_eq!(backend.depth(), 0);
    backend.rollback().await.unwrap();
    assert!(backend.execute("SELECT 1").await.is_err());
    backend.force_close().await.unwrap();
    let result = adapter
        .execute(&format!("SELECT id FROM {table} ORDER BY id"))
        .await
        .unwrap();
    assert_eq!(
        result.result_sets[0].rows,
        vec![vec![CellValue::Integer(20)]]
    );
    adapter
        .execute(&format!("DROP TABLE {table}"))
        .await
        .unwrap();
    adapter.close().await;
}
