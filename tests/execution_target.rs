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
        group_id: None,
        kind,
        url_format: ConnectionUrlFormat::default_for(kind),
        host: Some("localhost".into()),
        port: None,
        user: Some("user".into()),
        database: Some("app".into()),
        default_schema: Some("public".into()),
        sqlite_path: (kind == DatabaseKind::Sqlite).then(|| PathBuf::from("app.db")),
        mongodb_options: None,
        pg_options: None,
        snowflake_options: None,
        elasticsearch_options: None,
        duckdb_path: None,
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
        DatabaseKind::CockroachDb,
        DatabaseKind::MySql,
        DatabaseKind::SqlServer,
        DatabaseKind::Sqlite,
        DatabaseKind::DuckDb,
    ] {
        let mut profile = profile(kind);
        if kind == DatabaseKind::DuckDb {
            profile.database = Some("memory".into());
            profile.default_schema = Some("main".into());
            profile.catalog_scope =
                lazydb::profile::CatalogScope::for_profile(kind, "memory", Some("main"));
        }
        let target = ExecutionTarget::from_profile(&profile);
        assert!(target.is_valid(&profile), "{kind:?}: {target:?}");
    }
}

#[test]
fn duckdb_execution_target_uses_catalog_and_schema_not_file_path() {
    let mut profile = profile(DatabaseKind::DuckDb);
    profile.duckdb_path = Some(PathBuf::from("analytics.duckdb"));
    profile.database = Some("memory".into());
    profile.default_schema = Some("main".into());
    profile.catalog_scope =
        lazydb::profile::CatalogScope::for_profile(DatabaseKind::DuckDb, "memory", Some("main"));
    let target = ExecutionTarget::from_profile(&profile);
    assert_eq!(target.database, "memory");
    assert_eq!(target.schema.as_deref(), Some("main"));
    assert!(target.is_valid(&profile));
}

#[test]
fn cockroach_target_switch_preserves_tls_and_rejects_excluded_namespaces() {
    use lazydb::profile::{CatalogScope, CatalogSelection, DatabaseScope, import_connection_url};
    let mut profile = import_connection_url(
        "cockroachdb://root@db/app?sslmode=verify-full&sslrootcert=ca.crt&sslkey=client.key&options=--cluster%3Dtest",
        None,
    ).unwrap().profile;
    profile.catalog_scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![
            DatabaseScope {
                name: "app".into(),
                schemas: CatalogSelection::All,
            },
            DatabaseScope {
                name: "other".into(),
                schemas: CatalogSelection::Selected(vec!["tools".into()]),
            },
        ]),
    };
    let target = ExecutionTarget {
        profile_id: profile.id,
        database: "other".into(),
        schema: Some("tools".into()),
    };
    let configured = target.apply_to_profile(&profile).unwrap();
    assert_eq!(configured.kind, DatabaseKind::CockroachDb);
    assert_eq!(configured.database.as_deref(), Some("other"));
    assert_eq!(configured.default_schema.as_deref(), Some("tools"));
    assert_eq!(configured.pg_options, profile.pg_options);
    assert_eq!(configured.ssl_mode, SslMode::VerifyFull);
    let excluded = ExecutionTarget {
        schema: Some("public".into()),
        ..target
    };
    assert!(excluded.apply_to_profile(&profile).is_none());
}

#[test]
fn cockroach_database_selector_includes_saved_scope_before_catalog_is_loaded() {
    use lazydb::{
        action::Action,
        app::App,
        model::workspace::{ConnectionStatus, Overlay},
        profile::{CatalogScope, CatalogSelection, DatabaseScope},
    };
    let mut profile = profile(DatabaseKind::CockroachDb);
    profile.catalog_scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![
            DatabaseScope {
                name: "app".into(),
                schemas: CatalogSelection::All,
            },
            DatabaseScope {
                name: "other".into(),
                schemas: CatalogSelection::All,
            },
        ]),
    };
    let mut app = App::new(Vec::new());
    app.connection.profile_id = Some(profile.id);
    app.connection.generation = 1;
    app.connection.status = ConnectionStatus::Connected;
    app.connection.target = Some(ExecutionTarget::from_profile(&profile));
    app.profiles.push(profile);
    app.update(Action::OpenDatabaseSelector);
    let Some(Overlay::DatabaseSelector(selector)) = app.overlay.as_ref() else {
        panic!("database selector must open");
    };
    assert_eq!(
        selector
            .candidates
            .iter()
            .map(|target| target.database.as_str())
            .collect::<Vec<_>>(),
        vec!["app", "other"]
    );
    assert!(
        selector
            .candidates
            .iter()
            .all(|target| target.schema.is_none())
    );
}

#[test]
fn mysql_schema_is_the_selected_database() {
    let profile = profile(DatabaseKind::MySql);
    let target = ExecutionTarget::from_profile(&profile);
    assert_eq!(target.schema.as_deref(), Some("app"));
}

#[test]
fn clickhouse_target_uses_database_as_read_only_schema_alias() {
    let mut profile = profile(DatabaseKind::ClickHouse);
    profile.default_schema = Some("app".into());
    let target = ExecutionTarget::from_profile(&profile);
    assert_eq!(target.schema.as_deref(), Some("app"));
    assert!(target.is_valid(&profile));

    let mut different_schema = target.clone();
    different_schema.schema = Some("other".into());
    assert!(!different_schema.is_valid(&profile));

    let configured = target.apply_to_profile(&profile).unwrap();
    assert_eq!(configured.database.as_deref(), Some("app"));
    assert_eq!(configured.default_schema.as_deref(), Some("app"));
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
fn sql_server_target_validation_and_override_match_postgres_namespaces() {
    let mut profile = profile(DatabaseKind::SqlServer);
    profile.default_schema = Some("dbo".into());
    profile.catalog_scope = lazydb::profile::CatalogScope {
        databases: lazydb::profile::CatalogSelection::Selected(vec![
            lazydb::profile::DatabaseScope {
                name: "app".into(),
                schemas: lazydb::profile::CatalogSelection::Selected(vec!["dbo".into()]),
            },
            lazydb::profile::DatabaseScope {
                name: "analytics".into(),
                schemas: lazydb::profile::CatalogSelection::Selected(vec!["reporting".into()]),
            },
        ]),
    };

    let mut target = ExecutionTarget::from_profile(&profile);
    assert_eq!(target.schema.as_deref(), Some("dbo"));
    assert!(target.is_valid(&profile));
    target.database = "analytics".into();
    target.schema = Some("reporting".into());
    assert!(target.is_valid(&profile));

    let configured = target.apply_to_profile(&profile).unwrap();
    assert_eq!(configured.database.as_deref(), Some("analytics"));
    assert_eq!(configured.default_schema.as_deref(), Some("reporting"));

    target.schema = Some("private".into());
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

#[test]
fn snowflake_saved_profile_and_schema_less_targets_use_public() {
    use lazydb::profile::{CatalogScope, CatalogSelection, DatabaseScope};
    let mut profile = profile(DatabaseKind::Snowflake);
    profile.default_schema = None;
    profile.catalog_scope =
        CatalogScope::for_profile(DatabaseKind::Snowflake, "app", Some("PUBLIC"));
    let mut target = ExecutionTarget::from_profile(&profile);
    assert_eq!(target.schema.as_deref(), Some("PUBLIC"));
    assert!(target.is_valid(&profile));
    assert_eq!(
        target
            .apply_to_profile(&profile)
            .unwrap()
            .default_schema
            .as_deref(),
        Some("PUBLIC")
    );

    // Saved targets and database explorer selections may have no schema.
    target.schema = None;
    assert!(target.is_valid(&profile));
    let configured = target.apply_to_profile(&profile).unwrap();
    assert_eq!(configured.default_schema.as_deref(), Some("PUBLIC"));
    assert_eq!(
        ExecutionTarget::from_profile(&configured).schema.as_deref(),
        Some("PUBLIC")
    );

    // An omitted schema must not bypass a selection excluding PUBLIC.
    profile.catalog_scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: "app".into(),
            schemas: CatalogSelection::Selected(vec!["RAW".into()]),
        }]),
    };
    assert!(!target.is_valid(&profile));
    assert!(target.apply_to_profile(&profile).is_none());
    target.schema = Some("RAW".into());
    assert!(target.is_valid(&profile));
    assert_eq!(
        target
            .apply_to_profile(&profile)
            .unwrap()
            .default_schema
            .as_deref(),
        Some("RAW")
    );
    target.database = "OTHER".into();
    assert!(!target.is_valid(&profile));
}

#[test]
fn snowflake_execution_target_keeps_database_schema_and_session_options() {
    let mut profile = lazydb::profile::import_connection_url(
        "snowflake://alice@org-account.snowflakecomputing.com/DB?schema=RAW&warehouse=WH&role=READER", None,
    ).unwrap().profile;
    profile.catalog_scope.databases = lazydb::profile::CatalogSelection::All;
    let mut target = ExecutionTarget::from_profile(&profile);
    assert_eq!(target.database, "DB");
    assert_eq!(target.schema.as_deref(), Some("RAW"));
    target.database = "OTHER_DB".into();
    target.schema = Some("OTHER_SCHEMA".into());
    let configured = target.apply_to_profile(&profile).unwrap();
    assert_eq!(configured.database.as_deref(), Some("OTHER_DB"));
    assert_eq!(configured.default_schema.as_deref(), Some("OTHER_SCHEMA"));
    assert_eq!(configured.snowflake_options, profile.snowflake_options);
}
