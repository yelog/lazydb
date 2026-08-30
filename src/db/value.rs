use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum CellValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Unsigned(u64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    Date(NaiveDate),
    Time(NaiveTime),
    DateTime(NaiveDateTime),
    Timestamp(DateTime<FixedOffset>),
    Unsupported { type_name: String, preview: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CellPreview {
    pub text: String,
    pub original_len: usize,
    pub truncated: bool,
}

impl CellValue {
    pub fn clipboard_text(&self) -> String {
        match self {
            Self::Null => String::new(),
            Self::Boolean(value) => value.to_string(),
            Self::Integer(value) => value.to_string(),
            Self::Unsigned(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
            Self::Text(value) => value.clone(),
            Self::Bytes(value) => {
                let mut text = String::with_capacity(2 + value.len() * 2);
                text.push_str("0x");
                for byte in value {
                    use std::fmt::Write;
                    let _ = write!(text, "{byte:02X}");
                }
                text
            }
            Self::Date(value) => value.format("%Y-%m-%d").to_string(),
            Self::Time(value) => format_time(*value),
            Self::DateTime(value) => format_datetime(*value),
            Self::Timestamp(value) => format_timestamp(*value),
            Self::Unsupported { preview, .. } => preview.clone(),
        }
    }

    pub fn preview(&self, max_len: usize) -> CellPreview {
        match self {
            Self::Null => CellPreview::complete("NULL", 0),
            Self::Boolean(value) => CellPreview::complete(if *value { "true" } else { "false" }, 1),
            Self::Integer(value) => CellPreview::complete(value.to_string(), 1),
            Self::Unsigned(value) => CellPreview::complete(value.to_string(), 1),
            Self::Float(value) => CellPreview::complete(value.to_string(), 1),
            Self::Text(value) => preview_text(value, max_len),
            Self::Bytes(value) => preview_bytes(value, max_len),
            Self::Date(value) => preview_text(&value.format("%Y-%m-%d").to_string(), max_len),
            Self::Time(value) => preview_text(&format_time(*value), max_len),
            Self::DateTime(value) => preview_text(&format_datetime(*value), max_len),
            Self::Timestamp(value) => preview_text(&format_timestamp(*value), max_len),
            Self::Unsupported { preview, .. } => preview_text(preview, max_len),
        }
    }
}

fn format_time(value: NaiveTime) -> String {
    format_seconds(value.format("%H:%M:%S").to_string(), value.nanosecond())
}

fn format_datetime(value: NaiveDateTime) -> String {
    format_seconds(
        value.format("%Y-%m-%d %H:%M:%S").to_string(),
        value.nanosecond(),
    )
}

fn format_timestamp(value: DateTime<FixedOffset>) -> String {
    let base = value.format("%Y-%m-%d %H:%M:%S").to_string();
    let with_fraction = format_seconds(base, value.nanosecond());
    format!("{with_fraction}{}", value.format("%:z"))
}

fn format_seconds(mut base: String, nanoseconds: u32) -> String {
    if nanoseconds != 0 {
        let fraction = format!("{nanoseconds:09}").trim_end_matches('0').to_owned();
        base.push('.');
        base.push_str(&fraction);
    }
    base
}

impl CellPreview {
    fn complete(text: impl Into<String>, original_len: usize) -> Self {
        Self {
            text: text.into(),
            original_len,
            truncated: false,
        }
    }
}

fn preview_text(value: &str, max_len: usize) -> CellPreview {
    let original_len = value.chars().count();
    let truncated = original_len > max_len;
    let mut text = value.chars().take(max_len).collect::<String>();
    if truncated {
        text.push_str("...");
    }
    CellPreview {
        text,
        original_len,
        truncated,
    }
}

fn preview_bytes(value: &[u8], max_len: usize) -> CellPreview {
    let truncated = value.len() > max_len;
    let mut text = String::with_capacity(2 + max_len.saturating_mul(2) + 3);
    text.push_str("0x");
    for byte in value.iter().take(max_len) {
        use std::fmt::Write;
        let _ = write!(text, "{byte:02X}");
    }
    if truncated {
        text.push_str("...");
    }
    CellPreview {
        text,
        original_len: value.len(),
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};

    use super::CellValue;

    #[test]
    fn null_is_distinct_from_empty_values() {
        assert_ne!(CellValue::Null, CellValue::Text(String::new()));
        assert_ne!(CellValue::Null, CellValue::Bytes(Vec::new()));
    }

    #[test]
    fn preview_truncates_without_losing_length() {
        let value = CellValue::Text("alpha-beta".into());
        let preview = value.preview(5);

        assert_eq!(preview.text, "alpha...");
        assert_eq!(preview.original_len, 10);
        assert!(preview.truncated);
    }

    #[test]
    fn binary_preview_is_safe_and_bounded() {
        let value = CellValue::Bytes(vec![0, 1, 2, 255]);
        let preview = value.preview(2);

        assert_eq!(preview.text, "0x0001...");
        assert_eq!(preview.original_len, 4);
    }

    #[test]
    fn temporal_previews_use_database_friendly_formats() {
        let date = NaiveDate::from_ymd_opt(2026, 8, 28).unwrap();
        let time = NaiveTime::from_hms_micro_opt(10, 20, 31, 120_000).unwrap();
        let datetime = NaiveDateTime::new(date, time);
        let zoned = DateTime::<FixedOffset>::from_naive_utc_and_offset(
            datetime,
            FixedOffset::east_opt(8 * 60 * 60).unwrap(),
        );

        assert_eq!(CellValue::Date(date).preview(40).text, "2026-08-28");
        assert_eq!(CellValue::Time(time).preview(40).text, "10:20:31.12");
        assert_eq!(
            CellValue::DateTime(datetime).preview(40).text,
            "2026-08-28 10:20:31.12"
        );
        assert_eq!(
            CellValue::Timestamp(zoned).preview(40).text,
            "2026-08-28 18:20:31.12+08:00"
        );
    }

    #[test]
    fn temporal_preview_omits_zero_fractional_seconds() {
        let value = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 8, 28).unwrap(),
            NaiveTime::from_hms_opt(10, 20, 31).unwrap(),
        );

        assert_eq!(
            CellValue::DateTime(value).preview(40).text,
            "2026-08-28 10:20:31"
        );
    }
}
