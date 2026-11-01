use std::time::{Duration, Instant};

use sqlparser::ast::{Query, SetExpr, Statement};
use sqlparser::dialect::ClickHouseDialect;
use sqlparser::parser::Parser;

use crate::db::{
    DatabaseError, ErrorCategory, RelationPreview,
    catalog::{
        CatalogEntry, CatalogId, CatalogKind, CatalogMetadata, DdlProvenance, OptionalMetadata,
        QualifiedName, RelationDdl,
    },
    query::{QueryOutcome, QueryStats},
    value::CellValue,
};
use crate::model::{
    data_query::DataQueryOptions,
    pagination::{PageRequest, ResultPagination},
};
use crate::profile::CatalogScope;
use crate::security::sanitize_terminal_text;

use super::transport::ClickHouseAdapter;

impl ClickHouseAdapter {
    pub async fn preview_relation(
        &self,
        relation: &CatalogId,
        options: &DataQueryOptions,
        page: PageRequest,
    ) -> Result<RelationPreview, DatabaseError> {
        self.preview_relation_with_scope(relation, &self.catalog_scope, options, page)
            .await
    }

    pub async fn preview_relation_with_scope(
        &self,
        relation: &CatalogId,
        scope: &CatalogScope,
        options: &DataQueryOptions,
        mut page: PageRequest,
    ) -> Result<RelationPreview, DatabaseError> {
        if relation.connection_id != self.profile_id()
            || relation.kind != CatalogKind::Table
            || relation.native_path.len() != 3
        {
            return Err(catalog_error(
                "ClickHouse relation preview requires a table identity from this profile",
            ));
        }
        let database = relation.native_path[0].as_str();
        let table = relation.native_path[2].as_str();
        if relation.native_path[1] != database
            || !scope.allows_schema(database, database)
            || !self.catalog_scope.allows_schema(database, database)
        {
            return Err(catalog_error(
                "ClickHouse relation is outside the captured catalog scope",
            ));
        }
        let escaped_database = database.replace('\\', "\\\\").replace('\'', "\\'");
        let escaped_table = table.replace('\\', "\\\\").replace('\'', "\\'");
        let found = self
            .catalog_rows(
                "SELECT engine FROM system.tables WHERE database = ? AND name = ?",
                &[&escaped_database, &escaped_table],
            )
            .await?;
        let engine = found
            .first()
            .and_then(|row| match row.first() {
                Some(CellValue::Text(value)) => Some(value.as_str()),
                _ => None,
            })
            .ok_or_else(|| catalog_error("ClickHouse relation no longer exists"))?;
        if !matches!(
            engine,
            "MergeTree"
                | "ReplacingMergeTree"
                | "SummingMergeTree"
                | "AggregatingMergeTree"
                | "CollapsingMergeTree"
                | "VersionedCollapsingMergeTree"
                | "GraphiteMergeTree"
                | "Log"
                | "TinyLog"
                | "StripeLog"
                | "Memory"
                | "Buffer"
                | "Distributed"
                | "Join"
                | "Set"
                | "Null"
        ) {
            return Err(catalog_error(
                "ClickHouse relation preview is unavailable for views, materialized views, and target-backed objects",
            ));
        }
        let qualified = format!("{}.{}", quote_identifier(database), quote_identifier(table));
        let mut business = format!("SELECT * FROM {qualified}");
        if page.resolve_total
            && options
                .order_by_clause
                .as_deref()
                .is_some_and(|clause| !clause.trim().is_empty())
        {
            return Err(catalog_error(
                "exact total is unavailable for ordered ClickHouse relation previews",
            ));
        }
        let validated = crate::sql::validate_relation_preview_options(
            options.where_clause.as_deref().unwrap_or_default(),
            options.order_by_clause.as_deref().unwrap_or_default(),
            crate::sql::SqlDialect::ClickHouse,
        )
        .map_err(|error| catalog_error(&error.to_string()))?;
        append_options(
            &mut business,
            &DataQueryOptions {
                where_clause: validated.where_clause,
                order_by_clause: validated.order_by_clause,
            },
        )?;
        let total = if page.resolve_total {
            if business.to_ascii_lowercase().contains(" settings ")
                || business.to_ascii_lowercase().contains(" format ")
                || business.to_ascii_lowercase().contains(" with totals")
            {
                return Err(catalog_error(
                    "exact total is unavailable for SETTINGS, FORMAT, or WITH TOTALS relation previews",
                ));
            }
            let sql = format!("SELECT count() FROM ({business}) AS lazydb_count");
            let outcome = self.execute(&sql).await?;
            let value = outcome
                .result_sets
                .first()
                .and_then(|set| set.rows.first())
                .and_then(|row| match row.first() {
                    Some(CellValue::Unsigned(value)) => Some(*value),
                    Some(CellValue::Integer(value)) if *value >= 0 => Some(*value as u64),
                    _ => None,
                })
                .ok_or_else(|| catalog_error("ClickHouse count query returned an invalid value"))?;
            page.offset = crate::model::pagination::ResultPagination::last_offset(page.size, value);
            Some(value)
        } else {
            None
        };
        let sql = format!(
            "{business} LIMIT {} OFFSET {}",
            page.size.lookahead_limit(),
            page.offset
        );
        let started = Instant::now();
        let outcome = self.execute(&sql).await?;
        let mut result = outcome.result_sets.into_iter().next().unwrap_or_default();
        let fetched = result.rows.len();
        result.rows.truncate(page.size.get());
        let outcome = QueryOutcome {
            stats: QueryStats {
                execution: started.elapsed(),
                fetch: Duration::ZERO,
                row_count: result.rows.len(),
                fetched_row_count: fetched,
                truncated: fetched > page.size.get(),
                response_error: None,
            },
            result_sets: vec![result],
        };
        let pagination = if let Some(total) = total {
            let visible_rows = fetched.min(page.size.get());
            ResultPagination {
                page_size: page.size,
                offset: page.offset,
                visible_rows,
                has_next: page.offset.saturating_add(visible_rows as u64) < total,
                total: crate::model::pagination::TotalRows::Exact(total),
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

    pub async fn resolve_relation_identity(
        &self,
        relation: &CatalogId,
    ) -> Result<Option<CatalogEntry>, DatabaseError> {
        if relation.connection_id != self.profile_id()
            || relation.native_path.len() != 3
            || !relation.kind.is_relation()
        {
            return Ok(None);
        }
        let database = &relation.native_path[0];
        let table = &relation.native_path[2];
        if relation.native_path[1] != *database
            || !self.catalog_scope.allows_schema(database, database)
        {
            return Ok(None);
        }
        let rows = self
            .catalog_rows(
                "SELECT engine, comment FROM system.tables WHERE database = ? AND name = ?",
                &[database, table],
            )
            .await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let Some(CellValue::Text(engine)) = row.first() else {
            return Ok(None);
        };
        let actual_kind = match engine.as_str() {
            "View" => CatalogKind::View,
            "MaterializedView" => CatalogKind::MaterializedView,
            value
                if value.contains("MergeTree")
                    || matches!(
                        value,
                        "Memory"
                            | "Log"
                            | "TinyLog"
                            | "StripeLog"
                            | "Distributed"
                            | "Buffer"
                            | "Null"
                            | "Join"
                            | "Set"
                    ) =>
            {
                CatalogKind::Table
            }
            _ => return Ok(None),
        };
        if actual_kind != relation.kind {
            return Ok(None);
        }
        let schema = CatalogId::new(
            self.profile_id(),
            CatalogKind::Schema,
            [database.clone(), database.clone()],
        );
        let entry = CatalogEntry::relation(
            relation.clone(),
            schema,
            QualifiedName {
                database: Some(database.clone()),
                schema: Some(database.clone()),
                object: table.clone(),
            },
            engine.clone(),
            OptionalMetadata::Unsupported,
            true,
        )
        .map_err(|error| catalog_error(&error.to_string()))?;
        Ok(Some(entry))
    }

    pub async fn relation_ddl_with_scope(
        &self,
        relation: &CatalogId,
        scope: &CatalogScope,
    ) -> Result<RelationDdl, DatabaseError> {
        if relation.connection_id != self.profile_id()
            || !matches!(
                relation.kind,
                CatalogKind::Table | CatalogKind::View | CatalogKind::MaterializedView
            )
            || relation.native_path.len() != 3
        {
            return Err(catalog_error(
                "ClickHouse DDL target has an invalid identity",
            ));
        }
        let database = relation.native_path[0].as_str();
        let table = relation.native_path[2].as_str();
        if relation.native_path[1] != database
            || !scope.allows_schema(database, database)
            || !self.catalog_scope.allows_schema(database, database)
        {
            return Err(catalog_error(
                "ClickHouse DDL target is outside the captured scope",
            ));
        }
        let rows = self
            .catalog_rows(
                "SELECT engine, comment FROM system.tables WHERE database = ? AND name = ?",
                &[database, table],
            )
            .await?;
        let row = rows
            .first()
            .ok_or_else(|| catalog_error("ClickHouse relation no longer exists"))?;
        let engine = match row.first() {
            Some(CellValue::Text(value)) => value.as_str(),
            _ => {
                return Err(catalog_error(
                    "ClickHouse returned an invalid relation engine",
                ));
            }
        };
        let comment = match row.get(1) {
            Some(CellValue::Text(value)) if !value.is_empty() => {
                OptionalMetadata::Supported(Some(value.clone()))
            }
            Some(CellValue::Text(_)) | Some(CellValue::Null) => OptionalMetadata::Supported(None),
            _ => OptionalMetadata::Unsupported,
        };
        let expected_kind = match engine {
            "View" => CatalogKind::View,
            "MaterializedView" => CatalogKind::MaterializedView,
            value
                if value.contains("MergeTree")
                    || matches!(
                        value,
                        "Memory"
                            | "Log"
                            | "TinyLog"
                            | "StripeLog"
                            | "Distributed"
                            | "Buffer"
                            | "Null"
                            | "Join"
                            | "Set"
                    ) =>
            {
                CatalogKind::Table
            }
            _ => {
                return Err(catalog_error(
                    "ClickHouse relation engine cannot be safely represented as table/view DDL",
                ));
            }
        };
        if relation.kind != expected_kind {
            return Err(catalog_error(
                "ClickHouse relation identity no longer matches its native kind",
            ));
        }
        let sql = format!(
            "SHOW CREATE TABLE {}.{}",
            quote_identifier(database),
            quote_identifier(table)
        );
        let ddl_rows = self.catalog_rows(&sql, &[]).await?;
        let create = ddl_rows
            .first()
            .and_then(|row| row.first())
            .and_then(|cell| match cell {
                CellValue::Text(value) => Some(value.clone()),
                _ => None,
            })
            .ok_or_else(|| catalog_error("ClickHouse SHOW CREATE returned no definition"))?;
        if create.to_ascii_lowercase().contains("password")
            || create.to_ascii_lowercase().contains("secret")
        {
            return Err(catalog_error(
                "ClickHouse DDL contains a secret-bearing source and was withheld",
            ));
        }
        let entry = CatalogEntry {
            id: relation.clone(),
            parent_id: Some(CatalogId::new(
                self.profile_id(),
                CatalogKind::Schema,
                [database.to_owned(), database.to_owned()],
            )),
            kind: relation.kind,
            native_kind: engine.to_owned(),
            qualified_name: QualifiedName {
                database: Some(database.to_owned()),
                schema: Some(database.to_owned()),
                object: table.to_owned(),
            },
            comment,
            metadata: CatalogMetadata::None,
            expandable: true,
            relation_id: Some(relation.clone()),
        };
        let empty_request = crate::db::catalog::CatalogRequest {
            key: crate::db::catalog::CatalogRequestKey {
                connection: crate::identity::ConnectionIdentity {
                    profile_id: self.profile_id(),
                    generation: 0,
                },
                catalog_epoch: 0,
                request_id: 0,
                target: crate::db::catalog::CatalogTarget::RelationChildren {
                    relation: relation.clone(),
                },
                cursor: None,
            },
            scope: scope.clone(),
            page_size: 1,
        };
        let children = crate::db::catalog::CatalogPage::new(
            &empty_request,
            Vec::new(),
            crate::db::catalog::CatalogCount::Unknown,
            None,
        )
        .map_err(|error| catalog_error(&error.to_string()))?;
        Ok(RelationDdl {
            relation: entry,
            children,
            sql: create,
            provenance: DdlProvenance::NativeCatalog,
        })
    }
}

fn append_options(sql: &mut String, options: &DataQueryOptions) -> Result<(), DatabaseError> {
    let dialect = ClickHouseDialect {};
    if let Some(where_clause) = options
        .where_clause
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let parsed = Parser::parse_sql(&dialect, &format!("SELECT 1 WHERE ({where_clause})"))
            .map_err(|_| catalog_error("invalid ClickHouse WHERE expression"))?;
        let valid = matches!(parsed.as_slice(), [Statement::Query(query)]
            if matches!(query.as_ref(), Query { body, order_by, limit_clause, fetch, locks, .. }
                if matches!(body.as_ref(), SetExpr::Select(_))
                    && order_by.is_none()
                    && limit_clause.is_none()
                    && fetch.is_none()
                    && locks.is_empty()));
        if !valid {
            return Err(catalog_error(
                "ClickHouse WHERE expression contains unsupported trailing SQL",
            ));
        }
        sql.push_str(" WHERE ");
        sql.push_str(where_clause);
    }
    if let Some(order_by) = options
        .order_by_clause
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let parsed = Parser::parse_sql(&dialect, &format!("SELECT 1 ORDER BY {order_by}"))
            .map_err(|_| catalog_error("invalid ClickHouse ORDER BY expression"))?;
        let valid = matches!(parsed.as_slice(), [Statement::Query(query)]
            if matches!(query.as_ref(), Query { body, order_by, limit_clause, fetch, locks, .. }
                if matches!(body.as_ref(), SetExpr::Select(_))
                    && order_by.is_some()
                    && limit_clause.is_none()
                    && fetch.is_none()
                    && locks.is_empty()));
        if !valid {
            return Err(catalog_error(
                "ClickHouse ORDER BY expression contains unsupported trailing SQL",
            ));
        }
        sql.push_str(" ORDER BY ");
        sql.push_str(order_by);
    }
    Ok(())
}

fn quote_identifier(value: &str) -> String {
    format!("`{}`", value.replace('`', "``"))
}

fn catalog_error(message: &str) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Sql,
        code: Some("clickhouse_relation_preview".into()),
        message: sanitize_terminal_text(message),
        diagnostic: None,
    }
}
