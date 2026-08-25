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
    Unsupported { type_name: String, preview: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CellPreview {
    pub text: String,
    pub original_len: usize,
    pub truncated: bool,
}

impl CellValue {
    pub fn preview(&self, max_len: usize) -> CellPreview {
        match self {
            Self::Null => CellPreview::complete("NULL", 0),
            Self::Boolean(value) => CellPreview::complete(if *value { "true" } else { "false" }, 1),
            Self::Integer(value) => CellPreview::complete(value.to_string(), 1),
            Self::Unsigned(value) => CellPreview::complete(value.to_string(), 1),
            Self::Float(value) => CellPreview::complete(value.to_string(), 1),
            Self::Text(value) => preview_text(value, max_len),
            Self::Bytes(value) => preview_bytes(value, max_len),
            Self::Unsupported { preview, .. } => preview_text(preview, max_len),
        }
    }
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
}
