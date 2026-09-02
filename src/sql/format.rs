use sqlparser::{
    dialect::{
        Dialect as ParserDialect, GenericDialect, MsSqlDialect, MySqlDialect, PostgreSqlDialect,
        SQLiteDialect,
    },
    tokenizer::{Token, Tokenizer},
};

use super::SqlDialect;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum FormatError {
    #[error("formatting is not supported for dollar-quoted procedural bodies")]
    ProceduralBody,
    #[error("formatter changed SQL token meaning; buffer was preserved")]
    MeaningChanged,
}

pub fn format_sql(sql: &str, dialect: SqlDialect) -> Result<String, FormatError> {
    if has_dollar_quoted_body(sql) {
        return Err(FormatError::ProceduralBody);
    }
    let options = sqlformat::FormatOptions {
        uppercase: Some(true),
        dialect: match dialect {
            SqlDialect::Postgres => sqlformat::Dialect::PostgreSql,
            SqlDialect::SqlServer => sqlformat::Dialect::SQLServer,
            _ => sqlformat::Dialect::Generic,
        },
        ..Default::default()
    };
    let formatted = sqlformat::format(sql, &sqlformat::QueryParams::None, &options);
    let parser_dialect: &dyn ParserDialect = match dialect {
        SqlDialect::Postgres => &PostgreSqlDialect {},
        SqlDialect::MySql => &MySqlDialect {},
        SqlDialect::SqlServer => &MsSqlDialect {},
        SqlDialect::Sqlite => &SQLiteDialect {},
        SqlDialect::Generic => &GenericDialect {},
    };
    let before = meaningful_tokens(parser_dialect, sql);
    let after = meaningful_tokens(parser_dialect, &formatted);
    if before.is_none() || after.is_none() || before != after {
        return Err(FormatError::MeaningChanged);
    }
    Ok(formatted)
}

fn meaningful_tokens(dialect: &dyn ParserDialect, sql: &str) -> Option<Vec<String>> {
    Some(
        Tokenizer::new(dialect, sql)
            .tokenize()
            .ok()?
            .into_iter()
            .filter(|token| !matches!(token, Token::Whitespace(_)))
            .map(|token| match token {
                Token::Word(word) if word.keyword != sqlparser::keywords::Keyword::NoKeyword => {
                    format!("keyword:{:?}", word.keyword)
                }
                token => token.to_string(),
            })
            .collect(),
    )
}

fn has_dollar_quoted_body(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    for start in 0..bytes.len() {
        if bytes[start] != b'$' {
            continue;
        }
        let mut end = start + 1;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        if end >= bytes.len()
            || bytes[end] != b'$'
            || end == start + 1 && start + 1 < bytes.len() && bytes[start + 1].is_ascii_digit()
        {
            continue;
        }
        let tag = &sql[start..=end];
        if sql[end + 1..].contains(tag) {
            return true;
        }
    }
    false
}
