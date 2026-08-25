use uuid::Uuid;

use crate::db::query::QueryOutcome;
use crate::sql::CompletionCandidate;

use super::workspace::QueryStatus;
use super::{transaction::TransactionMode, transaction::TransactionState};
use crate::sql::ExecutionDraft;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ResultView {
    #[default]
    Data,
    Output,
    Plan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputKind {
    Info,
    Success,
    Error,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputEntry {
    pub kind: OutputKind,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConsoleTab {
    pub id: Uuid,
    pub name: String,
    pub generation: u64,
    pub query_status: QueryStatus,
    pub outcome: Option<QueryOutcome>,
    pub output: Vec<OutputEntry>,
    pub result_view: ResultView,
    pub selected_row: usize,
    pub selected_column: usize,
    pub completion: Option<CompletionPopup>,
    pub transaction_generation: u64,
    pub transaction_mode: TransactionMode,
    pub transaction_state: TransactionState,
    pub last_execution: Option<LastExecution>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionResult {
    Dispatched,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LastExecution {
    pub draft: ExecutionDraft,
    pub result: ExecutionResult,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionPopup {
    pub candidates: Vec<CompletionCandidate>,
    pub selected: usize,
}

impl ConsoleTab {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            generation: 0,
            query_status: QueryStatus::Idle,
            outcome: None,
            output: Vec::new(),
            result_view: ResultView::Data,
            selected_row: 0,
            selected_column: 0,
            completion: None,
            transaction_generation: 0,
            transaction_mode: TransactionMode::Auto,
            transaction_state: TransactionState::Idle,
            last_execution: None,
        }
    }
}
