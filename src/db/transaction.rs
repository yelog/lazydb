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

impl std::fmt::Display for TransactionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<DatabaseError> for TransactionError {
    fn from(error: DatabaseError) -> Self {
        Self(error.to_string())
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
