use sqlparser::tokenizer::Tokenizer;
use uuid::Uuid;

use super::{
    SqlDialect, TextRange, analysis::LineIndex, dialect::parser_dialect, scope::scan_statements,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqlDiagnostic {
    pub range: TextRange,
    pub message: String,
    pub code: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticScheduleKey {
    pub console_id: Uuid,
    pub document_revision: u64,
    pub connection: Option<crate::model::workspace::ConnectionIdentity>,
    pub target: Option<crate::model::execution_target::ExecutionTarget>,
    pub dialect: SqlDialect,
    pub catalog_generation: u64,
}

pub fn diagnose_sql(text: &str, dialect: SqlDialect) -> Vec<SqlDiagnostic> {
    let statements = scan_statements(text, dialect);
    if statements.is_empty() {
        return diagnose_statement(text, dialect, 0);
    }
    let mut diagnostics = Vec::new();
    for range in statements {
        let Some(statement) = range.get(text) else {
            continue;
        };
        diagnostics.extend(diagnose_statement(statement, dialect, range.start));
    }
    diagnostics
}

fn diagnose_statement(text: &str, dialect: SqlDialect, offset: usize) -> Vec<SqlDiagnostic> {
    let tokenized = if dialect == SqlDialect::BigQuery {
        super::bigquery::tokenize(text).map(|_| ())
    } else {
        Tokenizer::new(parser_dialect(dialect), text)
            .tokenize()
            .map(|_| ())
    };
    if let Err(error) = tokenized {
        let index = LineIndex::new(text);
        let start = offset + index.offset(text, error.location.line, error.location.column);
        let end = start
            + text
                .get(start.saturating_sub(offset)..)
                .and_then(|suffix| suffix.chars().next())
                .map_or(0, char::len_utf8);
        return vec![SqlDiagnostic {
            range: TextRange::new(start, end),
            message: error.to_string(),
            code: "sql-tokenizer",
        }];
    }
    if let Err(error) = super::dialect::parse_sql(dialect, text) {
        let message = error.to_string();
        // sqlparser exposes parser locations only as a trailing message suffix.
        let location = message.rsplit_once(" at Line: ").and_then(|(_, location)| {
            let (line, column) = location.split_once(", Column: ")?;
            Some((line.parse::<u64>().ok()?, column.parse::<u64>().ok()?))
        });
        let range = if let Some((line, column)) = location {
            let start = offset + LineIndex::new(text).offset(text, line, column);
            TextRange::new(
                start,
                start
                    + text
                        .get(start.saturating_sub(offset)..)
                        .and_then(|suffix| suffix.chars().next())
                        .map_or(0, char::len_utf8),
            )
        } else if message.ends_with("found: EOF") {
            let end = offset + text.trim_end().len();
            TextRange::new(end, end)
        } else {
            TextRange::new(offset, offset + text.len())
        };
        return vec![SqlDiagnostic {
            range,
            message,
            code: "sql-parser",
        }];
    }
    Vec::new()
}
