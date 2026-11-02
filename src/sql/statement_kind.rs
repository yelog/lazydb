use sqlparser::ast::{Query, SetExpr, Statement, TableFactor};

use super::{SqlDialect, split_sql_server_batches};

/// A user-facing family of SQL statements.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SqlStatementKind {
    Dql,
    Dml,
    Ddl,
    Dcl,
    Tcl,
    Mixed,
    Other,
}

pub fn classify_statement_kind(sql: &str, dialect: SqlDialect) -> SqlStatementKind {
    let batches = if dialect == SqlDialect::SqlServer {
        match split_sql_server_batches(sql) {
            Ok(batches) => batches,
            Err(_) => return SqlStatementKind::Other,
        }
    } else {
        vec![sql]
    };

    let mut kinds = Vec::new();
    for batch in batches {
        let statements = match super::dialect::parse_sql(dialect, batch) {
            Ok(statements) => statements,
            Err(_) => return SqlStatementKind::Other,
        };
        for statement in statements {
            kinds.push(classify_statement(&statement));
        }
    }

    if kinds.is_empty() || kinds.contains(&SqlStatementKind::Other) {
        return SqlStatementKind::Other;
    }
    let first = kinds[0];
    if kinds.iter().all(|kind| *kind == first) {
        first
    } else {
        SqlStatementKind::Mixed
    }
}

fn classify_statement(statement: &Statement) -> SqlStatementKind {
    match statement {
        Statement::Query(query) => classify_query(query),
        Statement::Insert(_) => SqlStatementKind::Dml,
        Statement::Update(_) | Statement::Delete(_) | Statement::Merge(_) => SqlStatementKind::Dml,
        Statement::Explain { statement, .. } => classify_statement(statement),
        Statement::ExplainTable { .. }
        | Statement::ShowFunctions { .. }
        | Statement::ShowVariable { .. }
        | Statement::ShowStatus { .. }
        | Statement::ShowVariables { .. }
        | Statement::ShowCreate { .. }
        | Statement::ShowColumns { .. }
        | Statement::ShowCatalogs { .. }
        | Statement::ShowDatabases { .. }
        | Statement::ShowProcessList { .. }
        | Statement::ShowSchemas { .. }
        | Statement::ShowCharset(_)
        | Statement::ShowObjects(_)
        | Statement::ShowTables { .. }
        | Statement::ShowViews { .. }
        | Statement::ShowCollation { .. } => SqlStatementKind::Dql,
        Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. } => SqlStatementKind::Tcl,
        Statement::Grant(_) | Statement::Deny(_) | Statement::Revoke(_) => SqlStatementKind::Dcl,
        Statement::CreateView(_)
        | Statement::CreateTable(_)
        | Statement::CreateVirtualTable { .. }
        | Statement::CreateIndex(_)
        | Statement::CreateRole(_)
        | Statement::CreateSecret { .. }
        | Statement::CreateServer(_)
        | Statement::CreatePolicy(_)
        | Statement::CreateConnector(_)
        | Statement::CreateOperator(_)
        | Statement::CreateOperatorFamily(_)
        | Statement::CreateOperatorClass(_)
        | Statement::AlterTable(_)
        | Statement::AlterSchema(_)
        | Statement::AlterIndex { .. }
        | Statement::AlterView { .. }
        | Statement::AlterFunction(_)
        | Statement::AlterType(_)
        | Statement::AlterCollation(_)
        | Statement::AlterOperator(_)
        | Statement::AlterOperatorFamily(_)
        | Statement::AlterOperatorClass(_)
        | Statement::AlterRole { .. }
        | Statement::AlterPolicy(_)
        | Statement::AlterConnector { .. }
        | Statement::AlterSession { .. }
        | Statement::Drop { .. }
        | Statement::DropFunction(_)
        | Statement::DropDomain(_)
        | Statement::DropProcedure { .. }
        | Statement::DropSecret { .. }
        | Statement::DropPolicy(_)
        | Statement::DropConnector { .. }
        | Statement::CreateExtension(_)
        | Statement::CreateCollation(_)
        | Statement::DropExtension(_)
        | Statement::DropOperator(_)
        | Statement::DropOperatorFamily(_)
        | Statement::DropOperatorClass(_)
        | Statement::CreateSchema { .. }
        | Statement::CreateDatabase { .. }
        | Statement::CreateFunction(_)
        | Statement::CreateTrigger(_)
        | Statement::DropTrigger(_)
        | Statement::CreateProcedure { .. }
        | Statement::CreateMacro { .. }
        | Statement::CreateStage { .. }
        | Statement::Truncate(_) => SqlStatementKind::Ddl,
        _ => SqlStatementKind::Other,
    }
}

fn classify_query(query: &Query) -> SqlStatementKind {
    let mut kind = classify_set_expr(&query.body);
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            kind = combine(kind, classify_query(&cte.query));
        }
    }
    kind
}

fn classify_set_expr(expr: &SetExpr) -> SqlStatementKind {
    match expr {
        SetExpr::Select(select) => {
            if select.into.is_some() {
                SqlStatementKind::Ddl
            } else {
                select
                    .from
                    .iter()
                    .flat_map(|table| {
                        std::iter::once(&table.relation)
                            .chain(table.joins.iter().map(|join| &join.relation))
                    })
                    .map(classify_table_factor)
                    .fold(SqlStatementKind::Dql, combine)
            }
        }
        SetExpr::Query(query) => classify_query(query),
        SetExpr::SetOperation { left, right, .. } => {
            combine(classify_set_expr(left), classify_set_expr(right))
        }
        SetExpr::Values(_) | SetExpr::Table(_) => SqlStatementKind::Dql,
        SetExpr::Insert(statement)
        | SetExpr::Update(statement)
        | SetExpr::Delete(statement)
        | SetExpr::Merge(statement) => classify_statement(statement),
    }
}

fn classify_table_factor(factor: &TableFactor) -> SqlStatementKind {
    match factor {
        TableFactor::Derived { subquery, .. } => classify_query(subquery),
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => table_with_joins
            .joins
            .iter()
            .map(|join| classify_table_factor(&join.relation))
            .fold(classify_table_factor(&table_with_joins.relation), combine),
        _ => SqlStatementKind::Dql,
    }
}

fn combine(left: SqlStatementKind, right: SqlStatementKind) -> SqlStatementKind {
    if left == right {
        left
    } else if left == SqlStatementKind::Other || right == SqlStatementKind::Other {
        SqlStatementKind::Other
    } else if left == SqlStatementKind::Dql {
        right
    } else if right == SqlStatementKind::Dql {
        left
    } else {
        SqlStatementKind::Mixed
    }
}
