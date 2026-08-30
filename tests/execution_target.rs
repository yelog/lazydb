use lazydb::{
    model::execution_target::ExecutionTarget,
    profile::{
        ConnectionProfile, ConnectionUrlFormat, CredentialPolicy, DatabaseKind, Environment,
        ProfileAccess, SslMode,
    },
};
use std::path::PathBuf;
use uuid::Uuid;

fn profile(kind: DatabaseKind) -> ConnectionProfile {
    let id = Uuid::new_v4();
    ConnectionProfile {
        id,
        name: "target".into(),
        access: ProfileAccess::Global,
        kind,
        url_format: ConnectionUrlFormat::default_for(kind),
        host: Some("localhost".into()),
        port: None,
        user: Some("user".into()),
        database: Some("app".into()),
        default_schema: Some("public".into()),
        sqlite_path: (kind == DatabaseKind::Sqlite).then(|| PathBuf::from("app.db")),
        ssl_mode: SslMode::Prefer,
        credential_policy: CredentialPolicy::None,
        read_only: false,
        environment: Environment::Development,
        catalog_scope: lazydb::profile::CatalogScope::for_profile(
            kind,
            "app",
            Some(if kind == DatabaseKind::Sqlite {
                "main"
            } else {
                "public"
            }),
        ),
    }
}

#[test]
fn profile_defaults_produce_backend_valid_targets() {
    for kind in [
        DatabaseKind::Postgres,
        DatabaseKind::MySql,
        DatabaseKind::Sqlite,
    ] {
        let profile = profile(kind);
        let target = ExecutionTarget::from_profile(&profile);
        assert!(target.is_valid(&profile), "{kind:?}: {target:?}");
    }
}

#[test]
fn mysql_schema_is_the_selected_database() {
    let profile = profile(DatabaseKind::MySql);
    let target = ExecutionTarget::from_profile(&profile);
    assert_eq!(target.schema.as_deref(), Some("app"));
}

#[test]
fn sqlite_rejects_unknown_schema_aliases() {
    let profile = profile(DatabaseKind::Sqlite);
    let mut target = ExecutionTarget::from_profile(&profile);
    target.schema = Some("attached".into());
    assert!(!target.is_valid(&profile));
}

#[test]
fn target_profile_identity_is_part_of_execution_target_equality() {
    let first = profile(DatabaseKind::Sqlite);
    let mut second = profile(DatabaseKind::Sqlite);
    second.database = first.database.clone();
    let first_target = ExecutionTarget::from_profile(&first);
    let second_target = ExecutionTarget::from_profile(&second);
    assert_ne!(first_target, second_target);
}

#[test]
fn postgres_target_validation_enforces_database_and_schema_scope() {
    let mut profile = profile(DatabaseKind::Postgres);
    profile.catalog_scope = lazydb::profile::CatalogScope {
        databases: lazydb::profile::CatalogSelection::Selected(vec![
            lazydb::profile::DatabaseScope {
                name: "app".into(),
                schemas: lazydb::profile::CatalogSelection::Selected(vec!["public".into()]),
            },
        ]),
    };
    let mut target = ExecutionTarget::from_profile(&profile);
    assert!(target.is_valid(&profile));

    target.schema = Some("private".into());
    assert!(!target.is_valid(&profile));
    target.schema = Some("public".into());
    target.database = "other".into();
    assert!(!target.is_valid(&profile));
}

#[test]
fn target_override_changes_server_namespace_but_not_sqlite_file() {
    let mut postgres = profile(DatabaseKind::Postgres);
    postgres.catalog_scope = lazydb::profile::CatalogScope {
        databases: lazydb::profile::CatalogSelection::All,
    };
    let target = ExecutionTarget {
        profile_id: postgres.id,
        database: "analytics".into(),
        schema: Some("audit".into()),
    };
    let configured = target.apply_to_profile(&postgres).unwrap();
    assert_eq!(configured.database.as_deref(), Some("analytics"));
    assert_eq!(configured.default_schema.as_deref(), Some("audit"));

    let mut sqlite = profile(DatabaseKind::Sqlite);
    sqlite.catalog_scope = lazydb::profile::CatalogScope {
        databases: lazydb::profile::CatalogSelection::All,
    };
    let alias = ExecutionTarget {
        profile_id: sqlite.id,
        database: "app".into(),
        schema: Some("attached".into()),
    };
    let configured = alias.apply_to_profile(&sqlite).unwrap();
    assert_eq!(configured.sqlite_path, sqlite.sqlite_path);
    assert_eq!(configured.database, sqlite.database);
}
