use sqlparser::{
    ast::{LimitClause, Statement},
    parser::Parser,
};

use super::{
    SqlDialect, dialect::parser_dialect, relation_filter::validate_relation_preview_options,
};
use crate::db::query::RELATION_PREVIEW_LIMIT;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DerivedQueryError(pub String);

impl std::fmt::Display for DerivedQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DerivedQueryError {}

pub fn derived_query_capable(source: &str, dialect: SqlDialect) -> bool {
    parse_source(source, dialect).is_ok()
}

/// Adds the result cap to a single read-only SELECT without changing queries
/// that already specify their own row limit.
pub fn bounded_query(source: &str, dialect: SqlDialect) -> Option<String> {
    let source = parse_source(source, dialect).ok()?;
    let statements = Parser::parse_sql(parser_dialect(dialect), &source).ok()?;
    let [Statement::Query(query)] = statements.as_slice() else {
        return None;
    };
    let has_row_limit = query.limit_clause.as_ref().is_some_and(|limit| {
        matches!(
            limit,
            LimitClause::LimitOffset { limit: Some(_), .. } | LimitClause::OffsetCommaLimit { .. }
        )
    });
    if has_row_limit || query.fetch.is_some() {
        return Some(source);
    }
    Some(format!(
        "SELECT * FROM ({source}) AS __lazydb_query LIMIT {RELATION_PREVIEW_LIMIT}"
    ))
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
    sql.push_str(" LIMIT 500");
    Ok(sql)
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
            "SELECT * FROM (SELECT id FROM users) AS __lazydb_result WHERE id > 1 ORDER BY id DESC LIMIT 500"
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
            Some("SELECT * FROM (SELECT * FROM users) AS __lazydb_query LIMIT 500".into())
        );
    }

    #[test]
    fn preserves_existing_limits() {
        assert_eq!(
            bounded_query("SELECT * FROM users LIMIT 20;", SqlDialect::Sqlite),
            Some("SELECT * FROM users LIMIT 20".into())
        );
    }

    #[test]
    fn preserves_offset_and_supports_set_queries() {
        assert_eq!(
            bounded_query("SELECT * FROM users OFFSET 10", SqlDialect::Postgres),
            Some(
                "SELECT * FROM (SELECT * FROM users OFFSET 10) AS __lazydb_query LIMIT 500".into()
            )
        );
        assert_eq!(
            bounded_query("SELECT id FROM users UNION SELECT id FROM admins", SqlDialect::Sqlite),
            Some(
                "SELECT * FROM (SELECT id FROM users UNION SELECT id FROM admins) AS __lazydb_query LIMIT 500".into()
            )
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
