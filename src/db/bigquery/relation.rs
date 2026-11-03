//! Native DDL: https://cloud.google.com/bigquery/docs/information-schema-tables
//! GoogleSQL paths: https://cloud.google.com/bigquery/docs/reference/standard-sql/lexical
//! Both previews and DDL use the adapter's configured maximumBytesBilled policy.

use crate::{
    db::{
        DatabaseError, RelationPreview,
        catalog::{
            CatalogId, CatalogKind, CatalogRequest, CatalogRequestKey, CatalogTarget,
            DdlProvenance, MAX_CATALOG_PAGE_SIZE, RelationDdl,
        },
        query::{QueryBudget, QueryOutcome},
        value::CellValue,
    },
    identity::ConnectionIdentity,
    model::{
        pagination::{PageRequest, ResultPagination},
        relation::RelationPreviewOptions,
    },
    profile::CatalogScope,
    sql::{SqlDialect, validate_relation_preview_options},
};

use super::{
    BigQueryAdapter, catalog::validate_relation_identity, invalid_response, quote_identifier,
    quote_literal,
};

const MAX_PREVIEW_BYTES: usize = 32 * 1024 * 1024;
const MAX_DDL_BYTES: usize = 16 * 1024 * 1024;

impl BigQueryAdapter {
    pub async fn preview_relation(
        &self,
        relation: &CatalogId,
        options: &RelationPreviewOptions,
        page: PageRequest,
    ) -> Result<RelationPreview, DatabaseError> {
        self.preview_relation_with_scope(relation, &self.catalog_scope, options, page)
            .await
    }

    pub async fn preview_relation_with_scope(
        &self,
        relation: &CatalogId,
        scope: &CatalogScope,
        options: &RelationPreviewOptions,
        page: PageRequest,
    ) -> Result<RelationPreview, DatabaseError> {
        let (project, dataset, name) =
            validate_relation_identity(self.profile_id, &self.catalog_scope, scope, relation)?;
        validate_page(page)?;
        let select = preview_query(project, dataset, name, options)?;
        let Some((_, table)) = self.relation_resource(relation, scope).await? else {
            return Err(invalid_response(
                "BigQuery preview relation no longer exists with the requested kind",
            ));
        };
        validate_location(
            self.location.as_deref(),
            table.get("location").and_then(serde_json::Value::as_str),
        )?;
        let sql = paged_preview_query(&select, page);
        let outcome = self
            .execute_with_budget(
                &sql,
                QueryBudget {
                    max_rows: page.size.lookahead_limit(),
                    max_bytes: MAX_PREVIEW_BYTES,
                },
            )
            .await?;
        finish_preview(sql, outcome, page)
    }

    pub async fn relation_ddl(&self, relation: &CatalogId) -> Result<RelationDdl, DatabaseError> {
        self.relation_ddl_with_scope(relation, &self.catalog_scope)
            .await
    }

    pub async fn relation_ddl_with_scope(
        &self,
        relation: &CatalogId,
        scope: &CatalogScope,
    ) -> Result<RelationDdl, DatabaseError> {
        let (project, dataset, name) =
            validate_relation_identity(self.profile_id, &self.catalog_scope, scope, relation)?;
        let Some((entry, table)) = self.relation_resource(relation, scope).await? else {
            return Err(invalid_response(
                "BigQuery DDL relation no longer exists with the requested kind",
            ));
        };
        validate_location(
            self.location.as_deref(),
            table.get("location").and_then(serde_json::Value::as_str),
        )?;
        let query = ddl_query(relation.kind, project, dataset, name)?;
        let outcome = self
            .execute_with_budget(
                &query,
                QueryBudget {
                    max_rows: 1,
                    max_bytes: MAX_DDL_BYTES,
                },
            )
            .await?;
        let ddl = native_ddl(&outcome)?;
        let request = CatalogRequest {
            key: CatalogRequestKey {
                connection: ConnectionIdentity {
                    profile_id: self.profile_id,
                    generation: 0,
                },
                catalog_epoch: 0,
                request_id: 0,
                target: CatalogTarget::RelationChildren {
                    relation: relation.clone(),
                },
                cursor: None,
            },
            scope: scope.clone(),
            page_size: MAX_CATALOG_PAGE_SIZE,
        };
        let children = self.load_catalog_page(&request).await?;
        Ok(RelationDdl {
            relation: entry,
            children,
            sql: ddl,
            provenance: DdlProvenance::NativeCatalog,
        })
    }
}

fn validate_page(page: PageRequest) -> Result<(), DatabaseError> {
    if page.resolve_total {
        return Err(DatabaseError::unsupported(
            "BigQuery preview does not resolve exact totals or last pages; COUNT queries can scan billable data",
        ));
    }
    // GoogleSQL OFFSET is INT64. Reject values that cannot be represented before submitting a job.
    if page.offset > i64::MAX as u64 {
        return Err(DatabaseError::configuration(
            "BigQuery preview offset exceeds INT64",
        ));
    }
    Ok(())
}

fn validate_location(configured: Option<&str>, actual: Option<&str>) -> Result<(), DatabaseError> {
    if let (Some(configured), Some(actual)) = (configured, actual)
        && !configured.eq_ignore_ascii_case(actual)
    {
        return Err(DatabaseError::configuration(format!(
            "BigQuery dataset location {actual} differs from configured job location {configured}",
        )));
    }
    Ok(())
}

fn qualified_relation(project: &str, dataset: &str, name: &str) -> String {
    // GoogleSQL quotes the complete path once: `project.dataset.table`.
    quote_identifier(&format!("{project}.{dataset}.{name}"))
}

fn preview_query(
    project: &str,
    dataset: &str,
    name: &str,
    options: &RelationPreviewOptions,
) -> Result<String, DatabaseError> {
    let options = validate_relation_preview_options(
        options.where_clause.as_deref().unwrap_or_default(),
        options.order_by_clause.as_deref().unwrap_or_default(),
        SqlDialect::BigQuery,
    )
    .map_err(|error| DatabaseError::configuration(error.to_string()))?;
    let mut query = format!(
        "SELECT * FROM {}",
        qualified_relation(project, dataset, name)
    );
    if let Some(filter) = options.where_clause {
        query.push_str(" WHERE ");
        query.push_str(&filter);
    }
    if let Some(order) = options.order_by_clause {
        query.push_str(" ORDER BY ");
        query.push_str(&order);
    }
    Ok(query)
}

fn paged_preview_query(select: &str, page: PageRequest) -> String {
    format!(
        "{select} LIMIT {} OFFSET {}",
        page.size.lookahead_limit(),
        page.offset
    )
}

fn finish_preview(
    sql: String,
    mut outcome: QueryOutcome,
    page: PageRequest,
) -> Result<RelationPreview, DatabaseError> {
    if outcome.stats.truncated || outcome.stats.response_error.is_some() {
        return Err(invalid_response(
            "BigQuery preview is incomplete or exceeded its result budget; reduce the page size",
        ));
    }
    if outcome.result_sets.len() != 1 {
        return Err(invalid_response(
            "BigQuery preview returned an invalid number of result sets",
        ));
    }
    let result = &mut outcome.result_sets[0];
    let fetched = result.rows.len();
    if fetched > page.size.lookahead_limit() {
        return Err(invalid_response("BigQuery preview exceeded its row budget"));
    }
    result.rows.truncate(page.size.get());
    outcome.stats.row_count = result.rows.len();
    outcome.stats.fetched_row_count = fetched;
    outcome.stats.truncated = fetched > page.size.get();
    Ok(RelationPreview {
        sql,
        result: outcome,
        pagination: ResultPagination::from_page(page, fetched),
        row_versions: None,
    })
}

fn ddl_query(
    kind: CatalogKind,
    project: &str,
    dataset: &str,
    name: &str,
) -> Result<String, DatabaseError> {
    let types = match kind {
        CatalogKind::Table => "'BASE TABLE', 'CLONE', 'SNAPSHOT', 'EXTERNAL'",
        CatalogKind::View => "'VIEW'",
        CatalogKind::MaterializedView => "'MATERIALIZED VIEW'",
        _ => {
            return Err(DatabaseError::configuration(
                "BigQuery native DDL requires a relation",
            ));
        }
    };
    Ok(format!(
        "SELECT ddl FROM {} WHERE table_name = {} AND table_type IN ({types}) LIMIT 1",
        quote_identifier(&format!("{project}.{dataset}.INFORMATION_SCHEMA.TABLES")),
        quote_literal(name),
    ))
}

fn native_ddl(outcome: &QueryOutcome) -> Result<String, DatabaseError> {
    if outcome.stats.truncated
        || outcome.stats.response_error.is_some()
        || outcome.result_sets.len() != 1
    {
        return Err(invalid_response(
            "BigQuery native DDL query returned an incomplete response",
        ));
    }
    let rows = &outcome.result_sets[0].rows;
    if rows.len() != 1 {
        return Err(DatabaseError::unsupported(
            "BigQuery INFORMATION_SCHEMA.TABLES did not return native DDL for this relation",
        ));
    }
    match rows[0].as_slice() {
        [CellValue::Text(ddl)] if !ddl.trim().is_empty() => Ok(ddl.clone()),
        [CellValue::Null] => Err(DatabaseError::unsupported(
            "BigQuery INFORMATION_SCHEMA.TABLES exposes no native DDL for this relation",
        )),
        _ => Err(invalid_response(
            "BigQuery native DDL is missing or invalid",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::query::{ColumnMeta, QueryStats, ResultSet},
        model::pagination::{PageSize, TotalRows},
    };
    use std::time::Duration;

    fn outcome(rows: Vec<Vec<CellValue>>) -> QueryOutcome {
        let count = rows.len();
        QueryOutcome {
            result_sets: vec![ResultSet {
                columns: vec![ColumnMeta {
                    name: "value".into(),
                    type_name: "INT64".into(),
                }],
                rows,
                affected_rows: 0,
            }],
            stats: QueryStats::new(Duration::ZERO, Duration::ZERO, count),
        }
    }

    fn fixture_relation(adapter: &BigQueryAdapter) -> CatalogId {
        CatalogId::new(
            adapter.profile_id,
            CatalogKind::Table,
            ["data-project", "public", "users"],
        )
    }

    fn table_resource() -> serde_json::Value {
        serde_json::json!({"tableReference":{"projectId":"data-project","datasetId":"public","tableId":"users"}, "type":"TABLE", "schema":{"fields":[{"name":"value","type":"INT64"}]}})
    }

    #[tokio::test]
    async fn native_ddl_reads_information_schema_and_returns_rest_columns() {
        use super::super::catalog::tests::fixture_server;
        use serde_json::json;
        let ddl = "CREATE TABLE `data-project.public.users` (value INT64);";
        let (endpoint, server) = fixture_server(vec![
            (200, table_resource()),
            (200, json!({"status":{"state":"DONE"}})),
            (200, json!({"jobComplete":true, "totalRows":"1", "schema":{"fields":[{"name":"ddl","type":"STRING"}]}, "rows":[{"f":[{"v":ddl}]}]})),
            (200, table_resource()),
        ]).await;
        let mut adapter = BigQueryAdapter::fixture(endpoint);
        adapter.catalog_scope = crate::profile::CatalogScope {
            databases: crate::profile::CatalogSelection::All,
        };
        let result = adapter
            .relation_ddl(&fixture_relation(&adapter))
            .await
            .unwrap();
        assert_eq!(result.sql, ddl);
        assert_eq!(result.provenance, DdlProvenance::NativeCatalog);
        assert_eq!(result.children.entries.len(), 1);
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 4);
        let query = requests[1]
            .1
            .pointer("/configuration/query/query")
            .unwrap()
            .as_str()
            .unwrap();
        assert!(
            query.starts_with("SELECT ddl FROM `data-project.public.INFORMATION_SCHEMA.TABLES`")
        );
        assert!(
            requests[1]
                .1
                .pointer("/configuration/query/maximumBytesBilled")
                .is_some()
        );
        assert!(
            requests[0]
                .0
                .starts_with("GET /bigquery/v2/projects/data-project/datasets/public/tables/users")
        );
    }

    #[tokio::test]
    async fn preview_submits_bounded_fully_qualified_query_and_preserves_lookahead() {
        use super::super::catalog::tests::fixture_server;
        use serde_json::json;
        let rows = (0..11)
            .map(|number| json!({"f":[{"v":number.to_string()}]}))
            .collect::<Vec<_>>();
        let (endpoint, server) = fixture_server(vec![
            (200, table_resource()),
            (200, json!({"status":{"state":"DONE"}})),
            (200, json!({"jobComplete":true, "totalRows":"11", "schema":{"fields":[{"name":"value","type":"INT64"}]}, "rows":rows})),
        ]).await;
        let mut adapter = BigQueryAdapter::fixture(endpoint);
        adapter.catalog_scope = crate::profile::CatalogScope {
            databases: crate::profile::CatalogSelection::All,
        };
        let result = adapter
            .preview_relation(
                &fixture_relation(&adapter),
                &RelationPreviewOptions::default(),
                PageRequest::at(PageSize::Ten, 20),
            )
            .await
            .unwrap();
        assert_eq!(result.result.result_sets[0].rows.len(), 10);
        assert!(result.pagination.has_next);
        let requests = server.await.unwrap();
        assert_eq!(
            requests[1].1.pointer("/configuration/query/query").unwrap(),
            "SELECT * FROM `data-project.public.users` LIMIT 11 OFFSET 20"
        );
        assert!(
            requests[1]
                .1
                .pointer("/configuration/query/maximumBytesBilled")
                .is_some()
        );
        assert!(
            requests
                .iter()
                .all(|(_, body)| !body.to_string().contains("COUNT("))
        );
    }

    #[test]
    fn preview_quotes_complete_path_and_uses_google_sql_and_lookahead() {
        let options = RelationPreviewOptions {
            where_clause: Some("`amount` > 3 AND DATE(created_at) >= DATE '2025-01-01'".into()),
            order_by_clause: Some("`amount` DESC NULLS LAST".into()),
        };
        let select = preview_query("data-project", "dataset", "table", &options).unwrap();
        assert_eq!(
            select,
            "SELECT * FROM `data-project.dataset.table` WHERE `amount` > 3 AND DATE(created_at) >= DATE '2025-01-01' ORDER BY `amount` DESC NULLS LAST"
        );
        let sql = paged_preview_query(&select, PageRequest::at(PageSize::Ten, 20));
        assert!(sql.ends_with("LIMIT 11 OFFSET 20"));
        assert!(!sql.contains("COUNT"));
    }

    #[test]
    fn preview_rejects_injected_statements_and_query_controls() {
        for clause in [
            "TRUE; DROP TABLE x",
            "TRUE -- comment",
            "TRUE LIMIT 1",
            "TRUE UNION ALL SELECT * FROM x",
        ] {
            assert!(
                preview_query(
                    "p",
                    "d",
                    "t",
                    &RelationPreviewOptions {
                        where_clause: Some(clause.into()),
                        order_by_clause: None
                    }
                )
                .is_err(),
                "{clause}"
            );
        }
        assert!(
            preview_query(
                "p",
                "d",
                "t",
                &RelationPreviewOptions {
                    where_clause: None,
                    order_by_clause: Some("x LIMIT 1".into())
                }
            )
            .is_err()
        );
    }

    #[test]
    fn last_page_and_overflow_offsets_are_explicitly_rejected() {
        let mut page = PageRequest::at(PageSize::Ten, 0);
        assert!(validate_page(page).is_ok());
        page.resolve_total = true;
        assert_eq!(
            validate_page(page).unwrap_err().category,
            crate::db::ErrorCategory::Unsupported
        );
        page.resolve_total = false;
        page.offset = u64::MAX;
        assert!(validate_page(page).is_err());
        assert!(validate_location(Some("US"), Some("us")).is_ok());
        assert!(validate_location(Some("US"), Some("EU")).is_err());
    }

    #[test]
    fn lookahead_preserves_empty_metadata_and_never_claims_exact_total() {
        let page = PageRequest::at(PageSize::Ten, 0);
        let empty = finish_preview("SELECT".into(), outcome(Vec::new()), page).unwrap();
        assert_eq!(empty.result.result_sets[0].columns.len(), 1);
        assert!(!empty.pagination.has_next);
        let rows = (0..11)
            .map(|value| vec![CellValue::Integer(value)])
            .collect();
        let preview = finish_preview("SELECT".into(), outcome(rows), page).unwrap();
        assert_eq!(preview.result.result_sets[0].rows.len(), 10);
        assert_eq!(preview.result.stats.fetched_row_count, 11);
        assert!(preview.pagination.has_next);
        assert!(!matches!(preview.pagination.total, TotalRows::Exact(_)));
        let mut incomplete = outcome(Vec::new());
        incomplete.stats.truncated = true;
        assert!(finish_preview("SELECT".into(), incomplete, page).is_err());
    }

    #[test]
    fn ddl_only_uses_native_information_schema_and_checks_kind() {
        let sql = ddl_query(
            CatalogKind::MaterializedView,
            "data-project",
            "dataset",
            "O'Reilly",
        )
        .unwrap();
        assert!(
            sql.starts_with("SELECT ddl FROM `data-project.dataset.INFORMATION_SCHEMA.TABLES`")
        );
        assert!(sql.contains("table_type IN ('MATERIALIZED VIEW')"));
        assert!(sql.ends_with("LIMIT 1"));
        assert!(
            ddl_query(CatalogKind::Table, "p", "d", "t")
                .unwrap()
                .contains("'CLONE', 'SNAPSHOT', 'EXTERNAL'")
        );
        assert!(ddl_query(CatalogKind::Column, "p", "d", "t").is_err());
        let ddl = "CREATE TABLE `p.d.t` (x INT64);";
        assert_eq!(
            native_ddl(&outcome(vec![vec![CellValue::Text(ddl.into())]])).unwrap(),
            ddl
        );
        assert_eq!(
            native_ddl(&outcome(vec![vec![CellValue::Null]]))
                .unwrap_err()
                .category,
            crate::db::ErrorCategory::Unsupported
        );
        assert!(native_ddl(&outcome(Vec::new())).is_err());
    }
}
