use sqlparser::{ast::Statement, parser::Parser};

use super::{
    SqlDialect, dialect::parser_dialect, relation_filter::validate_relation_preview_options,
};
use crate::model::pagination::PageRequest;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DerivedQueryError(pub String);

impl std::fmt::Display for DerivedQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DerivedQueryError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaginatedSql {
    pub page_sql: String,
    pub count_sql: String,
}

pub fn derived_query_capable(source: &str, dialect: SqlDialect) -> bool {
    parse_source(source, dialect).is_ok()
}

/// Builds the default page query for a single read-only SELECT.
pub fn bounded_query(source: &str, dialect: SqlDialect) -> Option<String> {
    build_paginated_query(source, dialect, PageRequest::first(Default::default()))
        .ok()
        .map(|query| query.page_sql)
}

pub fn build_paginated_query(
    source: &str,
    dialect: SqlDialect,
    page: PageRequest,
) -> Result<PaginatedSql, DerivedQueryError> {
    let source = parse_source(source, dialect)?;
    Ok(wrap_paginated_source(&source, page))
}

pub fn build_derived_query(
    source: &str,
    where_clause: &str,
    order_by_clause: &str,
    dialect: SqlDialect,
) -> Result<String, DerivedQueryError> {
    let source = parse_source(source, dialect)?;
    let options = validate_relation_preview_options(where_clause, order_by_clause, dialect)
        .map_err(|error| DerivedQueryError(error.to_string()))?;
    let mut sql = format!("SELECT * FROM ({source}) AS __lazydb_result");
    if let Some(clause) = options.where_clause {
        sql.push_str(" WHERE ");
        sql.push_str(&clause);
    }
    if let Some(clause) = options.order_by_clause {
        sql.push_str(" ORDER BY ");
        sql.push_str(&clause);
    }
    Ok(wrap_paginated_source(&sql, PageRequest::first(Default::default())).page_sql)
}

pub fn build_derived_paginated_query(
    source: &str,
    where_clause: &str,
    order_by_clause: &str,
    dialect: SqlDialect,
    page: PageRequest,
) -> Result<PaginatedSql, DerivedQueryError> {
    let source = parse_source(source, dialect)?;
    let options = validate_relation_preview_options(where_clause, order_by_clause, dialect)
        .map_err(|error| DerivedQueryError(error.to_string()))?;
    let mut sql = format!("SELECT * FROM ({source}) AS __lazydb_result");
    if let Some(clause) = options.where_clause {
        sql.push_str(" WHERE ");
        sql.push_str(&clause);
    }
    if let Some(clause) = options.order_by_clause {
        sql.push_str(" ORDER BY ");
        sql.push_str(&clause);
    }
    Ok(wrap_paginated_source(&sql, page))
}

fn wrap_paginated_source(source: &str, page: PageRequest) -> PaginatedSql {
    let limit = page.size.lookahead_limit();
    let offset = page.offset;
    PaginatedSql {
        page_sql: format!(
            "SELECT * FROM ({source}) AS __lazydb_page LIMIT {limit} OFFSET {offset}"
        ),
        count_sql: format!("SELECT COUNT(*) FROM ({source}) AS __lazydb_count"),
    }
}

fn parse_source(source: &str, dialect: SqlDialect) -> Result<String, DerivedQueryError> {
    let statements = Parser::parse_sql(parser_dialect(dialect), source)
        .map_err(|error| DerivedQueryError(format!("source query cannot be parsed: {error}")))?;
    let [Statement::Query(query)] = statements.as_slice() else {
        return Err(DerivedQueryError(
            "derived results require one read-only SELECT query".into(),
        ));
    };
    if !query.locks.is_empty() {
        return Err(DerivedQueryError(
            "lock-bearing queries are not supported".into(),
        ));
    }
    let risk = super::classify_sql(source, dialect);
    if risk.statement_count != 1 || risk.risks != [super::SqlRisk::ReadOnly] {
        return Err(DerivedQueryError(
            "derived results require one read-only SELECT query".into(),
        ));
    }
    let source = source.trim_end();
    Ok(source
        .strip_suffix(';')
        .unwrap_or(source)
        .trim_end()
        .to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_source_and_fragments_with_limit() {
        assert_eq!(
            build_derived_query(
                "SELECT id FROM users;",
                "id > 1",
                "id DESC",
                SqlDialect::Sqlite
            )
            .unwrap(),
            "SELECT * FROM (SELECT * FROM (SELECT id FROM users) AS __lazydb_result WHERE id > 1 ORDER BY id DESC) AS __lazydb_page LIMIT 501 OFFSET 0"
        );
    }

    #[test]
    fn rejects_unsafe_sources() {
        for source in [
            "SELECT 1; SELECT 2",
            "EXPLAIN SELECT 1",
            "UPDATE t SET x = 1 RETURNING x",
            "SELECT * FROM t FOR UPDATE",
        ] {
            assert!(
                !derived_query_capable(source, SqlDialect::Sqlite),
                "{source}"
            );
        }
    }

    #[test]
    fn bounds_uncapped_selects() {
        assert_eq!(
            bounded_query("SELECT * FROM users;", SqlDialect::Sqlite),
            Some("SELECT * FROM (SELECT * FROM users) AS __lazydb_page LIMIT 501 OFFSET 0".into())
        );
    }

    #[test]
    fn preserves_existing_limits() {
        assert_eq!(
            bounded_query("SELECT * FROM users LIMIT 20;", SqlDialect::Sqlite),
            Some(
                "SELECT * FROM (SELECT * FROM users LIMIT 20) AS __lazydb_page LIMIT 501 OFFSET 0"
                    .into()
            )
        );
    }

    #[test]
    fn preserves_offset_and_supports_set_queries() {
        assert_eq!(
            bounded_query("SELECT * FROM users OFFSET 10", SqlDialect::Postgres),
            Some(
                "SELECT * FROM (SELECT * FROM users OFFSET 10) AS __lazydb_page LIMIT 501 OFFSET 0"
                    .into()
            )
        );
        assert_eq!(
            bounded_query("SELECT id FROM users UNION SELECT id FROM admins", SqlDialect::Sqlite),
            Some(
                "SELECT * FROM (SELECT id FROM users UNION SELECT id FROM admins) AS __lazydb_page LIMIT 501 OFFSET 0".into()
            )
        );
    }

    #[test]
    fn builds_page_and_count_queries_for_requested_page() {
        let query = build_paginated_query(
            "SELECT id FROM users;",
            SqlDialect::Sqlite,
            PageRequest::at(crate::model::pagination::PageSize::Ten, 20),
        )
        .unwrap();

        assert_eq!(
            query.page_sql,
            "SELECT * FROM (SELECT id FROM users) AS __lazydb_page LIMIT 11 OFFSET 20"
        );
        assert_eq!(
            query.count_sql,
            "SELECT COUNT(*) FROM (SELECT id FROM users) AS __lazydb_count"
        );
    }

    #[test]
    fn builds_filtered_page_and_count_queries_together() {
        let query = build_derived_paginated_query(
            "SELECT id FROM users;",
            "id > 1",
            "id DESC",
            SqlDialect::Sqlite,
            PageRequest::at(crate::model::pagination::PageSize::OneThousand, 1000),
        )
        .unwrap();

        assert_eq!(
            query.page_sql,
            "SELECT * FROM (SELECT * FROM (SELECT id FROM users) AS __lazydb_result WHERE id > 1 ORDER BY id DESC) AS __lazydb_page LIMIT 1001 OFFSET 1000"
        );
        assert_eq!(
            query.count_sql,
            "SELECT COUNT(*) FROM (SELECT * FROM (SELECT id FROM users) AS __lazydb_result WHERE id > 1 ORDER BY id DESC) AS __lazydb_count"
        );
    }

    #[test]
    fn ignores_non_select_statements() {
        assert_eq!(
            bounded_query("UPDATE users SET id = 1", SqlDialect::Sqlite),
            None
        );
        assert_eq!(
            bounded_query("SELECT 1; SELECT 2", SqlDialect::Sqlite),
            None
        );
    }
}
