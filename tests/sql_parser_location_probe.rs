use lazydb::sql::SqlDialect;
use sqlparser::{parser::Parser, tokenizer::Tokenizer};

fn dialect(dialect: SqlDialect) -> &'static dyn sqlparser::dialect::Dialect {
    match dialect {
        SqlDialect::Postgres => &sqlparser::dialect::PostgreSqlDialect {},
        SqlDialect::MySql | SqlDialect::MariaDb => &sqlparser::dialect::MySqlDialect {},
        SqlDialect::SqlServer => &sqlparser::dialect::MsSqlDialect {},
        SqlDialect::Sqlite => &sqlparser::dialect::SQLiteDialect {},
        SqlDialect::ClickHouse => &sqlparser::dialect::ClickHouseDialect {},
        SqlDialect::DuckDb => &sqlparser::dialect::DuckDbDialect {},
        SqlDialect::Snowflake => &sqlparser::dialect::SnowflakeDialect {},
        SqlDialect::Generic | SqlDialect::Oracle => &sqlparser::dialect::GenericDialect {},
    }
}

#[test]
fn parser_errors_may_not_expose_a_source_location() {
    let error = Parser::parse_sql(dialect(SqlDialect::Generic), "select *\nfrom users\nwhere")
        .expect_err("incomplete predicate should not parse");
    let message = error.to_string();

    assert_eq!(
        message,
        "sql parser error: Expected: an expression, found: EOF"
    );
}

#[test]
fn tokenizer_errors_expose_a_structured_location() {
    let error = Tokenizer::new(dialect(SqlDialect::Generic), "select *\nfrom users\n'")
        .tokenize()
        .expect_err("unterminated string should not tokenize");

    assert_eq!(error.location.line, 3);
    assert_eq!(error.location.column, 1);
}

#[test]
fn parser_error_locations_are_not_a_structured_parser_error_field() {
    let error = Parser::parse_sql(dialect(SqlDialect::Generic), "select (")
        .expect_err("unclosed expression should not parse");
    let debug = format!("{error:?}");

    assert!(debug.starts_with("ParserError::ParserError") || debug.starts_with("ParserError("));
    assert!(
        !debug.contains("location:"),
        "parser error unexpectedly changed shape: {debug}"
    );
}
