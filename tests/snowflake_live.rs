use lazydb::{
    db::{
        DatabaseConnection,
        catalog::{
            CatalogId, CatalogKind, CatalogRequest, CatalogRequestKey, CatalogSearchObjectScope,
            CatalogSearchRequest, CatalogTarget, ObjectGroup,
        },
        value::CellValue,
    },
    identity::ConnectionIdentity,
    model::execution_target::ExecutionTarget,
    model::{
        pagination::{PageRequest, PageSize, TotalRows},
        relation::RelationPreviewOptions,
    },
    profile::{CatalogSelection, import_connection_url},
};
use secrecy::SecretString;

/// Reuse after a real token expires; the replacement must have the same intended role.
#[tokio::test]
#[ignore = "requires a naturally expired token and a valid replacement in protected files"]
async fn snowflake_live_expired_token_and_replacement() -> anyhow::Result<()> {
    let url = std::env::var("LAZYDB_SNOWFLAKE_TEST_URL")?;
    let imported = import_connection_url(&url, Some("Snowflake token lifecycle"))?;
    anyhow::ensure!(
        imported.transient_password.is_none(),
        "test URL must not contain credentials"
    );
    let profile = imported.profile;
    let read_token = |variable| -> anyhow::Result<SecretString> {
        let path = std::env::var(variable)?;
        Ok(SecretString::from(
            std::fs::read_to_string(path)?.trim().to_owned(),
        ))
    };
    let expired = read_token("LAZYDB_SNOWFLAKE_EXPIRED_TOKEN_FILE")?;
    let error = match DatabaseConnection::connect(&profile, Some(&expired)).await {
        Ok(connection) => {
            connection.close().await;
            anyhow::bail!("the expired token was unexpectedly accepted");
        }
        Err(error) => error,
    };
    assert_eq!(error.category, lazydb::db::ErrorCategory::Authentication);

    let replacement = read_token("LAZYDB_SNOWFLAKE_TOKEN_FILE")?;
    let connection = DatabaseConnection::connect(&profile, Some(&replacement)).await?;
    let info = connection.probe().await?;
    assert_eq!(info.database, profile.database.as_deref().unwrap());
    let result = connection.execute("SELECT 42 AS ANSWER").await?;
    assert_eq!(result.result_sets[0].rows[0][0], CellValue::Integer(42));
    let discovery = connection.discover_catalog_scope().await?;
    assert!(discovery.databases.iter().any(|database| {
        Some(database.name.as_str()) == profile.database.as_deref()
            && database
                .schemas
                .iter()
                .any(|schema| Some(schema.as_str()) == profile.default_schema.as_deref())
    }));
    connection.close().await;
    Ok(())
}

/// Opt-in only: reads synthetic fixtures and credentials from a protected local file.
#[tokio::test]
#[ignore = "requires LAZYDB_SNOWFLAKE_TEST_URL and LAZYDB_SNOWFLAKE_TOKEN_FILE"]
async fn snowflake_live_readonly_workflows() -> anyhow::Result<()> {
    let url = std::env::var("LAZYDB_SNOWFLAKE_TEST_URL")?;
    let token_path = std::env::var("LAZYDB_SNOWFLAKE_TOKEN_FILE")?;
    let token = SecretString::from(std::fs::read_to_string(token_path)?.trim().to_owned());
    let profile = import_connection_url(&url, Some("Snowflake live validation"))?.profile;
    let database = profile.database.as_deref().unwrap();
    let schema = profile.default_schema.as_deref().unwrap();
    let connection = DatabaseConnection::connect(&profile, Some(&token)).await?;
    let info = connection.probe().await?;
    assert_eq!(info.database, database);
    let identity = ConnectionIdentity {
        profile_id: profile.id,
        generation: 1,
    };
    let request = |target| CatalogRequest {
        key: CatalogRequestKey {
            connection: identity,
            catalog_epoch: 0,
            request_id: 1,
            target,
            cursor: None,
        },
        scope: profile.catalog_scope.clone(),
        page_size: 100,
    };
    let discovery = connection.discover_catalog_scope().await?;
    assert!(
        discovery
            .databases
            .iter()
            .any(|entry| entry.name == database && entry.schemas.iter().any(|name| name == schema))
    );
    let db_id = CatalogId::new(profile.id, CatalogKind::Database, [database]);
    let schemas = connection
        .load_catalog_page(&request(CatalogTarget::schemas(db_id)?))
        .await?;
    assert!(
        schemas
            .entries
            .iter()
            .any(|entry| entry.qualified_name.object == schema)
    );
    let schema_id = CatalogId::new(profile.id, CatalogKind::Schema, [database, schema]);
    let groups = connection
        .load_catalog_page(&request(CatalogTarget::groups(schema_id.clone())?))
        .await?;
    assert_eq!(groups.group_summaries.len(), 2);
    let tables = connection
        .load_catalog_page(&request(CatalogTarget::objects(
            schema_id.clone(),
            ObjectGroup::Tables,
        )?))
        .await?;
    assert!(
        tables
            .entries
            .iter()
            .any(|entry| entry.qualified_name.object == "Type Samples")
    );
    let views = connection
        .load_catalog_page(&request(CatalogTarget::objects(
            schema_id,
            ObjectGroup::Views,
        )?))
        .await?;
    assert!(
        views
            .entries
            .iter()
            .any(|entry| entry.qualified_name.object == "Sample View")
    );
    let table_id = CatalogId::new(
        profile.id,
        CatalogKind::Table,
        [database, schema, "Type Samples"],
    );
    let columns = connection
        .load_catalog_page(&request(CatalogTarget::relation_children(
            table_id.clone(),
        )?))
        .await?;
    assert_eq!(columns.entries.len(), 11);
    let search = connection
        .search_catalog(&CatalogSearchRequest {
            connection: identity,
            session_id: 1,
            generation: 1,
            query: "type samples".into(),
            scope: profile.catalog_scope.clone(),
            object_scope: CatalogSearchObjectScope::RelationsOnly,
            limit: 10,
        })
        .await?;
    assert!(search.hits.iter().any(|hit| hit.entry.id == table_id));
    let options = RelationPreviewOptions {
        where_clause: None,
        order_by_clause: Some("ID ASC".into()),
    };
    let first = connection
        .preview_relation_with_scope(
            &table_id,
            &profile.catalog_scope,
            &options,
            PageRequest::first(PageSize::Ten),
        )
        .await?;
    assert_eq!(first.pagination.visible_rows, 10);
    assert!(first.pagination.has_next);
    let row = &first.result.result_sets[0].rows[0];
    assert_eq!(row[0], CellValue::Integer(0));
    assert_eq!(
        row[1],
        CellValue::Text("12345678901234567890.123456789012345678".into())
    );
    assert_eq!(row[2], CellValue::Null);
    assert!(matches!(row[4], CellValue::Date(_)));
    assert!(matches!(row[5], CellValue::Time(_)));
    assert!(matches!(row[6], CellValue::DateTime(_)));
    assert!(matches!(row[7], CellValue::Timestamp(_)));
    assert!(matches!(row[8], CellValue::Timestamp(_)));
    assert_eq!(row[9], CellValue::Bytes(vec![0, 255, 16]));
    let last = connection
        .preview_relation_with_scope(
            &table_id,
            &profile.catalog_scope,
            &options,
            PageRequest::last(PageSize::Ten, 125),
        )
        .await?;
    assert_eq!(last.pagination.total, TotalRows::Exact(125));
    assert_eq!(last.pagination.offset, 120);
    assert_eq!(last.pagination.visible_rows, 5);
    assert!(!last.pagination.has_next);
    let ddl = connection
        .relation_ddl_with_scope(&table_id, &profile.catalog_scope)
        .await?;
    assert!(ddl.sql.to_ascii_uppercase().contains("CREATE"));
    let view_id = CatalogId::new(
        profile.id,
        CatalogKind::View,
        [database, schema, "Sample View"],
    );
    let view = connection
        .preview_relation_with_scope(
            &view_id,
            &profile.catalog_scope,
            &options,
            PageRequest::first(PageSize::Ten),
        )
        .await?;
    assert_eq!(view.pagination.visible_rows, 10);
    connection
        .relation_ddl_with_scope(&view_id, &profile.catalog_scope)
        .await?;
    let error = connection
        .execute("SELECT * FROM LAZYDB_DOES_NOT_EXIST_20261101")
        .await
        .unwrap_err();
    assert!(error.code.is_some());
    let mut expanded = profile.clone();
    if let CatalogSelection::Selected(databases) = &mut expanded.catalog_scope.databases {
        databases[0].schemas = CatalogSelection::All;
    }
    let target = ExecutionTarget {
        profile_id: profile.id,
        database: database.to_owned(),
        schema: Some("CaseSchema".into()),
    };
    let quoted = DatabaseConnection::connect_target(&expanded, Some(&token), &target).await?;
    let result = quoted
        .execute("SELECT \"CaseColumn\" FROM \"Quoted Table\"")
        .await?;
    assert_eq!(
        result.result_sets[0].rows[0][0],
        CellValue::Text("synthetic quoted namespace".into())
    );
    quoted.close().await;
    let mut all_scope = expanded.catalog_scope.clone();
    all_scope.databases = CatalogSelection::All;
    expanded.catalog_scope = all_scope;
    let all = DatabaseConnection::connect(&expanded, Some(&token)).await?;
    let mut db_request = request(CatalogTarget::Databases);
    db_request.scope = expanded.catalog_scope.clone();
    db_request.page_size = 1;
    let db_page = all.load_catalog_page(&db_request).await?;
    assert_eq!(db_page.entries.len(), 1);
    if let Some(cursor) = db_page.next_cursor {
        db_request.key.cursor = Some(cursor);
        assert!(!all.load_catalog_page(&db_request).await?.entries.is_empty());
    }
    all.close().await;
    let partitioned = connection.execute(
        "SELECT SEQ4() AS ID, SHA2(SEQ4()::VARCHAR) || SHA2((SEQ4()+100000)::VARCHAR) || SHA2((SEQ4()+200000)::VARCHAR) || SHA2((SEQ4()+300000)::VARCHAR) AS PAYLOAD FROM TABLE(GENERATOR(ROWCOUNT => 20000))"
    ).await?;
    assert_eq!(partitioned.stats.row_count, 20000);
    assert!(!partitioned.stats.truncated);
    assert_eq!(
        partitioned.result_sets[0].rows[19999][0],
        CellValue::Integer(19999)
    );
    for row in &partitioned.result_sets[0].rows {
        assert!(matches!(&row[1], CellValue::Text(value) if value.len() == 256));
    }
    connection.close().await;
    Ok(())
}
