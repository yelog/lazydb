use lazydb::model::sql_history::{
    ExecutionHistory, HistoryEvent, HistoryEventReducer, HistoryExecutionStatus,
    HistoryResultCertainty, HistoryTransactionOutcome,
};
use uuid::Uuid;

fn execution() -> ExecutionHistory {
    ExecutionHistory {
        execution_id: Uuid::new_v4(),
        operation_id: Uuid::new_v4(),
        transaction_id: Some(Uuid::new_v4()),
        sql: "UPDATE users SET enabled = false".into(),
        status: HistoryExecutionStatus::Running,
        certainty: HistoryResultCertainty::Confirmed,
        transaction_outcome: HistoryTransactionOutcome::Pending,
        affected_rows: None,
        returned_rows: None,
        requested_at: 0,
    }
}

#[test]
fn execution_success_can_later_be_marked_rolled_back() {
    let mut reducer = HistoryEventReducer::new(execution());

    reducer.apply(HistoryEvent::Finished {
        status: HistoryExecutionStatus::Succeeded,
        certainty: HistoryResultCertainty::Confirmed,
        affected_rows: Some(3),
        returned_rows: None,
    });
    reducer.apply(HistoryEvent::TransactionResolved {
        outcome: HistoryTransactionOutcome::RolledBack,
    });

    assert_eq!(
        reducer.execution().status,
        HistoryExecutionStatus::Succeeded
    );
    assert_eq!(reducer.execution().affected_rows, Some(3));
    assert_eq!(
        reducer.execution().transaction_outcome,
        HistoryTransactionOutcome::RolledBack
    );
}

#[test]
fn late_cancellation_does_not_override_confirmed_success() {
    let mut reducer = HistoryEventReducer::new(execution());

    reducer.apply(HistoryEvent::Finished {
        status: HistoryExecutionStatus::Succeeded,
        certainty: HistoryResultCertainty::Confirmed,
        affected_rows: Some(0),
        returned_rows: Some(0),
    });
    reducer.apply(HistoryEvent::Finished {
        status: HistoryExecutionStatus::Cancelled,
        certainty: HistoryResultCertainty::Unknown,
        affected_rows: None,
        returned_rows: None,
    });

    assert_eq!(
        reducer.execution().status,
        HistoryExecutionStatus::Succeeded
    );
    assert_eq!(reducer.execution().affected_rows, Some(0));
}

#[test]
fn unknown_row_count_is_not_zero() {
    let history = execution();
    assert_eq!(history.affected_rows, None);
    assert_eq!(history.returned_rows, None);
}

#[test]
fn unknown_transaction_outcome_is_not_changed_by_clear_outcome() {
    let mut reducer = HistoryEventReducer::new(execution());
    reducer.apply(HistoryEvent::TransactionResolved {
        outcome: HistoryTransactionOutcome::Unknown,
    });
    reducer.apply(HistoryEvent::ClearOutcome);

    assert_eq!(
        reducer.execution().transaction_outcome,
        HistoryTransactionOutcome::Unknown
    );
}

#[test]
fn transaction_resolution_can_be_projected_after_statement_completion() {
    let transaction_id = Uuid::new_v4();
    let mut reducer = HistoryEventReducer::new(ExecutionHistory {
        transaction_id: Some(transaction_id),
        ..execution()
    });
    reducer.apply(HistoryEvent::Finished {
        status: HistoryExecutionStatus::Succeeded,
        certainty: HistoryResultCertainty::Confirmed,
        affected_rows: Some(4),
        returned_rows: None,
    });
    reducer.apply(HistoryEvent::TransactionResolved {
        outcome: HistoryTransactionOutcome::Committed,
    });
    assert_eq!(reducer.execution().transaction_id, Some(transaction_id));
    assert_eq!(
        reducer.execution().transaction_outcome,
        HistoryTransactionOutcome::Committed
    );
}
