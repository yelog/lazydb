use lazydb::{
    model::execution_target::ExecutionTarget,
    profile::{ConnectionProfile, DatabaseKind, Environment, SslMode},
};
use std::path::PathBuf;
use uuid::Uuid;

fn profile(kind: DatabaseKind) -> ConnectionProfile {
    let id = Uuid::new_v4();
    ConnectionProfile {
        id,
        name: "target".into(),
        kind,
        host: Some("localhost".into()),
        port: None,
        user: Some("user".into()),
        database: Some("app".into()),
        default_schema: Some("public".into()),
        sqlite_path: (kind == DatabaseKind::Sqlite).then(|| PathBuf::from("app.db")),
        ssl_mode: SslMode::Prefer,
        secret_ref: None,
        read_only: false,
        environment: Environment::Development,
        include_databases: Vec::new(),
        include_schemas: Vec::new(),
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
