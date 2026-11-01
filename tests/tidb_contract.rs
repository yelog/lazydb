use lazydb::{
    db::{
        capabilities::DatabaseCapabilities,
        mutation::{self, EditDisabledReason, MetadataFingerprint, MutationCapability},
    },
    model::execution_target::ExecutionTarget,
    profile::{CatalogScope, CatalogSelection, DatabaseKind, DatabaseScope, import_connection_url},
    sql::{SqlDialect, bounded_query},
};

#[test]
fn tidb_static_capabilities_do_not_enable_unverified_workflows() {
    assert_eq!(
        DatabaseCapabilities::for_kind(DatabaseKind::TiDb),
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
fn tidb_grid_writes_remain_disabled_even_with_a_primary_key() {
    let metadata = MetadataFingerprint {
        relation: "items".into(),
        columns: vec![("id".into(), "bigint".into(), false)],
        primary_key: vec!["id".into()],
    };
    let capabilities = mutation::relation_mutation_capabilities(DatabaseKind::TiDb, &metadata);
    assert_eq!(
        capabilities.insert,
        MutationCapability::Unavailable(EditDisabledReason::UnsupportedDatabase)
    );
    assert_eq!(
        capabilities.update,
        MutationCapability::Unavailable(EditDisabledReason::UnsupportedDatabase)
    );
    assert_eq!(
        capabilities.delete,
        MutationCapability::Unavailable(EditDisabledReason::UnsupportedDatabase)
    );
}

#[test]
fn tidb_execution_targets_switch_database_without_creating_a_separate_schema() {
    let mut profile = import_connection_url("tidb://root@localhost/app", None)
        .unwrap()
        .profile;
    profile.catalog_scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![
            DatabaseScope {
                name: "app".into(),
                schemas: CatalogSelection::All,
            },
            DatabaseScope {
                name: "analytics".into(),
                schemas: CatalogSelection::All,
            },
        ]),
    };
    let mut target = ExecutionTarget::from_profile(&profile);
    assert_eq!(target.schema.as_deref(), Some("app"));
    assert!(target.is_valid(&profile));
    target.database = "analytics".into();
    assert!(!target.is_valid(&profile));
    target.schema = Some("analytics".into());
    let configured = target.apply_to_profile(&profile).unwrap();
    assert_eq!(configured.kind, DatabaseKind::TiDb);
    assert_eq!(configured.database.as_deref(), Some("analytics"));
    assert_eq!(configured.default_schema.as_deref(), Some("analytics"));
    target.database = "excluded".into();
    target.schema = Some("excluded".into());
    assert!(!target.is_valid(&profile));
}

#[test]
fn tidb_uses_mysql_parser_for_read_only_query_paging() {
    let dialect = SqlDialect::for_database_kind(DatabaseKind::TiDb);
    assert_eq!(dialect, SqlDialect::MySql);
    let query = bounded_query("SELECT `id` FROM `items`", dialect).unwrap();
    assert!(query.contains("LIMIT"));
    assert!(query.contains("`items`"));
    assert!(bounded_query("UPDATE `items` SET `id` = 1", dialect).is_none());
}
