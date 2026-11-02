use std::{sync::Arc, time::Duration};
use tokio::time::Instant;
use uuid::Uuid;

pub mod capabilities;
pub mod catalog;
pub mod catalog_change_set;
pub mod catalog_drop;
pub mod catalog_mutation;
pub mod clickhouse;
pub mod cockroach;
pub mod column_type;
pub(crate) mod ddl;
pub mod descriptor;
pub mod duckdb;
pub mod duckdb_runtime;
pub mod elasticsearch;
pub mod mongodb;
pub mod monitor;
pub mod mssql;
pub mod mutation;
pub mod mysql;
pub mod oracle;
pub mod oracle_client;
pub(crate) mod pgwire;
pub mod postgres;
pub mod principal;
pub mod principal_drop;
pub mod query;
pub mod redis;
pub mod relation_plan;
pub mod snowflake;
pub mod sqlite;
pub mod transaction;
pub mod value;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    identity::ConnectionIdentity,
    model::execution_target::ExecutionTarget,
    profile::{ConnectionProfile, DatabaseKind},
    security::sanitize_terminal_text,
};

use self::{
    catalog::{
        CatalogCapabilities, CatalogDiscovery, CatalogId, CatalogKind, CatalogPage, CatalogRequest,
        CatalogSearchPage, CatalogSearchRequest, CatalogTarget, CatalogValidationError,
        RelationDdl,
    },
    catalog_mutation::CatalogMutationCapabilities,
    catalog_mutation::{CatalogObjectDefinition, CatalogObjectDefinitionRequest},
    clickhouse::ClickHouseAdapter,
    cockroach::CockroachAdapter,
    elasticsearch::ElasticsearchAdapter,
    mssql::MsSqlAdapter,
    mysql::MySqlAdapter,
    oracle::OracleAdapter,
    postgres::PostgresAdapter,
    principal::{PrincipalDdl, PrincipalEntry, PrincipalPage},
    query::{QueryBudget, QueryOutcome},
    redis::RedisAdapter,
    sqlite::SqliteAdapter,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServerInfo {
    pub kind: DatabaseKind,
    pub version: String,
    pub database: String,
    #[serde(default)]
    pub current_user: Option<String>,
}

pub use query::RELATION_PREVIEW_LIMIT;

#[derive(Clone, Debug, PartialEq)]
pub struct RelationPreview {
    pub sql: String,
    pub result: QueryOutcome,
    pub pagination: crate::model::pagination::ResultPagination,
    pub row_versions: Option<Vec<mutation::RowVersion>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorCategory {
    Configuration,
    Authentication,
    Network,
    Permission,
    Sql,
    Constraint,
    Unsupported,
    Cancelled,
    Internal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryErrorSnapshot {
    pub category: ErrorCategory,
    pub code: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DatabaseErrorPosition {
    Original(usize),
    Internal { position: usize, query: String },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DatabaseDiagnostic {
    pub severity: Option<String>,
    pub detail: Option<String>,
    pub hint: Option<String>,
    pub position: Option<DatabaseErrorPosition>,
    pub context: Option<String>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("{message}")]
pub struct DatabaseError {
    pub category: ErrorCategory,
    pub code: Option<String>,
    pub message: String,
    pub diagnostic: Option<Box<DatabaseDiagnostic>>,
}

impl DatabaseError {
    pub(crate) fn internal(message: impl AsRef<str>) -> Self {
        Self {
            category: ErrorCategory::Internal,
            code: Some("internal".into()),
            message: sanitize_terminal_text(message.as_ref()),
            diagnostic: None,
        }
    }
    pub fn history_snapshot(&self) -> HistoryErrorSnapshot {
        HistoryErrorSnapshot {
            category: self.category,
            code: self.code.clone(),
            message: self.message.clone(),
        }
    }

    pub fn configuration(message: impl AsRef<str>) -> Self {
        Self {
            category: ErrorCategory::Configuration,
            code: None,
            message: sanitize_terminal_text(message.as_ref()),
            diagnostic: None,
        }
    }

    pub fn unsupported(message: impl AsRef<str>) -> Self {
        Self {
            category: ErrorCategory::Unsupported,
            code: Some("unsupported".to_owned()),
            message: sanitize_terminal_text(message.as_ref()),
            diagnostic: None,
        }
    }

    fn invalid_catalog_request(error: CatalogValidationError) -> Self {
        Self {
            category: ErrorCategory::Configuration,
            code: Some("invalid_catalog_request".to_owned()),
            message: sanitize_terminal_text(&format!("invalid catalog request: {error}")),
            diagnostic: None,
        }
    }

    fn unsupported_catalog_target(kind: DatabaseKind, target: &CatalogTarget) -> Self {
        Self {
            category: ErrorCategory::Unsupported,
            code: Some("catalog_target_unsupported".to_owned()),
            message: format!(
                "{} catalog target is not implemented for {kind:?}",
                target.description()
            ),
            diagnostic: None,
        }
    }

    pub(crate) fn from_sqlx(error: sqlx::Error, default_category: ErrorCategory) -> Self {
        if let sqlx::Error::Database(database) = &error {
            let message = sanitize_terminal_text(database.message());
            let lowered = message.to_ascii_lowercase();
            let category = if lowered.contains("constraint")
                || lowered.contains("duplicate")
                || lowered.contains("unique")
                || lowered.contains("foreign key")
            {
                ErrorCategory::Constraint
            } else if lowered.contains("permission") || lowered.contains("not authorized") {
                ErrorCategory::Permission
            } else if lowered.contains("password") || lowered.contains("authentication") {
                ErrorCategory::Authentication
            } else {
                default_category
            };
            let diagnostic = database
                .try_downcast_ref::<sqlx::postgres::PgDatabaseError>()
                .map(|postgres| {
                    Box::new(DatabaseDiagnostic {
                        severity: Some(format_postgres_severity(postgres.severity())),
                        detail: sanitized_optional(postgres.detail()),
                        hint: sanitized_optional(postgres.hint()),
                        position: postgres.position().map(|position| match position {
                            sqlx::postgres::PgErrorPosition::Original(position) => {
                                DatabaseErrorPosition::Original(position)
                            }
                            sqlx::postgres::PgErrorPosition::Internal { position, query } => {
                                DatabaseErrorPosition::Internal {
                                    position,
                                    query: sanitize_terminal_text(query),
                                }
                            }
                        }),
                        context: sanitized_optional(postgres.r#where()),
                    })
                });
            return Self {
                category,
                code: database.code().map(|code| code.into_owned()),
                message,
                diagnostic,
            };
        }

        Self {
            category: default_category,
            code: None,
            message: sanitize_terminal_text(&error.to_string()),
            diagnostic: None,
        }
    }

    pub fn output_message(&self) -> String {
        let mut lines = Vec::new();
        let severity = self
            .diagnostic
            .as_ref()
            .and_then(|diagnostic| diagnostic.severity.as_deref())
            .unwrap_or("ERROR");
        let code = self
            .code
            .as_deref()
            .map(|code| format!("[{code}] "))
            .unwrap_or_default();
        lines.push(format!("{code}{severity}: {}", self.message));

        if let Some(diagnostic) = &self.diagnostic {
            if let Some(detail) = &diagnostic.detail {
                lines.push(format_diagnostic_lines("Detail", detail));
            }
            if let Some(hint) = &diagnostic.hint {
                lines.push(format_diagnostic_lines("Hint", hint));
            }
            if let Some(position) = &diagnostic.position {
                match position {
                    DatabaseErrorPosition::Original(position) => {
                        lines.push(format!("Position: {position}"));
                    }
                    DatabaseErrorPosition::Internal { position, query } => {
                        lines.push(format!("Internal Position: {position}"));
                        lines.push(format_diagnostic_lines("Internal Query", query));
                    }
                }
            }
            if let Some(context) = &diagnostic.context {
                lines.push(format_diagnostic_lines("Context", context));
            }
        }
        lines.join("\n")
    }
}

fn format_diagnostic_lines(label: &str, value: &str) -> String {
    format!("{label}: {value}")
}

fn sanitized_optional(value: Option<&str>) -> Option<String> {
    value
        .map(sanitize_terminal_text)
        .filter(|value| !value.is_empty())
}

fn format_postgres_severity(severity: sqlx::postgres::PgSeverity) -> String {
    match severity {
        sqlx::postgres::PgSeverity::Panic => "PANIC",
        sqlx::postgres::PgSeverity::Fatal => "FATAL",
        sqlx::postgres::PgSeverity::Error => "ERROR",
        sqlx::postgres::PgSeverity::Warning => "WARNING",
        sqlx::postgres::PgSeverity::Notice => "NOTICE",
        sqlx::postgres::PgSeverity::Debug => "DEBUG",
        sqlx::postgres::PgSeverity::Info => "INFO",
        sqlx::postgres::PgSeverity::Log => "LOG",
    }
    .to_owned()
}

#[derive(Clone, Debug)]
pub enum DatabaseConnection {
    Postgres(PostgresAdapter),
    CockroachDb(CockroachAdapter),
    MySql(MySqlAdapter),
    MariaDb(MySqlAdapter),
    TiDb(MySqlAdapter),
    Oracle(OracleAdapter),
    Sqlite(SqliteAdapter),
    DuckDb(duckdb::DuckDbAdapter),
    SqlServer(MsSqlAdapter),
    Redis(RedisAdapter),
    MongoDb(mongodb::MongoAdapter),
    ClickHouse(ClickHouseAdapter),
    Snowflake(snowflake::SnowflakeAdapter),
    Elasticsearch(ElasticsearchAdapter),
    OpenSearch(ElasticsearchAdapter),
}

impl DatabaseConnection {
    pub fn principal_capability(&self) -> principal::PrincipalCapability {
        match self {
            Self::Snowflake(_) => principal::PrincipalCapability::Unsupported,
            Self::Postgres(_) => principal::PrincipalCapability::DetailsAndMutation,
            Self::MySql(_) | Self::MariaDb(_) | Self::SqlServer(_) | Self::Oracle(_) => {
                principal::PrincipalCapability::DdlOnly
            }
            Self::CockroachDb(_)
            | Self::TiDb(_)
            | Self::Sqlite(_)
            | Self::DuckDb(_)
            | Self::Redis(_) => principal::PrincipalCapability::Unsupported,
            Self::MongoDb(_) | Self::Elasticsearch(_) | Self::OpenSearch(_) => {
                principal::PrincipalCapability::Unsupported
            }
            Self::ClickHouse(_) => ClickHouseAdapter::principal_capability(),
        }
    }

    pub fn principal_mutation_capabilities(&self) -> principal::PrincipalMutationCapabilities {
        match self {
            Self::Snowflake(_) => principal::PrincipalMutationCapabilities {
                permission_targets: Vec::new(),
                privileges: Vec::new(),
                memberships: false,
            },
            Self::Postgres(_) => principal::PrincipalMutationCapabilities {
                permission_targets: vec![
                    principal::PrincipalMutationTargetKind::Database,
                    principal::PrincipalMutationTargetKind::Schema,
                    principal::PrincipalMutationTargetKind::Relation,
                    principal::PrincipalMutationTargetKind::Column,
                    principal::PrincipalMutationTargetKind::Membership,
                ],
                privileges: ["SELECT", "INSERT", "UPDATE", "DELETE", "USAGE", "EXECUTE"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                memberships: true,
            },
            Self::MySql(_) | Self::MariaDb(_) => principal::PrincipalMutationCapabilities {
                permission_targets: vec![principal::PrincipalMutationTargetKind::Relation],
                privileges: Vec::new(),
                memberships: false,
            },
            Self::SqlServer(_) | Self::Oracle(_) => principal::PrincipalMutationCapabilities {
                permission_targets: Vec::new(),
                privileges: Vec::new(),
                memberships: false,
            },
            Self::CockroachDb(_)
            | Self::TiDb(_)
            | Self::Sqlite(_)
            | Self::DuckDb(_)
            | Self::Redis(_)
            | Self::MongoDb(_)
            | Self::Elasticsearch(_)
            | Self::OpenSearch(_) => principal::PrincipalMutationCapabilities {
                permission_targets: Vec::new(),
                privileges: Vec::new(),
                memberships: false,
            },
            Self::ClickHouse(_) => ClickHouseAdapter::principal_mutation_capabilities(),
        }
    }

    pub async fn list_principals(&self) -> Result<PrincipalPage, DatabaseError> {
        match self {
            Self::Snowflake(_) => Err(DatabaseError::unsupported(
                "Snowflake principal management is not implemented",
            )),
            Self::Postgres(adapter) => adapter.list_principals().await,
            Self::CockroachDb(_) => Err(DatabaseError::unsupported(
                "CockroachDB principal management is not implemented",
            )),
            Self::MySql(adapter) | Self::MariaDb(adapter) => adapter.list_principals().await,
            Self::SqlServer(adapter) => adapter.list_principals().await,
            Self::Oracle(adapter) => adapter.list_principals().await,
            Self::Sqlite(adapter) => adapter.list_principals().await,
            Self::TiDb(_) => Err(DatabaseError::unsupported(
                "TiDB principal management is not implemented",
            )),
            Self::DuckDb(_) => Err(DatabaseError::unsupported("DuckDB has no SQL principals")),
            Self::Redis(_) => Err(DatabaseError::unsupported("Redis has no SQL principals")),
            Self::MongoDb(_) => Err(DatabaseError::unsupported(
                "MongoDB uses document principals",
            )),
            Self::ClickHouse(adapter) => adapter.list_principals().await,
            Self::Elasticsearch(_) | Self::OpenSearch(_) => {
                Err(DatabaseError::unsupported(format!(
                    "{} principal management is not supported",
                    descriptor::descriptor(self.kind()).display_name
                )))
            }
        }
    }

    pub async fn principal_ddl(
        &self,
        principal: &PrincipalEntry,
    ) -> Result<PrincipalDdl, DatabaseError> {
        match self {
            Self::Snowflake(_) => Err(DatabaseError::unsupported(
                "Snowflake principal management is not implemented",
            )),
            Self::Postgres(adapter) => adapter.principal_ddl(principal).await,
            Self::MySql(adapter) | Self::MariaDb(adapter) => adapter.principal_ddl(principal).await,
            Self::SqlServer(adapter) => adapter.principal_ddl(principal).await,
            Self::Oracle(adapter) => adapter.principal_ddl(principal).await,
            Self::TiDb(_) => Err(DatabaseError::unsupported(
                "TiDB principal management is not implemented",
            )),
            Self::CockroachDb(_) | Self::Sqlite(_) | Self::DuckDb(_) => Err(
                DatabaseError::unsupported("This database has no SQL principals"),
            ),
            Self::Redis(_) => Err(DatabaseError::unsupported("Redis has no SQL principals")),
            Self::MongoDb(_) => Err(DatabaseError::unsupported(
                "MongoDB uses document principals",
            )),
            Self::ClickHouse(adapter) => adapter.principal_ddl(principal).await,
            Self::Elasticsearch(_) | Self::OpenSearch(_) => {
                Err(DatabaseError::unsupported(format!(
                    "{} principal DDL is not supported",
                    descriptor::descriptor(self.kind()).display_name
                )))
            }
        }
    }

    pub async fn principal_details(
        &self,
        principal: &PrincipalEntry,
        target: &principal::PrincipalReadTarget,
    ) -> Result<principal::PrincipalDetails, DatabaseError> {
        match self {
            Self::Snowflake(_) => Err(DatabaseError::unsupported(
                "Snowflake principal management is not implemented",
            )),
            Self::Postgres(adapter) => adapter.principal_details(principal, target).await,
            Self::MySql(adapter) | Self::MariaDb(adapter) => {
                adapter.principal_details(principal, target).await
            }
            Self::SqlServer(adapter) => adapter.principal_details(principal, target).await,
            Self::Oracle(adapter) => adapter.principal_details(principal, target).await,
            Self::CockroachDb(_)
            | Self::TiDb(_)
            | Self::Sqlite(_)
            | Self::DuckDb(_)
            | Self::Redis(_)
            | Self::MongoDb(_)
            | Self::Elasticsearch(_)
            | Self::OpenSearch(_) => Err(DatabaseError::unsupported(
                "This database does not support SQL principal permission details",
            )),
            Self::ClickHouse(adapter) => adapter.principal_details(principal, target).await,
        }
    }

    pub fn plan_principal_mutation(
        &self,
        principal: &PrincipalEntry,
        connection: ConnectionIdentity,
        database: Option<&str>,
        mutation: principal::PrincipalMutation,
    ) -> Result<principal::PrincipalMutationPlan, DatabaseError> {
        match self {
            Self::Postgres(adapter) => {
                adapter.plan_principal_mutation(principal, connection, database, mutation)
            }
            Self::MySql(adapter) | Self::MariaDb(adapter) => {
                adapter.plan_principal_mutation(principal, connection, database, mutation)
            }
            Self::SqlServer(adapter) => {
                adapter.plan_principal_mutation(principal, connection, database, mutation)
            }
            Self::Oracle(_) => {
                oracle::plan_principal_mutation(principal, connection, database, mutation)
            }
            Self::ClickHouse(adapter) => {
                adapter.plan_principal_mutation(principal, connection, database, mutation)
            }
            _ => Err(DatabaseError::unsupported(
                "principal mutations are DDL-only for this adapter (structured mutations are not implemented)",
            )),
        }
    }
    pub async fn load_monitor_snapshot(&self) -> Result<monitor::MonitorSnapshot, DatabaseError> {
        match self {
            Self::Snowflake(_) => Err(DatabaseError::unsupported(
                "Snowflake monitoring is not implemented",
            )),
            Self::Postgres(adapter) => adapter.load_monitor_snapshot().await,
            Self::CockroachDb(_) => Err(DatabaseError::unsupported(
                "CockroachDB monitoring is not implemented",
            )),
            Self::MySql(adapter) => adapter.load_monitor_snapshot().await,
            Self::MariaDb(adapter) => adapter.load_monitor_snapshot().await,
            Self::TiDb(_) => Err(DatabaseError::unsupported(
                "TiDB monitoring is not implemented",
            )),
            Self::Oracle(_) => Err(DatabaseError {
                category: ErrorCategory::Unsupported,
                code: Some("oracle_monitoring_unsupported".into()),
                message: "Oracle monitoring is not implemented yet".into(),
                diagnostic: None,
            }),
            Self::SqlServer(adapter) => adapter.load_monitor_snapshot().await,
            Self::Sqlite(_) => Err(DatabaseError {
                category: ErrorCategory::Unsupported,
                code: Some("monitoring_unsupported".into()),
                message: "SQLite does not expose server monitoring metrics".into(),
                diagnostic: None,
            }),
            Self::DuckDb(_) => Err(DatabaseError::unsupported(
                "DuckDB does not expose server monitoring metrics",
            )),
            Self::Redis(adapter) => adapter.load_monitor_snapshot().await,
            Self::MongoDb(_) => Err(DatabaseError::unsupported(
                "MongoDB monitor integration is not implemented yet",
            )),
            Self::ClickHouse(adapter) => adapter.load_monitor_snapshot().await,
            Self::Elasticsearch(_) | Self::OpenSearch(_) => {
                Err(DatabaseError::unsupported(format!(
                    "{} monitoring is not supported",
                    descriptor::descriptor(self.kind()).display_name
                )))
            }
        }
    }

    pub async fn load_monitor_metadata(&self) -> Result<monitor::MonitorMetadata, DatabaseError> {
        match self {
            Self::Snowflake(_) => Ok(monitor::MonitorMetadata::default()),
            Self::Postgres(adapter) => adapter.load_monitor_metadata().await,
            Self::CockroachDb(_) => Ok(monitor::MonitorMetadata::default()),
            Self::MySql(adapter) => adapter.load_monitor_metadata().await,
            Self::MariaDb(adapter) => adapter.load_monitor_metadata().await,
            Self::TiDb(_) => Err(DatabaseError::unsupported(
                "TiDB monitoring is not implemented",
            )),
            Self::Oracle(_) => Ok(monitor::MonitorMetadata::default()),
            Self::SqlServer(adapter) => adapter.load_monitor_metadata().await,
            Self::Sqlite(_) | Self::DuckDb(_) => Ok(monitor::MonitorMetadata::default()),
            Self::Redis(_) => Ok(monitor::MonitorMetadata::default()),
            Self::MongoDb(_) => Ok(monitor::MonitorMetadata::default()),
            Self::ClickHouse(_) | Self::Elasticsearch(_) | Self::OpenSearch(_) => {
                Ok(monitor::MonitorMetadata::default())
            }
        }
    }

    pub async fn load_process_snapshot(&self) -> Result<monitor::ProcessSnapshot, DatabaseError> {
        match self {
            Self::Snowflake(_) => Err(DatabaseError::unsupported(
                "Snowflake process monitoring is not implemented",
            )),
            Self::Postgres(adapter) => adapter.load_process_snapshot().await,
            Self::CockroachDb(_) => Err(DatabaseError::unsupported(
                "CockroachDB process monitoring is not implemented",
            )),
            Self::MySql(adapter) => adapter.load_process_snapshot().await,
            Self::MariaDb(adapter) => adapter.load_process_snapshot().await,
            Self::TiDb(_) => Err(DatabaseError::unsupported(
                "TiDB process metrics are not implemented",
            )),
            Self::Oracle(_) => Err(DatabaseError {
                category: ErrorCategory::Unsupported,
                code: Some("oracle_process_list_unsupported".into()),
                message: "Oracle process metrics are not implemented yet".into(),
                diagnostic: None,
            }),
            Self::SqlServer(adapter) => adapter.load_process_snapshot().await,
            Self::Sqlite(_) => Err(DatabaseError {
                category: ErrorCategory::Unsupported,
                code: Some("process_list_unsupported".into()),
                message: "SQLite does not expose a server process list".into(),
                diagnostic: None,
            }),
            Self::DuckDb(_) => Err(DatabaseError::unsupported(
                "DuckDB does not expose a server process list",
            )),
            Self::Redis(_) => Err(DatabaseError {
                category: ErrorCategory::Unsupported,
                code: Some("redis_process_list_unsupported".into()),
                message: "Redis process list is not implemented yet".into(),
                diagnostic: None,
            }),
            Self::MongoDb(_) => Err(DatabaseError::unsupported(
                "MongoDB process list is not implemented yet",
            )),
            Self::ClickHouse(adapter) => adapter.load_process_snapshot().await,
            Self::Elasticsearch(_) | Self::OpenSearch(_) => {
                Err(DatabaseError::unsupported(format!(
                    "{} process monitoring is not supported",
                    descriptor::descriptor(self.kind()).display_name
                )))
            }
        }
    }

    pub(crate) async fn start_transaction_worker_with_forced_close(
        &self,
        forced_close: crate::runtime::transaction::ForcedCloseHandle,
    ) -> Result<crate::runtime::transaction::TransactionWorkerHandle, DatabaseError> {
        match self {
            Self::CockroachDb(adapter) => Ok(
                crate::runtime::transaction::spawn_transaction_worker_with_forced_close(
                    adapter.transaction_backend().await?,
                    forced_close,
                ),
            ),
            Self::Snowflake(_) => Err(DatabaseError::unsupported(
                "Snowflake SQL API does not retain transactions across requests",
            )),
            Self::Postgres(adapter) => Ok(
                crate::runtime::transaction::spawn_transaction_worker_with_forced_close(
                    adapter.transaction_backend().await?,
                    forced_close,
                ),
            ),
            Self::MySql(adapter) => Ok(
                crate::runtime::transaction::spawn_transaction_worker_with_forced_close(
                    adapter.transaction_backend().await?,
                    forced_close,
                ),
            ),
            Self::MariaDb(adapter) => Ok(
                crate::runtime::transaction::spawn_transaction_worker_with_forced_close(
                    adapter.transaction_backend().await?,
                    forced_close,
                ),
            ),
            Self::Oracle(adapter) => Ok(
                crate::runtime::transaction::spawn_transaction_worker_with_forced_close(
                    adapter.transaction_backend().await?,
                    forced_close,
                ),
            ),
            Self::Sqlite(adapter) => Ok(
                crate::runtime::transaction::spawn_transaction_worker_with_forced_close(
                    adapter.transaction_backend().await?,
                    forced_close,
                ),
            ),
            Self::DuckDb(adapter) => Ok(
                crate::runtime::transaction::spawn_transaction_worker_with_forced_close(
                    adapter.transaction_backend().await?,
                    forced_close,
                ),
            ),
            Self::SqlServer(adapter) => Ok(
                crate::runtime::transaction::spawn_transaction_worker_with_forced_close(
                    adapter.transaction_backend().await?,
                    forced_close,
                ),
            ),
            Self::Redis(_) => Err(DatabaseError::configuration(
                "Redis does not support SQL transactions",
            )),
            Self::MongoDb(_) => Err(DatabaseError::configuration(
                "MongoDB uses session transactions",
            )),
            Self::TiDb(_) => Err(DatabaseError::unsupported(
                "TiDB manual transactions are not implemented",
            )),
            Self::ClickHouse(_) => Err(DatabaseError::unsupported(
                "ClickHouse does not support manual SQL transactions",
            )),
            Self::Elasticsearch(_) | Self::OpenSearch(_) => {
                Err(DatabaseError::unsupported(format!(
                    "{} does not support manual transactions",
                    descriptor::descriptor(self.kind()).display_name
                )))
            }
        }
    }

    pub async fn connect(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
    ) -> Result<Self, DatabaseError> {
        match profile.kind {
            DatabaseKind::Snowflake => snowflake::SnowflakeAdapter::connect(profile, password)
                .await
                .map(Self::Snowflake),
            DatabaseKind::Sqlite => SqliteAdapter::connect(profile).await.map(Self::Sqlite),
            DatabaseKind::DuckDb => duckdb::DuckDbAdapter::connect(profile)
                .await
                .map(Self::DuckDb),
            DatabaseKind::CockroachDb => CockroachAdapter::connect(profile, password)
                .await
                .map(Self::CockroachDb),
            DatabaseKind::Postgres => PostgresAdapter::connect(profile, password)
                .await
                .map(Self::Postgres),
            DatabaseKind::MySql => MySqlAdapter::connect(profile, password)
                .await
                .map(Self::MySql),
            DatabaseKind::MariaDb => MySqlAdapter::connect(profile, password)
                .await
                .map(Self::MariaDb),
            DatabaseKind::TiDb => MySqlAdapter::connect(profile, password)
                .await
                .map(Self::TiDb),
            DatabaseKind::Oracle => OracleAdapter::connect(profile, password)
                .await
                .map(Self::Oracle),
            DatabaseKind::SqlServer => MsSqlAdapter::connect(profile, password)
                .await
                .map(Self::SqlServer),
            DatabaseKind::Redis | DatabaseKind::Valkey => RedisAdapter::connect(profile, password)
                .await
                .map(Self::Redis),
            DatabaseKind::MongoDb => mongodb::MongoAdapter::connect(profile, password)
                .await
                .map(Self::MongoDb),
            DatabaseKind::ClickHouse => ClickHouseAdapter::connect(profile, password)
                .await
                .map(Self::ClickHouse),
            DatabaseKind::Elasticsearch => ElasticsearchAdapter::connect(profile, password)
                .await
                .map(Self::Elasticsearch),
            DatabaseKind::OpenSearch => ElasticsearchAdapter::connect(profile, password)
                .await
                .map(Self::OpenSearch),
        }
    }

    pub async fn connect_target(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
        target: &ExecutionTarget,
    ) -> Result<Self, DatabaseError> {
        let configured = target.apply_to_profile(profile).ok_or_else(|| {
            DatabaseError::configuration("execution target is invalid for this profile")
        })?;
        Self::connect(&configured, password).await
    }

    pub fn kind(&self) -> DatabaseKind {
        match self {
            Self::Snowflake(_) => DatabaseKind::Snowflake,
            Self::Postgres(_) => DatabaseKind::Postgres,
            Self::CockroachDb(_) => DatabaseKind::CockroachDb,
            Self::MySql(_) => DatabaseKind::MySql,
            Self::MariaDb(_) => DatabaseKind::MariaDb,
            Self::TiDb(_) => DatabaseKind::TiDb,
            Self::Oracle(_) => DatabaseKind::Oracle,
            Self::Sqlite(_) => DatabaseKind::Sqlite,
            Self::DuckDb(_) => DatabaseKind::DuckDb,
            Self::SqlServer(_) => DatabaseKind::SqlServer,
            Self::Redis(adapter) => adapter.kind(),
            Self::MongoDb(_) => DatabaseKind::MongoDb,
            Self::ClickHouse(_) => DatabaseKind::ClickHouse,
            Self::Elasticsearch(_) => DatabaseKind::Elasticsearch,
            Self::OpenSearch(_) => DatabaseKind::OpenSearch,
        }
    }

    pub fn duckdb_session_clone(&self) -> Result<Option<Self>, DatabaseError> {
        match self {
            Self::DuckDb(adapter) => adapter.database_clone().map(Self::DuckDb).map(Some),
            _ => Ok(None),
        }
    }

    pub fn capabilities(&self) -> capabilities::DatabaseCapabilities {
        capabilities::DatabaseCapabilities::for_kind(self.kind())
    }

    pub fn catalog_capabilities(&self) -> CatalogCapabilities {
        match self {
            Self::Snowflake(_) => snowflake::SnowflakeAdapter::catalog_capabilities(),
            Self::Postgres(_) => PostgresAdapter::catalog_capabilities(),
            Self::CockroachDb(_) => CockroachAdapter::catalog_capabilities(),
            Self::MySql(_) => MySqlAdapter::catalog_capabilities(),
            Self::MariaDb(_) => MySqlAdapter::mariadb_catalog_capabilities(),
            Self::TiDb(_) => MySqlAdapter::tidb_catalog_capabilities(),
            Self::Oracle(_) => catalog::CatalogCapabilities {
                namespace_model: catalog::NamespaceModel::DatabaseAndSchema,
                top_level_groups: vec![
                    catalog::ObjectGroup::Tables,
                    catalog::ObjectGroup::Views,
                    catalog::ObjectGroup::Sequences,
                ],
                column_metadata: catalog::ColumnMetadataCapabilities {
                    type_family: true,
                    default_expression: true,
                    numeric_precision_and_scale: true,
                    character_length: true,
                    comment: true,
                    ..Default::default()
                },
                supports_lazy_children: true,
            },
            Self::Sqlite(_) => SqliteAdapter::catalog_capabilities(),
            Self::DuckDb(_) => duckdb::DuckDbAdapter::catalog_capabilities(),
            Self::SqlServer(_) => MsSqlAdapter::catalog_capabilities(),
            Self::Redis(_) => CatalogCapabilities {
                namespace_model: catalog::NamespaceModel::DatabaseAndSchema,
                top_level_groups: Vec::new(),
                column_metadata: catalog::ColumnMetadataCapabilities::default(),
                supports_lazy_children: false,
            },
            Self::MongoDb(_) => CatalogCapabilities {
                namespace_model: catalog::NamespaceModel::DatabaseIsSchema,
                top_level_groups: Vec::new(),
                column_metadata: catalog::ColumnMetadataCapabilities {
                    type_family: false,
                    default_expression: false,
                    numeric_precision_and_scale: false,
                    character_length: false,
                    comment: false,
                    identity: false,
                    auto_increment: false,
                    generated_expression: false,
                    hidden: false,
                    collation: false,
                    character_set: false,
                },
                supports_lazy_children: false,
            },
            Self::ClickHouse(_) => crate::db::clickhouse::ClickHouseAdapter::catalog_capabilities(),
            Self::Elasticsearch(_) | Self::OpenSearch(_) => {
                ElasticsearchAdapter::catalog_capabilities()
            }
        }
    }

    pub fn catalog_mutation_capabilities(&self) -> CatalogMutationCapabilities {
        match self {
            Self::Snowflake(_) => CatalogMutationCapabilities::default(),
            Self::Postgres(adapter) => adapter.mutation_capabilities(),
            Self::CockroachDb(_) => CatalogMutationCapabilities::default(),
            Self::MySql(_) => MySqlAdapter::catalog_mutation_capabilities(),
            Self::MariaDb(_) => MySqlAdapter::catalog_mutation_capabilities(),
            Self::TiDb(_) => CatalogMutationCapabilities::default(),
            Self::Oracle(_) => OracleAdapter::catalog_mutation_capabilities(),
            Self::Sqlite(_) => SqliteAdapter::catalog_mutation_capabilities(),
            Self::DuckDb(_) => duckdb::DuckDbAdapter::catalog_mutation_capabilities(),
            Self::SqlServer(_) => MsSqlAdapter::catalog_mutation_capabilities(),
            Self::Redis(_) => CatalogMutationCapabilities::default(),
            Self::MongoDb(_) => CatalogMutationCapabilities::default(),
            Self::ClickHouse(_) => ClickHouseAdapter::catalog_mutation_capabilities(),
            Self::Elasticsearch(_) | Self::OpenSearch(_) => CatalogMutationCapabilities::default(),
        }
    }

    pub fn catalog_drop_availability(
        &self,
        kind: CatalogKind,
    ) -> catalog_mutation::CatalogMutationAvailability {
        match self {
            Self::CockroachDb(_) => catalog_mutation::CatalogMutationAvailability::Unavailable {
                reason: "CockroachDB structured schema editing is not implemented",
            },
            Self::TiDb(_) => catalog_mutation::CatalogMutationAvailability::Unavailable {
                reason: "TiDB structured catalog mutations are not implemented",
            },
            Self::Snowflake(_) => catalog_mutation::CatalogMutationAvailability::Unavailable {
                reason: "Snowflake structured catalog changes are not implemented; use SQL",
            },
            Self::Oracle(_) => OracleAdapter::catalog_drop_availability(kind),
            Self::Elasticsearch(_) | Self::OpenSearch(_) => {
                catalog_mutation::CatalogMutationAvailability::Unavailable {
                    reason: "Search REST catalog DROP is not supported",
                }
            }
            Self::DuckDb(_)
                if matches!(
                    kind,
                    CatalogKind::Schema
                        | CatalogKind::Type
                        | CatalogKind::Table
                        | CatalogKind::View
                        | CatalogKind::Sequence
                        | CatalogKind::Index
                ) =>
            {
                catalog_mutation::CatalogMutationAvailability::Available
            }
            Self::DuckDb(_) => catalog_mutation::CatalogMutationAvailability::Unavailable {
                reason: "DuckDB DROP is not enabled for this catalog object kind",
            },
            _ => catalog_mutation::CatalogMutationAvailability::Available,
        }
    }

    pub fn plan_catalog_drop(
        &self,
        request: catalog_drop::CatalogDropRequest,
        entry: &catalog::CatalogEntry,
    ) -> Result<catalog_drop::CatalogDropPlan, catalog_drop::CatalogDropError> {
        match self {
            Self::Snowflake(_) => Err(catalog_drop::CatalogDropError::Unsupported {
                kind: entry.kind,
                reason: "Snowflake structured catalog changes are not implemented; use SQL".into(),
            }),
            Self::Postgres(_) => PostgresAdapter::plan_catalog_drop(request, entry),
            Self::CockroachDb(_) => Err(catalog_drop::CatalogDropError::Unsupported {
                kind: entry.kind,
                reason: "CockroachDB structured schema editing is not implemented".into(),
            }),
            Self::MySql(_) => MySqlAdapter::plan_catalog_drop(request, entry),
            Self::MariaDb(_) => MySqlAdapter::plan_catalog_drop(request, entry),
            Self::TiDb(_) => Err(catalog_drop::CatalogDropError::Unsupported {
                kind: entry.kind,
                reason: "TiDB structured catalog mutations are not implemented".to_owned(),
            }),
            Self::Oracle(_) => OracleAdapter::plan_catalog_drop(request, entry),
            Self::Sqlite(_) => SqliteAdapter::plan_catalog_drop(request, entry),
            Self::DuckDb(_) => duckdb::DuckDbAdapter::plan_catalog_drop(request, entry),
            Self::SqlServer(_) => MsSqlAdapter::plan_catalog_drop(request, entry),
            Self::Redis(_) => Err(catalog_drop::CatalogDropError::Unsupported {
                kind: entry.kind,
                reason: "Redis does not have SQL catalog objects".to_owned(),
            }),
            Self::MongoDb(_) => Err(catalog_drop::CatalogDropError::Unsupported {
                kind: entry.kind,
                reason: "MongoDB does not expose SQL catalog objects".to_owned(),
            }),
            Self::ClickHouse(_) => ClickHouseAdapter::plan_catalog_drop(request, entry),
            Self::Elasticsearch(_) | Self::OpenSearch(_) => {
                Err(catalog_drop::CatalogDropError::Unsupported {
                    kind: entry.kind,
                    reason: format!(
                        "{} catalog DROP is not supported",
                        descriptor::descriptor(self.kind()).display_name
                    )
                    .to_owned(),
                })
            }
        }
    }

    pub fn plan_principal_drop(
        &self,
        request: principal_drop::PrincipalDropRequest,
    ) -> Result<principal_drop::PrincipalDropPlan, principal_drop::PrincipalDropError> {
        match self {
            Self::Postgres(_) => PostgresAdapter::plan_principal_drop(request),
            _ => Err(principal_drop::PrincipalDropError::Unsupported),
        }
    }

    pub fn plan_catalog_mutation(
        &self,
        request: catalog_mutation::CatalogMutationRequest,
        draft: crate::model::catalog_editor::CatalogDraft,
        baseline: Option<catalog_mutation::CatalogObjectDefinition>,
    ) -> Result<catalog_mutation::CatalogMutationPlan, catalog_mutation::CatalogMutationError> {
        match self {
            Self::Snowflake(_) => Err(
                catalog_mutation::CatalogMutationError::UnsupportedOperation {
                    object_type: request.object_type,
                },
            ),
            Self::Postgres(adapter) => {
                adapter.plan_catalog_mutation_for_adapter(request, draft, baseline)
            }
            Self::TiDb(_) => Err(
                catalog_mutation::CatalogMutationError::UnsupportedOperation {
                    object_type: request.object_type,
                },
            ),
            Self::MySql(_) | Self::MariaDb(_) => {
                MySqlAdapter::plan_catalog_mutation(request, draft, baseline)
            }
            Self::Sqlite(_) => SqliteAdapter::plan_catalog_mutation(request, draft, baseline),
            Self::DuckDb(_) => {
                duckdb::DuckDbAdapter::plan_catalog_mutation(request, draft, baseline)
            }
            Self::SqlServer(_) => MsSqlAdapter::plan_catalog_mutation(request, draft, baseline),
            Self::Oracle(_) => OracleAdapter::plan_catalog_mutation(request, draft, baseline),
            Self::CockroachDb(_) | Self::Redis(_) => Err(
                catalog_mutation::CatalogMutationError::UnsupportedOperation {
                    object_type: request.object_type,
                },
            ),
            Self::MongoDb(_) | Self::Elasticsearch(_) | Self::OpenSearch(_) => Err(
                catalog_mutation::CatalogMutationError::UnsupportedOperation {
                    object_type: request.object_type,
                },
            ),
            Self::ClickHouse(adapter) => ClickHouseAdapter::plan_catalog_mutation(
                request,
                draft,
                baseline,
                adapter.server_database(),
            ),
        }
    }

    pub async fn execute_catalog_mutation(
        &self,
        plan: &catalog_mutation::CatalogMutationPlan,
    ) -> Result<QueryOutcome, DatabaseError> {
        match self {
            Self::Snowflake(_) => Err(DatabaseError::unsupported(
                "Snowflake structured catalog changes are not implemented; use SQL",
            )),
            Self::Postgres(adapter) => adapter.execute_catalog_mutation(plan).await,
            Self::CockroachDb(_) => Err(DatabaseError::unsupported(
                "CockroachDB structured schema editing is not implemented",
            )),
            Self::TiDb(_) => Err(DatabaseError::unsupported(
                "TiDB structured catalog mutations are not implemented",
            )),
            Self::MySql(adapter) | Self::MariaDb(adapter) => {
                adapter.execute_catalog_mutation(plan).await
            }
            Self::Sqlite(adapter) => adapter.execute_catalog_mutation(plan).await,
            Self::DuckDb(adapter) => adapter.execute_catalog_mutation(plan).await,
            Self::SqlServer(adapter) => adapter.execute_catalog_mutation(plan).await,
            Self::Oracle(adapter) => adapter.execute_catalog_mutation(plan).await,
            Self::Redis(_) => Err(DatabaseError::configuration(
                "catalog mutation is not supported for Redis-compatible databases",
            )),
            Self::MongoDb(_) => Err(DatabaseError::unsupported(
                "MongoDB does not support SQL catalog mutation",
            )),
            Self::ClickHouse(adapter) => adapter.execute_catalog_mutation(plan).await,
            Self::Elasticsearch(_) | Self::OpenSearch(_) => {
                Err(DatabaseError::unsupported(format!(
                    "{} catalog mutation is not supported",
                    descriptor::descriptor(self.kind()).display_name
                )))
            }
        }
    }

    pub async fn execute_catalog_drop(
        &self,
        plan: &catalog_drop::CatalogDropPlan,
    ) -> Result<QueryOutcome, DatabaseError> {
        plan.validate()
            .map_err(|error| DatabaseError::configuration(error.to_string()))?;
        match self {
            Self::TiDb(_) => Err(DatabaseError::unsupported(
                "TiDB structured catalog mutations are not implemented",
            )),
            Self::DuckDb(adapter) => adapter.execute(plan.sql()).await,
            Self::Elasticsearch(_) | Self::OpenSearch(_) => {
                Err(DatabaseError::unsupported(format!(
                    "{} catalog DROP is not supported",
                    descriptor::descriptor(self.kind()).display_name
                )))
            }
            _ => self.execute(plan.sql()).await,
        }
    }

    pub fn plan_redis_mutation(
        &self,
        request: crate::db::redis::mutation::RedisMutationRequest,
        draft: crate::db::redis::mutation::RedisValueDraft,
        ttl_millis: Option<u64>,
        baseline: Option<crate::db::redis::mutation::RedisKeyBaseline>,
    ) -> Result<
        crate::db::redis::mutation::RedisMutationPlan,
        crate::db::redis::mutation::RedisMutationError,
    > {
        match self {
            Self::Redis(_) => {
                crate::db::redis::RedisAdapter::plan_mutation(request, draft, ttl_millis, baseline)
            }
            _ => Err(
                crate::db::redis::mutation::RedisMutationError::InvalidRequest {
                    reason: "Redis mutation requires a Redis-compatible connection".into(),
                },
            ),
        }
    }

    pub fn plan_redis_operation(
        &self,
        request: crate::db::redis::mutation::RedisMutationRequest,
        operation: crate::db::redis::mutation::RedisMutationOperation,
        ttl: crate::db::redis::mutation::RedisTtlMutation,
        baseline: Option<crate::db::redis::mutation::RedisKeyBaseline>,
    ) -> Result<
        crate::db::redis::mutation::RedisMutationPlan,
        crate::db::redis::mutation::RedisMutationError,
    > {
        match self {
            Self::Redis(_) => {
                crate::db::redis::RedisAdapter::plan_operation(request, operation, ttl, baseline)
            }
            _ => Err(
                crate::db::redis::mutation::RedisMutationError::InvalidRequest {
                    reason: "Redis mutation requires a Redis-compatible connection".into(),
                },
            ),
        }
    }

    pub async fn execute_redis_mutation(
        &self,
        plan: &crate::db::redis::mutation::RedisMutationPlan,
    ) -> Result<crate::db::redis::mutation::RedisMutationResult, DatabaseError> {
        match self {
            Self::Redis(adapter) => adapter.execute_mutation(plan).await,
            Self::MongoDb(_) => Err(DatabaseError::unsupported(
                "MongoDB document mutation is not implemented yet",
            )),
            _ => Err(DatabaseError::configuration(
                "Redis mutation requires a Redis-compatible connection",
            )),
        }
    }

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        match self {
            Self::Snowflake(adapter) => adapter.probe().await,
            Self::Postgres(adapter) => adapter.probe().await,
            Self::CockroachDb(adapter) => adapter.probe().await,
            Self::MySql(adapter) => adapter.probe().await,
            Self::MariaDb(adapter) => adapter.probe().await,
            Self::TiDb(adapter) => adapter.probe().await,
            Self::Oracle(adapter) => adapter.probe().await,
            Self::Sqlite(adapter) => adapter.probe().await,
            Self::DuckDb(adapter) => adapter.probe().await,
            Self::SqlServer(adapter) => adapter.probe().await,
            Self::Redis(adapter) => adapter.probe().await,
            Self::MongoDb(adapter) => adapter.probe().await,
            Self::ClickHouse(adapter) => adapter.probe().await,
            Self::Elasticsearch(adapter) | Self::OpenSearch(adapter) => adapter.probe().await,
        }
    }

    pub async fn discover_redis_databases(
        &self,
        current_database: u32,
        explicit_databases: &[u32],
    ) -> Result<redis::discovery::RedisDatabaseDiscovery, DatabaseError> {
        match self {
            Self::Redis(adapter) => {
                adapter
                    .discover_databases(current_database, explicit_databases)
                    .await
            }
            _ => Err(DatabaseError::configuration(
                "Redis-compatible database discovery requires a Redis-compatible connection",
            )),
        }
    }

    pub async fn discover_catalog_scope(&self) -> Result<CatalogDiscovery, DatabaseError> {
        match self {
            Self::Snowflake(adapter) => adapter.discover_catalog_scope().await,
            Self::Postgres(adapter) => adapter.discover_catalog_scope().await,
            Self::CockroachDb(adapter) => adapter.discover_catalog_scope().await,
            Self::MySql(adapter) => adapter.discover_catalog_scope().await,
            Self::MariaDb(adapter) => adapter.discover_catalog_scope().await,
            Self::TiDb(adapter) => adapter.discover_catalog_scope().await,
            Self::Oracle(adapter) => adapter.discover_catalog_scope().await,
            Self::Sqlite(adapter) => adapter.discover_catalog_scope().await,
            Self::DuckDb(adapter) => adapter.discover_catalog_scope().await,
            Self::SqlServer(adapter) => adapter.discover_catalog_scope().await,
            Self::Redis(_) => Err(DatabaseError::configuration(
                "Redis-compatible databases use keyspace discovery instead of SQL catalog discovery",
            )),
            Self::MongoDb(_) => Err(DatabaseError::unsupported(
                "MongoDB uses document namespace discovery",
            )),
            Self::ClickHouse(_) => Err(DatabaseError::unsupported(
                "ClickHouse catalog scope is profile-defined",
            )),
            Self::Elasticsearch(_) | Self::OpenSearch(_) => {
                Err(DatabaseError::unsupported(format!(
                    "{} catalog scope is profile-defined",
                    descriptor::descriptor(self.kind()).display_name
                )))
            }
        }
    }

    pub async fn discoverable_postgres_databases(
        &self,
    ) -> Result<Option<Vec<String>>, DatabaseError> {
        match self {
            Self::Snowflake(_) => Ok(None),
            Self::Postgres(adapter) => adapter.discoverable_databases().await.map(Some),
            _ => Ok(None),
        }
    }

    pub async fn discoverable_pgwire_databases(
        &self,
    ) -> Result<Option<Vec<String>>, DatabaseError> {
        match self {
            Self::Snowflake(_) => Ok(None),
            Self::Postgres(adapter) => adapter.discoverable_databases().await.map(Some),
            Self::CockroachDb(adapter) => adapter.discoverable_databases().await.map(Some),
            Self::MySql(_)
            | Self::MariaDb(_)
            | Self::TiDb(_)
            | Self::Oracle(_)
            | Self::Sqlite(_)
            | Self::DuckDb(_)
            | Self::SqlServer(_) => Ok(None),
            Self::Redis(_) => Ok(None),
            Self::MongoDb(_) => Ok(None),
            Self::ClickHouse(_) | Self::Elasticsearch(_) | Self::OpenSearch(_) => Ok(None),
        }
    }

    pub async fn load_catalog_page(
        &self,
        request: &CatalogRequest,
    ) -> Result<CatalogPage, DatabaseError> {
        match self {
            Self::Snowflake(adapter) => adapter.load_catalog_page(request).await,
            Self::Postgres(adapter) => adapter.load_catalog_page(request).await,
            Self::CockroachDb(adapter) => adapter.load_catalog_page(request).await,
            Self::MySql(adapter) => adapter.load_catalog_page(request).await,
            Self::MariaDb(adapter) => adapter.load_catalog_page(request).await,
            Self::TiDb(adapter) => adapter.load_catalog_page(request).await,
            Self::Oracle(adapter) => adapter.load_catalog_page(request).await,
            Self::Sqlite(adapter) => adapter.load_catalog_page(request).await,
            Self::DuckDb(adapter) => {
                let discovery = adapter.discover_catalog_scope().await?;
                let scope = request.scope.clone().with_discovered_databases(
                    adapter.current_database(),
                    discovery
                        .databases
                        .into_iter()
                        .map(|database| database.name),
                );
                adapter
                    .load_catalog_page(&request.clone().with_scope(scope))
                    .await
            }
            Self::SqlServer(adapter) => adapter.load_catalog_page(request).await,
            Self::Redis(_) => Err(DatabaseError::configuration(
                "Redis does not support SQL catalog pages",
            )),
            Self::MongoDb(_) => Err(DatabaseError::unsupported(
                "MongoDB does not expose SQL catalog pages",
            )),
            Self::ClickHouse(adapter) => adapter.load_catalog_page(request).await,
            Self::Elasticsearch(adapter) | Self::OpenSearch(adapter) => {
                adapter.load_catalog_page(request).await
            }
        }
    }

    pub async fn resolve_relation_identity(
        &self,
        relation: &CatalogId,
    ) -> Result<Option<catalog::CatalogEntry>, DatabaseError> {
        match self {
            Self::Snowflake(adapter) => adapter.resolve_relation_identity(relation).await,
            Self::Postgres(adapter) => adapter.resolve_relation_identity(relation).await,
            Self::CockroachDb(adapter) => adapter.resolve_relation_identity(relation).await,
            Self::MySql(_)
            | Self::MariaDb(_)
            | Self::TiDb(_)
            | Self::Oracle(_)
            | Self::Sqlite(_)
            | Self::DuckDb(_)
            | Self::SqlServer(_) => Ok(None),
            Self::Redis(_) => Ok(None),
            Self::MongoDb(_) => Ok(None),
            Self::ClickHouse(adapter) => adapter.resolve_relation_identity(relation).await,
            Self::Elasticsearch(adapter) | Self::OpenSearch(adapter) => {
                Ok(adapter.resolve_relation_identity(relation))
            }
        }
    }

    pub async fn resolve_relation_identity_with_scope(
        &self,
        relation: &CatalogId,
        scope: &crate::profile::CatalogScope,
    ) -> Result<Option<catalog::CatalogEntry>, DatabaseError> {
        match self {
            Self::Snowflake(adapter) => {
                adapter
                    .resolve_relation_identity_with_scope(relation, scope)
                    .await
            }
            Self::Postgres(adapter) => {
                adapter
                    .resolve_relation_identity_with_scope(relation, scope)
                    .await
            }
            Self::CockroachDb(adapter) => {
                adapter
                    .resolve_relation_identity_with_scope(relation, scope)
                    .await
            }
            Self::MySql(_)
            | Self::MariaDb(_)
            | Self::TiDb(_)
            | Self::Oracle(_)
            | Self::Sqlite(_)
            | Self::DuckDb(_)
            | Self::SqlServer(_) => Ok(None),
            Self::Redis(_) => Ok(None),
            Self::MongoDb(_) => Ok(None),
            Self::ClickHouse(adapter) => {
                let mut adapter = adapter.clone();
                adapter.catalog_scope = scope.clone();
                adapter.resolve_relation_identity(relation).await
            }
            Self::Elasticsearch(adapter) | Self::OpenSearch(adapter) => {
                elasticsearch::catalog::validate_relation_scope(
                    adapter.profile_id,
                    &adapter.database,
                    relation,
                    scope,
                )?;
                Ok(adapter.resolve_relation_identity(relation))
            }
        }
    }

    pub async fn load_catalog_object_definition(
        &self,
        request: &CatalogObjectDefinitionRequest,
    ) -> Result<CatalogObjectDefinition, DatabaseError> {
        match self {
            Self::Snowflake(_) => Err(DatabaseError::unsupported(
                "Snowflake structured object definitions are not implemented; use DDL",
            )),
            Self::Postgres(adapter) => adapter.load_catalog_object_definition(request).await,
            Self::CockroachDb(_) => Err(DatabaseError::unsupported(
                "CockroachDB structured schema editing is not implemented",
            )),
            Self::MySql(adapter) | Self::MariaDb(adapter) | Self::TiDb(adapter) => {
                adapter.load_catalog_object_definition(request).await
            }
            Self::Sqlite(adapter) => adapter.load_catalog_object_definition(request).await,
            Self::DuckDb(adapter) => adapter.load_catalog_object_definition(request).await,
            Self::SqlServer(adapter) => adapter.load_catalog_object_definition(request).await,
            Self::Oracle(adapter) => adapter.load_catalog_object_definition(request).await,
            Self::Redis(_) => Err(DatabaseError::configuration(
                "Redis does not support SQL catalog object definitions",
            )),
            Self::MongoDb(_) => Err(DatabaseError::unsupported(
                "MongoDB has no SQL catalog object definitions",
            )),
            Self::ClickHouse(adapter) => adapter.load_catalog_object_definition(request).await,
            Self::Elasticsearch(_) | Self::OpenSearch(_) => {
                Err(DatabaseError::unsupported(format!(
                    "{} catalog object definitions are not supported",
                    descriptor::descriptor(self.kind()).display_name
                )))
            }
        }
    }

    pub async fn load_catalog_owner_context(
        &self,
        request: &catalog_mutation::CatalogOwnerContextRequest,
    ) -> Result<Option<catalog_mutation::CatalogOwnerContext>, DatabaseError> {
        match self {
            Self::Snowflake(_) => Ok(None),
            Self::Postgres(adapter) => adapter.load_catalog_owner_context(request).await.map(Some),
            Self::CockroachDb(_)
            | Self::MySql(_)
            | Self::MariaDb(_)
            | Self::TiDb(_)
            | Self::Oracle(_)
            | Self::Sqlite(_)
            | Self::DuckDb(_)
            | Self::SqlServer(_) => Ok(None),
            Self::Redis(_) => Ok(None),
            Self::MongoDb(_) => Ok(None),
            Self::ClickHouse(_) | Self::Elasticsearch(_) | Self::OpenSearch(_) => Ok(None),
        }
    }

    pub async fn search_catalog(
        &self,
        request: &CatalogSearchRequest,
    ) -> Result<CatalogSearchPage, DatabaseError> {
        match self {
            Self::Snowflake(adapter) => adapter.search_catalog(request).await,
            Self::Postgres(adapter) => adapter.search_catalog(request).await,
            Self::CockroachDb(adapter) => adapter.search_catalog(request).await,
            Self::MySql(adapter) => adapter.search_catalog(request).await,
            Self::MariaDb(adapter) => adapter.search_catalog(request).await,
            Self::TiDb(adapter) => adapter.search_catalog(request).await,
            Self::Oracle(_) => Err(DatabaseError::configuration(
                "Oracle catalog search is not implemented yet",
            )),
            Self::Sqlite(adapter) => adapter.search_catalog(request).await,
            Self::DuckDb(adapter) => {
                let discovery = adapter.discover_catalog_scope().await?;
                let scope = request.scope.clone().with_discovered_databases(
                    adapter.current_database(),
                    discovery
                        .databases
                        .into_iter()
                        .map(|database| database.name),
                );
                let mut request = request.clone();
                request.scope = scope;
                adapter.search_catalog(&request).await
            }
            Self::SqlServer(adapter) => adapter.search_catalog(request).await,
            Self::Redis(_) => Err(DatabaseError::configuration(
                "Redis does not support SQL catalog search",
            )),
            Self::MongoDb(_) => Err(DatabaseError::unsupported(
                "MongoDB does not support SQL catalog search",
            )),
            Self::ClickHouse(adapter) => adapter.search_catalog(request).await,
            Self::Elasticsearch(adapter) | Self::OpenSearch(adapter) => {
                adapter.search_catalog(request).await
            }
        }
    }

    pub async fn execute(&self, sql: &str) -> Result<QueryOutcome, DatabaseError> {
        self.execute_with_budget(sql, QueryBudget::UNBOUNDED).await
    }

    pub(crate) async fn execute_with_budget(
        &self,
        sql: &str,
        budget: QueryBudget,
    ) -> Result<QueryOutcome, DatabaseError> {
        match self {
            Self::Snowflake(adapter) => adapter.execute_with_budget(sql, budget).await,
            Self::Postgres(adapter) => adapter.execute_pool_with_budget(sql, budget).await,
            Self::CockroachDb(adapter) => adapter.execute_pool_with_budget(sql, budget).await,
            Self::MySql(adapter) => adapter.execute_pool_with_budget(sql, budget).await,
            Self::MariaDb(adapter) => adapter.execute_pool_with_budget(sql, budget).await,
            Self::TiDb(adapter) => adapter.execute_pool_with_budget(sql, budget).await,
            Self::Oracle(adapter) => adapter.execute_pool_with_budget(sql, budget).await,
            Self::Sqlite(adapter) => adapter.execute_pool_with_budget(sql, budget).await,
            Self::DuckDb(adapter) => adapter.execute_with_budget(sql, budget).await,
            Self::SqlServer(adapter) => adapter.execute_pool_with_budget(sql, budget).await,
            Self::ClickHouse(adapter) => adapter.execute_with_budget(sql, budget).await,
            Self::Elasticsearch(adapter) | Self::OpenSearch(adapter) => {
                adapter.execute_with_budget(sql, budget).await
            }
            Self::Redis(_) => Err(DatabaseError::configuration(
                "Redis commands do not use SQL execution",
            )),
            Self::MongoDb(_) => Err(DatabaseError::unsupported(
                "MongoDB does not support SQL execution",
            )),
        }
    }

    pub(crate) async fn execute_with_budget_cancellable(
        &self,
        sql: &str,
        budget: QueryBudget,
        operation_id: Uuid,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
        on_start: impl FnOnce(duckdb::DuckDbOperationHandle) + Send + 'static,
    ) -> Result<QueryOutcome, DatabaseError> {
        match self {
            Self::DuckDb(adapter) => {
                adapter
                    .execute_with_budget_cancellable(sql, budget, operation_id, cancelled, on_start)
                    .await
            }
            _ => self.execute_with_budget(sql, budget).await,
        }
    }

    pub async fn preview_relation(
        &self,
        relation: &CatalogId,
        options: &crate::model::relation::RelationPreviewOptions,
        page: crate::model::pagination::PageRequest,
    ) -> Result<RelationPreview, DatabaseError> {
        match self {
            Self::Snowflake(adapter) => {
                adapter
                    .preview_relation_with_scope(relation, &adapter.catalog_scope, options, page)
                    .await
            }
            Self::Elasticsearch(adapter) | Self::OpenSearch(adapter) => {
                adapter.preview_relation(relation, options, page).await
            }
            Self::Postgres(adapter) => adapter.preview_relation(relation, options, page).await,
            Self::CockroachDb(adapter) => adapter.preview_relation(relation, options, page).await,
            Self::MySql(adapter) => adapter.preview_relation(relation, options, page).await,
            Self::MariaDb(adapter) => adapter.preview_relation(relation, options, page).await,
            Self::TiDb(adapter) => adapter.preview_relation(relation, options, page).await,
            Self::Oracle(adapter) => adapter.preview_relation(relation, options, page).await,
            Self::Sqlite(adapter) => adapter.preview_relation(relation, options, page).await,
            Self::DuckDb(adapter) => {
                adapter
                    .preview_relation(relation, adapter.profile_scope(), options, page)
                    .await
            }
            Self::SqlServer(adapter) => adapter.preview_relation(relation, options, page).await,
            Self::Redis(_) => Err(DatabaseError::configuration(
                "Redis does not support relation previews",
            )),
            Self::MongoDb(_) => Err(DatabaseError::unsupported(
                "MongoDB does not support SQL relation previews",
            )),
            Self::ClickHouse(adapter) => {
                adapter
                    .preview_relation_with_scope(
                        relation,
                        &crate::profile::CatalogScope::for_profile(
                            DatabaseKind::ClickHouse,
                            "default",
                            None,
                        ),
                        options,
                        page,
                    )
                    .await
            }
        }
    }

    pub async fn preview_relation_with_scope(
        &self,
        relation: &CatalogId,
        scope: &crate::profile::CatalogScope,
        options: &crate::model::relation::RelationPreviewOptions,
        page: crate::model::pagination::PageRequest,
    ) -> Result<RelationPreview, DatabaseError> {
        match self {
            Self::Elasticsearch(adapter) | Self::OpenSearch(adapter) => {
                adapter
                    .preview_relation_with_scope(relation, scope, options, page)
                    .await
            }
            Self::Snowflake(adapter) => {
                adapter
                    .preview_relation_with_scope(relation, scope, options, page)
                    .await
            }
            Self::Postgres(adapter) => {
                adapter
                    .preview_relation_with_scope(relation, scope, options, page)
                    .await
            }
            Self::CockroachDb(adapter) => {
                adapter
                    .preview_relation_with_scope(relation, scope, options, page)
                    .await
            }
            Self::MySql(adapter) => {
                adapter
                    .preview_relation_with_scope(relation, scope, options, page)
                    .await
            }
            Self::MariaDb(adapter) | Self::TiDb(adapter) => {
                adapter
                    .preview_relation_with_scope(relation, scope, options, page)
                    .await
            }
            Self::Oracle(adapter) => adapter.preview_relation(relation, options, page).await,
            Self::Sqlite(adapter) => {
                adapter
                    .preview_relation_with_scope(relation, scope, options, page)
                    .await
            }
            Self::DuckDb(adapter) => {
                let discovery = adapter.discover_catalog_scope().await?;
                let scope = scope.clone().with_attached_databases(
                    adapter.current_database(),
                    discovery
                        .databases
                        .into_iter()
                        .map(|database| database.name),
                );
                adapter
                    .preview_relation(relation, &scope, options, page)
                    .await
            }
            Self::SqlServer(adapter) => {
                adapter
                    .preview_relation_with_scope(relation, scope, options, page)
                    .await
            }
            Self::Redis(_) => Err(DatabaseError::configuration(
                "Redis does not support relation previews",
            )),
            Self::MongoDb(_) => Err(DatabaseError::unsupported(
                "MongoDB does not support SQL relation previews",
            )),
            Self::ClickHouse(adapter) => {
                adapter
                    .preview_relation_with_scope(relation, scope, options, page)
                    .await
            }
        }
    }

    pub async fn relation_ddl(&self, relation: &CatalogId) -> Result<RelationDdl, DatabaseError> {
        match self {
            Self::Snowflake(adapter) => {
                adapter
                    .relation_ddl_with_scope(relation, &adapter.catalog_scope)
                    .await
            }
            Self::Elasticsearch(_) | Self::OpenSearch(_) => {
                Err(DatabaseError::unsupported(format!(
                    "{} has mappings rather than SQL DDL",
                    descriptor::descriptor(self.kind()).display_name
                )))
            }
            Self::Postgres(adapter) => adapter.relation_ddl(relation).await,
            Self::CockroachDb(adapter) => adapter.relation_ddl(relation).await,
            Self::MySql(adapter) => adapter.relation_ddl(relation).await,
            Self::MariaDb(adapter) => adapter.relation_ddl(relation).await,
            Self::TiDb(adapter) => adapter.relation_ddl(relation).await,
            Self::Oracle(adapter) => adapter.relation_ddl(relation).await,
            Self::Sqlite(adapter) => adapter.relation_ddl(relation).await,
            Self::DuckDb(adapter) => {
                adapter
                    .relation_ddl_with_scope(relation, adapter.profile_scope())
                    .await
            }
            Self::SqlServer(adapter) => adapter.relation_ddl(relation).await,
            Self::Redis(_) => Err(DatabaseError::configuration(
                "Redis does not support relation DDL",
            )),
            Self::MongoDb(_) => Err(DatabaseError::unsupported(
                "MongoDB does not support SQL relation DDL",
            )),
            Self::ClickHouse(adapter) => {
                adapter
                    .relation_ddl_with_scope(
                        relation,
                        &crate::profile::CatalogScope::for_profile(
                            DatabaseKind::ClickHouse,
                            "default",
                            None,
                        ),
                    )
                    .await
            }
        }
    }

    pub async fn relation_ddl_with_scope(
        &self,
        relation: &CatalogId,
        scope: &crate::profile::CatalogScope,
    ) -> Result<RelationDdl, DatabaseError> {
        match self {
            Self::Snowflake(adapter) => adapter.relation_ddl_with_scope(relation, scope).await,
            Self::Postgres(adapter) => adapter.relation_ddl_with_scope(relation, scope).await,
            Self::CockroachDb(adapter) => adapter.relation_ddl_with_scope(relation, scope).await,
            Self::MySql(adapter) => adapter.relation_ddl_with_scope(relation, scope).await,
            Self::MariaDb(adapter) => adapter.relation_ddl_with_scope(relation, scope).await,
            Self::TiDb(adapter) => adapter.relation_ddl_with_scope(relation, scope).await,
            Self::Oracle(adapter) => adapter.relation_ddl_with_scope(relation, scope).await,
            Self::Sqlite(adapter) => adapter.relation_ddl_with_scope(relation, scope).await,
            Self::DuckDb(adapter) => adapter.relation_ddl_with_scope(relation, scope).await,
            Self::SqlServer(adapter) => adapter.relation_ddl_with_scope(relation, scope).await,
            Self::Redis(_) => Err(DatabaseError::configuration(
                "Redis does not support relation DDL",
            )),
            Self::MongoDb(_) => Err(DatabaseError::unsupported(
                "MongoDB does not support SQL relation DDL",
            )),
            Self::ClickHouse(adapter) => adapter.relation_ddl_with_scope(relation, scope).await,
            Self::Elasticsearch(_) | Self::OpenSearch(_) => {
                Err(DatabaseError::unsupported(format!(
                    "{} relation DDL is not supported",
                    descriptor::descriptor(self.kind()).display_name
                )))
            }
        }
    }

    pub async fn object_ddl(
        &self,
        kind: CatalogKind,
        schema: &str,
        name: &str,
    ) -> Result<Option<String>, DatabaseError> {
        match self {
            Self::Snowflake(_) => Ok(None),
            Self::Postgres(adapter) => adapter.object_ddl(kind, schema, name).await,
            Self::CockroachDb(adapter) => adapter.object_ddl(kind, schema, name).await,
            Self::MySql(adapter) => adapter.object_ddl(kind, schema, name).await,
            Self::MariaDb(adapter) => adapter.object_ddl(kind, schema, name).await,
            Self::TiDb(adapter) => adapter.object_ddl(kind, schema, name).await,
            Self::Oracle(_) => Ok(None),
            Self::Sqlite(adapter) => adapter.object_ddl(kind, schema, name).await,
            Self::DuckDb(adapter) => adapter.object_ddl(kind, schema, name).await,
            Self::SqlServer(adapter) => adapter.object_ddl(kind, schema, name).await,
            Self::Redis(_) => Ok(None),
            Self::MongoDb(_) => Ok(None),
            Self::ClickHouse(_) | Self::Elasticsearch(_) | Self::OpenSearch(_) => Ok(None),
        }
    }

    pub fn quote_identifier(&self, value: &str) -> String {
        match self {
            Self::Snowflake(_) => snowflake::quote_identifier(value),
            Self::Postgres(_) | Self::CockroachDb(_) => postgres::quote_identifier(value),
            Self::MySql(_) => mysql::quote_identifier(value),
            Self::MariaDb(_) => mysql::quote_identifier(value),
            Self::TiDb(_) => mysql::quote_identifier(value),
            Self::Oracle(_) => format!("\"{}\"", value.replace('"', "\"\"")),
            Self::Sqlite(adapter) => adapter.quote_identifier(value),
            Self::DuckDb(_) => format!("\"{}\"", value.replace('"', "\"\"")),
            Self::SqlServer(adapter) => adapter.quote_identifier(value),
            Self::Redis(_) => value.to_owned(),
            Self::MongoDb(_) | Self::Elasticsearch(_) | Self::OpenSearch(_) => value.to_owned(),
            Self::ClickHouse(_) => format!("`{}`", value.replace('`', "``")),
        }
    }

    pub async fn close(self) {
        match self {
            Self::Snowflake(adapter) => adapter.close().await,
            Self::Postgres(adapter) => adapter.close().await,
            Self::CockroachDb(adapter) => adapter.close().await,
            Self::MySql(adapter) => adapter.close().await,
            Self::MariaDb(adapter) => adapter.close().await,
            Self::TiDb(adapter) => adapter.close().await,
            Self::Oracle(adapter) => adapter.close().await,
            Self::Sqlite(adapter) => adapter.close().await,
            Self::DuckDb(adapter) => adapter.close().await,
            Self::SqlServer(adapter) => adapter.close().await,
            Self::Redis(adapter) => adapter.close().await,
            Self::MongoDb(adapter) => {
                adapter
                    .close_until(Instant::now() + Duration::from_secs(1))
                    .await;
            }
            Self::ClickHouse(adapter) => adapter.close().await,
            Self::Elasticsearch(adapter) | Self::OpenSearch(adapter) => adapter.close().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DatabaseDiagnostic, DatabaseError, DatabaseErrorPosition, ErrorCategory};

    #[test]
    fn output_message_formats_postgres_diagnostics_without_changing_display() {
        let error = DatabaseError {
            category: ErrorCategory::Sql,
            code: Some("42P01".into()),
            message: "relation \"sdfsdf\" does not exist".into(),
            diagnostic: Some(Box::new(DatabaseDiagnostic {
                severity: Some("ERROR".into()),
                detail: None,
                hint: None,
                position: Some(DatabaseErrorPosition::Original(15)),
                context: None,
            })),
        };

        assert_eq!(
            error.output_message(),
            "[42P01] ERROR: relation \"sdfsdf\" does not exist\nPosition: 15"
        );
        assert_eq!(error.to_string(), "relation \"sdfsdf\" does not exist");
    }

    #[test]
    fn output_message_formats_optional_and_internal_diagnostics() {
        let error = DatabaseError {
            category: ErrorCategory::Sql,
            code: None,
            message: "failed".into(),
            diagnostic: Some(Box::new(DatabaseDiagnostic {
                severity: Some("ERROR".into()),
                detail: Some("first line\nsecond line".into()),
                hint: Some("try again".into()),
                position: Some(DatabaseErrorPosition::Internal {
                    position: 3,
                    query: "SELECT 1".into(),
                }),
                context: Some("in function f".into()),
            })),
        };

        assert_eq!(
            error.output_message(),
            "ERROR: failed\nDetail: first line\nsecond line\nHint: try again\nInternal Position: 3\nInternal Query: SELECT 1\nContext: in function f"
        );
    }
}
