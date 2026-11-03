use sqlparser::{
    dialect::{
        ClickHouseDialect, Dialect as ParserDialect, GenericDialect, MsSqlDialect, MySqlDialect,
        PostgreSqlDialect, SQLiteDialect,
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
    let (format_source, protected) = if dialect == SqlDialect::BigQuery {
        protect_google_sql_tokens(sql)?
    } else {
        (sql.to_owned(), Vec::new())
    };
    let options = sqlformat::FormatOptions {
        uppercase: Some(true),
        dialect: match dialect {
            SqlDialect::Postgres => sqlformat::Dialect::PostgreSql,
            SqlDialect::SqlServer => sqlformat::Dialect::SQLServer,
            _ => sqlformat::Dialect::Generic,
        },
        ..Default::default()
    };
    let mut formatted = sqlformat::format(&format_source, &sqlformat::QueryParams::None, &options);
    for (placeholder, original) in protected {
        formatted = formatted.replace(&placeholder, &original);
    }
    let parser_dialect: &dyn ParserDialect = match dialect {
        SqlDialect::Postgres => &PostgreSqlDialect {},
        SqlDialect::MySql | SqlDialect::MariaDb => &MySqlDialect {},
        SqlDialect::SqlServer => &MsSqlDialect {},
        SqlDialect::Sqlite => &SQLiteDialect {},
        SqlDialect::DuckDb => &sqlparser::dialect::DuckDbDialect {},
        SqlDialect::Generic | SqlDialect::Oracle => &GenericDialect {},
        SqlDialect::ClickHouse => &ClickHouseDialect {},
        SqlDialect::Snowflake => &sqlparser::dialect::SnowflakeDialect {},
        SqlDialect::BigQuery => &sqlparser::dialect::BigQueryDialect {},
    };
    let before = if dialect == SqlDialect::BigQuery {
        google_sql_meaningful_tokens(sql)
    } else {
        meaningful_tokens(parser_dialect, sql)
    };
    let after = if dialect == SqlDialect::BigQuery {
        google_sql_meaningful_tokens(&formatted)
    } else {
        meaningful_tokens(parser_dialect, &formatted)
    };
    if before.is_none() || after.is_none() || before != after {
        return Err(FormatError::MeaningChanged);
    }
    Ok(formatted)
}

fn protect_google_sql_tokens(sql: &str) -> Result<(String, Vec<(String, String)>), FormatError> {
    // sqlformat has no GoogleSQL lexer. Protect quoted tokens with ordinary
    // string placeholders, then restore exact source bytes before token checks.
    let tokens = super::bigquery::tokenize(sql).map_err(|_| FormatError::MeaningChanged)?;
    let lines = super::LineIndex::new(sql);
    let mut source = String::new();
    let mut protected = Vec::new();
    let mut end = 0;
    for token in tokens {
        let range = lines.range(sql, token.span.start, token.span.end);
        let original = range.get(sql).ok_or(FormatError::MeaningChanged)?;
        if matches!(token.token, Token::Whitespace(_) | Token::Word(_))
            && !original.starts_with('`')
        {
            continue;
        }
        if !original.contains(['\'', '"', '`']) {
            continue;
        }
        let mut placeholder = format!("'__lazydb_google_sql_{}__'", protected.len());
        while sql.contains(&placeholder) {
            placeholder.insert(1, '_');
        }
        source.push_str(&sql[end..range.start]);
        source.push_str(&placeholder);
        protected.push((placeholder, original.to_owned()));
        end = range.end;
    }
    source.push_str(&sql[end..]);
    Ok((source, protected))
}

fn google_sql_meaningful_tokens(sql: &str) -> Option<Vec<String>> {
    let lines = super::LineIndex::new(sql);
    super::bigquery::tokenize(sql)
        .ok()?
        .into_iter()
        .filter(|token| !matches!(token.token, Token::Whitespace(_)))
        .map(|token| match token.token {
            Token::Word(word)
                if word.quote_style.is_none()
                    && word.keyword != sqlparser::keywords::Keyword::NoKeyword =>
            {
                Some(format!("keyword:{:?}", word.keyword))
            }
            _ => lines
                .range(sql, token.span.start, token.span.end)
                .get(sql)
                .map(str::to_owned),
        })
        .collect()
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
