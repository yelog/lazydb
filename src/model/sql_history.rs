use uuid::Uuid;

/// The state of one SQL execution as observed by LazyDB.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryExecutionStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
    Interrupted,
    NotExecuted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryResultCertainty {
    Confirmed,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryTransactionOutcome {
    NotApplicable,
    Pending,
    AutoCommitted,
    Committed,
    RolledBack,
    RolledBackToSavepoint,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionHistory {
    pub execution_id: Uuid,
    pub operation_id: Uuid,
    pub transaction_id: Option<Uuid>,
    pub sql: String,
    pub status: HistoryExecutionStatus,
    pub certainty: HistoryResultCertainty,
    pub transaction_outcome: HistoryTransactionOutcome,
    pub affected_rows: Option<u64>,
    pub returned_rows: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HistoryEvent {
    Finished {
        status: HistoryExecutionStatus,
        certainty: HistoryResultCertainty,
        affected_rows: Option<u64>,
        returned_rows: Option<usize>,
    },
    TransactionResolved {
        outcome: HistoryTransactionOutcome,
    },
    ClearOutcome,
}

/// Pure projection of lifecycle facts onto the history row.
///
/// Runtime events can arrive out of order: for example a cancellation request
/// may race with a confirmed database completion. Once a confirmed terminal
/// result exists, a weaker late result must not replace it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryEventReducer {
    execution: ExecutionHistory,
}

impl HistoryEventReducer {
    pub fn new(execution: ExecutionHistory) -> Self {
        Self { execution }
    }

    pub fn execution(&self) -> &ExecutionHistory {
        &self.execution
    }

    pub fn apply(&mut self, event: HistoryEvent) {
        match event {
            HistoryEvent::Finished {
                status,
                certainty,
                affected_rows,
                returned_rows,
            } => {
                if !confirmed_terminal(&self.execution)
                    || certainty == HistoryResultCertainty::Confirmed
                {
                    self.execution.status = status;
                    self.execution.certainty = certainty;
                    self.execution.affected_rows = affected_rows;
                    self.execution.returned_rows = returned_rows;
                }
            }
            HistoryEvent::TransactionResolved { outcome } => {
                self.execution.transaction_outcome = outcome;
            }
            // Clearing the in-memory transaction prompt must not rewrite the
            // durable fact that a commit/rollback outcome was unknown.
            HistoryEvent::ClearOutcome => {}
        }
    }
}

fn confirmed_terminal(execution: &ExecutionHistory) -> bool {
    execution.certainty == HistoryResultCertainty::Confirmed
        && matches!(
            execution.status,
            HistoryExecutionStatus::Succeeded
                | HistoryExecutionStatus::Failed
                | HistoryExecutionStatus::TimedOut
                | HistoryExecutionStatus::Cancelled
                | HistoryExecutionStatus::Interrupted
                | HistoryExecutionStatus::NotExecuted
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmed_terminal_result_is_not_replaced_by_unknown_result() {
        let mut reducer = HistoryEventReducer::new(ExecutionHistory {
            execution_id: Uuid::nil(),
            operation_id: Uuid::nil(),
            transaction_id: None,
            sql: "select 1".into(),
            status: HistoryExecutionStatus::Succeeded,
            certainty: HistoryResultCertainty::Confirmed,
            transaction_outcome: HistoryTransactionOutcome::NotApplicable,
            affected_rows: None,
            returned_rows: Some(1),
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
    }
}
