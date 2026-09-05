use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::value::CellValue;

pub const RELATION_PREVIEW_LIMIT: usize = 500;

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
    pub fetched_row_count: usize,
    pub truncated: bool,
}

impl QueryStats {
    pub fn new(execution: Duration, fetch: Duration, row_count: usize) -> Self {
        Self {
            execution,
            fetch,
            row_count,
            fetched_row_count: row_count,
            truncated: false,
        }
    }

    pub fn total(&self) -> Duration {
        self.execution + self.fetch
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueryBudget {
    pub max_rows: usize,
    pub max_bytes: usize,
}

impl QueryBudget {
    pub const UNBOUNDED: Self = Self {
        max_rows: usize::MAX,
        max_bytes: usize::MAX,
    };
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueryOutcome {
    pub result_sets: Vec<ResultSet>,
    pub stats: QueryStats,
}

impl QueryOutcome {
    pub(crate) fn from_result_set(
        result_set: ResultSet,
        execution: Duration,
        fetch: Duration,
    ) -> Self {
        let row_count = result_set.rows.len();
        Self {
            result_sets: vec![result_set],
            stats: QueryStats::new(execution, fetch, row_count),
        }
    }
}

/// Collects SQLx stream events without knowing which database produced them.
pub(crate) struct QueryOutcomeAccumulator {
    started: Instant,
    first_event: Option<Duration>,
    result_sets: Vec<ResultSet>,
    current: Option<ResultSet>,
    budget: QueryBudget,
    retained_rows: usize,
    retained_bytes: usize,
    fetched_rows: usize,
    truncated: bool,
}

impl QueryOutcomeAccumulator {
    pub(crate) fn with_budget(budget: QueryBudget) -> Self {
        Self {
            started: Instant::now(),
            first_event: None,
            result_sets: Vec::new(),
            current: None,
            budget,
            retained_rows: 0,
            retained_bytes: 0,
            fetched_rows: 0,
            truncated: false,
        }
    }

    pub(crate) fn row(&mut self, columns: Vec<ColumnMeta>, row: Vec<CellValue>) {
        self.mark_event();
        self.fetched_rows = self.fetched_rows.saturating_add(1);
        self.current.get_or_insert_with(|| ResultSet {
            columns,
            rows: Vec::new(),
            affected_rows: 0,
        });
        if self.truncated {
            return;
        }
        let row_bytes = if self.budget.max_bytes == usize::MAX {
            0
        } else {
            row.iter()
                .map(|cell| serde_json::to_vec(cell).map_or(0, |value| value.len()))
                .fold(0usize, usize::saturating_add)
        };
        let retain = self.retained_rows < self.budget.max_rows
            && (self.budget.max_bytes == usize::MAX
                || self.retained_bytes.saturating_add(row_bytes) <= self.budget.max_bytes);
        if !retain {
            self.truncated = true;
            return;
        }
        self.retained_rows = self.retained_rows.saturating_add(1);
        self.retained_bytes = self.retained_bytes.saturating_add(row_bytes);
        self.current
            .as_mut()
            .expect("result metadata was initialized above")
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
            stats: QueryStats {
                execution,
                fetch: total.saturating_sub(execution),
                row_count,
                fetched_row_count: self.fetched_rows,
                truncated: self.truncated,
            },
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

    use super::{QueryBudget, QueryOutcomeAccumulator, QueryStats};

    #[test]
    fn tracks_execution_and_fetch_separately() {
        let stats = QueryStats::new(Duration::from_millis(24), Duration::from_millis(352), 1);

        assert_eq!(stats.execution, Duration::from_millis(24));
        assert_eq!(stats.fetch, Duration::from_millis(352));
        assert_eq!(stats.total(), Duration::from_millis(376));
        assert_eq!(stats.row_count, 1);
        assert_eq!(stats.fetched_row_count, 1);
        assert!(!stats.truncated);
    }

    #[test]
    fn accumulator_retains_a_shared_row_budget_but_consumes_all_events() {
        let mut accumulator = QueryOutcomeAccumulator::with_budget(QueryBudget {
            max_rows: 2,
            max_bytes: usize::MAX,
        });
        let columns = vec![super::ColumnMeta {
            name: "value".into(),
            type_name: "TEXT".into(),
        }];
        for value in ["one", "two", "three"] {
            accumulator.row(columns.clone(), vec![super::CellValue::Text(value.into())]);
        }
        accumulator.done(0);
        let outcome = accumulator.finish();

        assert_eq!(outcome.result_sets[0].rows.len(), 2);
        assert_eq!(outcome.stats.row_count, 2);
        assert_eq!(outcome.stats.fetched_row_count, 3);
        assert!(outcome.stats.truncated);
    }

    #[test]
    fn accumulator_applies_byte_budget_to_a_contiguous_prefix() {
        let mut accumulator = QueryOutcomeAccumulator::with_budget(QueryBudget {
            max_rows: usize::MAX,
            max_bytes: 25,
        });
        let columns = vec![super::ColumnMeta {
            name: "value".into(),
            type_name: "TEXT".into(),
        }];
        accumulator.row(
            columns.clone(),
            vec![super::CellValue::Text("12345".into())],
        );
        accumulator.row(columns, vec![super::CellValue::Text("67890".into())]);
        let outcome = accumulator.finish();

        assert_eq!(outcome.result_sets[0].rows.len(), 1);
        assert_eq!(outcome.stats.fetched_row_count, 2);
        assert!(outcome.stats.truncated);
    }

    #[test]
    fn byte_budget_exhaustion_does_not_resume_in_later_results() {
        let mut accumulator = QueryOutcomeAccumulator::with_budget(QueryBudget {
            max_rows: 10,
            max_bytes: 25,
        });
        let columns = vec![super::ColumnMeta {
            name: "value".into(),
            type_name: "TEXT".into(),
        }];
        accumulator.row(
            columns.clone(),
            vec![super::CellValue::Text("x".repeat(100))],
        );
        accumulator.done(7);
        accumulator.row(columns.clone(), vec![super::CellValue::Null]);
        accumulator.done(3);
        let outcome = accumulator.finish();
        assert_eq!(outcome.result_sets.len(), 2);
        assert_eq!(outcome.result_sets[0].columns, columns);
        assert!(
            outcome
                .result_sets
                .iter()
                .all(|result| result.rows.is_empty())
        );
        assert_eq!(outcome.result_sets[0].affected_rows, 7);
        assert_eq!(outcome.result_sets[1].affected_rows, 3);
        assert_eq!(outcome.stats.fetched_row_count, 2);
        assert!(outcome.stats.truncated);
    }
}
