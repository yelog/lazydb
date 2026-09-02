use lazydb::{
    agent::policy::{
        PolicyError, WriteCapability, WritePolicy, authorize_query, authorize_write,
        write_capability,
    },
    profile::{Environment, import_connection_url},
};

fn profile(read_only: bool, environment: Environment) -> lazydb::profile::ConnectionProfile {
    let mut profile = import_connection_url(":memory:", Some("test"))
        .unwrap()
        .profile;
    profile.read_only = read_only;
    profile.environment = environment;
    profile
}

#[test]
fn query_path_accepts_only_single_read_only_statements() {
    let profile = profile(false, Environment::Development);
    assert_eq!(authorize_query(&profile, "SELECT 1"), Ok(()));
    assert_eq!(
        authorize_query(&profile, "SELECT 1; SELECT 2"),
        Err(PolicyError::ReadOnlyQueryRequired)
    );
    assert_eq!(
        authorize_query(&profile, "UPDATE users SET name = 'x'"),
        Err(PolicyError::ReadOnlyQueryRequired)
    );
}

#[test]
fn writes_obey_profile_environment_and_server_policy() {
    let read_only = profile(true, Environment::Development);
    assert_eq!(
        authorize_write(&read_only, WritePolicy::All, "UPDATE t SET x = 1"),
        Err(PolicyError::ProfileReadOnly)
    );

    let development = profile(false, Environment::Development);
    assert_eq!(
        authorize_write(&development, WritePolicy::Deny, "UPDATE t SET x = 1"),
        Err(PolicyError::ServerWritePolicyDenied)
    );
    assert_eq!(
        PolicyError::ServerWritePolicyDenied.to_string(),
        "the MCP server write policy is deny; restart it with --write-policy non-production for writable development or staging connections"
    );
    assert_eq!(
        PolicyError::ProfileReadOnly.to_string(),
        "the selected connection is configured as read-only"
    );
    assert_eq!(
        PolicyError::ProductionWriteDisabled.to_string(),
        "production writes require --write-policy all"
    );
    assert_eq!(
        authorize_write(
            &development,
            WritePolicy::NonProduction,
            "UPDATE t SET x = 1"
        ),
        Ok(())
    );

    let production = profile(false, Environment::Production);
    assert_eq!(
        authorize_write(
            &production,
            WritePolicy::NonProduction,
            "UPDATE t SET x = 1"
        ),
        Err(PolicyError::ProductionWriteDisabled)
    );
    assert_eq!(
        authorize_write(&production, WritePolicy::All, "UPDATE t SET x = 1"),
        Ok(())
    );
}

#[test]
fn writes_reject_unknown_and_transaction_control_sql() {
    let profile = profile(false, Environment::Development);
    assert_eq!(
        authorize_write(&profile, WritePolicy::All, "CALL do_work()"),
        Err(PolicyError::UnknownSql)
    );
    assert_eq!(
        authorize_write(&profile, WritePolicy::All, "BEGIN"),
        Err(PolicyError::TransactionControlDisabled)
    );
    assert_eq!(
        authorize_write(&profile, WritePolicy::All, ""),
        Err(PolicyError::EmptySql)
    );
}

#[test]
fn effective_write_capability_matches_profile_and_server_policy() {
    let development = profile(false, Environment::Development);
    assert_eq!(
        write_capability(&development, WritePolicy::Deny),
        WriteCapability::Denied(PolicyError::ServerWritePolicyDenied)
    );
    assert_eq!(
        write_capability(&development, WritePolicy::NonProduction),
        WriteCapability::Allowed
    );

    let read_only = profile(true, Environment::Development);
    assert_eq!(
        write_capability(&read_only, WritePolicy::All),
        WriteCapability::Denied(PolicyError::ProfileReadOnly)
    );

    let production = profile(false, Environment::Production);
    assert_eq!(
        write_capability(&production, WritePolicy::NonProduction),
        WriteCapability::Denied(PolicyError::ProductionWriteDisabled)
    );
    assert_eq!(
        write_capability(&production, WritePolicy::All),
        WriteCapability::Allowed
    );
}
