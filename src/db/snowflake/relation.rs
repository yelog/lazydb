use crate::{
    db::{
        DatabaseError, RelationPreview,
        catalog::{
            CatalogId, CatalogKind, CatalogRequest, CatalogRequestKey, CatalogTarget,
            DdlProvenance, MAX_CATALOG_PAGE_SIZE, RelationDdl,
        },
        query::QueryBudget,
    },
    identity::ConnectionIdentity,
    model::{
        pagination::{PageRequest, ResultPagination, TotalRows},
        relation::RelationPreviewOptions,
    },
    profile::CatalogScope,
    sql::{SqlDialect, validate_relation_preview_options},
};

use super::{
    SnowflakeAdapter,
    catalog::{text, unsigned, validate_relation_identity},
    invalid_response, quote_identifier, quote_literal,
};

const MAX_PREVIEW_BYTES: usize = 32 * 1024 * 1024;

impl SnowflakeAdapter {
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
        mut page: PageRequest,
    ) -> Result<RelationPreview, DatabaseError> {
        let (database, schema, name) =
            validate_relation_identity(self.profile_id, &self.catalog_scope, scope, relation)?;
        let (select, count) = preview_queries(database, schema, name, options)?;
        if self
            .resolve_relation_identity_with_scope(relation, scope)
            .await?
            .is_none()
        {
            return Err(invalid_response(
                "Snowflake preview relation no longer exists with the requested kind",
            ));
        }
        let total = if page.resolve_total {
            let result = self.catalog_result(&count, 1).await?;
            let row = result
                .rows
                .first()
                .ok_or_else(|| invalid_response("Snowflake preview count returned no row"))?;
            let total = unsigned(row, 0)?
                .ok_or_else(|| invalid_response("Snowflake preview count returned NULL"))?;
            page.offset = ResultPagination::last_offset(page.size, total);
            Some(total)
        } else {
            None
        };
        let sql = paged_preview_query(&select, page);
        let mut outcome = self
            .execute_with_budget(
                &sql,
                QueryBudget {
                    max_rows: page.size.lookahead_limit(),
                    max_bytes: MAX_PREVIEW_BYTES,
                },
            )
            .await?;
        if outcome.stats.truncated {
            // An incomplete byte-limited page cannot safely advance by the requested page size.
            return Err(invalid_response(
                "Snowflake preview exceeded its byte budget; reduce the page size",
            ));
        }
        if outcome.result_sets.len() != 1 {
            return Err(invalid_response(
                "Snowflake preview returned an invalid number of result sets",
            ));
        }
        let result = &mut outcome.result_sets[0];
        let fetched = result.rows.len();
        result.rows.truncate(page.size.get());
        outcome.stats.row_count = result.rows.len();
        outcome.stats.fetched_row_count = fetched;
        outcome.stats.truncated = fetched > page.size.get();
        let pagination = if let Some(total) = total {
            ResultPagination {
                page_size: page.size,
                offset: page.offset,
                visible_rows: result.rows.len(),
                has_next: page.offset.saturating_add(result.rows.len() as u64) < total,
                total: TotalRows::Exact(total),
            }
        } else {
            ResultPagination::from_page(page, fetched)
        };
        Ok(RelationPreview {
            sql,
            result: outcome,
            pagination,
            row_versions: None,
        })
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
        let (database, schema, name) =
            validate_relation_identity(self.profile_id, &self.catalog_scope, scope, relation)?;
        let entry = self
            .resolve_relation_identity_with_scope(relation, scope)
            .await?
            .ok_or_else(|| {
                invalid_response("Snowflake DDL relation no longer exists with the requested kind")
            })?;
        let sql = ddl_query(relation.kind, database, schema, name)?;
        let result = self.catalog_result(&sql, 1).await?;
        let row = result
            .rows
            .first()
            .ok_or_else(|| invalid_response("Snowflake GET_DDL returned no row"))?;
        let ddl = text(row, 0)?.to_owned();
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

fn qualified_relation(database: &str, schema: &str, name: &str) -> String {
    [database, schema, name]
        .into_iter()
        .map(quote_identifier)
        .collect::<Vec<_>>()
        .join(".")
}

fn preview_queries(
    database: &str,
    schema: &str,
    name: &str,
    options: &RelationPreviewOptions,
) -> Result<(String, String), DatabaseError> {
    let options = validate_relation_preview_options(
        options.where_clause.as_deref().unwrap_or_default(),
        options.order_by_clause.as_deref().unwrap_or_default(),
        SqlDialect::Snowflake,
    )
    .map_err(|error| DatabaseError::configuration(error.to_string()))?;
    let qualified = qualified_relation(database, schema, name);
    let filter = options
        .where_clause
        .as_deref()
        .map(|clause| format!(" WHERE {clause}"))
        .unwrap_or_default();
    let mut select = format!("SELECT * FROM {qualified}{filter}");
    if let Some(order) = options.order_by_clause {
        select.push_str(" ORDER BY ");
        select.push_str(&order);
    }
    Ok((select, format!("SELECT COUNT(*) FROM {qualified}{filter}")))
}

fn paged_preview_query(select: &str, page: PageRequest) -> String {
    format!(
        "{select} LIMIT {} OFFSET {}",
        page.size.lookahead_limit(),
        page.offset
    )
}

fn ddl_query(
    kind: CatalogKind,
    database: &str,
    schema: &str,
    name: &str,
) -> Result<String, DatabaseError> {
    let native_kind = match kind {
        CatalogKind::Table => "TABLE",
        CatalogKind::View => "VIEW",
        _ => {
            return Err(DatabaseError::configuration(
                "Snowflake GET_DDL supports only table and view targets",
            ));
        }
    };
    Ok(format!(
        "SELECT GET_DDL({}, {}, TRUE)",
        quote_literal(native_kind),
        quote_literal(&qualified_relation(database, schema, name))
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::pagination::PageSize,
        profile::{CatalogSelection, DatabaseScope},
    };
    use uuid::Uuid;

    #[test]
    fn preview_quotes_full_names_keeps_count_filter_and_uses_lookahead() {
        let options = RelationPreviewOptions {
            where_clause: Some("\"Amount\" > 3".into()),
            order_by_clause: Some("\"Amount\" DESC NULLS LAST".into()),
        };
        let (select, count) =
            preview_queries("Db.Prod", "Case\"Schema", "O'Reilly", &options).unwrap();
        assert_eq!(
            select,
            "SELECT * FROM \"Db.Prod\".\"Case\"\"Schema\".\"O'Reilly\" WHERE \"Amount\" > 3 ORDER BY \"Amount\" DESC NULLS LAST"
        );
        assert_eq!(
            count,
            "SELECT COUNT(*) FROM \"Db.Prod\".\"Case\"\"Schema\".\"O'Reilly\" WHERE \"Amount\" > 3"
        );
        assert!(
            paged_preview_query(&select, PageRequest::at(PageSize::Ten, 20))
                .ends_with("LIMIT 11 OFFSET 20")
        );
    }

    #[test]
    fn preview_rejects_statement_injection_and_trailing_page_clauses() {
        for clause in [
            "TRUE; DROP TABLE x",
            "TRUE -- comment",
            "TRUE LIMIT 1",
            "TRUE UNION SELECT * FROM x",
        ] {
            let options = RelationPreviewOptions {
                where_clause: Some(clause.into()),
                order_by_clause: None,
            };
            assert!(
                preview_queries("DB", "S", "T", &options).is_err(),
                "{clause}"
            );
        }
        let options = RelationPreviewOptions {
            where_clause: None,
            order_by_clause: Some("x LIMIT 1".into()),
        };
        assert!(preview_queries("DB", "S", "T", &options).is_err());
    }

    #[test]
    fn ddl_quotes_the_identifier_then_escapes_the_full_name_as_a_literal() {
        assert_eq!(
            ddl_query(CatalogKind::View, "D.B", "s\"q", "O'Reilly\\x").unwrap(),
            "SELECT GET_DDL('VIEW', '\"D.B\".\"s\"\"q\".\"O''Reilly\\\\x\"', TRUE)"
        );
        assert!(ddl_query(CatalogKind::MaterializedView, "D", "S", "T").is_err());
    }

    #[test]
    fn relation_validation_rejects_bad_paths_profiles_kinds_and_scope_widening() {
        let profile = Uuid::new_v4();
        let scope = CatalogScope {
            databases: CatalogSelection::Selected(vec![DatabaseScope {
                name: "DB".into(),
                schemas: CatalogSelection::Selected(vec!["PUBLIC".into()]),
            }]),
        };
        let all = CatalogScope {
            databases: CatalogSelection::All,
        };
        let mut relation = CatalogId::new(profile, CatalogKind::Table, ["DB", "PUBLIC", "T"]);
        assert!(validate_relation_identity(profile, &scope, &all, &relation).is_ok());
        assert!(validate_relation_identity(Uuid::new_v4(), &scope, &all, &relation).is_err());
        relation.native_path[1] = "PRIVATE".into();
        assert!(validate_relation_identity(profile, &scope, &all, &relation).is_err());
        assert!(validate_relation_identity(profile, &all, &scope, &relation).is_err());
        relation.native_path[1] = "PUBLIC".into();
        relation.native_path.push("extra".into());
        assert!(validate_relation_identity(profile, &all, &all, &relation).is_err());
        relation.native_path.pop();
        relation.kind = CatalogKind::MaterializedView;
        assert!(validate_relation_identity(profile, &all, &all, &relation).is_err());
    }
}
