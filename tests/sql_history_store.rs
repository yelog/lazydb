use lazydb::{
    model::sql_history::{
        ExecutionHistory, HistoryExecutionStatus, HistoryResultCertainty, HistoryTransactionOutcome,
    },
    persistence::sql_history::{HistoryPageRequest, HistoryStore},
};
use tempfile::TempDir;
use uuid::Uuid;

fn history(id: Uuid, sql: &str) -> ExecutionHistory {
    ExecutionHistory {
        execution_id: id,
        operation_id: Uuid::new_v4(),
        transaction_id: None,
        sql: sql.into(),
        status: HistoryExecutionStatus::Running,
        certainty: HistoryResultCertainty::Confirmed,
        transaction_outcome: HistoryTransactionOutcome::NotApplicable,
        affected_rows: None,
        returned_rows: None,
    }
}

#[tokio::test]
async fn store_round_trips_full_sql_and_updates_completion_idempotently() {
    let temp = TempDir::new().unwrap();
    let store = HistoryStore::open(temp.path().join("history.sqlite3"))
        .await
        .unwrap();
    let id = Uuid::new_v4();
    store
        .insert(history(id, "SELECT '完整的 SQL';"))
        .await
        .unwrap();
    store
        .finish(
            id,
            HistoryExecutionStatus::Succeeded,
            HistoryResultCertainty::Confirmed,
            Some(0),
            Some(1),
        )
        .await
        .unwrap();
    store
        .finish(
            id,
            HistoryExecutionStatus::Succeeded,
            HistoryResultCertainty::Confirmed,
            Some(0),
            Some(1),
        )
        .await
        .unwrap();

    let detail = store.detail(id).await.unwrap().unwrap();
    assert_eq!(detail.sql, "SELECT '完整的 SQL';");
    assert_eq!(detail.status, HistoryExecutionStatus::Succeeded);
    assert_eq!(detail.affected_rows, Some(0));
    assert_eq!(detail.returned_rows, Some(1));
}

#[tokio::test]
async fn page_uses_a_stable_cursor_when_timestamps_are_equal() {
    let temp = TempDir::new().unwrap();
    let store = HistoryStore::open(temp.path().join("history.sqlite3"))
        .await
        .unwrap();
    for sql in ["one", "two", "three"] {
        store.insert(history(Uuid::new_v4(), sql)).await.unwrap();
    }

    let first = store
        .page(HistoryPageRequest {
            limit: 2,
            cursor: None,
            search: None,
        })
        .await
        .unwrap();
    assert_eq!(first.items.len(), 2);
    assert!(first.next_cursor.is_some());

    let second = store
        .page(HistoryPageRequest {
            limit: 2,
            cursor: first.next_cursor,
            search: None,
        })
        .await
        .unwrap();
    assert_eq!(second.items.len(), 1);
    assert!(first.items.iter().all(|item| {
        second
            .items
            .iter()
            .all(|other| other.execution_id != item.execution_id)
    }));
}
