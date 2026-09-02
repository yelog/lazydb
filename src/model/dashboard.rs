use std::{collections::BTreeMap, time::Duration};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DEFAULT_HISTORY: Duration = Duration::from_secs(10 * 60);
pub const MAX_HISTORY_SAMPLES: usize = 3_600;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MetricKey {
    Transactions,
    Commits,
    Rollbacks,
    Queries,
    Selects,
    Inserts,
    Updates,
    Deletes,
    BlockHits,
    BlockReads,
    Connections,
    ActiveConnections,
    IdleConnections,
    CacheHitRatio,
    Deadlocks,
    TempFiles,
    TempBytes,
    BytesRead,
    BytesWritten,
    WalBytes,
    AbortedClients,
    AbortedConnections,
    ServerUptime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricKind {
    Gauge,
    Counter,
    Ratio,
}

impl MetricKey {
    pub const fn kind(self) -> MetricKind {
        match self {
            Self::CacheHitRatio => MetricKind::Ratio,
            Self::Connections
            | Self::ActiveConnections
            | Self::IdleConnections
            | Self::ServerUptime => MetricKind::Gauge,
            _ => MetricKind::Counter,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawSample {
    pub at_millis: u64,
    pub server_generation: u64,
    pub values: BTreeMap<MetricKey, f64>,
}

impl RawSample {
    pub fn new(at_millis: u64, server_generation: u64) -> Self {
        Self {
            at_millis,
            server_generation,
            values: BTreeMap::new(),
        }
    }

    pub fn with(mut self, key: MetricKey, value: f64) -> Self {
        self.values.insert(key, value);
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetricPoint {
    pub at_millis: u64,
    pub value: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetricHistory {
    samples: Vec<RawSample>,
    points: BTreeMap<MetricKey, Vec<MetricPoint>>,
}

impl MetricHistory {
    pub fn samples(&self) -> &[RawSample] {
        &self.samples
    }

    pub fn points(&self, key: MetricKey) -> &[MetricPoint] {
        self.points.get(&key).map_or(&[], Vec::as_slice)
    }

    pub fn push(&mut self, sample: RawSample) {
        let previous = self.samples.last();
        let generation_changed = previous.is_some_and(|previous| {
            previous.server_generation != sample.server_generation
                || previous
                    .values
                    .get(&MetricKey::ServerUptime)
                    .zip(sample.values.get(&MetricKey::ServerUptime))
                    .is_some_and(|(previous, current)| current < previous)
        });

        let mut keys = self.points.keys().copied().collect::<Vec<_>>();
        keys.extend(sample.values.keys().copied());
        keys.sort_unstable();
        keys.dedup();

        for key in keys {
            let value = previous.and_then(|previous| {
                (!generation_changed)
                    .then(|| rate(previous, &sample, key))
                    .flatten()
            });
            let points = self.points.entry(key).or_default();
            for sample in self.samples.iter().skip(points.len()) {
                points.push(MetricPoint {
                    at_millis: sample.at_millis,
                    value: None,
                });
            }
            points.push(MetricPoint {
                at_millis: sample.at_millis,
                value,
            });
        }

        self.samples.push(sample);
        self.prune();
    }

    pub fn reset(&mut self) {
        self.samples.clear();
        self.points.clear();
    }

    fn prune(&mut self) {
        let Some(latest) = self.samples.last().map(|sample| sample.at_millis) else {
            return;
        };
        let threshold = latest.saturating_sub(DEFAULT_HISTORY.as_millis() as u64);
        let remove_count = self
            .samples
            .partition_point(|sample| sample.at_millis < threshold);
        if remove_count > 0 {
            self.samples.drain(..remove_count);
            for points in self.points.values_mut() {
                points.drain(..remove_count.min(points.len()));
            }
        }
        if self.samples.len() > MAX_HISTORY_SAMPLES {
            let remove_count = self.samples.len() - MAX_HISTORY_SAMPLES;
            self.samples.drain(..remove_count);
            for points in self.points.values_mut() {
                points.drain(..remove_count.min(points.len()));
            }
        }
    }
}

fn rate(previous: &RawSample, current: &RawSample, key: MetricKey) -> Option<f64> {
    let elapsed = current.at_millis.checked_sub(previous.at_millis)? as f64 / 1_000.0;
    if elapsed <= 0.0 {
        return None;
    }
    if key == MetricKey::CacheHitRatio {
        let current_hits = *current.values.get(&MetricKey::BlockHits)?;
        let previous_hits = *previous.values.get(&MetricKey::BlockHits)?;
        let current_reads = *current.values.get(&MetricKey::BlockReads)?;
        let previous_reads = *previous.values.get(&MetricKey::BlockReads)?;
        let hits = current_hits - previous_hits;
        let reads = current_reads - previous_reads;
        if hits < 0.0 || reads < 0.0 || hits + reads <= 0.0 {
            return None;
        }
        return Some(hits / (hits + reads) * 100.0);
    }
    let current_value = *current.values.get(&key)?;
    let previous_value = *previous.values.get(&key)?;
    if !current_value.is_finite() || !previous_value.is_finite() || current_value < previous_value {
        return None;
    }
    match key.kind() {
        MetricKind::Counter => Some((current_value - previous_value) / elapsed),
        MetricKind::Gauge | MetricKind::Ratio => Some(current_value),
    }
}

pub fn downsample_series(points: &[MetricPoint], max_points: usize) -> Vec<(f64, f64)> {
    if max_points == 0 || points.is_empty() {
        return Vec::new();
    }
    let valid = points.iter().filter_map(|point| {
        point
            .value
            .filter(|value| value.is_finite())
            .map(|value| (point.at_millis as f64 / 1_000.0, value))
    });
    let values = valid.collect::<Vec<_>>();
    if values.len() <= max_points {
        return values;
    }

    let bucket_count = (max_points / 2).max(1);
    let bucket_size = values.len().div_ceil(bucket_count);
    let mut result = Vec::with_capacity(max_points);
    for bucket in values.chunks(bucket_size) {
        let min = bucket
            .iter()
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .copied();
        let max = bucket
            .iter()
            .max_by(|left, right| left.1.total_cmp(&right.1))
            .copied();
        if let (Some(min), Some(max)) = (min, max) {
            if min.0 <= max.0 {
                result.extend([min, max]);
            } else {
                result.extend([max, min]);
            }
        }
    }
    result.truncate(max_points);
    result
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DashboardPage {
    #[default]
    Overview,
    Processes,
    Charts,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DashboardVisibility {
    Full,
    Restricted,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessRow {
    pub id: u64,
    pub user: String,
    pub database: Option<String>,
    pub client: Option<String>,
    pub application: Option<String>,
    pub state: Option<String>,
    pub wait: Option<String>,
    pub elapsed: Duration,
    pub query: Option<String>,
}

pub const PROCESS_COLUMN_COUNT: usize = 9;

fn optional_text(value: Option<&str>) -> crate::db::value::CellValue {
    value.map_or(crate::db::value::CellValue::Null, |value| {
        crate::db::value::CellValue::Text(value.to_owned())
    })
}

pub fn format_process_elapsed(elapsed: Duration) -> String {
    if elapsed < Duration::from_secs(1) {
        return format!("{}ms", elapsed.as_millis());
    }
    if elapsed < Duration::from_secs(60) {
        return format!("{:.3}s", elapsed.as_secs_f64());
    }
    let total = elapsed.as_secs();
    let seconds = total % 60;
    let minutes = (total / 60) % 60;
    let hours = total / 3_600;
    if hours > 0 {
        format!("{hours}h {minutes:02}m {seconds:02}s")
    } else {
        format!("{minutes}m {seconds:02}s")
    }
}

pub fn process_result_set(rows: &[ProcessRow], needle: &str) -> crate::db::query::ResultSet {
    use crate::db::{query::ColumnMeta, value::CellValue};

    let columns = [
        ("PID", "UNSIGNED"),
        ("USER", "TEXT"),
        ("DB", "TEXT"),
        ("CLIENT", "TEXT"),
        ("APPLICATION", "TEXT"),
        ("STATE", "TEXT"),
        ("WAIT", "TEXT"),
        ("ELAPSED", "DURATION"),
        ("QUERY", "TEXT"),
    ]
    .into_iter()
    .map(|(name, type_name)| ColumnMeta {
        name: name.into(),
        type_name: type_name.into(),
    })
    .collect();
    let rows = rows
        .iter()
        .filter(|row| process_matches(row, needle))
        .map(|row| {
            vec![
                CellValue::Unsigned(row.id),
                CellValue::Text(row.user.clone()),
                optional_text(row.database.as_deref()),
                optional_text(row.client.as_deref()),
                optional_text(row.application.as_deref()),
                optional_text(row.state.as_deref()),
                optional_text(row.wait.as_deref()),
                CellValue::Text(format_process_elapsed(row.elapsed)),
                optional_text(row.query.as_deref()),
            ]
        })
        .collect();
    crate::db::query::ResultSet {
        columns,
        rows,
        affected_rows: 0,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DashboardTab {
    pub id: Uuid,
    pub generation: u64,
    pub page: DashboardPage,
    pub refresh_enabled: bool,
    pub include_idle: bool,
    pub process_filter: String,
    pub process_filter_active: bool,
    pub process_filter_draft: Option<crate::model::text_input::TextInput>,
    pub history: MetricHistory,
    pub latest: Option<RawSample>,
    pub metadata: crate::db::monitor::MonitorMetadata,
    pub processes: Vec<ProcessRow>,
    pub process_truncated: bool,
    pub visibility: crate::db::monitor::MonitorVisibility,
    pub error: Option<String>,
    pub metadata_error: Option<String>,
    pub process_error: Option<String>,
    pub last_refresh_millis: Option<u64>,
    pub next_refresh_millis: u64,
    pub loading: bool,
    pub process_loading: bool,
    pub grid: crate::model::tab::DataGridState,
}

impl DashboardTab {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            generation: 0,
            page: DashboardPage::Overview,
            refresh_enabled: true,
            include_idle: false,
            process_filter: String::new(),
            process_filter_active: false,
            process_filter_draft: None,
            history: MetricHistory::default(),
            latest: None,
            metadata: Default::default(),
            processes: Vec::new(),
            process_truncated: false,
            visibility: Default::default(),
            error: None,
            metadata_error: None,
            process_error: None,
            last_refresh_millis: None,
            next_refresh_millis: 0,
            loading: false,
            process_loading: false,
            grid: Default::default(),
        }
    }

    pub fn process_result_set(&self) -> crate::db::query::ResultSet {
        process_result_set(&self.processes, self.effective_process_filter())
    }

    pub fn effective_process_filter(&self) -> &str {
        self.process_filter_draft
            .as_ref()
            .map_or(self.process_filter.as_str(), |draft| draft.value())
    }

    pub fn reconcile_process_grid(&mut self) {
        let row_count = self.process_result_set().rows.len();
        self.grid.clamp(row_count, PROCESS_COLUMN_COUNT);
        self.grid.ensure_row_visible(row_count);
    }
}

impl DashboardPage {
    pub const fn index(self) -> usize {
        match self {
            Self::Overview => 0,
            Self::Processes => 1,
            Self::Charts => 2,
        }
    }
}

pub fn process_matches(row: &ProcessRow, needle: &str) -> bool {
    let needle = needle.trim().to_ascii_lowercase();
    needle.is_empty()
        || [
            row.id.to_string(),
            row.user.clone(),
            row.database.clone().unwrap_or_default(),
            row.client.clone().unwrap_or_default(),
            row.application.clone().unwrap_or_default(),
            row.state.clone().unwrap_or_default(),
            row.wait.clone().unwrap_or_default(),
            row.query.clone().unwrap_or_default(),
        ]
        .iter()
        .any(|value| value.to_ascii_lowercase().contains(&needle))
}

impl Default for DashboardTab {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn sample(at_millis: u64, commits: f64) -> RawSample {
        RawSample::new(at_millis, 1)
            .with(MetricKey::Commits, commits)
            .with(MetricKey::Connections, 4.0)
    }

    #[test]
    fn first_sample_only_establishes_a_baseline() {
        let mut history = MetricHistory::default();
        history.push(sample(1_000, 10.0));

        assert_eq!(history.points(MetricKey::Commits)[0].value, None);
        assert_eq!(history.points(MetricKey::Connections)[0].value, None);
    }

    #[test]
    fn counters_use_actual_elapsed_time_and_gauges_use_current_value() {
        let mut history = MetricHistory::default();
        history.push(sample(1_000, 10.0));
        history.push(sample(3_000, 16.0));

        assert_eq!(history.points(MetricKey::Commits)[1].value, Some(3.0));
        assert_eq!(history.points(MetricKey::Connections)[1].value, Some(4.0));
    }

    #[test]
    fn uptime_is_a_gauge_and_write_bytes_are_a_rate_counter() {
        assert_eq!(MetricKey::ServerUptime.kind(), MetricKind::Gauge);
        assert_eq!(MetricKey::WalBytes.kind(), MetricKind::Counter);

        let mut history = MetricHistory::default();
        history.push(RawSample::new(1_000, 1).with(MetricKey::WalBytes, 1_000.0));
        history.push(RawSample::new(3_000, 1).with(MetricKey::WalBytes, 5_000.0));

        assert_eq!(history.points(MetricKey::WalBytes)[1].value, Some(2_000.0));
    }

    #[test]
    fn missing_or_reset_counters_create_a_gap() {
        let mut history = MetricHistory::default();
        history.push(sample(1_000, 10.0));
        history.push(RawSample::new(2_000, 1));
        history.push(sample(3_000, 2.0));
        assert_eq!(history.points(MetricKey::Commits)[1].value, None);
        assert_eq!(history.points(MetricKey::Commits)[2].value, None);
    }

    #[test]
    fn history_is_pruned_by_time() {
        let mut history = MetricHistory::default();
        history.push(sample(0, 1.0));
        history.push(sample(DEFAULT_HISTORY.as_millis() as u64 + 1, 2.0));

        assert_eq!(history.samples().len(), 1);
        assert_eq!(
            history.samples()[0].at_millis,
            DEFAULT_HISTORY.as_millis() as u64 + 1
        );
    }

    #[test]
    fn downsampling_preserves_extremes() {
        let points = (0..100)
            .map(|index| MetricPoint {
                at_millis: index,
                value: Some(if index == 51 { 1_000.0 } else { 1.0 }),
            })
            .collect::<Vec<_>>();
        let result = downsample_series(&points, 10);

        assert!(result.iter().any(|(_, value)| *value == 1_000.0));
        assert!(result.len() <= 10);
    }

    #[test]
    fn process_projection_preserves_all_fields_and_filters_case_insensitively() {
        let row = ProcessRow {
            id: 42,
            user: "Ada".into(),
            database: Some("demo".into()),
            client: Some("localhost".into()),
            application: Some("worker".into()),
            state: Some("active".into()),
            wait: Some("Lock".into()),
            elapsed: Duration::from_millis(1_250),
            query: Some("select * from users".into()),
        };

        let result = process_result_set(&[row], "ADA");
        assert_eq!(result.columns.len(), PROCESS_COLUMN_COUNT);
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0][0], crate::db::value::CellValue::Unsigned(42));
        assert_eq!(
            result.rows[0][7],
            crate::db::value::CellValue::Text("1.250s".into())
        );
        assert_eq!(
            result.rows[0][8],
            crate::db::value::CellValue::Text("select * from users".into())
        );
    }

    #[test]
    fn process_projection_uses_null_for_missing_optional_values() {
        let row = ProcessRow {
            id: 7,
            user: "u".into(),
            database: None,
            client: None,
            application: None,
            state: None,
            wait: None,
            elapsed: Duration::ZERO,
            query: None,
        };

        let result = process_result_set(&[row], "");
        assert!(matches!(
            result.rows[0][2],
            crate::db::value::CellValue::Null
        ));
        assert_eq!(format_process_elapsed(Duration::ZERO), "0ms");
        assert_eq!(format_process_elapsed(Duration::from_secs(65)), "1m 05s");
    }
}
