use sqlparser::{
    ast::{Query, Select, SetExpr, Statement, TableFactor},
    parser::Parser,
};

use super::{SqlDialect, dialect::parser_dialect};

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
    let parsed = Parser::parse_sql(parser_dialect(dialect), sql);
    let risks = match parsed {
        Ok(statements) => statements.iter().map(classify_statement).collect(),
        Err(_) => vec![SqlRisk::Unknown],
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
