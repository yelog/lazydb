use anyhow::Context;
use lazydb::{
    db::{
        DatabaseConnection,
        catalog::{
            CatalogId, CatalogKind, CatalogRequest, CatalogRequestKey, CatalogTarget, DdlProvenance,
        },
        value::CellValue,
    },
    identity::ConnectionIdentity,
    model::{
        pagination::{PageRequest, PageSize},
        relation::RelationPreviewOptions,
    },
    profile::{CatalogScope, CatalogSelection, DatabaseScope, import_connection_url},
    sql::{SqlDialect, quote_identifier},
};

/// Explicit opt-in against a disposable, existing dataset. Never discovers local ADC files.
#[tokio::test]
#[ignore = "requires explicit LAZYDB_TEST_BIGQUERY_{PROJECT,DATA_PROJECT,DATASET,LOCATION,CREDENTIALS_FILE}"]
async fn bigquery_live_types_catalog_preview_and_native_ddl() -> anyhow::Result<()> {
    let required = |name: &str| -> anyhow::Result<String> {
        let value =
            std::env::var(name).with_context(|| format!("{name} must be explicitly configured"))?;
        anyhow::ensure!(!value.trim().is_empty(), "{name} must not be empty");
        Ok(value)
    };
    let project = required("LAZYDB_TEST_BIGQUERY_PROJECT")?;
    let data_project = required("LAZYDB_TEST_BIGQUERY_DATA_PROJECT")?;
    let dataset = required("LAZYDB_TEST_BIGQUERY_DATASET")?;
    let location = required("LAZYDB_TEST_BIGQUERY_LOCATION")?;
    let credentials_file = required("LAZYDB_TEST_BIGQUERY_CREDENTIALS_FILE")?;
    anyhow::ensure!(
        std::path::Path::new(&credentials_file).is_absolute(),
        "credentials file must be an absolute path"
    );
    let mut profile = import_connection_url(
        &format!("bigquery://{project}/{dataset}"),
        Some("BigQuery isolated live test"),
    )?
    .profile;
    profile.read_only = false;
    let options = profile
        .bigquery_options
        .as_mut()
        .context("BigQuery options missing")?;
    options.data_project = Some(data_project.clone());
    options.location = Some(location.clone());
    options.credentials_file = Some(credentials_file);
    options.maximum_bytes_billed = 10 * 1024 * 1024;
    options.query_timeout_secs = 60;
    profile.catalog_scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: data_project.clone(),
            schemas: CatalogSelection::Selected(vec![dataset.clone()]),
        }]),
    };
    let connection = DatabaseConnection::connect(&profile, None).await?;
    let name = format!("lazydb_test_{}", uuid::Uuid::new_v4().simple());
    let qualified = quote_identifier(
        &format!("{data_project}.{dataset}.{name}"),
        SqlDialect::BigQuery,
    );
    let relation = CatalogId::new(
        profile.id,
        CatalogKind::Table,
        [data_project.as_str(), dataset.as_str(), name.as_str()],
    );
    // Check every result inside this block so cleanup also runs after assertion failures.
    let validation: anyhow::Result<()> = async {
        let empty = connection.execute("SELECT CAST(NULL AS STRING) AS nullable_value, '' AS empty_text FROM UNNEST([1]) WHERE FALSE").await?;
        anyhow::ensure!(empty.result_sets[0].columns.len() == 2 && empty.result_sets[0].rows.is_empty(), "empty results must retain schema");
        let native = connection.execute(r#"SELECT TRUE AS flag, 1.25 AS float_value, BIGNUMERIC '12345678901234567890.12345678901234567890' AS exact_value, b'abc' AS bytes_value, TIMESTAMP '1969-12-31 23:59:59.999999+00' AS pre_epoch, JSON '{"key":"value"}' AS json_value"#).await?;
        let native_row = &native.result_sets[0].rows[0];
        anyhow::ensure!(native_row[0] == CellValue::Boolean(true), "BOOL decoding failed");
        anyhow::ensure!(native_row[1] == CellValue::Float(1.25), "FLOAT64 decoding failed");
        anyhow::ensure!(native_row[2] == CellValue::Text("12345678901234567890.1234567890123456789".into()), "BIGNUMERIC must retain decimal precision");
        anyhow::ensure!(native_row[3] == CellValue::Bytes(b"abc".to_vec()), "nonempty BYTES decoding failed");
        anyhow::ensure!(matches!(&native_row[4], CellValue::Timestamp(value) if value.timestamp_micros() == -1), "pre-epoch TIMESTAMP decoding failed");
        let CellValue::Text(json) = &native_row[5] else { anyhow::bail!("JSON must retain text") };
        anyhow::ensure!(serde_json::from_str::<serde_json::Value>(json)? == serde_json::json!({"key": "value"}), "JSON decoding failed");
        let paged = connection.execute("SELECT n FROM UNNEST(GENERATE_ARRAY(1, 1005)) AS n ORDER BY n").await?;
        let rows = &paged.result_sets[0].rows;
        anyhow::ensure!(rows.len() == 1005 && rows.first() == Some(&vec![CellValue::Integer(1)]) && rows.last() == Some(&vec![CellValue::Integer(1005)]) && !paged.stats.truncated, "native pagination must preserve all rows across the 1000-row page boundary");
        let create = format!("CREATE TABLE {qualified} OPTIONS (expiration_timestamp = TIMESTAMP_ADD(CURRENT_TIMESTAMP(), INTERVAL 1 HOUR)) AS SELECT 42 AS id, CAST(NULL AS STRING) AS nullable_value, '' AS empty_text, b'' AS empty_bytes, NUMERIC '123.456' AS exact_number, DATE '2026-01-02' AS date_value, TIME '03:04:05' AS time_value, DATETIME '2026-01-02 03:04:05' AS datetime_value, TIMESTAMP '2026-01-02 03:04:05+00' AS timestamp_value, [1, 2] AS array_value, STRUCT(1 AS x) AS struct_value");
        connection.execute(&create).await?;
        let result = connection.execute(&format!("SELECT * FROM {qualified}")).await?;
        let row = &result.result_sets[0].rows[0];
        anyhow::ensure!(row[0] == CellValue::Integer(42), "INT64 decoding failed");
        anyhow::ensure!(row[1] == CellValue::Null, "NULL decoding failed");
        anyhow::ensure!(row[2] == CellValue::Text(String::new()), "empty STRING decoding failed");
        anyhow::ensure!(row[3] == CellValue::Bytes(vec![]), "empty BYTES decoding failed");
        anyhow::ensure!(row[4] == CellValue::Text("123.456".into()), "NUMERIC must retain decimal precision");
        anyhow::ensure!(matches!(row[5], CellValue::Date(_)), "DATE decoding failed");
        anyhow::ensure!(matches!(row[6], CellValue::Time(_)), "TIME decoding failed");
        anyhow::ensure!(matches!(row[7], CellValue::DateTime(_)), "DATETIME decoding failed");
        anyhow::ensure!(matches!(row[8], CellValue::Timestamp(_)), "TIMESTAMP decoding failed");
        for value in &row[9..11] {
            let CellValue::Text(value) = value else { anyhow::bail!("nested values must retain JSON text") };
            serde_json::from_str::<serde_json::Value>(value)?;
        }
        let stats = result.stats.bigquery.context("native BigQuery job stats missing")?;
        anyhow::ensure!(stats.project == project && stats.location.as_deref().is_some_and(|value| value.eq_ignore_ascii_case(&location)), "job must retain billing project and configured location");
        anyhow::ensure!(stats.total_bytes_billed.is_some_and(|bytes| bytes <= 10 * 1024 * 1024), "billing cap exceeded or stats missing");
        let request = CatalogRequest {
            key: CatalogRequestKey {
                connection: ConnectionIdentity { profile_id: profile.id, generation: 1 },
                catalog_epoch: 0, request_id: 1,
                target: CatalogTarget::relation_children(relation.clone())?, cursor: None,
            },
            scope: profile.catalog_scope.clone(), page_size: 100,
        };
        let columns = connection.load_catalog_page(&request).await?;
        anyhow::ensure!(columns.entries.len() == 11, "catalog schema did not retain all columns");
        let preview = connection.preview_relation_with_scope(&relation, &profile.catalog_scope, &RelationPreviewOptions::default(), PageRequest::first(PageSize::Ten)).await?;
        anyhow::ensure!(preview.pagination.visible_rows == 1 && !preview.pagination.has_next, "preview must retain one fixture row");
        let ddl = connection.relation_ddl_with_scope(&relation, &profile.catalog_scope).await?;
        anyhow::ensure!(ddl.provenance == DdlProvenance::NativeCatalog && ddl.sql.contains(&name), "native DDL missing fixture identity");
        Ok(())
    }.await;
    // The UUID path is the only object this test may delete. Expiry is a fallback
    // if the process itself is interrupted before reaching cleanup.
    let cleanup = connection
        .execute(&format!("DROP TABLE IF EXISTS {qualified}"))
        .await;
    connection.close().await;
    if let Err(error) = validation {
        if let Err(cleanup) = cleanup {
            return Err(error.context(format!("own-table cleanup also failed: {cleanup}")));
        }
        return Err(error);
    }
    cleanup.context("failed to clean up own BigQuery fixture table")?;
    Ok(())
}
