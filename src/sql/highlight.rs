use sqlparser::{
    dialect::{Dialect, GenericDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect},
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
    let dialect = dialect_ref(dialect);
    let mut tokenizer = Tokenizer::new(dialect, text);
    let mut tokens = Vec::new();
    let _ = tokenizer.tokenize_with_location_into_buf(&mut tokens);
    tokens
        .into_iter()
        .filter_map(|token| span_for(text, &index, token))
        .collect()
}

fn span_for(text: &str, index: &LineIndex, token: TokenWithSpan) -> Option<HighlightSpan> {
    let range = index.range(text, token.span.start, token.span.end);
    if range.is_empty() {
        return None;
    }
    let kind = match token.token {
        Token::Word(word) if word.keyword != sqlparser::keywords::Keyword::NoKeyword => {
            HighlightKind::Keyword
        }
        Token::Word(_) => HighlightKind::Identifier,
        Token::SingleQuotedString(_)
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
        SqlDialect::Sqlite => &SQLiteDialect {},
        SqlDialect::Generic => &GenericDialect {},
    }
}
