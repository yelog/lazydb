use super::{SqlDialect, TextRange};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeKind {
    VisualChar,
    VisualLine,
    VisualBlock,
    CurrentStatement,
    FullBuffer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScopeSource {
    Contiguous(TextRange),
    Block(Vec<TextRange>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopeSelection {
    pub kind: ScopeKind,
    pub source: ScopeSource,
}

impl ScopeSelection {
    pub const fn contiguous(kind: ScopeKind, range: TextRange) -> Self {
        Self {
            kind,
            source: ScopeSource::Contiguous(range),
        }
    }

    pub const fn block(ranges: Vec<TextRange>) -> Self {
        Self {
            kind: ScopeKind::VisualBlock,
            source: ScopeSource::Block(ranges),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedScope {
    pub kind: ScopeKind,
    pub source: ScopeSource,
    pub sql: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StatementSpan {
    execution: TextRange,
    activation: TextRange,
}

/// Resolves the exact Visual selection first, then the statement containing
/// `cursor`. A blank visual selection is intentionally a no-scope result.
pub fn resolve_scope(
    text: &str,
    cursor: usize,
    selection: Option<&ScopeSelection>,
    dialect: SqlDialect,
) -> Option<ResolvedScope> {
    if let Some(selection) = selection {
        return resolve_selection(text, selection, dialect);
    }

    let range = statement_at(text, cursor, dialect)?;
    Some(ResolvedScope {
        kind: ScopeKind::CurrentStatement,
        source: ScopeSource::Contiguous(range),
        sql: text.get(range.start..range.end)?.to_owned(),
    })
}

/// Returns executable statement ranges. Each range includes its terminating
/// semicolon, when present, and excludes only inter-statement whitespace.
/// MySQL-family dialects assume ANSI_QUOTES and NO_BACKSLASH_ESCAPES are off;
/// editor scope selection has no access to the connection's session SQL mode.
pub fn scan_statements(text: &str, dialect: SqlDialect) -> Vec<TextRange> {
    scan_statement_spans(text, dialect)
        .into_iter()
        .map(|statement| statement.execution)
        .collect()
}

/// Scans using the SQL mode of the physical MySQL-protocol connection.
/// These flags apply to the entire input. Callers executing mode-changing SQL
/// must rescan the remaining input after refreshing the connection's mode.
/// Executable comments are preserved verbatim; their version guards are left
/// for the server to evaluate.
pub(crate) fn scan_mysql_statements(
    text: &str,
    ansi_quotes: bool,
    no_backslash_escapes: bool,
) -> Vec<TextRange> {
    scan_statement_spans_with_mode(text, SqlDialect::MySql, ansi_quotes, no_backslash_escapes)
        .into_iter()
        .map(|statement| statement.execution)
        .collect()
}

fn scan_statement_spans(text: &str, dialect: SqlDialect) -> Vec<StatementSpan> {
    scan_statement_spans_with_mode(text, dialect, false, false)
}

fn scan_statement_spans_with_mode(
    text: &str,
    dialect: SqlDialect,
    ansi_quotes: bool,
    no_backslash_escapes: bool,
) -> Vec<StatementSpan> {
    statement_boundaries(text, dialect, ansi_quotes, no_backslash_escapes)
        .into_iter()
        .filter_map(|(start, end)| {
            let execution = meaningful_range(text, TextRange::new(start, end), dialect)?;
            let sql = execution.get(text)?;
            let activation_start = execution.start + first_code_offset(sql, dialect)?;
            Some(StatementSpan {
                execution,
                activation: TextRange::new(activation_start, execution.end),
            })
        })
        .collect()
}

fn resolve_selection(
    text: &str,
    selection: &ScopeSelection,
    dialect: SqlDialect,
) -> Option<ResolvedScope> {
    match &selection.source {
        ScopeSource::Contiguous(range) => {
            let sql = range.get(text)?;
            if !has_code(sql, dialect) {
                return None;
            }
            Some(ResolvedScope {
                kind: selection.kind,
                source: ScopeSource::Contiguous(*range),
                sql: sql.to_owned(),
            })
        }
        ScopeSource::Block(ranges) => {
            let mut sql = String::new();
            for (index, range) in ranges.iter().enumerate() {
                let slice = range.get(text)?;
                if index != 0 {
                    sql.push('\n');
                }
                sql.push_str(slice);
            }
            if !has_code(&sql, dialect) {
                return None;
            }
            Some(ResolvedScope {
                kind: selection.kind,
                source: ScopeSource::Block(ranges.clone()),
                sql,
            })
        }
    }
}

fn statement_at(text: &str, cursor: usize, dialect: SqlDialect) -> Option<TextRange> {
    let cursor = cursor.min(text.len());
    let at_cursor = |statements: Vec<StatementSpan>| {
        statements
            .into_iter()
            .find(|statement| {
                statement.activation.start <= cursor && cursor < statement.activation.end
            })
            .map(|statement| statement.execution)
    };
    let range = at_cursor(scan_statement_spans(text, dialect))?;
    if matches!(dialect, SqlDialect::MySql | SqlDialect::MariaDb) {
        // The editor cannot know the session mode. Refuse automatic selection
        // when another supported quote mode could execute a different range;
        // an explicit Visual selection still preserves the user's exact SQL.
        for (ansi_quotes, no_backslash_escapes) in [(true, false), (false, true), (true, true)] {
            if at_cursor(scan_statement_spans_with_mode(
                text,
                dialect,
                ansi_quotes,
                no_backslash_escapes,
            )) != Some(range)
            {
                return None;
            }
        }
    }
    Some(range)
}

fn meaningful_range(text: &str, range: TextRange, dialect: SqlDialect) -> Option<TextRange> {
    let slice = text.get(range.start..range.end)?;
    if !has_code(slice, dialect) {
        return None;
    }
    let start = slice
        .char_indices()
        .find(|(_, c)| !c.is_whitespace())
        .map(|(index, _)| range.start + index)?;
    let end = slice
        .char_indices()
        .rev()
        .find(|(_, c)| !c.is_whitespace())
        .map(|(index, c)| range.start + index + c.len_utf8())?;
    Some(TextRange::new(start, end))
}

fn has_code(text: &str, dialect: SqlDialect) -> bool {
    first_code_offset(text, dialect).is_some()
}

fn first_code_offset(text: &str, dialect: SqlDialect) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_whitespace() {
            index += 1;
        } else if bytes[index] == b'-'
            && bytes.get(index + 1) == Some(&b'-')
            && dash_comment_allowed(bytes, index, dialect)
        {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
        } else if bytes[index] == b'#'
            && matches!(
                dialect,
                SqlDialect::MySql
                    | SqlDialect::MariaDb
                    | SqlDialect::Generic
                    | SqlDialect::ClickHouse
            )
        {
            index += 1;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
        } else if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            if (matches!(dialect, SqlDialect::MySql | SqlDialect::MariaDb)
                && bytes.get(index + 2) == Some(&b'!'))
                || (dialect == SqlDialect::MariaDb
                    && bytes.get(index + 2..index + 4) == Some(b"M!"))
            {
                return Some(index);
            }
            index = skip_block_comment(text, index);
        } else {
            return Some(index);
        }
    }
    None
}

fn statement_boundaries(
    text: &str,
    dialect: SqlDialect,
    ansi_quotes: bool,
    no_backslash_escapes: bool,
) -> Vec<(usize, usize)> {
    if dialect == SqlDialect::Oracle {
        return super::oracle::scan_statement_boundaries(text).unwrap_or_default();
    }
    let bytes = text.as_bytes();
    let mut boundaries = Vec::new();
    let mut start = 0;
    let mut index = 0;
    let mut block_depth = 0usize;
    let mut state = State::Normal;
    while index < bytes.len() {
        match state {
            State::Normal => match bytes[index] {
                b'\'' => {
                    state = State::SingleQuote;
                    index += 1;
                }
                b'"' => {
                    state = State::DoubleQuote;
                    index += 1;
                }
                b'q' | b'Q' | b'n' | b'N'
                    if matches!(dialect, SqlDialect::Oracle)
                        && oracle_quote_delimiter(bytes, index).is_some() =>
                {
                    let (end, closing) = oracle_quote_delimiter(bytes, index).unwrap();
                    state = State::OracleQuote(closing);
                    index = end;
                }
                b'`' if matches!(
                    dialect,
                    SqlDialect::MySql
                        | SqlDialect::MariaDb
                        | SqlDialect::Generic
                        | SqlDialect::ClickHouse
                ) =>
                {
                    state = State::Backtick;
                    index += 1;
                }
                b'[' if matches!(
                    dialect,
                    SqlDialect::SqlServer
                        | SqlDialect::Sqlite
                        | SqlDialect::DuckDb
                        | SqlDialect::Generic
                ) =>
                {
                    state = State::Bracket;
                    index += 1;
                }
                b'-' if bytes.get(index + 1) == Some(&b'-')
                    && dash_comment_allowed(bytes, index, dialect) =>
                {
                    state = State::LineComment;
                    index += 2;
                }
                b'#' if matches!(
                    dialect,
                    SqlDialect::MySql
                        | SqlDialect::MariaDb
                        | SqlDialect::Generic
                        | SqlDialect::ClickHouse
                ) =>
                {
                    state = State::LineComment;
                    index += 1;
                }
                b'/' if bytes.get(index + 1) == Some(&b'*') => {
                    block_depth = 1;
                    state = State::BlockComment;
                    index += 2;
                }
                b'$' if matches!(
                    dialect,
                    SqlDialect::Postgres
                        | SqlDialect::DuckDb
                        | SqlDialect::Generic
                        | SqlDialect::Snowflake
                ) =>
                {
                    if let Some(end) = dollar_tag_end(text, index)
                        .filter(|end| dialect != SqlDialect::Snowflake || *end == index + 2)
                    {
                        state = State::DollarQuote(TextRange::new(index, end));
                        index = end;
                    } else {
                        index += 1;
                    }
                }
                b';' => {
                    boundaries.push((start, index + 1));
                    start = index + 1;
                    index += 1;
                }
                _ => index += 1,
            },
            State::SingleQuote => {
                advance_quoted(bytes, &mut index, b'\'', &mut state, !no_backslash_escapes)
            }
            State::DoubleQuote => advance_quoted(
                bytes,
                &mut index,
                b'"',
                &mut state,
                matches!(dialect, SqlDialect::MySql | SqlDialect::MariaDb)
                    && !ansi_quotes
                    && !no_backslash_escapes,
            ),
            State::Backtick => advance_quoted(bytes, &mut index, b'`', &mut state, false),
            State::Bracket => advance_quoted(bytes, &mut index, b']', &mut state, false),
            State::LineComment => {
                if bytes[index] == b'\n' {
                    state = State::Normal;
                }
                index += 1;
            }
            State::BlockComment => {
                if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
                    block_depth += 1;
                    index += 2;
                } else if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
                    block_depth -= 1;
                    index += 2;
                    if block_depth == 0 {
                        state = State::Normal;
                    }
                } else {
                    index += 1;
                }
            }
            State::DollarQuote(tag) => {
                if bytes[index..].starts_with(&bytes[tag.start..tag.end]) {
                    index += tag.end - tag.start;
                    state = State::Normal;
                } else {
                    index += 1;
                }
            }
            State::OracleQuote(closing) => {
                if bytes[index] == closing && bytes.get(index + 1) == Some(&b'\'') {
                    index += 2;
                    state = State::Normal;
                } else {
                    index += 1;
                }
            }
        }
    }
    boundaries.push((start, bytes.len()));
    boundaries
}

#[derive(Clone, Copy)]
enum State {
    Normal,
    SingleQuote,
    DoubleQuote,
    Backtick,
    Bracket,
    LineComment,
    BlockComment,
    DollarQuote(TextRange),
    OracleQuote(u8),
}

fn advance_quoted(
    bytes: &[u8],
    index: &mut usize,
    quote: u8,
    state: &mut State,
    backslash_escape: bool,
) {
    if bytes[*index] == quote {
        if bytes.get(*index + 1) == Some(&quote) {
            *index += 2;
        } else {
            *index += 1;
            *state = State::Normal;
        }
    } else if bytes[*index] == b'\\' && backslash_escape {
        *index = (*index + 2).min(bytes.len());
    } else {
        *index += 1;
    }
}

fn dash_comment_allowed(bytes: &[u8], index: usize, dialect: SqlDialect) -> bool {
    if matches!(dialect, SqlDialect::MySql | SqlDialect::MariaDb) {
        bytes
            .get(index + 2)
            .is_none_or(|byte| byte.is_ascii_whitespace() || *byte == 0x1a)
    } else {
        true
    }
}

fn oracle_quote_delimiter(bytes: &[u8], index: usize) -> Option<(usize, u8)> {
    let quote_index = if matches!(bytes.get(index), Some(b'n' | b'N'))
        && matches!(bytes.get(index + 1), Some(b'q' | b'Q'))
    {
        index + 2
    } else if matches!(bytes.get(index), Some(b'q' | b'Q')) {
        index + 1
    } else {
        return None;
    };
    if bytes.get(quote_index) != Some(&b'\'') {
        return None;
    }
    let closing = match bytes.get(quote_index + 1).copied()? {
        b'[' => b']',
        b'(' => b')',
        b'{' => b'}',
        b'<' => b'>',
        delimiter => delimiter,
    };
    Some((quote_index + 2, closing))
}

fn skip_block_comment(text: &str, mut index: usize) -> usize {
    let bytes = text.as_bytes();
    let mut depth: usize = 0;
    while index < bytes.len() {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            depth += 1;
            index += 2;
        } else if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
            depth = depth.saturating_sub(1);
            index += 2;
            if depth == 0 {
                break;
            }
        } else {
            index += 1;
        }
    }
    index
}

fn dollar_tag_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = start + 1;
    while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_') {
        index += 1;
    }
    (bytes.get(index) == Some(&b'$')).then_some(index + 1)
}

#[cfg(test)]
mod tests {
    use super::scan_mysql_statements;

    #[test]
    fn mysql_session_modes_control_string_and_identifier_boundaries() {
        for ansi_quotes in [false, true] {
            for no_backslash_escapes in [false, true] {
                for (text, backslash_escapes) in [
                    (
                        r#"SELECT "路径\"; SELECT 2;"#,
                        !ansi_quotes && !no_backslash_escapes,
                    ),
                    (r"SELECT '路径\'; SELECT 2;", !no_backslash_escapes),
                ] {
                    let ranges = scan_mysql_statements(text, ansi_quotes, no_backslash_escapes);
                    let expected = if backslash_escapes {
                        vec![text]
                    } else {
                        vec![&text[..text.find(';').unwrap() + 1], "SELECT 2;"]
                    };
                    assert_eq!(
                        ranges
                            .iter()
                            .map(|range| range.get(text).unwrap())
                            .collect::<Vec<_>>(),
                        expected,
                        "ANSI_QUOTES={ansi_quotes}, NO_BACKSLASH_ESCAPES={no_backslash_escapes}, SQL={text}"
                    );
                }

                let text = r#"SELECT 'a'';b', "c"";d", `e``;f\`; SELECT 2;"#;
                let ranges = scan_mysql_statements(text, ansi_quotes, no_backslash_escapes);
                assert_eq!(ranges.len(), 2);
                assert_eq!(
                    ranges[0].get(text),
                    Some(&text[..text.rfind(" SELECT 2;").unwrap()])
                );
                assert_eq!(ranges[1].get(text), Some("SELECT 2;"));

                let text =
                    "# ordinary ;\n/*!40101 SET NAMES utf8mb4 */; SELECT 1--not comment; SELECT 2;";
                assert_eq!(
                    scan_mysql_statements(text, ansi_quotes, no_backslash_escapes).len(),
                    3
                );
                assert!(
                    scan_mysql_statements("/* comment */", ansi_quotes, no_backslash_escapes)
                        .is_empty()
                );
            }
        }
    }

    #[test]
    fn mysql_mode_change_can_be_followed_by_rescanning_remaining_source() {
        let text = "SET sql_mode='ANSI_QUOTES,NO_BACKSLASH_ESCAPES'; SELECT \"路径\\\"; SELECT '路径\\'; SELECT 2;";
        let first = scan_mysql_statements(text, false, false)[0];
        assert_eq!(
            first.get(text),
            Some("SET sql_mode='ANSI_QUOTES,NO_BACKSLASH_ESCAPES';")
        );
        let remaining = &text[first.end..];
        let ranges = scan_mysql_statements(remaining, true, true);
        assert_eq!(
            ranges
                .iter()
                .map(|range| range.get(remaining).unwrap())
                .collect::<Vec<_>>(),
            vec!["SELECT \"路径\\\";", "SELECT '路径\\';", "SELECT 2;"]
        );
    }
}
