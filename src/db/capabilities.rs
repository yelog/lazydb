use crate::profile::DatabaseKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractionModel {
    Relational,
    KeyValue,
    Document,
    Search,
}

pub const fn interaction_model(kind: DatabaseKind) -> InteractionModel {
    match kind {
        DatabaseKind::Redis | DatabaseKind::Valkey => InteractionModel::KeyValue,
        DatabaseKind::MongoDb => InteractionModel::Document,
        DatabaseKind::Elasticsearch | DatabaseKind::OpenSearch => InteractionModel::Search,
        _ => InteractionModel::Relational,
    }
}

/// Static capabilities describe what an adapter can expose before connecting.
/// Permission errors and server-version gates remain runtime results.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DatabaseCapabilities {
    pub catalog: bool,
    pub relation_ddl: bool,
    pub relation_edit: bool,
    pub monitoring: bool,
    pub manual_transactions: bool,
    pub cancellation: bool,
}

impl DatabaseCapabilities {
    pub const fn for_kind(kind: DatabaseKind) -> Self {
        match kind {
            DatabaseKind::Postgres => Self {
                catalog: true,
                relation_ddl: true,
                relation_edit: true,
                monitoring: true,
                manual_transactions: true,
                cancellation: true,
            },
            DatabaseKind::CockroachDb => Self {
                catalog: true,
                relation_ddl: true,
                relation_edit: false,
                monitoring: false,
                manual_transactions: true,
                cancellation: true,
            },
            DatabaseKind::MySql => Self {
                catalog: true,
                relation_ddl: true,
                relation_edit: false,
                monitoring: true,
                manual_transactions: true,
                cancellation: true,
            },
            DatabaseKind::MariaDb => Self {
                catalog: true,
                relation_ddl: true,
                relation_edit: false,
                monitoring: true,
                manual_transactions: true,
                cancellation: true,
            },
            DatabaseKind::TiDb => Self {
                catalog: true,
                relation_ddl: true,
                relation_edit: false,
                monitoring: false,
                manual_transactions: false,
                cancellation: false,
            },
            DatabaseKind::SqlServer => Self {
                catalog: true,
                relation_ddl: true,
                relation_edit: false,
                monitoring: false,
                manual_transactions: true,
                cancellation: true,
            },
            DatabaseKind::Sqlite => Self {
                catalog: true,
                relation_ddl: true,
                relation_edit: false,
                monitoring: false,
                manual_transactions: true,
                cancellation: true,
            },
            DatabaseKind::DuckDb => Self {
                catalog: true,
                relation_ddl: true,
                relation_edit: true,
                monitoring: false,
                manual_transactions: true,
                cancellation: true,
            },
            DatabaseKind::Oracle => Self {
                catalog: true,
                relation_ddl: true,
                relation_edit: false,
                monitoring: false,
                manual_transactions: true,
                cancellation: false,
            },
            DatabaseKind::Redis | DatabaseKind::Valkey => Self {
                catalog: false,
                relation_ddl: false,
                relation_edit: false,
                monitoring: false,
                manual_transactions: false,
                cancellation: false,
            },
            DatabaseKind::MongoDb => Self {
                catalog: true,
                relation_ddl: false,
                relation_edit: false,
                monitoring: false,
                manual_transactions: false,
                cancellation: false,
            },
            DatabaseKind::Elasticsearch | DatabaseKind::OpenSearch => Self {
                catalog: true,
                relation_ddl: false,
                relation_edit: false,
                monitoring: false,
                manual_transactions: false,
                cancellation: true,
            },
            DatabaseKind::Snowflake => Self {
                catalog: true,
                relation_ddl: true,
                relation_edit: false,
                monitoring: false,
                manual_transactions: false,
                cancellation: false,
            },
            DatabaseKind::ClickHouse => Self {
                catalog: true,
                relation_ddl: false,
                relation_edit: false,
                monitoring: true,
                manual_transactions: false,
                cancellation: false,
            },
        }
    }
}
