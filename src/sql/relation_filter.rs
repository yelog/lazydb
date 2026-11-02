use sqlparser::ast::{Expr, Ident, OrderByExpr, OrderByKind, Query, SetExpr, Statement};

use super::{SqlDialect, dialect::parse_sql, quote_identifier};
use crate::model::relation::RelationPreviewOptions;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationFilterError(pub String);

impl std::fmt::Display for RelationFilterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for RelationFilterError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RelationColumnSort {
    pub direction: SortDirection,
    pub priority: usize,
}

pub fn relation_column_sort_projection(
    order_by_clause: &str,
    columns: &[impl AsRef<str>],
    dialect: SqlDialect,
) -> Result<Vec<Option<RelationColumnSort>>, RelationFilterError> {
    let options = validate_relation_preview_options("", order_by_clause, dialect)?;
    let Some(clause) = options.order_by_clause else {
        return Ok(vec![None; columns.len()]);
    };
    let order_by = parse_order_by_items(Some(&clause), dialect)?;

    let mut projected = vec![None; columns.len()];
    for (priority, item) in order_by.iter().enumerate() {
        if let Some(column) = direct_column(item, columns, dialect) {
            projected[column].get_or_insert(RelationColumnSort {
                direction: direction(item),
                priority,
            });
        }
    }
    Ok(projected)
}

pub fn cycle_relation_column_sort(
    order_by_clause: &str,
    columns: &[impl AsRef<str>],
    column: usize,
    dialect: SqlDialect,
) -> Result<String, RelationFilterError> {
    let options = validate_relation_preview_options("", order_by_clause, dialect)?;
    let mut items = parse_order_by_items(options.order_by_clause.as_deref(), dialect)?;
    let target = columns
        .get(column)
        .ok_or_else(|| RelationFilterError("column index is out of range".into()))?;
    if items.is_empty() {
        items.push(order_by_column(target.as_ref(), dialect));
        items.last_mut().expect("just pushed").options.asc = Some(false);
        return Ok(format_order_by(&items, dialect));
    }

    let matches: Vec<usize> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            (direct_column(item, columns, dialect) == Some(column)).then_some(index)
        })
        .collect();
    let next = match matches.first().copied() {
        None => SortDirection::Desc,
        Some(index) if items[index].options.asc == Some(false) => SortDirection::Asc,
        Some(_) => {
            for index in matches.into_iter().rev() {
                items.remove(index);
            }
            return Ok(format_order_by(&items, dialect));
        }
    };

    if let Some(index) = matches.first().copied() {
        items[index].options.asc = Some(next == SortDirection::Asc);
    } else {
        items.push(order_by_column(target.as_ref(), dialect));
        items.last_mut().expect("just pushed").options.asc = Some(next == SortDirection::Asc);
    }
    Ok(format_order_by(&items, dialect))
}

fn order_by_column(name: &str, dialect: SqlDialect) -> OrderByExpr {
    if dialect == SqlDialect::BigQuery {
        return OrderByExpr::from(Ident::with_quote('`', name));
    }
    let quote = if dialect == SqlDialect::SqlServer {
        '['
    } else if dialect == SqlDialect::MySql {
        '`'
    } else {
        '"'
    };
    let name = if quote == '[' {
        name.replace(']', "]]")
    } else if quote == '`' {
        name.replace('`', "``")
    } else {
        name.replace('"', "\"\"")
    };
    OrderByExpr::from(Ident::with_quote(quote, name))
}

fn parse_order_by_items(
    order_by_clause: Option<&str>,
    dialect: SqlDialect,
) -> Result<Vec<OrderByExpr>, RelationFilterError> {
    let statements = parse_preview_query(None, order_by_clause, dialect)?;
    let Some(Statement::Query(query)) = statements.first() else {
        return Err(RelationFilterError("invalid preview query shape".into()));
    };
    match query.order_by.as_ref().map(|order| &order.kind) {
        Some(OrderByKind::Expressions(items)) => Ok(items.clone()),
        Some(OrderByKind::All(_)) | None => Ok(Vec::new()),
    }
}

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
    let statements =
        parse_preview_query(where_clause.as_deref(), order_by_clause.as_deref(), dialect)?;
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

fn parse_preview_query(
    where_clause: Option<&str>,
    order_by_clause: Option<&str>,
    dialect: SqlDialect,
) -> Result<Vec<Statement>, RelationFilterError> {
    let mut query = "SELECT * FROM __lazydb_relation".to_owned();
    if let Some(clause) = where_clause {
        query.push_str(" WHERE ");
        query.push_str(clause);
    }
    if let Some(clause) = order_by_clause {
        query.push_str(" ORDER BY ");
        query.push_str(clause);
    }
    parse_sql(dialect, &query)
        .map_err(|error| RelationFilterError(format!("invalid preview clause: {error}")))
}

fn direct_column(
    item: &OrderByExpr,
    columns: &[impl AsRef<str>],
    dialect: SqlDialect,
) -> Option<usize> {
    let name = match &item.expr {
        Expr::Identifier(identifier) => identifier,
        _ => return None,
    };
    columns.iter().position(|column| {
        if name.quote_style.is_some() {
            name.value == column.as_ref()
        } else {
            name.value.eq_ignore_ascii_case(column.as_ref())
                || quote_identifier(column.as_ref(), dialect) == name.to_string()
        }
    })
}

fn direction(item: &OrderByExpr) -> SortDirection {
    match item.options.asc {
        Some(true) | None => SortDirection::Asc,
        Some(false) => SortDirection::Desc,
    }
}

fn format_order_by(items: &[OrderByExpr], dialect: SqlDialect) -> String {
    items
        .iter()
        .map(|item| {
            if dialect == SqlDialect::BigQuery
                && let Expr::Identifier(name) = &item.expr
            {
                // sqlparser doubles backticks; GoogleSQL requires backslash escapes.
                let mut item = item.clone();
                item.expr = Expr::Identifier(Ident::new(quote_identifier(&name.value, dialect)));
                item.to_string()
            } else {
                item.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
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

    #[test]
    fn header_sort_projection_maps_direction_and_priority() {
        let projection = relation_column_sort_projection(
            "id DESC, name, created_at ASC",
            &["id", "name", "created_at", "other"],
            SqlDialect::Postgres,
        )
        .unwrap();
        assert_eq!(
            projection,
            vec![
                Some(RelationColumnSort {
                    direction: SortDirection::Desc,
                    priority: 0,
                }),
                Some(RelationColumnSort {
                    direction: SortDirection::Asc,
                    priority: 1,
                }),
                Some(RelationColumnSort {
                    direction: SortDirection::Asc,
                    priority: 2,
                }),
                None,
            ]
        );
    }

    #[test]
    fn header_sort_projection_ignores_complex_terms_and_supports_quoted_names() {
        let projection = relation_column_sort_projection(
            "COALESCE(name, username) DESC, \"display, name\" ASC",
            &["name", "username", "display, name"],
            SqlDialect::Postgres,
        )
        .unwrap();
        assert_eq!(projection[0], None);
        assert_eq!(projection[1], None);
        assert_eq!(
            projection[2],
            Some(RelationColumnSort {
                direction: SortDirection::Asc,
                priority: 1,
            })
        );
    }

    #[test]
    fn header_sort_cycle_is_desc_asc_none_and_appends() {
        let columns = ["id", "name"];
        assert_eq!(
            cycle_relation_column_sort("", &columns, 0, SqlDialect::Postgres).unwrap(),
            "\"id\" DESC"
        );
        assert_eq!(
            cycle_relation_column_sort("id DESC", &columns, 0, SqlDialect::Postgres).unwrap(),
            "id ASC"
        );
        assert_eq!(
            cycle_relation_column_sort("id ASC", &columns, 0, SqlDialect::Postgres).unwrap(),
            ""
        );
        assert_eq!(
            cycle_relation_column_sort("id DESC", &columns, 1, SqlDialect::Postgres).unwrap(),
            "id DESC, \"name\" DESC"
        );
    }

    #[test]
    fn header_sort_cycle_preserves_other_order_modifiers() {
        assert_eq!(
            cycle_relation_column_sort(
                "created_at DESC NULLS LAST, id ASC",
                &["created_at", "id"],
                0,
                SqlDialect::Postgres,
            )
            .unwrap(),
            "created_at ASC NULLS LAST, id ASC"
        );
    }

    #[test]
    fn header_sort_cycle_quotes_special_names_for_each_dialect() {
        assert_eq!(
            cycle_relation_column_sort("", &["order"], 0, SqlDialect::MySql).unwrap(),
            "`order` DESC"
        );
        assert_eq!(
            cycle_relation_column_sort("", &["customer]id"], 0, SqlDialect::SqlServer).unwrap(),
            "[customer]]id] DESC"
        );
    }
}
