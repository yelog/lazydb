use sqlparser::{
    ast::{Query, SetExpr, Statement},
    parser::Parser,
};

use super::{SqlDialect, dialect::parser_dialect};
use crate::model::relation::RelationPreviewOptions;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationFilterError(pub String);

impl std::fmt::Display for RelationFilterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for RelationFilterError {}

pub fn validate_relation_preview_options(
    where_clause: &str,
    order_by_clause: &str,
    dialect: SqlDialect,
) -> Result<RelationPreviewOptions, RelationFilterError> {
    let where_clause = normalize(where_clause);
    let order_by_clause = normalize(order_by_clause);
    for fragment in [&where_clause, &order_by_clause].into_iter().flatten() {
        if fragment.contains(';')
            || fragment.contains("--")
            || fragment.contains("/*")
            || fragment.contains("*/")
        {
            return Err(RelationFilterError(
                "comments and multiple statements are not allowed".into(),
            ));
        }
    }
    let mut query = "SELECT * FROM __lazydb_relation".to_owned();
    if let Some(clause) = &where_clause {
        query.push_str(" WHERE ");
        query.push_str(clause);
    }
    if let Some(clause) = &order_by_clause {
        query.push_str(" ORDER BY ");
        query.push_str(clause);
    }
    let statements = Parser::parse_sql(parser_dialect(dialect), &query)
        .map_err(|error| RelationFilterError(format!("invalid preview clause: {error}")))?;
    let valid_shape = matches!(
        statements.first(),
        Some(Statement::Query(query))
            if matches!(query.as_ref(), Query { body, order_by, limit_clause, fetch, locks, .. }
                if matches!(body.as_ref(), SetExpr::Select(_))
                    && order_by.as_ref().is_none_or(|order| !matches!(order.kind, sqlparser::ast::OrderByKind::All(_)))
                    && order_by.is_some() == order_by_clause.is_some()
                    && query_select_has_where(body, where_clause.is_some())
                    && limit_clause.is_none()
                    && fetch.is_none()
                    && locks.is_empty())
    );
    if statements.len() != 1 || !valid_shape {
        return Err(RelationFilterError("invalid preview query shape".into()));
    }
    Ok(RelationPreviewOptions {
        where_clause,
        order_by_clause,
    })
}

fn query_select_has_where(body: &SetExpr, expected: bool) -> bool {
    body.as_select()
        .is_some_and(|select| select.selection.is_some() == expected)
}

fn normalize(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_common_filter_and_order_fragments() {
        let options = validate_relation_preview_options(
            "id > 10 and name like 'John%'",
            "name desc, id asc",
            SqlDialect::Postgres,
        )
        .unwrap();
        assert_eq!(
            options.where_clause.as_deref(),
            Some("id > 10 and name like 'John%'")
        );
        assert_eq!(
            options.order_by_clause.as_deref(),
            Some("name desc, id asc")
        );
    }

    #[test]
    fn rejects_multiple_statements_and_query_controls() {
        for (where_clause, order_by) in [
            ("id > 1; delete from users", ""),
            ("id > 1", "name desc limit 1"),
            ("id > 1", "name desc --"),
        ] {
            assert!(
                validate_relation_preview_options(where_clause, order_by, SqlDialect::Sqlite)
                    .is_err()
            );
        }
    }

    #[test]
    fn whitespace_only_clauses_are_absent() {
        assert_eq!(
            validate_relation_preview_options(" ", "\t", SqlDialect::Generic).unwrap(),
            RelationPreviewOptions::default()
        );
    }
}
