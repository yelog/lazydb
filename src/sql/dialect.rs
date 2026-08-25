#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum SqlDialect {
    Postgres,
    MySql,
    Sqlite,
    #[default]
    Generic,
}
