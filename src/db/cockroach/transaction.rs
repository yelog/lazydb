use super::super::{
    DatabaseError, ErrorCategory, pgwire,
    query::{QueryBudget, QueryOutcome},
    transaction::{
        RelationMutationCategory, RelationMutationDiagnostic, TransactionBackend, TransactionError,
    },
};
use sqlx::{Postgres, pool::PoolConnection};
use sqlx_core::transaction::TransactionManager;

pub struct CockroachTransactionBackend {
    // Taking the connection on cancellation guarantees it cannot return to the pool.
    connection: Option<PoolConnection<Postgres>>,
}

impl CockroachTransactionBackend {
    pub(super) fn new(connection: PoolConnection<Postgres>) -> Self {
        Self {
            connection: Some(connection),
        }
    }

    fn connection(&mut self) -> Result<&mut PoolConnection<Postgres>, TransactionError> {
        self.connection.as_mut().ok_or_else(|| {
            TransactionError(
                "CockroachDB transaction session has been closed; restart the transaction".into(),
            )
        })
    }

    async fn close_connection(&mut self) -> Result<(), TransactionError> {
        if let Some(connection) = self.connection.take() {
            // Drop the detached socket immediately: SQLx's TLS close_hard can wait
            // for peer I/O. This session must neither block cancellation nor be recycled.
            drop(connection.detach());
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl TransactionBackend for CockroachTransactionBackend {
    async fn begin(&mut self) -> Result<(), TransactionError> {
        <Postgres as sqlx::Database>::TransactionManager::begin(self.connection()?, None)
            .await
            .map_err(|error| {
                transaction_error(DatabaseError::from_sqlx(error, ErrorCategory::Sql), "begin")
            })
    }

    async fn execute(&mut self, sql: &str) -> Result<QueryOutcome, TransactionError> {
        pgwire::execute_connection(self.connection()?, sql, QueryBudget::UNBOUNDED)
            .await
            .map_err(|error| transaction_error(error, "execute"))
    }

    async fn commit(&mut self) -> Result<(), TransactionError> {
        <Postgres as sqlx::Database>::TransactionManager::commit(self.connection()?)
            .await
            .map_err(|error| {
                let category = if matches!(&error, sqlx::Error::Database(_)) {
                    ErrorCategory::Sql
                } else {
                    ErrorCategory::Network
                };
                transaction_error(DatabaseError::from_sqlx(error, category), "commit")
            })
    }

    fn commit_failure_requires_rollback(&self, error: &TransactionError) -> bool {
        self.depth() > 0 && error.history_snapshot().category != ErrorCategory::Network
    }

    async fn rollback(&mut self) -> Result<(), TransactionError> {
        // The runtime rolls back after cancel. The session was already destroyed.
        let Some(connection) = self.connection.as_mut() else {
            return Ok(());
        };
        <Postgres as sqlx::Database>::TransactionManager::rollback(connection)
            .await
            .map_err(|error| {
                transaction_error(
                    DatabaseError::from_sqlx(error, ErrorCategory::Sql),
                    "rollback",
                )
            })
    }

    async fn cancel(&mut self) -> Result<(), TransactionError> {
        self.close_connection().await
    }

    fn depth(&self) -> usize {
        self.connection.as_ref().map_or(0, |connection| {
            <Postgres as sqlx::Database>::TransactionManager::get_transaction_depth(connection)
        })
    }

    fn force_close(
        mut self,
    ) -> futures_util::future::BoxFuture<'static, Result<(), TransactionError>> {
        Box::pin(async move { self.close_connection().await })
    }
}

fn transaction_error(mut error: DatabaseError, operation: &str) -> TransactionError {
    // CockroachDB can explicitly report an ambiguous commit even when the
    // connection remains available. The session must still be quarantined.
    if operation == "commit" && error.code.as_deref() == Some("40003") {
        error.category = ErrorCategory::Network;
    }
    let mut message = error.message;
    if error.code.as_deref() == Some("40001") {
        message.push_str("; ROLLBACK and restart the entire transaction manually");
    } else if operation == "commit"
        && matches!(
            error.category,
            ErrorCategory::Network | ErrorCategory::Internal
        )
    {
        message.push_str(
            "; commit outcome is unknown; verify the result before starting another transaction",
        );
    }
    TransactionError::relation(RelationMutationDiagnostic {
        category: match error.category {
            ErrorCategory::Constraint => RelationMutationCategory::Constraint,
            ErrorCategory::Network | ErrorCategory::Internal => {
                RelationMutationCategory::ConnectionUnknown
            }
            ErrorCategory::Unsupported => RelationMutationCategory::UnsupportedComparison,
            _ => RelationMutationCategory::InvalidRequest,
        },
        sqlstate: error.code,
        operation: operation.into(),
        relation: "CockroachDB transaction".into(),
        context: None,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commit_network_error_warns_about_unknown_outcome() {
        let error = transaction_error(
            DatabaseError {
                category: ErrorCategory::Network,
                code: None,
                message: "connection closed".into(),
                diagnostic: None,
            },
            "commit",
        );
        let diagnostic = error.relation_diagnostic().unwrap();
        assert_eq!(
            diagnostic.category,
            RelationMutationCategory::ConnectionUnknown
        );
        assert!(diagnostic.message.contains("commit outcome is unknown"));
        assert!(diagnostic.message.contains("verify the result"));
    }

    #[test]
    fn ambiguous_commit_sqlstate_requires_quarantine_and_preserves_recovery_guidance() {
        let error = transaction_error(
            DatabaseError {
                category: ErrorCategory::Sql,
                code: Some("40003".into()),
                message: "statement completion unknown".into(),
                diagnostic: None,
            },
            "commit",
        );
        let diagnostic = error.relation_diagnostic().unwrap();
        assert_eq!(
            diagnostic.category,
            RelationMutationCategory::ConnectionUnknown
        );
        assert_eq!(error.history_snapshot().category, ErrorCategory::Network);
        assert_eq!(diagnostic.sqlstate.as_deref(), Some("40003"));
        assert!(diagnostic.safe_message().contains("SQLSTATE 40003"));
        assert!(diagnostic.message.contains("commit outcome is unknown"));
        assert!(diagnostic.message.contains("verify the result"));
        assert!(!diagnostic.message.contains("ROLLBACK and restart"));
    }

    #[test]
    fn retryable_error_preserves_sqlstate_and_requires_manual_restart() {
        let error = transaction_error(
            DatabaseError {
                category: ErrorCategory::Sql,
                code: Some("40001".into()),
                message: "restart transaction".into(),
                diagnostic: None,
            },
            "execute",
        );
        assert_eq!(error.history_snapshot().code.as_deref(), Some("40001"));
        assert!(
            error
                .relation_diagnostic()
                .unwrap()
                .message
                .contains("ROLLBACK and restart")
        );
    }
}
