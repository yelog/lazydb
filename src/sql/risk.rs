use sqlparser::{
    ast::{Query, Select, SetExpr, Statement, TableFactor},
    parser::Parser,
};

use super::{SqlDialect, dialect::parser_dialect, scan_statements, split_sql_server_batches};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SqlRisk {
    ReadOnly,
    Dml,
    Ddl,
    TransactionControl,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SqlRiskAggregate {
    Empty,
    Single(SqlRisk),
    MultiStatement,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqlRiskAnalysis {
    pub statement_count: usize,
    pub risks: Vec<SqlRisk>,
    pub aggregate: SqlRiskAggregate,
}

pub fn classify_sql(sql: &str, dialect: SqlDialect) -> SqlRiskAnalysis {
    let risks = if dialect == SqlDialect::ClickHouse {
        if contains_clickhouse_mutation(sql) {
            vec![SqlRisk::Unknown]
        } else {
            parse_risks(sql, dialect)
        }
    } else if dialect == SqlDialect::SqlServer {
        match split_sql_server_batches(sql) {
            Ok(batches) => batches
                .into_iter()
                .flat_map(|batch| parse_risks(batch, dialect))
                .collect(),
            Err(_) => vec![SqlRisk::Unknown],
        }
    } else {
        scan_statements(sql, dialect)
            .into_iter()
            .flat_map(|range| {
                range.get(sql).map_or_else(
                    || vec![SqlRisk::Unknown],
                    |statement| parse_risks(statement, dialect),
                )
            })
            .collect()
    };
    let statement_count = risks.len();
    let aggregate = match risks.as_slice() {
        [] => SqlRiskAggregate::Empty,
        [risk] => SqlRiskAggregate::Single(*risk),
        _ => SqlRiskAggregate::MultiStatement,
    };
    SqlRiskAnalysis {
        statement_count,
        risks,
        aggregate,
    }
}

fn contains_clickhouse_mutation(sql: &str) -> bool {
    let statements = match Parser::parse_sql(parser_dialect(SqlDialect::ClickHouse), sql) {
        Ok(statements) => statements,
        Err(_) => return true,
    };
    if statements.is_empty() {
        return false;
    }
    let known_count = super::scope::scan_statements(sql, SqlDialect::ClickHouse).len();
    if known_count != statements.len() {
        return true;
    }
    statements.iter().any(|statement| match statement {
        Statement::Update(_) | Statement::Delete(_) => true,
        Statement::AlterTable(_) => {
            // ClickHouse mutations use a non-transactional ALTER syntax. The
            // generic AST does not represent those operations reliably, so
            // keep every ClickHouse ALTER behind explicit write approval.
            true
        }
        Statement::Query(_) => false,
        Statement::Set(_) => true,
        Statement::Kill { .. } => true,
        Statement::OptimizeTable { .. } | Statement::Truncate(_) => true,
        Statement::CreateUser(_) | Statement::AlterUser(_) | Statement::CreateRole(_) => true,
        _ => false,
    })
}

fn parse_risks(sql: &str, dialect: SqlDialect) -> Vec<SqlRisk> {
    match Parser::parse_sql(parser_dialect(dialect), sql) {
        Ok(statements) => statements
            .iter()
            .map(|statement| {
                if dialect == SqlDialect::DuckDb
                    && (matches!(statement, Statement::CreateSecret { .. })
                        || contains_duckdb_sensitive_function(statement))
                {
                    SqlRisk::Unknown
                } else if dialect == SqlDialect::Snowflake
                    && contains_snowflake_side_effect(statement)
                {
                    SqlRisk::Unknown
                } else {
                    classify_statement(statement)
                }
            })
            .collect(),
        Err(_) => vec![SqlRisk::Unknown],
    }
}

fn classify_statement(statement: &Statement) -> SqlRisk {
    match statement {
        Statement::Query(query) => classify_query(query),
        Statement::Insert(insert) => combine(
            SqlRisk::Dml,
            insert
                .source
                .as_deref()
                .map(classify_query)
                .unwrap_or(SqlRisk::ReadOnly),
        ),
        Statement::Update(_) | Statement::Delete(_) | Statement::Merge(_) => SqlRisk::Dml,
        Statement::Call(_) | Statement::Execute { .. } | Statement::Prepare { .. } => {
            SqlRisk::Unknown
        }
        Statement::Explain { statement, .. } => classify_statement(statement),
        Statement::ExplainTable { .. } => SqlRisk::ReadOnly,
        Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. }
        | Statement::Savepoint { .. }
        | Statement::ReleaseSavepoint { .. } => SqlRisk::TransactionControl,
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
        | Statement::CreateSequence { .. }
        | Statement::CreateDomain(_)
        | Statement::CreateType { .. }
        | Statement::CreateUser(_)
        | Statement::AlterUser(_) => SqlRisk::Ddl,
        Statement::Truncate(_) => SqlRisk::Ddl,
        _ => SqlRisk::Unknown,
    }
}

fn contains_duckdb_sensitive_function(statement: &Statement) -> bool {
    fn query_contains(query: &Query) -> bool {
        let sql = query.to_string().to_ascii_lowercase();
        [
            "nextval(",
            "setval(",
            "read_csv",
            "read_parquet",
            "read_json",
            "glob(",
        ]
        .iter()
        .any(|function| sql.contains(function))
            || query
                .with
                .as_ref()
                .is_some_and(|with| with.cte_tables.iter().any(|cte| query_contains(&cte.query)))
    }
    match statement {
        Statement::Query(query) => query_contains(query),
        Statement::Explain { statement, .. } => contains_duckdb_sensitive_function(statement),
        _ => false,
    }
}

fn contains_snowflake_side_effect(statement: &Statement) -> bool {
    use std::ops::ControlFlow;

    use sqlparser::ast::Expr;

    // Inspect expressions rather than rendered SQL so strings/comments do not
    // trigger approval, and nested queries cannot hide sequence advancement.
    sqlparser::ast::visit_expressions(statement, |expr| {
        let sensitive = match expr {
            Expr::CompoundIdentifier(parts) => parts
                .last()
                .is_some_and(|ident| ident.value.eq_ignore_ascii_case("NEXTVAL")),
            Expr::Function(function) => function
                .name
                .0
                .last()
                .and_then(|part| part.as_ident())
                .is_some_and(|ident| {
                    let name = ident.value.to_ascii_uppercase();
                    // SYSTEM$ includes administrative and state-changing APIs.
                    // Keep the entire family behind explicit approval.
                    name.starts_with("SYSTEM$") || matches!(name.as_str(), "NEXTVAL" | "GETNEXTVAL")
                }),
            _ => false,
        };
        if sensitive {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })
    .is_break()
}

fn classify_query(query: &Query) -> SqlRisk {
    let mut risk = query
        .with
        .as_ref()
        .map(|with| {
            with.cte_tables
                .iter()
                .map(|cte| classify_query(&cte.query))
                .fold(SqlRisk::ReadOnly, combine)
        })
        .unwrap_or(SqlRisk::ReadOnly);
    risk = combine(risk, classify_set_expr(&query.body));
    if !query.locks.is_empty() {
        risk = combine(risk, SqlRisk::Dml);
    }
    risk
}

fn classify_set_expr(expr: &SetExpr) -> SqlRisk {
    match expr {
        SetExpr::Select(select) => classify_select(select),
        SetExpr::Query(query) => classify_query(query),
        SetExpr::SetOperation { left, right, .. } => {
            combine(classify_set_expr(left), classify_set_expr(right))
        }
        SetExpr::Values(_) | SetExpr::Table(_) => SqlRisk::ReadOnly,
        SetExpr::Insert(statement)
        | SetExpr::Update(statement)
        | SetExpr::Delete(statement)
        | SetExpr::Merge(statement) => classify_statement(statement),
    }
}

fn classify_select(select: &Select) -> SqlRisk {
    let mut risk = if select.into.is_some() {
        SqlRisk::Dml
    } else {
        SqlRisk::ReadOnly
    };
    for table in &select.from {
        risk = combine(risk, classify_table_factor(&table.relation));
        for join in &table.joins {
            risk = combine(risk, classify_table_factor(&join.relation));
        }
    }
    risk
}

fn classify_table_factor(factor: &TableFactor) -> SqlRisk {
    match factor {
        TableFactor::Derived { subquery, .. } => classify_query(subquery),
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => {
            let mut risk = classify_table_factor(&table_with_joins.relation);
            for join in &table_with_joins.joins {
                risk = combine(risk, classify_table_factor(&join.relation));
            }
            risk
        }
        _ => SqlRisk::ReadOnly,
    }
}

fn combine(left: SqlRisk, right: SqlRisk) -> SqlRisk {
    left.max(right)
}
