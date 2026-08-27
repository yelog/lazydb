use uuid::Uuid;

use crate::db::query::QueryOutcome;
use crate::sql::CompletionCandidate;

use super::execution_target::ExecutionTarget;
use super::workspace::QueryStatus;
use super::{transaction::TransactionMode, transaction::TransactionState};
use crate::sql::ExecutionDraft;

use super::data_query::DataQueryOptions;
use super::data_query::DataQueryState;
use super::relation::RelationTab;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TabKind {
    Sql,
    Relation,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DataGridState {
    pub selected_row: usize,
    pub selected_column: usize,
    pub column_widths: Vec<Option<u16>>,
}

/// Compatibility name for the shared grid state used by the current renderer.
pub type GridState = DataGridState;

#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum WorkspaceTab {
    Sql(ConsoleTab),
    Relation(RelationTab),
}

impl WorkspaceTab {
    pub fn id(&self) -> Uuid {
        match self {
            Self::Sql(tab) => tab.id,
            Self::Relation(tab) => tab.id,
        }
    }

    pub fn title(&self) -> &str {
        match self {
            Self::Sql(tab) => &tab.name,
            Self::Relation(tab) => tab.title(),
        }
    }

    pub fn kind(&self) -> TabKind {
        match self {
            Self::Sql(_) => TabKind::Sql,
            Self::Relation(_) => TabKind::Relation,
        }
    }

    pub fn as_console(&self) -> Option<&ConsoleTab> {
        match self {
            Self::Sql(tab) => Some(tab),
            Self::Relation(_) => None,
        }
    }

    pub fn as_console_mut(&mut self) -> Option<&mut ConsoleTab> {
        match self {
            Self::Sql(tab) => Some(tab),
            Self::Relation(_) => None,
        }
    }
}

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
    pub grid: DataGridState,
    pub completion: Option<CompletionPopup>,
    pub transaction_generation: u64,
    pub transaction_mode: TransactionMode,
    pub transaction_state: TransactionState,
    pub last_execution: Option<LastExecution>,
    pub execution_target: Option<ExecutionTarget>,
    pub query: DataQueryState,
    pub derived: Option<DerivedResultState>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DerivedResultState {
    pub source: LastExecution,
    pub query: DataQueryOptions,
    pub generation: u64,
    pub outcome: Option<QueryOutcome>,
    pub error: Option<String>,
    pub running: bool,
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
            grid: DataGridState::default(),
            completion: None,
            transaction_generation: 0,
            transaction_mode: TransactionMode::Auto,
            transaction_state: TransactionState::Idle,
            last_execution: None,
            execution_target: None,
            query: DataQueryState::default(),
            derived: None,
        }
    }
}

impl DataGridState {
    pub fn clamp(&mut self, row_count: usize, column_count: usize) {
        self.selected_row = self.selected_row.min(row_count.saturating_sub(1));
        self.selected_column = self.selected_column.min(column_count.saturating_sub(1));
        self.column_widths.truncate(column_count);
    }
}

#[cfg(test)]
mod tests {
    use super::DataGridState;

    #[test]
    fn clamping_keeps_selection_and_widths_inside_result_dimensions() {
        let mut state = DataGridState {
            selected_row: 9,
            selected_column: 8,
            column_widths: vec![Some(10), Some(11), Some(12), Some(13)],
        };

        state.clamp(2, 3);

        assert_eq!(state.selected_row, 1);
        assert_eq!(state.selected_column, 2);
        assert_eq!(state.column_widths, vec![Some(10), Some(11), Some(12)]);
    }

    #[test]
    fn clamping_empty_dimensions_resets_selection() {
        let mut state = DataGridState {
            selected_row: 3,
            selected_column: 4,
            column_widths: vec![Some(10)],
        };

        state.clamp(0, 0);

        assert_eq!(state.selected_row, 0);
        assert_eq!(state.selected_column, 0);
        assert!(state.column_widths.is_empty());
    }
}
