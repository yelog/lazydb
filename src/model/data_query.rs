use crate::model::text_input::TextInput;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataQueryCandidate {
    pub name: String,
    pub type_name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataQueryCompletion {
    pub candidates: Vec<DataQueryCandidate>,
    pub selected: usize,
    pub replace: crate::sql::TextRange,
}

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct DataQueryOptions {
    pub where_clause: Option<String>,
    pub order_by_clause: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataQueryInput {
    Where,
    OrderBy,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DataQueryState {
    pub where_input: TextInput,
    pub order_by_input: TextInput,
    pub submitted: DataQueryOptions,
    pub focus: Option<DataQueryInput>,
    pub error: Option<String>,
    pub capability: DataQueryCapability,
    pub completion: Option<DataQueryCompletion>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataQueryCapability {
    Relation,
    Sql,
    Unavailable(String),
}

impl Default for DataQueryCapability {
    fn default() -> Self {
        Self::Unavailable("SQL result filtering is not implemented yet".into())
    }
}
