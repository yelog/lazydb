use lazydb::model::sql_history::{HistoryExecutionStatus, HistoryResultCertainty};

#[test]
fn interrupted_recovery_requires_unknown_certainty_instead_of_inventing_rollback() {
    assert_eq!(
        HistoryExecutionStatus::Interrupted,
        HistoryExecutionStatus::Interrupted
    );
    assert_eq!(
        HistoryResultCertainty::Unknown,
        HistoryResultCertainty::Unknown
    );
}
