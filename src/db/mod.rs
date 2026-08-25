pub mod catalog;
pub mod mysql;
pub mod postgres;
pub mod query;
mod sqlite;
pub mod transaction;
pub mod value;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    profile::{ConnectionProfile, DatabaseKind},
    security::sanitize_terminal_text,
};

use self::{
    catalog::{CatalogKind, CatalogNode},
    mysql::MySqlAdapter,
    postgres::PostgresAdapter,
    query::QueryOutcome,
    sqlite::SqliteAdapter,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServerInfo {
    pub kind: DatabaseKind,
    pub version: String,
    pub database: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorCategory {
    Configuration,
    Authentication,
    Network,
    Permission,
    Sql,
    Constraint,
    Cancelled,
    Internal,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("{message}")]
pub struct DatabaseError {
    pub category: ErrorCategory,
    pub code: Option<String>,
    pub message: String,
}

impl DatabaseError {
    pub fn configuration(message: impl AsRef<str>) -> Self {
        Self {
            category: ErrorCategory::Configuration,
            code: None,
            message: sanitize_terminal_text(message.as_ref()),
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
            return Self {
                category,
                code: database.code().map(|code| code.into_owned()),
                message,
            };
        }

        Self {
            category: default_category,
            code: None,
            message: sanitize_terminal_text(&error.to_string()),
        }
    }
}

#[derive(Clone, Debug)]
pub enum DatabaseConnection {
    Postgres(PostgresAdapter),
    MySql(MySqlAdapter),
    Sqlite(SqliteAdapter),
}

impl DatabaseConnection {
    pub(crate) async fn start_transaction_worker(
        &self,
    ) -> Result<crate::runtime::transaction::TransactionWorkerHandle, DatabaseError> {
        match self {
            Self::Postgres(adapter) => Ok(crate::runtime::transaction::spawn_transaction_worker(
                adapter.transaction_backend().await?,
            )),
            Self::MySql(adapter) => Ok(crate::runtime::transaction::spawn_transaction_worker(
                adapter.transaction_backend().await?,
            )),
            Self::Sqlite(adapter) => Ok(crate::runtime::transaction::spawn_transaction_worker(
                adapter.transaction_backend().await?,
            )),
        }
    }

    pub async fn connect(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
    ) -> Result<Self, DatabaseError> {
        match profile.kind {
            DatabaseKind::Sqlite => SqliteAdapter::connect(profile).await.map(Self::Sqlite),
            DatabaseKind::Postgres => PostgresAdapter::connect(profile, password)
                .await
                .map(Self::Postgres),
            DatabaseKind::MySql => MySqlAdapter::connect(profile, password)
                .await
                .map(Self::MySql),
        }
    }

    pub fn kind(&self) -> DatabaseKind {
        match self {
            Self::Postgres(_) => DatabaseKind::Postgres,
            Self::MySql(_) => DatabaseKind::MySql,
            Self::Sqlite(_) => DatabaseKind::Sqlite,
        }
    }

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        match self {
            Self::Postgres(adapter) => adapter.probe().await,
            Self::MySql(adapter) => adapter.probe().await,
            Self::Sqlite(adapter) => adapter.probe().await,
        }
    }

    pub async fn load_catalog(&self) -> Result<Vec<CatalogNode>, DatabaseError> {
        match self {
            Self::Postgres(adapter) => adapter.load_catalog().await,
            Self::MySql(adapter) => adapter.load_catalog().await,
            Self::Sqlite(adapter) => adapter.load_catalog().await,
        }
    }

    pub async fn execute(&self, sql: &str) -> Result<QueryOutcome, DatabaseError> {
        match self {
            Self::Postgres(adapter) => adapter.execute_pool(sql).await,
            Self::MySql(adapter) => adapter.execute_pool(sql).await,
            Self::Sqlite(adapter) => adapter.execute_pool(sql).await,
        }
    }

    pub async fn object_ddl(
        &self,
        kind: CatalogKind,
        schema: &str,
        name: &str,
    ) -> Result<Option<String>, DatabaseError> {
        match self {
            Self::Postgres(adapter) => adapter.object_ddl(kind, schema, name).await,
            Self::MySql(adapter) => adapter.object_ddl(kind, schema, name).await,
            Self::Sqlite(adapter) => adapter.object_ddl(kind, schema, name).await,
        }
    }

    pub fn quote_identifier(&self, value: &str) -> String {
        match self {
            Self::Postgres(_) => postgres::quote_identifier(value),
            Self::MySql(_) => mysql::quote_identifier(value),
            Self::Sqlite(adapter) => adapter.quote_identifier(value),
        }
    }

    pub async fn close(self) {
        match self {
            Self::Postgres(adapter) => adapter.close().await,
            Self::MySql(adapter) => adapter.close().await,
            Self::Sqlite(adapter) => adapter.close().await,
        }
    }
}
