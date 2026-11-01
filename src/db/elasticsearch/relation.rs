use reqwest::Method;
use serde_json::json;

use crate::{
    db::{DatabaseError, RelationPreview, catalog::CatalogId, query::QueryBudget},
    model::{
        pagination::{PageRequest, ResultPagination, TotalRows},
        relation::RelationPreviewOptions,
    },
    profile::CatalogScope,
};

use super::{
    ElasticsearchAdapter, catalog::validate_relation_scope, index_path, outcome,
    partial_search_response,
};

impl ElasticsearchAdapter {
    pub async fn preview_relation(
        &self,
        relation: &CatalogId,
        options: &RelationPreviewOptions,
        page: PageRequest,
    ) -> Result<RelationPreview, DatabaseError> {
        self.preview_relation_with_scope(
            relation,
            &CatalogScope::for_profile(
                crate::profile::DatabaseKind::Elasticsearch,
                &self.database,
                None,
            ),
            options,
            page,
        )
        .await
    }

    pub async fn preview_relation_with_scope(
        &self,
        relation: &CatalogId,
        scope: &CatalogScope,
        options: &RelationPreviewOptions,
        page: PageRequest,
    ) -> Result<RelationPreview, DatabaseError> {
        validate_relation_scope(self.profile_id, &self.database, relation, scope)?;
        if options
            .where_clause
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
            || options
                .order_by_clause
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(DatabaseError::unsupported(
                "Elasticsearch filters and sorting use Query DSL in the REST console",
            ));
        }
        let path = index_path(&relation.native_path[2], "_search");
        let mut offset = page.offset;
        let mut total = None;
        if page.resolve_total {
            let count = self
                .request_json(
                    Method::GET,
                    &index_path(&relation.native_path[2], "_count"),
                    None,
                )
                .await?;
            if partial_search_response(&count) {
                return Err(DatabaseError::configuration(
                    "Elasticsearch count returned a partial response; retry after resolving shard failures",
                ));
            }
            let count = count
                .get("count")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| {
                    DatabaseError::configuration("Elasticsearch count response has no count")
                })?;
            offset = ResultPagination::last_offset(page.size, count);
            total = Some(count);
        }
        // Keep the lookahead inside the default result window as well.
        let remaining = 10_000u64.checked_sub(offset).filter(|remaining| *remaining > 0)
            .ok_or_else(|| DatabaseError::unsupported("Elasticsearch previews support the first 10,000 documents; use search_after in the REST console for deeper searches"))?;
        let size = (page.size.lookahead_limit() as u64).min(remaining);
        let body = json!({"from":offset,"size":size,"query":{"match_all":{}},"sort":["_doc"],"track_total_hits":true}).to_string();
        let started = std::time::Instant::now();
        let value = self.request_json(Method::POST, &path, Some(&body)).await?;
        let execution = started.elapsed();
        if partial_search_response(&value) {
            return Err(DatabaseError::configuration(
                "Elasticsearch preview returned a partial search response; inspect shard failures or retry in the REST console",
            ));
        }
        let hits = value
            .pointer("/hits/hits")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                DatabaseError::configuration("Elasticsearch search response has no hits array")
            })?;
        let fetched = hits.len();
        let exact_total = total.or_else(|| {
            let hits = value.pointer("/hits/total")?;
            hits.as_u64().or_else(|| {
                (hits["relation"] == "eq")
                    .then(|| hits["value"].as_u64())
                    .flatten()
            })
        });
        let mut pagination = ResultPagination::from_page(PageRequest { offset, ..page }, fetched);
        if let Some(total) = exact_total {
            pagination.total = TotalRows::Exact(total);
        }
        // Reaching the result window must not advertise an unusable next page.
        if offset.saturating_add(page.size.as_u64()) >= 10_000 {
            pagination.has_next = false;
        }
        let mut result = outcome(value, QueryBudget::UNBOUNDED)?;
        result.result_sets.truncate(1);
        if let Some(set) = result.result_sets.first_mut() {
            set.rows.truncate(page.size.get());
        }
        result.stats.execution = execution;
        result.stats.row_count = pagination.visible_rows;
        result.stats.fetched_row_count = fetched;
        result.stats.truncated = false;
        Ok(RelationPreview {
            sql: format!("POST {path}\n{body}"),
            result,
            pagination,
            row_versions: None,
        })
    }
}
