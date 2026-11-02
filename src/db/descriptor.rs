use crate::profile::DatabaseKind;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DatabaseCategory {
    Relational,
    NonRelational,
}

impl DatabaseCategory {
    pub const ALL: [Self; 2] = [Self::Relational, Self::NonRelational];
    pub const fn label(self) -> &'static str {
        match self {
            Self::Relational => "Relational",
            Self::NonRelational => "Non-relational",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DatabaseDescriptor {
    pub kind: DatabaseKind,
    pub name: &'static str,
    pub display_name: &'static str,
    pub category: DatabaseCategory,
}

pub const DRIVERS: [DatabaseDescriptor; 16] = [
    DatabaseDescriptor {
        kind: DatabaseKind::Postgres,
        name: "postgres",
        display_name: "PostgreSQL",
        category: DatabaseCategory::Relational,
    },
    DatabaseDescriptor {
        kind: DatabaseKind::MySql,
        name: "mysql",
        display_name: "MySQL",
        category: DatabaseCategory::Relational,
    },
    DatabaseDescriptor {
        kind: DatabaseKind::MariaDb,
        name: "mariadb",
        display_name: "MariaDB",
        category: DatabaseCategory::Relational,
    },
    DatabaseDescriptor {
        kind: DatabaseKind::Oracle,
        name: "oracle",
        display_name: "Oracle",
        category: DatabaseCategory::Relational,
    },
    DatabaseDescriptor {
        kind: DatabaseKind::SqlServer,
        name: "sqlserver",
        display_name: "SQL Server",
        category: DatabaseCategory::Relational,
    },
    DatabaseDescriptor {
        kind: DatabaseKind::Sqlite,
        name: "sqlite",
        display_name: "SQLite",
        category: DatabaseCategory::Relational,
    },
    DatabaseDescriptor {
        kind: DatabaseKind::DuckDb,
        name: "duckdb",
        display_name: "DuckDB",
        category: DatabaseCategory::Relational,
    },
    DatabaseDescriptor {
        kind: DatabaseKind::Redis,
        name: "redis",
        display_name: "Redis",
        category: DatabaseCategory::NonRelational,
    },
    DatabaseDescriptor {
        kind: DatabaseKind::Valkey,
        name: "valkey",
        display_name: "Valkey",
        category: DatabaseCategory::NonRelational,
    },
    DatabaseDescriptor {
        kind: DatabaseKind::MongoDb,
        name: "mongodb",
        display_name: "MongoDB",
        category: DatabaseCategory::NonRelational,
    },
    DatabaseDescriptor {
        kind: DatabaseKind::ClickHouse,
        name: "clickhouse",
        display_name: "ClickHouse",
        category: DatabaseCategory::Relational,
    },
    DatabaseDescriptor {
        kind: DatabaseKind::TiDb,
        name: "tidb",
        display_name: "TiDB",
        category: DatabaseCategory::Relational,
    },
    DatabaseDescriptor {
        kind: DatabaseKind::CockroachDb,
        name: "cockroachdb",
        display_name: "CockroachDB",
        category: DatabaseCategory::Relational,
    },
    DatabaseDescriptor {
        kind: DatabaseKind::Snowflake,
        name: "snowflake",
        display_name: "Snowflake",
        category: DatabaseCategory::Relational,
    },
    DatabaseDescriptor {
        kind: DatabaseKind::Elasticsearch,
        name: "elasticsearch",
        display_name: "Elasticsearch",
        category: DatabaseCategory::NonRelational,
    },
    DatabaseDescriptor {
        kind: DatabaseKind::OpenSearch,
        name: "opensearch",
        display_name: "OpenSearch",
        category: DatabaseCategory::NonRelational,
    },
];

pub const DRIVER_NAMES: [&str; DRIVERS.len()] = [
    DRIVERS[0].name,
    DRIVERS[1].name,
    DRIVERS[2].name,
    DRIVERS[3].name,
    DRIVERS[4].name,
    DRIVERS[5].name,
    DRIVERS[6].name,
    DRIVERS[7].name,
    DRIVERS[8].name,
    DRIVERS[9].name,
    DRIVERS[10].name,
    DRIVERS[11].name,
    DRIVERS[12].name,
    DRIVERS[13].name,
    DRIVERS[14].name,
    DRIVERS[15].name,
];

pub const DRIVER_LIST: &str = "postgres,mysql,mariadb,oracle,sqlserver,sqlite,duckdb,redis,valkey,mongodb,clickhouse,tidb,cockroachdb,snowflake,elasticsearch,opensearch";

pub fn kinds() -> impl Iterator<Item = DatabaseKind> {
    DRIVERS.iter().map(|descriptor| descriptor.kind)
}

pub fn descriptor(kind: DatabaseKind) -> &'static DatabaseDescriptor {
    DRIVERS
        .iter()
        .find(|driver| driver.kind == kind)
        .expect("registered database kind")
}

pub fn drivers_in(category: DatabaseCategory) -> impl Iterator<Item = &'static DatabaseDescriptor> {
    DRIVERS
        .iter()
        .filter(move |driver| driver.category == category)
}
