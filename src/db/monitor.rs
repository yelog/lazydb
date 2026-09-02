use std::{collections::BTreeMap, time::Duration};

use crate::model::dashboard::{MetricKey, ProcessRow};

pub const MAX_PROCESS_ROWS: usize = 2_000;

#[derive(Clone, Debug, PartialEq)]
pub struct MonitorSnapshot {
    pub server_time_millis: u64,
    pub server_generation: u64,
    pub values: BTreeMap<MetricKey, f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MonitorMetadata {
    pub version: Option<String>,
    pub max_connections: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessSnapshot {
    pub rows: Vec<ProcessRow>,
    pub truncated: bool,
    pub visibility: MonitorVisibility,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MonitorVisibility {
    Full,
    Restricted,
    #[default]
    Unknown,
}

pub fn parse_process_duration(seconds: Option<f64>) -> Duration {
    Duration::from_secs_f64(seconds.unwrap_or_default().max(0.0))
}

pub fn status_value(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::{parse_process_duration, status_value};
    use std::time::Duration;

    #[test]
    fn status_values_reject_invalid_and_non_finite_numbers() {
        assert_eq!(status_value(" 42.5 "), Some(42.5));
        assert_eq!(status_value("NaN"), None);
        assert_eq!(status_value("not-a-number"), None);
    }

    #[test]
    fn process_duration_never_becomes_negative() {
        assert_eq!(parse_process_duration(Some(-2.0)), Duration::ZERO);
        assert_eq!(
            parse_process_duration(Some(2.5)),
            Duration::from_secs_f64(2.5)
        );
    }

    #[test]
    fn status_values_preserve_missing_fields() {
        assert_eq!(status_value(""), None);
        assert_eq!(status_value("12"), Some(12.0));
    }
}
