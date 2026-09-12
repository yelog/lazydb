use async_trait::async_trait;
use futures_util::future::BoxFuture;

use super::{
    DatabaseError,
    mutation::{MutationResult, RelationMutationRequest},
    query::QueryOutcome,
};

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum TransactionRequest {
    Execute {
        query_generation: u64,
        sql: String,
        cancel: tokio::sync::oneshot::Receiver<()>,
        reply: tokio::sync::oneshot::Sender<Result<QueryOutcome, TransactionError>>,
    },
    Page {
        source_sql: String,
        dialect: crate::sql::SqlDialect,
        count_sql: String,
        page: crate::model::pagination::PageRequest,
        reply: tokio::sync::oneshot::Sender<
            Result<(QueryOutcome, crate::model::pagination::ResultPagination), TransactionError>,
        >,
    },
    RelationMutation {
        request: RelationMutationRequest,
        cancel: tokio::sync::oneshot::Receiver<()>,
        reply: tokio::sync::oneshot::Sender<Result<MutationResult, TransactionError>>,
    },
    Commit {
        reply: tokio::sync::oneshot::Sender<Result<(), TransactionError>>,
    },
    Rollback {
        reply: tokio::sync::oneshot::Sender<Result<(), TransactionError>>,
    },
    Shutdown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkerDisposition {
    Committed,
    RolledBack,
    CancelledAndRolledBack,
    ImplicitlyEnded,
    Quarantine,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionError(pub String);

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationMutationCategory {
    Conflict,
    UnsupportedComparison,
    TypeMismatch,
    Constraint,
    ConnectionUnknown,
    InvalidRequest,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct RelationMutationDiagnostic {
    pub category: RelationMutationCategory,
    pub sqlstate: Option<String>,
    pub operation: String,
    pub relation: String,
    pub context: Option<String>,
    pub message: String,
}

impl RelationMutationDiagnostic {
    pub fn safe_message(&self) -> String {
        let mut message = format!("{} on {}: {}", self.operation, self.relation, self.message);
        if let Some(context) = &self.context {
            message.push_str(&format!(" ({context})"));
        }
        if let Some(sqlstate) = &self.sqlstate {
            message.push_str(&format!(" (SQLSTATE {sqlstate})"));
        }
        message
    }
}

const RELATION_DIAGNOSTIC_PREFIX: &str = "[lazydb-relation-diagnostic]";

impl TransactionError {
    pub fn history_snapshot(&self) -> super::HistoryErrorSnapshot {
        if let Some(diagnostic) = self.relation_diagnostic() {
            return super::HistoryErrorSnapshot {
                category: match diagnostic.category {
                    RelationMutationCategory::Constraint => super::ErrorCategory::Constraint,
                    RelationMutationCategory::ConnectionUnknown => super::ErrorCategory::Network,
                    RelationMutationCategory::UnsupportedComparison => {
                        super::ErrorCategory::Unsupported
                    }
                    RelationMutationCategory::Conflict
                    | RelationMutationCategory::TypeMismatch
                    | RelationMutationCategory::InvalidRequest => super::ErrorCategory::Sql,
                },
                code: diagnostic.sqlstate,
                message: diagnostic.message,
            };
        }
        super::HistoryErrorSnapshot {
            category: super::ErrorCategory::Internal,
            code: None,
            message: self.0.clone(),
        }
    }

    pub fn relation(diagnostic: RelationMutationDiagnostic) -> Self {
        // This is an internal bridge format. It deliberately contains only the
        // already-redacted, actionable fields and never server DETAIL/values.
        let payload =
            serde_json::to_string(&diagnostic).expect("relation diagnostic is serializable");
        Self(format!("{RELATION_DIAGNOSTIC_PREFIX}{payload}"))
    }

    pub fn relation_diagnostic(&self) -> Option<RelationMutationDiagnostic> {
        serde_json::from_str(self.0.strip_prefix(RELATION_DIAGNOSTIC_PREFIX)?).ok()
    }
}

impl std::fmt::Display for TransactionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<DatabaseError> for TransactionError {
    fn from(error: DatabaseError) -> Self {
        Self(error.output_message())
    }
}

#[async_trait]
pub trait TransactionBackend: Send + 'static {
    async fn begin(&mut self) -> Result<(), TransactionError>;
    async fn execute(&mut self, sql: &str) -> Result<QueryOutcome, TransactionError>;
    async fn relation_mutation(
        &mut self,
        _request: RelationMutationRequest,
    ) -> Result<MutationResult, TransactionError> {
        Err(TransactionError(
            "relation mutations are not supported by this backend yet".into(),
        ))
    }
    async fn commit(&mut self) -> Result<(), TransactionError>;
    async fn rollback(&mut self) -> Result<(), TransactionError>;
    async fn cancel(&mut self) -> Result<(), TransactionError>;
    fn depth(&self) -> usize;
    fn force_close(self) -> BoxFuture<'static, Result<(), TransactionError>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_diagnostic_round_trips_sqlstate_and_safe_context() {
        let error = TransactionError::relation(RelationMutationDiagnostic {
            category: RelationMutationCategory::Constraint,
            sqlstate: Some("23505".into()),
            operation: "insert".into(),
            relation: "public.items".into(),
            context: Some("row mutation".into()),
            message: "insert failed".into(),
        });
        let diagnostic = error.relation_diagnostic().unwrap();
        assert_eq!(diagnostic.sqlstate.as_deref(), Some("23505"));
        assert_eq!(
            diagnostic.safe_message(),
            "insert on public.items: insert failed (row mutation) (SQLSTATE 23505)"
        );
        assert!(!error.0.contains("DETAIL"));
    }
}
