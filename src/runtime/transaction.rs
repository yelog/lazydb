#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use tokio::{sync::mpsc, task::JoinHandle};

use crate::db::query::QueryOutcome;
use crate::db::transaction::{
    TransactionBackend, TransactionError, TransactionRequest, WorkerDisposition,
};

#[derive(Clone)]
pub struct ForcedCloseHandle {
    state: Arc<Mutex<ForcedCloseState>>,
}

#[derive(Default)]
struct ForcedCloseState {
    requested: bool,
    completed: bool,
}

impl ForcedCloseHandle {
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(ForcedCloseState::default())),
        }
    }

    pub fn requested(&self) -> bool {
        self.state
            .lock()
            .expect("forced-close mutex poisoned")
            .requested
    }

    pub fn completed(&self) -> bool {
        self.state
            .lock()
            .expect("forced-close mutex poisoned")
            .completed
    }

    fn request(&self) {
        self.state
            .lock()
            .expect("forced-close mutex poisoned")
            .requested = true;
    }

    fn complete(&self) {
        self.state
            .lock()
            .expect("forced-close mutex poisoned")
            .completed = true;
    }
}

pub struct TransactionWorkerHandle {
    pub(crate) requests: mpsc::UnboundedSender<TransactionRequest>,
    pub(crate) worker: JoinHandle<WorkerDisposition>,
    pub(crate) cancellation: Option<tokio::sync::oneshot::Sender<()>>,
    pub(crate) forced_close: ForcedCloseHandle,
    pub(crate) readiness: tokio::sync::oneshot::Receiver<Result<(), TransactionError>>,
}

struct ArmedGuard<B: TransactionBackend> {
    backend: Option<B>,
    armed: bool,
    forced_close: ForcedCloseHandle,
}

impl<B> ArmedGuard<B>
where
    B: TransactionBackend,
{
    fn new(backend: B, forced_close: ForcedCloseHandle) -> Self {
        Self {
            backend: Some(backend),
            armed: false,
            forced_close,
        }
    }

    fn backend_mut(&mut self) -> &mut B {
        self.backend.as_mut().expect("transaction backend missing")
    }

    fn arm(&mut self) {
        self.armed = true;
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl<B: TransactionBackend> Drop for ArmedGuard<B> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Some(backend) = self.backend.take() else {
            return;
        };
        let forced_close = self.forced_close.clone();
        forced_close.request();
        tokio::spawn(async move {
            let _ = backend.force_close().await;
            forced_close.complete();
        });
    }
}

pub fn spawn_transaction_worker<B>(backend: B) -> TransactionWorkerHandle
where
    B: TransactionBackend,
{
    let (requests, mut receiver) = mpsc::unbounded_channel();
    let (readiness_sender, readiness) = tokio::sync::oneshot::channel();
    let forced_close = ForcedCloseHandle::new();
    let worker_for_task = forced_close.clone();
    let worker = tokio::spawn(async move {
        let mut guard = ArmedGuard::new(backend, worker_for_task);
        guard.arm();
        let begin = guard.backend_mut().begin().await;
        let begin_failed = begin.is_err();
        let _ = readiness_sender.send(begin.map_err(|error| error.clone()));
        if begin_failed {
            return WorkerDisposition::Quarantine;
        }

        while let Some(request) = receiver.recv().await {
            match request {
                TransactionRequest::Execute {
                    sql, cancel, reply, ..
                } => {
                    let outcome = execute_or_cancel(guard.backend_mut(), &sql, cancel).await;
                    if let Some(result) = outcome {
                        let _ = reply.send(result);
                        if guard.backend_mut().depth() == 0 {
                            guard.disarm();
                            return WorkerDisposition::ImplicitlyEnded;
                        }
                    } else {
                        if guard.backend_mut().cancel().await.is_err() {
                            return WorkerDisposition::Quarantine;
                        }
                        if guard.backend_mut().rollback().await.is_err()
                            || guard.backend_mut().depth() != 0
                        {
                            return WorkerDisposition::Quarantine;
                        }
                        guard.disarm();
                        let _ = reply.send(Err(TransactionError("cancelled".to_owned())));
                        return WorkerDisposition::CancelledAndRolledBack;
                    }
                }
                TransactionRequest::Commit { reply } => {
                    let result = guard.backend_mut().commit().await;
                    let safe = result.is_ok() && guard.backend_mut().depth() == 0;
                    if safe {
                        guard.disarm();
                    }
                    let _ = reply.send(result);
                    if safe {
                        return WorkerDisposition::Committed;
                    }
                }
                TransactionRequest::Rollback { reply } => {
                    let result = guard.backend_mut().rollback().await;
                    let safe = result.is_ok() && guard.backend_mut().depth() == 0;
                    if safe {
                        guard.disarm();
                    }
                    let _ = reply.send(result);
                    if safe {
                        return WorkerDisposition::RolledBack;
                    }
                }
                TransactionRequest::Shutdown => {
                    let result = guard.backend_mut().rollback().await;
                    if result.is_ok() && guard.backend_mut().depth() == 0 {
                        guard.disarm();
                        return WorkerDisposition::RolledBack;
                    }
                    return WorkerDisposition::Quarantine;
                }
            }
        }
        WorkerDisposition::Quarantine
    });
    TransactionWorkerHandle {
        requests,
        worker,
        cancellation: None,
        forced_close,
        readiness,
    }
}

async fn execute_or_cancel<B: TransactionBackend>(
    backend: &mut B,
    sql: &str,
    cancel: tokio::sync::oneshot::Receiver<()>,
) -> Option<Result<QueryOutcome, TransactionError>> {
    let execute = backend.execute(sql);
    tokio::pin!(execute);
    tokio::select! {
        result = &mut execute => Some(result),
        cancellation = cancel => match cancellation {
            Ok(()) => None,
            Err(_) => Some((&mut execute).await),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::query::QueryOutcomeAccumulator;
    use async_trait::async_trait;
    use futures_util::future::BoxFuture;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tokio::sync::oneshot;

    #[derive(Clone, Default)]
    struct Fake {
        log: Arc<Mutex<Vec<&'static str>>>,
        depth: usize,
        begin_fails: bool,
        cancel_fails: bool,
        commit_fails: bool,
        rollback_fails: bool,
    }

    #[async_trait]
    impl TransactionBackend for Fake {
        async fn begin(&mut self) -> Result<(), TransactionError> {
            self.log.lock().unwrap().push("begin");
            if self.begin_fails {
                Err(TransactionError("begin".into()))
            } else {
                self.depth = 1;
                Ok(())
            }
        }
        async fn execute(&mut self, sql: &str) -> Result<QueryOutcome, TransactionError> {
            self.log
                .lock()
                .unwrap()
                .push(if sql == "slow" { "slow" } else { "execute" });
            if sql == "slow" {
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
            Ok(QueryOutcomeAccumulator::new().finish())
        }
        async fn commit(&mut self) -> Result<(), TransactionError> {
            self.log.lock().unwrap().push("commit");
            if self.commit_fails {
                Err(TransactionError("commit".into()))
            } else {
                self.depth = 0;
                Ok(())
            }
        }
        async fn rollback(&mut self) -> Result<(), TransactionError> {
            self.log.lock().unwrap().push("rollback");
            if self.rollback_fails {
                Err(TransactionError("rollback".into()))
            } else {
                self.depth = 0;
                Ok(())
            }
        }
        async fn cancel(&mut self) -> Result<(), TransactionError> {
            self.log.lock().unwrap().push("cancel");
            if self.cancel_fails {
                Err(TransactionError("cancel".into()))
            } else {
                Ok(())
            }
        }
        fn depth(&self) -> usize {
            self.depth
        }
        fn force_close(self) -> BoxFuture<'static, Result<(), TransactionError>> {
            let log = self.log;
            Box::pin(async move {
                log.lock().unwrap().push("force_close");
                Ok(())
            })
        }
    }

    async fn execute(
        worker: &TransactionWorkerHandle,
        sql: &str,
    ) -> Result<QueryOutcome, TransactionError> {
        let (reply, result) = oneshot::channel();
        let (_, cancel) = oneshot::channel();
        worker
            .requests
            .send(TransactionRequest::Execute {
                query_generation: 1,
                sql: sql.into(),
                cancel,
                reply,
            })
            .unwrap();
        result.await.unwrap()
    }

    #[tokio::test]
    async fn serial_execute_commit_and_rollback() {
        let fake = Fake::default();
        let log = fake.log.clone();
        let worker = spawn_transaction_worker(fake);
        execute(&worker, "one").await.unwrap();
        let (reply, result) = oneshot::channel();
        worker
            .requests
            .send(TransactionRequest::Rollback { reply })
            .unwrap();
        assert!(result.await.unwrap().is_ok());
        assert_eq!(*log.lock().unwrap(), vec!["begin", "execute", "rollback"]);
    }

    #[tokio::test]
    async fn cancellation_rolls_back_and_does_not_only_drop_future() {
        let fake = Fake::default();
        let log = fake.log.clone();
        let worker = spawn_transaction_worker(fake);
        let (reply, result) = oneshot::channel();
        let (cancel, cancellation) = oneshot::channel();
        worker
            .requests
            .send(TransactionRequest::Execute {
                query_generation: 1,
                sql: "slow".into(),
                cancel: cancellation,
                reply,
            })
            .unwrap();
        cancel.send(()).unwrap();
        assert!(result.await.unwrap().is_err());
        let log = log.lock().unwrap().clone();
        assert!(log.ends_with(&["cancel", "rollback"]));
        assert!(log.contains(&"slow") || log == vec!["begin", "cancel", "rollback"]);
    }

    #[tokio::test]
    async fn begin_failure_quarantines() {
        let fake = Fake {
            begin_fails: true,
            ..Fake::default()
        };
        let worker = spawn_transaction_worker(fake);
        assert_eq!(worker.worker.await.unwrap(), WorkerDisposition::Quarantine);
        assert!(worker.forced_close.completed());
    }

    #[tokio::test]
    async fn failed_cancel_forces_close() {
        let fake = Fake {
            cancel_fails: true,
            ..Fake::default()
        };
        let worker = spawn_transaction_worker(fake);
        let (reply, result) = oneshot::channel();
        let (cancel, cancellation) = oneshot::channel();
        worker
            .requests
            .send(TransactionRequest::Execute {
                query_generation: 1,
                sql: "slow".into(),
                cancel: cancellation,
                reply,
            })
            .unwrap();
        cancel.send(()).unwrap();
        drop(result);
        assert_eq!(worker.worker.await.unwrap(), WorkerDisposition::Quarantine);
        tokio::task::yield_now().await;
        assert!(worker.forced_close.requested());
    }

    #[tokio::test]
    async fn commit_rejection_keeps_guard_armed_until_shutdown_cleanup() {
        let fake = Fake {
            commit_fails: true,
            ..Fake::default()
        };
        let log = fake.log.clone();
        let worker = spawn_transaction_worker(fake);
        let (reply, result) = oneshot::channel();
        worker
            .requests
            .send(TransactionRequest::Commit { reply })
            .unwrap();
        assert!(result.await.unwrap().is_err());
        drop(worker.requests.clone());
        worker.worker.abort();
        let _ = worker.worker.await;
        tokio::task::yield_now().await;
        assert!(log.lock().unwrap().contains(&"force_close"));
    }

    #[tokio::test]
    async fn shutdown_rolls_back_and_ack_loss_does_not_change_worker_safety() {
        let fake = Fake::default();
        let log = fake.log.clone();
        let worker = spawn_transaction_worker(fake);
        let (reply, result) = oneshot::channel();
        worker
            .requests
            .send(TransactionRequest::Commit { reply })
            .unwrap();
        drop(result);
        let _ = worker.worker.await;
        assert_eq!(*log.lock().unwrap(), vec!["begin", "commit"]);

        let fake = Fake::default();
        let log = fake.log.clone();
        let worker = spawn_transaction_worker(fake);
        worker.requests.send(TransactionRequest::Shutdown).unwrap();
        assert_eq!(worker.worker.await.unwrap(), WorkerDisposition::RolledBack);
        assert_eq!(*log.lock().unwrap(), vec!["begin", "rollback"]);
    }
}
