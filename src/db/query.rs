use std::time::{Duration, Instant};

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

/// Collects SQLx stream events without knowing which database produced them.
pub(crate) struct QueryOutcomeAccumulator {
    started: Instant,
    first_event: Option<Duration>,
    result_sets: Vec<ResultSet>,
    current: Option<ResultSet>,
}

impl QueryOutcomeAccumulator {
    pub(crate) fn new() -> Self {
        Self {
            started: Instant::now(),
            first_event: None,
            result_sets: Vec::new(),
            current: None,
        }
    }

    pub(crate) fn row(&mut self, columns: Vec<ColumnMeta>, row: Vec<CellValue>) {
        self.mark_event();
        self.current
            .get_or_insert_with(|| ResultSet {
                columns,
                rows: Vec::new(),
                affected_rows: 0,
            })
            .rows
            .push(row);
    }

    pub(crate) fn done(&mut self, affected_rows: u64) {
        self.mark_event();
        let mut result = self.current.take().unwrap_or_default();
        result.affected_rows = affected_rows;
        self.result_sets.push(result);
    }

    pub(crate) fn finish(mut self) -> QueryOutcome {
        if let Some(result) = self.current.take() {
            self.result_sets.push(result);
        }
        let total = self.started.elapsed();
        let execution = self.first_event.unwrap_or(total);
        let row_count = self
            .result_sets
            .iter()
            .map(|result| result.rows.len())
            .sum();
        QueryOutcome {
            result_sets: self.result_sets,
            stats: QueryStats::new(execution, total.saturating_sub(execution), row_count),
        }
    }

    fn mark_event(&mut self) {
        self.first_event
            .get_or_insert_with(|| self.started.elapsed());
    }
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
