use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::value::CellValue;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ColumnMeta {
    pub name: String,
    pub type_name: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ResultSet {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<CellValue>>,
    pub affected_rows: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueryStats {
    pub execution: Duration,
    pub fetch: Duration,
    pub row_count: usize,
}

impl QueryStats {
    pub fn new(execution: Duration, fetch: Duration, row_count: usize) -> Self {
        Self {
            execution,
            fetch,
            row_count,
        }
    }

    pub fn total(&self) -> Duration {
        self.execution + self.fetch
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueryOutcome {
    pub result_sets: Vec<ResultSet>,
    pub stats: QueryStats,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::QueryStats;

    #[test]
    fn tracks_execution_and_fetch_separately() {
        let stats = QueryStats::new(Duration::from_millis(24), Duration::from_millis(352), 1);

        assert_eq!(stats.execution, Duration::from_millis(24));
        assert_eq!(stats.fetch, Duration::from_millis(352));
        assert_eq!(stats.total(), Duration::from_millis(376));
        assert_eq!(stats.row_count, 1);
    }
}
