use uuid::Uuid;

use super::TextRange;

/// Cache identity for lexical analysis. Cursor movement is deliberately absent.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AnalysisKey {
    pub console_id: Uuid,
    pub document_revision: u64,
    pub dialect: super::SqlDialect,
}

/// Converts sqlparser's one-based character locations to UTF-8 byte offsets.
#[derive(Clone, Debug)]
pub struct LineIndex {
    starts: Vec<usize>,
    text_len: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        for (index, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(index + 1);
            }
        }
        Self {
            starts,
            text_len: text.len(),
        }
    }

    /// `line` and `column` are one-based; column zero denotes an empty span.
    pub fn offset(&self, text: &str, line: u64, column: u64) -> usize {
        if line == 0 {
            return 0;
        }
        let line_start = *self.starts.get(line as usize - 1).unwrap_or(&self.text_len);
        if column <= 1 {
            return line_start;
        }
        text[line_start..]
            .char_indices()
            .nth(column as usize - 1)
            .map_or_else(
                || text[line_start..].len() + line_start,
                |(offset, _)| line_start + offset,
            )
    }

    pub fn range(
        &self,
        text: &str,
        start: sqlparser::tokenizer::Location,
        end: sqlparser::tokenizer::Location,
    ) -> TextRange {
        TextRange::new(
            self.offset(text, start.line, start.column),
            self.offset(text, end.line, end.column),
        )
    }
}
