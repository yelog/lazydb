use lazydb::{db::capabilities::DatabaseCapabilities, profile::DatabaseKind};

#[test]
fn static_capabilities_match_the_current_adapter_contract() {
    assert_eq!(
        DatabaseCapabilities::for_kind(DatabaseKind::Postgres),
        DatabaseCapabilities {
            catalog: true,
            relation_ddl: true,
            relation_edit: true,
            monitoring: true,
            manual_transactions: true,
            cancellation: true,
        }
    );
    assert!(!DatabaseCapabilities::for_kind(DatabaseKind::MySql).relation_edit);
    assert!(!DatabaseCapabilities::for_kind(DatabaseKind::SqlServer).monitoring);
    assert!(!DatabaseCapabilities::for_kind(DatabaseKind::Sqlite).monitoring);
}

#[test]
fn capability_fields_are_conservative_for_features_not_implemented_by_adapters() {
    for kind in [
        DatabaseKind::MySql,
        DatabaseKind::SqlServer,
        DatabaseKind::Sqlite,
    ] {
        let capabilities = DatabaseCapabilities::for_kind(kind);
        assert!(capabilities.catalog);
        assert!(capabilities.relation_ddl);
        assert!(!capabilities.relation_edit);
    }
}

#[test]
fn cockroach_enables_browsing_and_transactions_without_structured_writes() {
    assert_eq!(
        DatabaseCapabilities::for_kind(DatabaseKind::CockroachDb),
        DatabaseCapabilities {
            catalog: true,
            relation_ddl: true,
            relation_edit: false,
            monitoring: false,
            manual_transactions: true,
            cancellation: true,
        }
    );
}

#[test]
fn clickhouse_catalog_capability_is_enabled_with_the_native_adapter() {
    assert_eq!(
        DatabaseCapabilities::for_kind(DatabaseKind::ClickHouse),
        DatabaseCapabilities {
            catalog: true,
            relation_ddl: false,
            relation_edit: false,
            monitoring: true,
            manual_transactions: false,
            cancellation: false,
        }
    );
}

#[test]
fn snowflake_advertises_only_implemented_sql_api_workflows() {
    assert_eq!(
        DatabaseCapabilities::for_kind(DatabaseKind::Snowflake),
        DatabaseCapabilities {
            catalog: true,
            relation_ddl: true,
            relation_edit: false,
            monitoring: false,
            manual_transactions: false,
            cancellation: false,
        }
    );
}

#[test]
fn valkey_has_the_same_non_relational_static_capabilities_as_redis() {
    assert_eq!(
        DatabaseCapabilities::for_kind(DatabaseKind::Valkey),
        DatabaseCapabilities::for_kind(DatabaseKind::Redis)
    );
}

#[test]
fn opensearch_exposes_search_workflows_without_sql_controls() {
    use lazydb::db::capabilities::{InteractionModel, interaction_model};
    assert_eq!(
        interaction_model(DatabaseKind::OpenSearch),
        InteractionModel::Search
    );
    assert_eq!(
        DatabaseCapabilities::for_kind(DatabaseKind::OpenSearch),
        DatabaseCapabilities {
            catalog: true,
            relation_ddl: false,
            relation_edit: false,
            monitoring: false,
            manual_transactions: false,
            cancellation: true,
        }
    );
    assert_eq!(
        DatabaseCapabilities::for_kind(DatabaseKind::OpenSearch),
        DatabaseCapabilities::for_kind(DatabaseKind::Elasticsearch)
    );
}
