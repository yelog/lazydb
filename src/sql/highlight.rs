use sqlparser::{
    dialect::{
        Dialect, GenericDialect, MsSqlDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect,
    },
    tokenizer::{Token, TokenWithSpan, Tokenizer, Whitespace},
};

use super::{LineIndex, SqlDialect, TextRange};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HighlightKind {
    Keyword,
    Identifier,
    String,
    Number,
    Comment,
    Operator,
    Punctuation,
    Parameter,
    Plain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HighlightSpan {
    pub range: TextRange,
    pub kind: HighlightKind,
}

pub fn highlight_sql(text: &str, dialect: SqlDialect) -> Vec<HighlightSpan> {
    let index = LineIndex::new(text);
    let parser_dialect = dialect_ref(dialect);
    let mut tokenizer = Tokenizer::new(parser_dialect, text);
    let mut tokens = Vec::new();
    let _ = tokenizer.tokenize_with_location_into_buf(&mut tokens);
    let spans = tokens
        .into_iter()
        .filter_map(|token| span_for(text, &index, token, dialect))
        .collect::<Vec<_>>();
    merge_sql_server_variables(text, spans, dialect == SqlDialect::SqlServer)
}

fn merge_sql_server_variables(
    text: &str,
    spans: Vec<HighlightSpan>,
    sql_server: bool,
) -> Vec<HighlightSpan> {
    if !sql_server {
        return spans;
    }
    let mut merged: Vec<HighlightSpan> = Vec::with_capacity(spans.len());
    for span in spans {
        if let Some(previous) = merged.last_mut()
            && previous.kind == HighlightKind::Parameter
            && span.range.start == previous.range.end
            && (span.kind == HighlightKind::Identifier || span.kind == HighlightKind::Parameter)
            && previous
                .range
                .get(text)
                .is_some_and(|value| value.starts_with('@'))
        {
            previous.range.end = span.range.end;
            continue;
        }
        merged.push(span);
    }
    merged
}

pub fn highlight_sql_ranges(
    text: &str,
    ranges: &[TextRange],
    dialect: SqlDialect,
) -> Vec<HighlightSpan> {
    ranges
        .iter()
        .filter_map(|range| text.get(range.start..range.end).map(|sql| (*range, sql)))
        .flat_map(|(range, sql)| {
            highlight_sql(sql, dialect)
                .into_iter()
                .map(move |span| HighlightSpan {
                    range: TextRange::new(
                        range.start + span.range.start,
                        range.start + span.range.end,
                    ),
                    kind: span.kind,
                })
        })
        .collect()
}

fn span_for(
    text: &str,
    index: &LineIndex,
    token: TokenWithSpan,
    dialect: SqlDialect,
) -> Option<HighlightSpan> {
    let range = index.range(text, token.span.start, token.span.end);
    if range.is_empty() {
        return None;
    }
    let kind = match token.token {
        Token::Word(word) if word.keyword != sqlparser::keywords::Keyword::NoKeyword => {
            HighlightKind::Keyword
        }
        Token::Word(_)
            if dialect == SqlDialect::SqlServer
                && range.get(text).is_some_and(|value| value.starts_with('@')) =>
        {
            HighlightKind::Parameter
        }
        Token::Word(_) => HighlightKind::Identifier,
        Token::SingleQuotedString(_)
        | Token::NationalStringLiteral(_)
        | Token::DoubleQuotedString(_)
        | Token::DollarQuotedString(_)
        | Token::EscapedStringLiteral(_) => HighlightKind::String,
        Token::Number(_, _) => HighlightKind::Number,
        Token::Whitespace(
            Whitespace::SingleLineComment { .. } | Whitespace::MultiLineComment(_),
        ) => HighlightKind::Comment,
        Token::Whitespace(_) => return None,
        Token::AtSign | Token::Colon | Token::Placeholder(_) => HighlightKind::Parameter,
        Token::Comma | Token::LParen | Token::RParen | Token::Period | Token::SemiColon => {
            HighlightKind::Punctuation
        }
        Token::EOF => return None,
        _ => HighlightKind::Operator,
    };
    Some(HighlightSpan { range, kind })
}

fn dialect_ref(dialect: SqlDialect) -> &'static dyn Dialect {
    match dialect {
        SqlDialect::Postgres => &PostgreSqlDialect {},
        SqlDialect::MySql => &MySqlDialect {},
        SqlDialect::SqlServer => &MsSqlDialect {},
        SqlDialect::Sqlite => &SQLiteDialect {},
        SqlDialect::Generic => &GenericDialect {},
    }
}
