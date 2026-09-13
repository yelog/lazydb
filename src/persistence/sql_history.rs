use std::path::Path;

use sqlx::{Row, SqlitePool, sqlite::SqliteConnectOptions, sqlite::SqlitePoolOptions};
use thiserror::Error;
use uuid::Uuid;

use crate::model::sql_history::{
    ExecutionHistory, HistoryExecutionStatus, HistoryResultCertainty, HistoryTransactionOutcome,
};

const SCHEMA: &str = include_str!("sql_history/schema_v1.sql");

#[derive(Debug, Error)]
pub enum HistoryStoreError {
    #[error("history database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("invalid history identifier: {0}")]
    InvalidId(String),
    #[error("invalid history enum value: {0}")]
    InvalidEnum(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryCursor {
    pub requested_at: i64,
    pub execution_id: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryPageRequest {
    pub limit: u32,
    pub cursor: Option<HistoryCursor>,
    pub search: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryPage {
    pub items: Vec<ExecutionHistory>,
    pub next_cursor: Option<HistoryCursor>,
}

#[derive(Clone)]
pub struct HistoryStore {
    pool: SqlitePool,
}

impl HistoryStore {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, HistoryStoreError> {
        if let Some(parent) = path.as_ref().parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(sqlx::Error::Io)?;
        }
        let options = SqliteConnectOptions::new()
            .filename(path.as_ref())
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        sqlx::query("PRAGMA journal_mode = WAL")
            .execute(&pool)
            .await?;
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await?;
        sqlx::query(SCHEMA).execute(&pool).await?;
        migrate_columns(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn insert(&self, history: ExecutionHistory) -> Result<(), HistoryStoreError> {
        sqlx::query(
            "INSERT INTO history_executions
             (execution_id, operation_id, transaction_id, requested_at, sql, status,
              certainty, transaction_outcome, affected_rows, returned_rows, elapsed_millis)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(history.execution_id.to_string())
        .bind(history.operation_id.to_string())
        .bind(history.transaction_id.map(|id| id.to_string()))
        .bind(history.requested_at)
        .bind(history.sql)
        .bind(status_name(history.status))
        .bind(certainty_name(history.certainty))
        .bind(transaction_outcome_name(history.transaction_outcome))
        .bind(history.affected_rows.map(|value| value as i64))
        .bind(history.returned_rows.map(|value| value as i64))
        .bind(history.elapsed_millis.map(|value| value as i64))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn finish(
        &self,
        execution_id: Uuid,
        status: HistoryExecutionStatus,
        certainty: HistoryResultCertainty,
        affected_rows: Option<u64>,
        returned_rows: Option<usize>,
        elapsed_millis: Option<u128>,
    ) -> Result<(), HistoryStoreError> {
        let current = self.detail(execution_id).await?;
        if let Some(current) = current
            && current.certainty == HistoryResultCertainty::Confirmed
            && is_terminal(current.status)
            && certainty == HistoryResultCertainty::Unknown
        {
            return Ok(());
        }
        sqlx::query(
            "UPDATE history_executions
             SET status = ?, certainty = ?, affected_rows = ?, returned_rows = ?, elapsed_millis = ?
             WHERE execution_id = ?",
        )
        .bind(status_name(status))
        .bind(certainty_name(certainty))
        .bind(affected_rows.map(|value| value as i64))
        .bind(returned_rows.map(|value| value as i64))
        .bind(elapsed_millis.map(|value| value as i64))
        .bind(execution_id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn resolve_transaction(
        &self,
        transaction_id: Uuid,
        outcome: HistoryTransactionOutcome,
    ) -> Result<(), HistoryStoreError> {
        sqlx::query(
            "UPDATE history_executions SET transaction_outcome = ?
             WHERE transaction_id = ? AND transaction_outcome IN ('pending', 'unknown')",
        )
        .bind(transaction_outcome_name(outcome))
        .bind(transaction_id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn detail(
        &self,
        execution_id: Uuid,
    ) -> Result<Option<ExecutionHistory>, HistoryStoreError> {
        let row = sqlx::query(
            "SELECT execution_id, operation_id, transaction_id, sql, status, certainty,
                    transaction_outcome, affected_rows, returned_rows, requested_at, elapsed_millis
             FROM history_executions WHERE execution_id = ?",
        )
        .bind(execution_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_history).transpose()
    }

    pub async fn page(
        &self,
        request: HistoryPageRequest,
    ) -> Result<HistoryPage, HistoryStoreError> {
        let limit = request.limit.clamp(1, 500) as i64;
        let rows = match (request.cursor, request.search) {
            (Some(cursor), Some(search)) => {
                sqlx::query(
                    "SELECT execution_id, operation_id, transaction_id, sql, status, certainty,
                        transaction_outcome, affected_rows, returned_rows, requested_at, elapsed_millis
                 FROM history_executions
                 WHERE sql LIKE ? AND (requested_at < ? OR (requested_at = ? AND execution_id < ?))
                 ORDER BY requested_at DESC, execution_id DESC LIMIT ?",
                )
                .bind(format!(
                    "%{}%",
                    search.replace('%', "\\%").replace('_', "\\_")
                ))
                .bind(cursor.requested_at)
                .bind(cursor.requested_at)
                .bind(cursor.execution_id.to_string())
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            (Some(cursor), None) => {
                sqlx::query(
                    "SELECT execution_id, operation_id, transaction_id, sql, status, certainty,
                        transaction_outcome, affected_rows, returned_rows, requested_at, elapsed_millis
                 FROM history_executions
                 WHERE requested_at < ? OR (requested_at = ? AND execution_id < ?)
                 ORDER BY requested_at DESC, execution_id DESC LIMIT ?",
                )
                .bind(cursor.requested_at)
                .bind(cursor.requested_at)
                .bind(cursor.execution_id.to_string())
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            (None, Some(search)) => {
                sqlx::query(
                    "SELECT execution_id, operation_id, transaction_id, sql, status, certainty,
                        transaction_outcome, affected_rows, returned_rows, requested_at, elapsed_millis
                 FROM history_executions WHERE sql LIKE ?
                 ORDER BY requested_at DESC, execution_id DESC LIMIT ?",
                )
                .bind(format!(
                    "%{}%",
                    search.replace('%', "\\%").replace('_', "\\_")
                ))
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
            (None, None) => {
                sqlx::query(
                    "SELECT execution_id, operation_id, transaction_id, sql, status, certainty,
                        transaction_outcome, affected_rows, returned_rows, requested_at, elapsed_millis
                 FROM history_executions ORDER BY requested_at DESC, execution_id DESC LIMIT ?",
                )
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
        };
        let next_cursor = rows
            .last()
            .map(|row| {
                Ok::<HistoryCursor, HistoryStoreError>(HistoryCursor {
                    requested_at: row.get("requested_at"),
                    execution_id: parse_id(row.get::<String, _>("execution_id"))?,
                })
            })
            .transpose()?;
        let items = rows
            .into_iter()
            .map(row_to_history)
            .collect::<Result<_, _>>()?;
        Ok(HistoryPage { items, next_cursor })
    }
}

async fn migrate_columns(pool: &SqlitePool) -> Result<(), HistoryStoreError> {
    let columns = sqlx::query("PRAGMA table_info(history_executions)")
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect::<std::collections::HashSet<_>>();
    if !columns.contains("elapsed_millis") {
        sqlx::query("ALTER TABLE history_executions ADD COLUMN elapsed_millis INTEGER")
            .execute(pool)
            .await?;
    }
    Ok(())
}

fn row_to_history(row: sqlx::sqlite::SqliteRow) -> Result<ExecutionHistory, HistoryStoreError> {
    Ok(ExecutionHistory {
        execution_id: parse_id(row.get("execution_id"))?,
        operation_id: parse_id(row.get("operation_id"))?,
        transaction_id: row
            .try_get::<Option<String>, _>("transaction_id")?
            .map(parse_id)
            .transpose()?,
        sql: row.get("sql"),
        status: parse_status(row.get("status"))?,
        certainty: parse_certainty(row.get("certainty"))?,
        transaction_outcome: parse_transaction_outcome(row.get("transaction_outcome"))?,
        affected_rows: row
            .get::<Option<i64>, _>("affected_rows")
            .map(|value| value as u64),
        returned_rows: row
            .get::<Option<i64>, _>("returned_rows")
            .map(|value| value as usize),
        requested_at: row.get("requested_at"),
        elapsed_millis: row
            .get::<Option<i64>, _>("elapsed_millis")
            .map(|value| value as u128),
    })
}

fn parse_id(value: String) -> Result<Uuid, HistoryStoreError> {
    Uuid::parse_str(&value).map_err(|_| HistoryStoreError::InvalidId(value))
}

fn status_name(value: HistoryExecutionStatus) -> &'static str {
    match value {
        HistoryExecutionStatus::Queued => "queued",
        HistoryExecutionStatus::Running => "running",
        HistoryExecutionStatus::Succeeded => "succeeded",
        HistoryExecutionStatus::Failed => "failed",
        HistoryExecutionStatus::TimedOut => "timed_out",
        HistoryExecutionStatus::Cancelled => "cancelled",
        HistoryExecutionStatus::Interrupted => "interrupted",
        HistoryExecutionStatus::NotExecuted => "not_executed",
    }
}

fn certainty_name(value: HistoryResultCertainty) -> &'static str {
    match value {
        HistoryResultCertainty::Confirmed => "confirmed",
        HistoryResultCertainty::Unknown => "unknown",
    }
}

fn transaction_outcome_name(value: HistoryTransactionOutcome) -> &'static str {
    match value {
        HistoryTransactionOutcome::NotApplicable => "not_applicable",
        HistoryTransactionOutcome::Pending => "pending",
        HistoryTransactionOutcome::AutoCommitted => "auto_committed",
        HistoryTransactionOutcome::Committed => "committed",
        HistoryTransactionOutcome::RolledBack => "rolled_back",
        HistoryTransactionOutcome::RolledBackToSavepoint => "rolled_back_to_savepoint",
        HistoryTransactionOutcome::Unknown => "unknown",
    }
}

fn parse_status(value: String) -> Result<HistoryExecutionStatus, HistoryStoreError> {
    match value.as_str() {
        "queued" => Ok(HistoryExecutionStatus::Queued),
        "running" => Ok(HistoryExecutionStatus::Running),
        "succeeded" => Ok(HistoryExecutionStatus::Succeeded),
        "failed" => Ok(HistoryExecutionStatus::Failed),
        "timed_out" => Ok(HistoryExecutionStatus::TimedOut),
        "cancelled" => Ok(HistoryExecutionStatus::Cancelled),
        "interrupted" => Ok(HistoryExecutionStatus::Interrupted),
        "not_executed" => Ok(HistoryExecutionStatus::NotExecuted),
        _ => Err(HistoryStoreError::InvalidEnum(value)),
    }
}

fn parse_certainty(value: String) -> Result<HistoryResultCertainty, HistoryStoreError> {
    match value.as_str() {
        "confirmed" => Ok(HistoryResultCertainty::Confirmed),
        "unknown" => Ok(HistoryResultCertainty::Unknown),
        _ => Err(HistoryStoreError::InvalidEnum(value)),
    }
}

fn parse_transaction_outcome(
    value: String,
) -> Result<HistoryTransactionOutcome, HistoryStoreError> {
    match value.as_str() {
        "not_applicable" => Ok(HistoryTransactionOutcome::NotApplicable),
        "pending" => Ok(HistoryTransactionOutcome::Pending),
        "auto_committed" => Ok(HistoryTransactionOutcome::AutoCommitted),
        "committed" => Ok(HistoryTransactionOutcome::Committed),
        "rolled_back" => Ok(HistoryTransactionOutcome::RolledBack),
        "rolled_back_to_savepoint" => Ok(HistoryTransactionOutcome::RolledBackToSavepoint),
        "unknown" => Ok(HistoryTransactionOutcome::Unknown),
        _ => Err(HistoryStoreError::InvalidEnum(value)),
    }
}

fn is_terminal(status: HistoryExecutionStatus) -> bool {
    matches!(
        status,
        HistoryExecutionStatus::Succeeded
            | HistoryExecutionStatus::Failed
            | HistoryExecutionStatus::TimedOut
            | HistoryExecutionStatus::Cancelled
            | HistoryExecutionStatus::Interrupted
            | HistoryExecutionStatus::NotExecuted
    )
}
