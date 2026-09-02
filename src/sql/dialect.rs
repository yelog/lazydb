#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum SqlDialect {
    Postgres,
    MySql,
    SqlServer,
    Sqlite,
    #[default]
    Generic,
}

pub(crate) fn parser_dialect(dialect: SqlDialect) -> &'static dyn sqlparser::dialect::Dialect {
    match dialect {
        SqlDialect::Postgres => &sqlparser::dialect::PostgreSqlDialect {},
        SqlDialect::MySql => &sqlparser::dialect::MySqlDialect {},
        SqlDialect::SqlServer => &sqlparser::dialect::MsSqlDialect {},
        SqlDialect::Sqlite => &sqlparser::dialect::SQLiteDialect {},
        SqlDialect::Generic => &sqlparser::dialect::GenericDialect {},
    }
}
