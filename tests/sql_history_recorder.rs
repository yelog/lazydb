use lazydb::{
    history::{HistoryRecorder, HistoryRecorderError},
    model::sql_history::{
        ExecutionHistory, HistoryExecutionStatus, HistoryResultCertainty, HistoryTransactionOutcome,
    },
    persistence::sql_history::HistoryStore,
};
use tempfile::TempDir;
use uuid::Uuid;

fn execution(id: Uuid) -> ExecutionHistory {
    ExecutionHistory {
        execution_id: id,
        operation_id: Uuid::new_v4(),
        transaction_id: Some(Uuid::new_v4()),
        sql: "UPDATE items SET active = true".into(),
        status: HistoryExecutionStatus::Running,
        certainty: HistoryResultCertainty::Confirmed,
        transaction_outcome: HistoryTransactionOutcome::Pending,
        affected_rows: None,
        returned_rows: None,
    }
}

#[tokio::test]
async fn recorder_flushes_concurrent_events_and_keeps_completion_idempotent() {
    let temp = TempDir::new().unwrap();
    let store = HistoryStore::open(temp.path().join("history.sqlite3"))
        .await
        .unwrap();
    let recorder = HistoryRecorder::new(store.clone(), 8);
    let id = Uuid::new_v4();
    let transaction_id = execution(id).transaction_id.unwrap();
    let mut entry = execution(id);
    entry.transaction_id = Some(transaction_id);
    recorder.start(entry).await.unwrap();
    recorder
        .resolve_transaction(transaction_id, HistoryTransactionOutcome::RolledBack)
        .await
        .unwrap();

    let first = recorder.clone();
    let second = recorder.clone();
    let (first_result, second_result) = tokio::join!(
        first.finish(id, HistoryExecutionStatus::Succeeded, Some(2), None),
        second.finish(id, HistoryExecutionStatus::Succeeded, Some(2), None),
    );
    first_result.unwrap();
    second_result.unwrap();
    recorder.flush().await.unwrap();

    let saved = store.detail(id).await.unwrap().unwrap();
    assert_eq!(saved.status, HistoryExecutionStatus::Succeeded);
    assert_eq!(saved.affected_rows, Some(2));
    assert_eq!(
        saved.transaction_outcome,
        HistoryTransactionOutcome::RolledBack
    );
}

#[tokio::test]
async fn recorder_reports_shutdown_before_accepting_new_events() {
    let temp = TempDir::new().unwrap();
    let store = HistoryStore::open(temp.path().join("history.sqlite3"))
        .await
        .unwrap();
    let recorder = HistoryRecorder::new(store, 1);
    recorder.shutdown().await.unwrap();

    let error = recorder.start(execution(Uuid::new_v4())).await.unwrap_err();
    assert!(matches!(error, HistoryRecorderError::Closed));
}
