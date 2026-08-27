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
pub fn scan_statements(text: &str, dialect: SqlDialect) -> Vec<TextRange> {
    scan_statement_spans(text, dialect)
        .into_iter()
        .map(|statement| statement.execution)
        .collect()
}

fn scan_statement_spans(text: &str, dialect: SqlDialect) -> Vec<StatementSpan> {
    statement_boundaries(text, dialect)
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
    scan_statement_spans(text, dialect)
        .into_iter()
        .find(|statement| statement.activation.start <= cursor && cursor < statement.activation.end)
        .map(|statement| statement.execution)
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
        } else if bytes[index] == b'#' && matches!(dialect, SqlDialect::MySql | SqlDialect::Generic)
        {
            index += 1;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
        } else if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index = skip_block_comment(text, index);
        } else {
            return Some(index);
        }
    }
    None
}

fn statement_boundaries(text: &str, dialect: SqlDialect) -> Vec<(usize, usize)> {
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
                b'`' if matches!(dialect, SqlDialect::MySql | SqlDialect::Generic) => {
                    state = State::Backtick;
                    index += 1;
                }
                b'[' if matches!(dialect, SqlDialect::Sqlite | SqlDialect::Generic) => {
                    state = State::Bracket;
                    index += 1;
                }
                b'-' if bytes.get(index + 1) == Some(&b'-')
                    && dash_comment_allowed(bytes, index, dialect) =>
                {
                    state = State::LineComment;
                    index += 2;
                }
                b'#' if matches!(dialect, SqlDialect::MySql | SqlDialect::Generic) => {
                    state = State::LineComment;
                    index += 1;
                }
                b'/' if bytes.get(index + 1) == Some(&b'*') => {
                    block_depth = 1;
                    state = State::BlockComment;
                    index += 2;
                }
                b'$' if matches!(dialect, SqlDialect::Postgres | SqlDialect::Generic) => {
                    if let Some(end) = dollar_tag_end(text, index) {
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
            State::SingleQuote => advance_quoted(bytes, &mut index, b'\'', &mut state),
            State::DoubleQuote => advance_quoted(bytes, &mut index, b'"', &mut state),
            State::Backtick => advance_quoted(bytes, &mut index, b'`', &mut state),
            State::Bracket => advance_quoted(bytes, &mut index, b']', &mut state),
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
}

fn advance_quoted(bytes: &[u8], index: &mut usize, quote: u8, state: &mut State) {
    if bytes[*index] == quote {
        if bytes.get(*index + 1) == Some(&quote) {
            *index += 2;
        } else {
            *index += 1;
            *state = State::Normal;
        }
    } else if bytes[*index] == b'\\' && quote == b'\'' {
        *index = (*index + 2).min(bytes.len());
    } else {
        *index += 1;
    }
}

fn dash_comment_allowed(bytes: &[u8], index: usize, dialect: SqlDialect) -> bool {
    if !matches!(dialect, SqlDialect::MySql | SqlDialect::Generic) {
        return true;
    }
    if dialect == SqlDialect::MySql {
        bytes
            .get(index + 2)
            .is_none_or(|byte| byte.is_ascii_whitespace() || *byte == 0x1a)
    } else {
        true
    }
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
