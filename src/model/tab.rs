use uuid::Uuid;

use crate::db::query::QueryOutcome;

use super::{editor::EditorBuffer, workspace::QueryStatus};

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
    pub editor: EditorBuffer,
    pub generation: u64,
    pub query_status: QueryStatus,
    pub outcome: Option<QueryOutcome>,
    pub output: Vec<OutputEntry>,
    pub result_view: ResultView,
    pub selected_row: usize,
    pub selected_column: usize,
}

impl ConsoleTab {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            editor: EditorBuffer::default(),
            generation: 0,
            query_status: QueryStatus::Idle,
            outcome: None,
            output: Vec::new(),
            result_view: ResultView::Data,
            selected_row: 0,
            selected_column: 0,
        }
    }
}
