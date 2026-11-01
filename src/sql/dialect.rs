#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum SqlDialect {
    Postgres,
    MySql,
    MariaDb,
    SqlServer,
    Sqlite,
    DuckDb,
    Oracle,
    ClickHouse,
    Snowflake,
    #[default]
    Generic,
}

impl SqlDialect {
    pub const fn try_for_database_kind(kind: crate::profile::DatabaseKind) -> Option<Self> {
        match kind {
            crate::profile::DatabaseKind::Redis
            | crate::profile::DatabaseKind::Valkey
            | crate::profile::DatabaseKind::MongoDb
            | crate::profile::DatabaseKind::Elasticsearch
            | crate::profile::DatabaseKind::OpenSearch => None,
            _ => Some(Self::for_database_kind(kind)),
        }
    }

    pub const fn for_database_kind(kind: crate::profile::DatabaseKind) -> Self {
        match kind {
            crate::profile::DatabaseKind::Postgres | crate::profile::DatabaseKind::CockroachDb => {
                Self::Postgres
            }
            crate::profile::DatabaseKind::MySql | crate::profile::DatabaseKind::TiDb => Self::MySql,
            crate::profile::DatabaseKind::MariaDb => Self::MariaDb,
            crate::profile::DatabaseKind::SqlServer => Self::SqlServer,
            crate::profile::DatabaseKind::Sqlite => Self::Sqlite,
            crate::profile::DatabaseKind::DuckDb => Self::DuckDb,
            crate::profile::DatabaseKind::Oracle => Self::Oracle,
            crate::profile::DatabaseKind::Redis => Self::Generic,
            crate::profile::DatabaseKind::Valkey => Self::Generic,
            crate::profile::DatabaseKind::MongoDb
            | crate::profile::DatabaseKind::Elasticsearch
            | crate::profile::DatabaseKind::OpenSearch => Self::Generic,
            crate::profile::DatabaseKind::ClickHouse => Self::ClickHouse,
            crate::profile::DatabaseKind::Snowflake => Self::Snowflake,
        }
    }
}

pub(crate) fn parser_dialect(dialect: SqlDialect) -> &'static dyn sqlparser::dialect::Dialect {
    match dialect {
        SqlDialect::Postgres => &sqlparser::dialect::PostgreSqlDialect {},
        SqlDialect::MySql | SqlDialect::MariaDb => &sqlparser::dialect::MySqlDialect {},
        SqlDialect::SqlServer => &sqlparser::dialect::MsSqlDialect {},
        SqlDialect::Sqlite => &sqlparser::dialect::SQLiteDialect {},
        SqlDialect::DuckDb => &sqlparser::dialect::DuckDbDialect {},
        SqlDialect::Generic => &sqlparser::dialect::GenericDialect {},
        SqlDialect::Oracle => &sqlparser::dialect::GenericDialect {},
        SqlDialect::ClickHouse => &sqlparser::dialect::ClickHouseDialect {},
        SqlDialect::Snowflake => &sqlparser::dialect::SnowflakeDialect {},
    }
}

#[cfg(test)]
mod tests {
    use super::{SqlDialect, parser_dialect};
    use sqlparser::parser::Parser;

    #[test]
    fn clickhouse_dialect_parses_native_select_clauses() {
        let sql = [
            "SELECT number FROM numbers(4) ARRAY JOIN items AS item",
            "SELECT number FROM numbers(4) PREWHERE number > 0 LIMIT 1 BY number",
            "SELECT arrayMap(x -> x + 1, [1, 2])",
        ];
        for statement in sql {
            let parsed = Parser::parse_sql(parser_dialect(SqlDialect::ClickHouse), statement);
            assert!(
                parsed.is_ok(),
                "ClickHouse dialect rejected {statement}: {parsed:?}"
            );
        }
    }
}
