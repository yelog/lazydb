use std::sync::Arc;

use tokio::sync::{Mutex, Notify, mpsc, oneshot};
use uuid::Uuid;

use crate::{
    model::sql_history::{
        ExecutionHistory, HistoryExecutionStatus, HistoryResultCertainty, HistoryTransactionOutcome,
    },
    persistence::sql_history::{HistoryStore, HistoryStoreError},
};

const DEFAULT_QUEUE_CAPACITY: usize = 128;

#[derive(Debug, thiserror::Error)]
pub enum HistoryRecorderError {
    #[error("SQL history recorder is closed")]
    Closed,
    #[error("SQL history recorder failed: {0}")]
    Store(String),
}

enum HistoryCommand {
    Start(ExecutionHistory),
    Finish {
        execution_id: Uuid,
        status: HistoryExecutionStatus,
        certainty: HistoryResultCertainty,
        affected_rows: Option<u64>,
        returned_rows: Option<usize>,
    },
    ResolveTransaction {
        transaction_id: Uuid,
        outcome: HistoryTransactionOutcome,
    },
    Flush(oneshot::Sender<Result<(), HistoryRecorderError>>),
    Shutdown(oneshot::Sender<Result<(), HistoryRecorderError>>),
}

#[derive(Clone)]
pub struct HistoryRecorder {
    sender: Arc<Mutex<Option<mpsc::Sender<HistoryCommand>>>>,
    idle: Arc<Notify>,
}

impl HistoryRecorder {
    pub fn new(store: HistoryStore, capacity: usize) -> Self {
        let (sender, mut receiver) = mpsc::channel(capacity.max(1));
        let idle = Arc::new(Notify::new());
        let failure = Arc::new(Mutex::new(None));
        let worker_idle = Arc::clone(&idle);
        let worker_failure = Arc::clone(&failure);
        tokio::spawn(async move {
            while let Some(command) = receiver.recv().await {
                match command {
                    HistoryCommand::Start(history) => {
                        if let Err(error) = store.insert(history).await {
                            record_failure(&worker_failure, error).await;
                        }
                    }
                    HistoryCommand::Finish {
                        execution_id,
                        status,
                        certainty,
                        affected_rows,
                        returned_rows,
                    } => {
                        if let Err(error) = store
                            .finish(
                                execution_id,
                                status,
                                certainty,
                                affected_rows,
                                returned_rows,
                            )
                            .await
                        {
                            record_failure(&worker_failure, error).await;
                        }
                    }
                    HistoryCommand::ResolveTransaction {
                        transaction_id,
                        outcome,
                    } => {
                        if let Err(error) = store.resolve_transaction(transaction_id, outcome).await
                        {
                            record_failure(&worker_failure, error).await;
                        }
                    }
                    HistoryCommand::Flush(reply) => {
                        let result = worker_failure
                            .lock()
                            .await
                            .as_ref()
                            .map(|message| HistoryRecorderError::Store(message.clone()))
                            .map_or(Ok(()), Err);
                        let _ = reply.send(result);
                    }
                    HistoryCommand::Shutdown(reply) => {
                        let _ = reply.send(Ok(()));
                        break;
                    }
                }
                worker_idle.notify_waiters();
            }
            worker_idle.notify_waiters();
        });
        Self {
            sender: Arc::new(Mutex::new(Some(sender))),
            idle,
        }
    }

    pub fn default_capacity(store: HistoryStore) -> Self {
        Self::new(store, DEFAULT_QUEUE_CAPACITY)
    }

    pub async fn start(&self, history: ExecutionHistory) -> Result<(), HistoryRecorderError> {
        self.send(HistoryCommand::Start(history)).await
    }

    pub async fn finish(
        &self,
        execution_id: Uuid,
        status: HistoryExecutionStatus,
        affected_rows: Option<u64>,
        returned_rows: Option<usize>,
    ) -> Result<(), HistoryRecorderError> {
        self.send(HistoryCommand::Finish {
            execution_id,
            status,
            certainty: if matches!(
                status,
                HistoryExecutionStatus::Interrupted | HistoryExecutionStatus::TimedOut
            ) {
                HistoryResultCertainty::Unknown
            } else {
                HistoryResultCertainty::Confirmed
            },
            affected_rows,
            returned_rows,
        })
        .await
    }

    pub async fn finish_with_certainty(
        &self,
        execution_id: Uuid,
        status: HistoryExecutionStatus,
        certainty: HistoryResultCertainty,
        affected_rows: Option<u64>,
        returned_rows: Option<usize>,
    ) -> Result<(), HistoryRecorderError> {
        self.send(HistoryCommand::Finish {
            execution_id,
            status,
            certainty,
            affected_rows,
            returned_rows,
        })
        .await
    }

    pub async fn flush(&self) -> Result<(), HistoryRecorderError> {
        let (reply, result) = oneshot::channel();
        self.send(HistoryCommand::Flush(reply)).await?;
        result.await.map_err(|_| HistoryRecorderError::Closed)?
    }

    pub async fn resolve_transaction(
        &self,
        transaction_id: Uuid,
        outcome: HistoryTransactionOutcome,
    ) -> Result<(), HistoryRecorderError> {
        self.send(HistoryCommand::ResolveTransaction {
            transaction_id,
            outcome,
        })
        .await
    }

    pub async fn shutdown(&self) -> Result<(), HistoryRecorderError> {
        let (reply, result) = oneshot::channel();
        self.send(HistoryCommand::Shutdown(reply)).await?;
        let outcome = result.await.map_err(|_| HistoryRecorderError::Closed)?;
        self.sender.lock().await.take();
        outcome
    }

    async fn send(&self, command: HistoryCommand) -> Result<(), HistoryRecorderError> {
        let sender = self
            .sender
            .lock()
            .await
            .as_ref()
            .cloned()
            .ok_or(HistoryRecorderError::Closed)?;
        sender
            .send(command)
            .await
            .map_err(|_| HistoryRecorderError::Closed)
    }

    #[allow(dead_code)]
    pub async fn wait_until_idle(&self) {
        self.idle.notified().await;
    }
}

async fn record_failure(failure: &Mutex<Option<String>>, error: HistoryStoreError) {
    let mut slot = failure.lock().await;
    slot.get_or_insert_with(|| error.to_string());
}
