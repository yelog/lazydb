use lazydb::{profile::DatabaseKind, sql::SqlDialect};

#[test]
fn database_kind_mapping_has_one_canonical_sql_dialect() {
    assert_eq!(
        SqlDialect::for_database_kind(DatabaseKind::Postgres),
        SqlDialect::Postgres
    );
    assert_eq!(
        SqlDialect::for_database_kind(DatabaseKind::CockroachDb),
        SqlDialect::Postgres
    );
    assert_eq!(
        SqlDialect::for_database_kind(DatabaseKind::TiDb),
        SqlDialect::MySql
    );
    assert_eq!(
        SqlDialect::for_database_kind(DatabaseKind::MySql),
        SqlDialect::MySql
    );
    assert_eq!(
        SqlDialect::for_database_kind(DatabaseKind::SqlServer),
        SqlDialect::SqlServer
    );
    assert_eq!(
        SqlDialect::for_database_kind(DatabaseKind::Sqlite),
        SqlDialect::Sqlite
    );
    assert_eq!(
        SqlDialect::for_database_kind(DatabaseKind::ClickHouse),
        SqlDialect::ClickHouse
    );
    assert_eq!(
        SqlDialect::for_database_kind(DatabaseKind::DuckDb),
        SqlDialect::DuckDb
    );
    assert_eq!(
        SqlDialect::try_for_database_kind(DatabaseKind::Valkey),
        None
    );
}

#[test]
fn snowflake_uses_its_native_parser_and_lsp_dialect() {
    use lazydb::cli::LspDialect;
    assert_eq!(
        SqlDialect::for_database_kind(DatabaseKind::Snowflake),
        SqlDialect::Snowflake
    );
    assert!(matches!(
        LspDialect::from(DatabaseKind::Snowflake),
        LspDialect::Snowflake
    ));
    sqlparser::parser::Parser::parse_sql(
        &sqlparser::dialect::SnowflakeDialect {},
        "SELECT payload:key::VARCHAR FROM DB.PUBLIC.EVENTS QUALIFY ROW_NUMBER() OVER (PARTITION BY id ORDER BY created_at DESC) = 1",
    ).unwrap();
}
