use std::ops::ControlFlow;

use sqlparser::{
    ast::{Expr, Ident, ObjectName, Query, Select, Statement, TableFactor, Visit, Visitor},
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
                let mut visitor = RiskVisitor {
                    dialect,
                    risk: SqlRisk::ReadOnly,
                };
                let _ = statement.visit(&mut visitor);
                visitor.risk
            })
            .collect(),
        Err(_) => vec![SqlRisk::Unknown],
    }
}

fn classify_statement(statement: &Statement) -> SqlRisk {
    match statement {
        // Queries and nested statements are classified by RiskVisitor.
        Statement::Query(_) => SqlRisk::ReadOnly,
        Statement::Insert(_)
        | Statement::Update(_)
        | Statement::Delete(_)
        | Statement::Merge(_) => SqlRisk::Dml,
        Statement::Call(_) | Statement::Execute { .. } | Statement::Prepare { .. } => {
            SqlRisk::Unknown
        }
        Statement::Explain { .. } => SqlRisk::ReadOnly,
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

struct RiskVisitor {
    dialect: SqlDialect,
    risk: SqlRisk,
}

impl RiskVisitor {
    fn include(&mut self, risk: SqlRisk) {
        self.risk = self.risk.max(risk);
    }

    fn function(&mut self, name: &ObjectName) {
        let Some(name) = name.0.last().and_then(|part| part.as_ident()) else {
            return;
        };
        if matches!(
            self.dialect,
            SqlDialect::Postgres
                | SqlDialect::MySql
                | SqlDialect::MariaDb
                | SqlDialect::DuckDb
                | SqlDialect::Generic
        ) && (self.function_name_is(name, "nextval") || self.function_name_is(name, "setval"))
        {
            self.include(if self.dialect == SqlDialect::DuckDb {
                SqlRisk::Unknown
            } else {
                SqlRisk::Dml
            });
        }
        if self.dialect == SqlDialect::Snowflake {
            let name = name.value.to_ascii_uppercase();
            // SYSTEM$ includes administrative and state-changing APIs.
            // Keep the entire family behind explicit approval.
            if name.starts_with("SYSTEM$") || matches!(name.as_str(), "NEXTVAL" | "GETNEXTVAL") {
                self.include(SqlRisk::Unknown);
            }
        }
        if self.dialect == SqlDialect::DuckDb {
            let name = name.value.to_ascii_lowercase();
            if name.starts_with("read_csv")
                || name.starts_with("read_parquet")
                || name.starts_with("read_json")
                || name == "glob"
            {
                self.include(SqlRisk::Unknown);
            }
        }
    }

    fn function_name_is(&self, name: &Ident, expected: &str) -> bool {
        if self.dialect == SqlDialect::Postgres && name.quote_style.is_some() {
            name.value == expected
        } else {
            name.value.eq_ignore_ascii_case(expected)
        }
    }
}

impl Visitor for RiskVisitor {
    type Break = ();

    fn pre_visit_statement(&mut self, statement: &Statement) -> ControlFlow<Self::Break> {
        self.include(
            if self.dialect == SqlDialect::DuckDb
                && matches!(statement, Statement::CreateSecret { .. })
            {
                SqlRisk::Unknown
            } else {
                classify_statement(statement)
            },
        );
        ControlFlow::Continue(())
    }

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        if !query.locks.is_empty() {
            self.include(SqlRisk::Dml);
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_select(&mut self, select: &Select) -> ControlFlow<Self::Break> {
        if select.into.is_some() {
            self.include(SqlRisk::Dml);
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expression: &Expr) -> ControlFlow<Self::Break> {
        match expression {
            Expr::Function(function) => self.function(&function.name),
            // Inspect AST nodes so literals and aliases do not trigger approval.
            Expr::CompoundIdentifier(parts)
                if self.dialect == SqlDialect::Snowflake
                    && parts
                        .last()
                        .is_some_and(|name| name.value.eq_ignore_ascii_case("nextval")) =>
            {
                self.include(SqlRisk::Unknown);
            }
            // Oracle sequence pseudocolumns are identifiers, not function calls.
            Expr::CompoundIdentifier(parts)
                if self.dialect == SqlDialect::Oracle
                    && parts.last().is_some_and(|name| {
                        name.quote_style.is_none() && name.value.eq_ignore_ascii_case("nextval")
                    }) =>
            {
                self.include(SqlRisk::Dml);
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(&mut self, factor: &TableFactor) -> ControlFlow<Self::Break> {
        match factor {
            TableFactor::Table {
                name,
                args: Some(_),
                ..
            }
            | TableFactor::Function { name, .. } => {
                self.function(name);
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}
