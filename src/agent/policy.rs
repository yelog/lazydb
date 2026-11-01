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

impl WritePolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::NonProduction => "non-production",
            Self::All => "all",
        }
    }
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

impl PolicyError {
    pub const fn reason(self) -> &'static str {
        match self {
            Self::ProfileReadOnly => "profile_read_only",
            Self::ServerWritePolicyDenied => "server_policy",
            Self::ProductionWriteDisabled => "production_policy",
            Self::ReadOnlyQueryRequired
            | Self::UnknownSql
            | Self::TransactionControlDisabled
            | Self::EmptySql => "sql_policy",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteCapability {
    Allowed,
    Denied(PolicyError),
}

pub fn write_capability(profile: &ConnectionProfile, policy: WritePolicy) -> WriteCapability {
    if profile.read_only {
        WriteCapability::Denied(PolicyError::ProfileReadOnly)
    } else if policy == WritePolicy::Deny {
        WriteCapability::Denied(PolicyError::ServerWritePolicyDenied)
    } else if profile.environment == Environment::Production && policy != WritePolicy::All {
        WriteCapability::Denied(PolicyError::ProductionWriteDisabled)
    } else {
        WriteCapability::Allowed
    }
}

pub fn authorize_query(profile: &ConnectionProfile, sql: &str) -> Result<(), PolicyError> {
    if sql.trim().is_empty() {
        return Err(PolicyError::EmptySql);
    }
    if profile.kind == crate::profile::DatabaseKind::Elasticsearch {
        return crate::db::elasticsearch::ElasticsearchRequest::parse(sql)
            .ok()
            .filter(|request| request.is_read_only())
            .map(|_| ())
            .ok_or(PolicyError::ReadOnlyQueryRequired);
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
    if profile.kind == crate::profile::DatabaseKind::Elasticsearch {
        let request = crate::db::elasticsearch::ElasticsearchRequest::parse(sql)
            .map_err(|_| PolicyError::UnknownSql)?;
        if let WriteCapability::Denied(error) = write_capability(profile, policy) {
            return Err(error);
        }
        return if request.is_read_only() {
            Err(PolicyError::UnknownSql)
        } else {
            Ok(())
        };
    }
    let analysis = classify_sql(sql, dialect(profile));
    if analysis.risks.contains(&SqlRisk::Unknown) {
        return Err(PolicyError::UnknownSql);
    }
    if analysis.risks.contains(&SqlRisk::TransactionControl) {
        return Err(PolicyError::TransactionControlDisabled);
    }
    if let WriteCapability::Denied(error) = write_capability(profile, policy) {
        return Err(error);
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
    SqlDialect::for_database_kind(profile.kind)
}
