use crate::db::{query::ColumnMeta, value::CellValue};
use base64::{Engine as _, engine::general_purpose::STANDARD};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Osc52Error {
    PayloadTooLarge {
        encoded_bytes: usize,
        max_bytes: usize,
    },
}

impl std::fmt::Display for Osc52Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PayloadTooLarge {
                encoded_bytes,
                max_bytes,
            } => write!(
                f,
                "clipboard payload is too large after encoding ({encoded_bytes} bytes; limit {max_bytes})"
            ),
        }
    }
}

impl std::error::Error for Osc52Error {}

pub fn osc52_sequence(text: &str, max_bytes: usize) -> Result<String, Osc52Error> {
    let encoded = STANDARD.encode(text.as_bytes());
    if encoded.len() > max_bytes {
        return Err(Osc52Error::PayloadTooLarge {
            encoded_bytes: encoded.len(),
            max_bytes,
        });
    }
    Ok(format!("\x1b]52;c;{encoded}\x07"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardPayload {
    pub text: String,
    pub description: String,
    pub sensitive: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CopyTarget {
    EditorYank,
    EditorStatement,
    EditorBuffer,
    GridCell,
    GridRow { include_headers: bool },
}

pub fn copy_cell(label: &str, value: &CellValue) -> ClipboardPayload {
    let description = if matches!(value, CellValue::Null) {
        format!("cell {label} (NULL as empty value)")
    } else {
        format!("cell {label}")
    };
    ClipboardPayload {
        text: value.clipboard_text(),
        description,
        sensitive: false,
    }
}

pub fn copy_row_tsv(
    columns: &[ColumnMeta],
    row: &[CellValue],
    include_headers: bool,
) -> Option<ClipboardPayload> {
    if columns.is_empty() {
        return None;
    }

    let mut lines = Vec::with_capacity(2);
    if include_headers {
        lines.push(
            columns
                .iter()
                .map(|column| escape_tsv(column.name.clone()))
                .collect::<Vec<_>>()
                .join("\t"),
        );
    }
    lines.push(
        columns
            .iter()
            .enumerate()
            .map(|(index, _)| {
                row.get(index)
                    .map_or_else(String::new, CellValue::clipboard_text)
                    .pipe(escape_tsv)
            })
            .collect::<Vec<_>>()
            .join("\t"),
    );
    Some(ClipboardPayload {
        text: lines.join("\n"),
        description: if include_headers {
            format!("row: {} columns as TSV with headers", columns.len())
        } else {
            format!("row: {} columns as TSV", columns.len())
        },
        sensitive: false,
    })
}

fn escape_tsv(value: String) -> String {
    if value
        .chars()
        .any(|character| matches!(character, '\t' | '\r' | '\n' | '"'))
    {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value
    }
}

trait Pipe: Sized {
    fn pipe<T>(self, function: impl FnOnce(Self) -> T) -> T {
        function(self)
    }
}

impl<T> Pipe for T {}

#[cfg(test)]
mod tests {
    use super::{ClipboardPayload, Osc52Error, copy_cell, copy_row_tsv, osc52_sequence};
    use crate::db::{query::ColumnMeta, value::CellValue};

    #[test]
    fn cell_copy_uses_complete_values_instead_of_previews() {
        let value = CellValue::Text("alpha-beta".into());
        assert_eq!(
            copy_cell("users.name", &value),
            ClipboardPayload {
                text: "alpha-beta".into(),
                description: "cell users.name".into(),
                sensitive: false,
            }
        );
    }

    #[test]
    fn tsv_quotes_fields_that_would_break_the_grid_shape() {
        let columns = vec![
            ColumnMeta {
                name: "id".into(),
                type_name: "INT".into(),
            },
            ColumnMeta {
                name: "note".into(),
                type_name: "TEXT".into(),
            },
        ];
        let row = vec![CellValue::Integer(7), CellValue::Text("a\tb\n\"c\"".into())];
        assert_eq!(
            copy_row_tsv(&columns, &row, false).unwrap().text,
            "7\t\"a\tb\n\"\"c\"\"\""
        );
        assert_eq!(
            copy_row_tsv(&columns, &row, true).unwrap().text,
            "id\tnote\n7\t\"a\tb\n\"\"c\"\"\""
        );
    }

    #[test]
    fn null_and_empty_text_are_valid_but_described_differently() {
        let null = copy_cell("users.note", &CellValue::Null);
        let empty = copy_cell("users.note", &CellValue::Text(String::new()));
        assert_eq!(null.text, "");
        assert_eq!(empty.text, "");
        assert!(null.description.contains("NULL as empty value"));
        assert!(!empty.description.contains("NULL"));
    }

    #[test]
    fn bytes_are_complete_uppercase_hex() {
        assert_eq!(
            copy_cell("payload", &CellValue::Bytes(vec![0, 1, 2, 255])).text,
            "0x000102FF"
        );
    }

    #[test]
    fn osc52_encodes_utf8_and_empty_payload() {
        assert_eq!(
            osc52_sequence("你好", 100).unwrap(),
            "\x1b]52;c;5L2g5aW9\x07"
        );
        assert_eq!(osc52_sequence("", 0).unwrap(), "\x1b]52;c;\x07");
    }

    #[test]
    fn osc52_rejects_encoded_payload_over_limit_without_truncating() {
        assert_eq!(
            osc52_sequence("hello", 4),
            Err(Osc52Error::PayloadTooLarge {
                encoded_bytes: 8,
                max_bytes: 4
            })
        );
    }
}
