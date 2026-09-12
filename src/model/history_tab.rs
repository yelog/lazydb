use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryTab {
    pub id: Uuid,
    pub selected_execution: Option<Uuid>,
    pub list_offset: usize,
    pub query_generation: u64,
    pub items: Vec<crate::model::sql_history::ExecutionHistory>,
    pub loading: bool,
}

impl Default for HistoryTab {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            selected_execution: None,
            list_offset: 0,
            query_generation: 0,
            items: Vec::new(),
            loading: false,
        }
    }
}

impl HistoryTab {
    pub const TITLE: &'static str = "SQL History";
}
