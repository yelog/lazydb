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
    spawn_transaction_worker_with_forced_close(backend, ForcedCloseHandle::new())
}

pub(crate) fn spawn_transaction_worker_with_forced_close<B>(
    backend: B,
    forced_close: ForcedCloseHandle,
) -> TransactionWorkerHandle
where
    B: TransactionBackend,
{
    let (requests, mut receiver) = mpsc::unbounded_channel();
    let (readiness_sender, readiness) = tokio::sync::oneshot::channel();
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
                        // Some backends cancel by closing the session. Their depth is already
                        // zero and rollback would be an operation on a nonexistent client.
                        if guard.backend_mut().depth() == 0 {
                            guard.disarm();
                            let _ = reply.send(Err(TransactionError("cancelled".to_owned())));
                            return WorkerDisposition::CancelledAndRolledBack;
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
                TransactionRequest::Page {
                    source_sql,
                    dialect,
                    count_sql,
                    mut page,
                    reply,
                } => {
                    let total = if page.resolve_total {
                        match guard.backend_mut().execute(&count_sql).await {
                            Ok(outcome) => match count_from_outcome(&outcome) {
                                Ok(total) => Some(total),
                                Err(error) => {
                                    let _ = reply.send(Err(error));
                                    return WorkerDisposition::Quarantine;
                                }
                            },
                            Err(error) => {
                                let _ = reply.send(Err(error));
                                return WorkerDisposition::Quarantine;
                            }
                        }
                    } else {
                        None
                    };
                    if let Some(total) = total {
                        page.offset = crate::model::pagination::ResultPagination::last_offset(
                            page.size, total,
                        );
                    }
                    let page_sql =
                        match crate::sql::build_paginated_query(&source_sql, dialect, page) {
                            Ok(query) => query.page_sql,
                            Err(error) => {
                                let _ = reply.send(Err(TransactionError(error.to_string())));
                                return WorkerDisposition::Quarantine;
                            }
                        };
                    let result = guard.backend_mut().execute(&page_sql).await;
                    match result {
                        Ok(mut outcome) => {
                            let fetched = outcome.stats.row_count;
                            if let Some(result) = outcome.result_sets.first_mut() {
                                result.rows.truncate(page.size.get());
                            }
                            outcome.stats.row_count = outcome
                                .result_sets
                                .iter()
                                .map(|result| result.rows.len())
                                .sum();
                            let mut pagination =
                                crate::model::pagination::ResultPagination::from_page(
                                    page, fetched,
                                );
                            if let Some(total) = total {
                                pagination.total =
                                    crate::model::pagination::TotalRows::Exact(total);
                                pagination.has_next =
                                    page.offset.saturating_add(pagination.visible_rows as u64)
                                        < total;
                            }
                            let _ = reply.send(Ok((outcome, pagination)));
                        }
                        Err(error) => {
                            let _ = reply.send(Err(error));
                        }
                    }
                }
                TransactionRequest::RelationMutation {
                    request,
                    cancel,
                    reply,
                } => {
                    let outcome =
                        relation_mutation_or_cancel(guard.backend_mut(), request, cancel).await;
                    let Some(result) = outcome else {
                        let _ = reply.send(Err(TransactionError("cancelled".into())));
                        let cancelled = guard.backend_mut().cancel().await;
                        if cancelled.is_err() || guard.backend_mut().depth() == 0 {
                            return WorkerDisposition::Quarantine;
                        }
                        if guard.backend_mut().rollback().await.is_err()
                            || guard.backend_mut().depth() != 0
                        {
                            return WorkerDisposition::Quarantine;
                        }
                        guard.disarm();
                        return WorkerDisposition::CancelledAndRolledBack;
                    };
                    let _ = reply.send(result);
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
                    return WorkerDisposition::Quarantine;
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
                    return WorkerDisposition::Quarantine;
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
        biased;
        result = &mut execute => Some(result),
        cancellation = cancel => match cancellation {
            Ok(()) => None,
            Err(_) => Some((&mut execute).await),
        },
    }
}

async fn relation_mutation_or_cancel<B: TransactionBackend>(
    backend: &mut B,
    request: crate::db::mutation::RelationMutationRequest,
    cancel: tokio::sync::oneshot::Receiver<()>,
) -> Option<Result<crate::db::mutation::MutationResult, TransactionError>> {
    let mutation = backend.relation_mutation(request);
    tokio::pin!(mutation);
    tokio::select! {
        biased;
        result = &mut mutation => Some(result),
        cancellation = cancel => if cancellation.is_ok() {
            None
        } else {
            Some(mutation.await)
        },
    }
}

fn count_from_outcome(outcome: &QueryOutcome) -> Result<u64, TransactionError> {
    let value = outcome
        .result_sets
        .first()
        .and_then(|set| set.rows.first())
        .and_then(|row| row.first())
        .ok_or_else(|| TransactionError("count query returned no count value".into()))?;
    let count = match value {
        crate::db::value::CellValue::Integer(value) => (*value)
            .try_into()
            .map_err(|_| TransactionError("count query returned a negative value".into()))?,
        crate::db::value::CellValue::Unsigned(value) => *value,
        _ => {
            return Err(TransactionError(
                "count query returned a non-integer value".into(),
            ));
        }
    };
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::catalog::{CatalogId, CatalogKind};
    use crate::db::mutation::{
        InputValue, MetadataFingerprint, MutationResult, RelationMutation, RelationMutationRequest,
        RowLocator, UpdateCellMutation,
    };
    use crate::db::query::{QueryBudget, QueryOutcomeAccumulator};
    use crate::db::value::CellValue;
    use crate::{
        identity::ConnectionIdentity,
        model::{execution_target::ExecutionTarget, relation::RelationKey},
        profile::{CatalogScope, DatabaseKind},
    };
    use async_trait::async_trait;
    use futures_util::future::BoxFuture;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tokio::sync::oneshot;

    #[derive(Clone, Default)]
    struct Fake {
        log: Arc<Mutex<Vec<String>>>,
        depth: usize,
        begin_fails: bool,
        cancel_fails: bool,
        cancel_closes: bool,
        commit_fails: bool,
        rollback_fails: bool,
    }

    #[async_trait]
    impl TransactionBackend for Fake {
        async fn begin(&mut self) -> Result<(), TransactionError> {
            self.log.lock().unwrap().push("begin".into());
            if self.begin_fails {
                Err(TransactionError("begin".into()))
            } else {
                self.depth = 1;
                Ok(())
            }
        }
        async fn execute(&mut self, sql: &str) -> Result<QueryOutcome, TransactionError> {
            self.log.lock().unwrap().push(sql.to_owned());
            if sql == "slow" {
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
            if sql == "count" {
                let mut outcome = QueryOutcomeAccumulator::with_budget(QueryBudget::UNBOUNDED);
                outcome.row(Vec::new(), vec![CellValue::Integer(1234)]);
                return Ok(outcome.finish());
            }
            Ok(QueryOutcomeAccumulator::with_budget(QueryBudget::UNBOUNDED).finish())
        }
        async fn commit(&mut self) -> Result<(), TransactionError> {
            self.log.lock().unwrap().push("commit".into());
            if self.commit_fails {
                Err(TransactionError("commit".into()))
            } else {
                self.depth = 0;
                Ok(())
            }
        }
        async fn relation_mutation(
            &mut self,
            _request: RelationMutationRequest,
        ) -> Result<MutationResult, TransactionError> {
            self.log.lock().unwrap().push("relation_mutation".into());
            Ok(MutationResult::Updated {
                row: vec![CellValue::Integer(1), CellValue::Text("new".into())],
            })
        }
        async fn rollback(&mut self) -> Result<(), TransactionError> {
            self.log.lock().unwrap().push("rollback".into());
            if self.rollback_fails {
                Err(TransactionError("rollback".into()))
            } else {
                self.depth = 0;
                Ok(())
            }
        }
        async fn cancel(&mut self) -> Result<(), TransactionError> {
            self.log.lock().unwrap().push("cancel".into());
            if self.cancel_closes {
                self.depth = 0;
            }
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
                log.lock().unwrap().push("force_close".into());
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
        assert_eq!(
            *log.lock().unwrap(),
            vec![
                String::from("begin"),
                String::from("one"),
                String::from("rollback")
            ]
        );
    }

    #[tokio::test]
    async fn relation_mutation_is_executed_before_commit_on_the_same_worker() {
        let fake = Fake::default();
        let log = fake.log.clone();
        let worker = spawn_transaction_worker(fake);
        let id = uuid::Uuid::nil();
        let request = RelationMutationRequest {
            tab_id: id,
            tab_generation: 1,
            edit_generation: 1,
            row_id: crate::model::relation_edit::EditableRowId(1),
            connection: ConnectionIdentity {
                profile_id: id,
                generation: 1,
            },
            target: ExecutionTarget {
                profile_id: id,
                database: "db".into(),
                schema: Some("main".into()),
            },
            relation: CatalogId::new(id, CatalogKind::Table, ["db", "main", "items"]),
            relation_key: RelationKey {
                profile_id: id,
                object_id: CatalogId::new(id, CatalogKind::Table, ["db", "main", "items"]),
            },
            scope: CatalogScope::for_profile(DatabaseKind::Sqlite, "db", Some("main")),
            metadata: MetadataFingerprint {
                relation: "items".into(),
                columns: vec![
                    ("id".into(), "INTEGER".into(), false),
                    ("value".into(), "TEXT".into(), true),
                ],
                primary_key: vec!["id".into()],
            },
            operation: RelationMutation::UpdateCell(UpdateCellMutation {
                row: RowLocator {
                    columns: vec![0],
                    values: vec![CellValue::Integer(1)],
                },
                column: 1,
                original: CellValue::Text("old".into()),
                value: InputValue::Value(CellValue::Text("new".into())),
            }),
        };
        let (reply, result) = oneshot::channel();
        let (_, cancel) = oneshot::channel();
        worker
            .requests
            .send(TransactionRequest::RelationMutation {
                request,
                cancel,
                reply,
            })
            .unwrap();
        assert!(matches!(
            result.await.unwrap(),
            Ok(MutationResult::Updated { .. })
        ));
        let (reply, result) = oneshot::channel();
        worker
            .requests
            .send(TransactionRequest::Commit { reply })
            .unwrap();
        assert!(result.await.unwrap().is_ok());
        assert_eq!(
            *log.lock().unwrap(),
            vec!["begin", "relation_mutation", "commit"]
        );
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
        assert!(log.ends_with(&["cancel".into(), "rollback".into()]));
        assert!(
            log.iter().any(|entry| entry == "slow")
                || log
                    == vec![
                        String::from("begin"),
                        String::from("cancel"),
                        String::from("rollback"),
                    ]
        );
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
    async fn cancellation_of_a_closed_session_does_not_attempt_rollback() {
        let fake = Fake {
            cancel_closes: true,
            ..Fake::default()
        };
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
        assert_eq!(
            worker.worker.await.unwrap(),
            WorkerDisposition::CancelledAndRolledBack
        );
        assert!(!log.lock().unwrap().iter().any(|entry| entry == "rollback"));
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
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .any(|entry| entry == "force_close")
        );
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

    #[tokio::test]
    async fn resolved_last_page_rebuilds_page_sql_after_count() {
        let fake = Fake::default();
        let log = fake.log.clone();
        let worker = spawn_transaction_worker(fake);
        let (reply, result) = oneshot::channel();
        worker
            .requests
            .send(TransactionRequest::Page {
                source_sql: "SELECT * FROM items".into(),
                dialect: crate::sql::SqlDialect::Sqlite,
                count_sql: "count".into(),
                page: crate::model::pagination::PageRequest::last(
                    crate::model::pagination::PageSize::FiveHundred,
                    1,
                ),
                reply,
            })
            .unwrap();
        let _ = result.await.unwrap();
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 3);
        assert_eq!(log[1], "count");
        assert!(log[2].ends_with("LIMIT 501 OFFSET 1000"));
    }
}
