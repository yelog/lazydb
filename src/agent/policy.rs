use clap::ValueEnum;
use thiserror::Error;

use crate::{
    profile::{ConnectionProfile, Environment},
    sql::{SqlDialect, SqlRisk, SqlRiskAggregate, classify_sql},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum WritePolicy {
    #[default]
    Deny,
    NonProduction,
    All,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PolicyError {
    #[error("a single read-only SQL statement is required")]
    ReadOnlyQueryRequired,
    #[error("the selected connection is configured as read-only")]
    ProfileReadOnly,
    #[error(
        "the MCP server write policy is deny; restart it with --write-policy non-production for writable development or staging connections"
    )]
    ServerWritePolicyDenied,
    #[error("production writes require --write-policy all")]
    ProductionWriteDisabled,
    #[error("the SQL statement could not be classified safely")]
    UnknownSql,
    #[error("transaction-control statements are not supported by agent execution")]
    TransactionControlDisabled,
    #[error("SQL must not be empty")]
    EmptySql,
}

pub fn authorize_query(profile: &ConnectionProfile, sql: &str) -> Result<(), PolicyError> {
    if sql.trim().is_empty() {
        return Err(PolicyError::EmptySql);
    }
    let analysis = classify_sql(sql, dialect(profile));
    if is_read_only(&analysis) {
        Ok(())
    } else {
        Err(PolicyError::ReadOnlyQueryRequired)
    }
}

pub fn authorize_write(
    profile: &ConnectionProfile,
    policy: WritePolicy,
    sql: &str,
) -> Result<(), PolicyError> {
    if sql.trim().is_empty() {
        return Err(PolicyError::EmptySql);
    }
    let analysis = classify_sql(sql, dialect(profile));
    if analysis.risks.contains(&SqlRisk::Unknown) {
        return Err(PolicyError::UnknownSql);
    }
    if analysis.risks.contains(&SqlRisk::TransactionControl) {
        return Err(PolicyError::TransactionControlDisabled);
    }
    if profile.read_only {
        return Err(PolicyError::ProfileReadOnly);
    }
    if policy == WritePolicy::Deny {
        return Err(PolicyError::ServerWritePolicyDenied);
    }
    if profile.environment == Environment::Production && policy != WritePolicy::All {
        return Err(PolicyError::ProductionWriteDisabled);
    }
    if matches!(
        analysis.aggregate,
        SqlRiskAggregate::Single(SqlRisk::Dml | SqlRisk::Ddl)
    ) || matches!(analysis.aggregate, SqlRiskAggregate::MultiStatement)
    {
        Ok(())
    } else {
        Err(PolicyError::UnknownSql)
    }
}

fn is_read_only(analysis: &crate::sql::SqlRiskAnalysis) -> bool {
    matches!(
        analysis.aggregate,
        SqlRiskAggregate::Single(SqlRisk::ReadOnly)
    )
}

fn dialect(profile: &ConnectionProfile) -> SqlDialect {
    match profile.kind {
        crate::profile::DatabaseKind::Postgres => SqlDialect::Postgres,
        crate::profile::DatabaseKind::MySql => SqlDialect::MySql,
        crate::profile::DatabaseKind::Sqlite => SqlDialect::Sqlite,
    }
}
