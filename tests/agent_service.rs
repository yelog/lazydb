use std::{fs, sync::Arc};

use async_trait::async_trait;
use lazydb::agent::policy::WritePolicy;
use lazydb::{
    agent::{context::AgentProjectContext, selection::AgentErrorCode, service::AgentService},
    persistence::{
        credentials::CredentialResolver,
        local_credentials::LocalCredentialStore,
        secrets::{SecretStore, SecretStoreError},
    },
    profile::{ConnectionProfile, Environment, ProfileAccess, import_connection_url},
};
use secrecy::SecretString;
use tempfile::TempDir;
use uuid::Uuid;

#[derive(Default)]
struct EmptySecrets;

#[async_trait]
impl SecretStore for EmptySecrets {
    async fn available(&self) -> Result<(), SecretStoreError> {
        Ok(())
    }
    async fn get(&self, _id: Uuid) -> Result<Option<SecretString>, SecretStoreError> {
        Ok(None)
    }
    async fn set(&self, _id: Uuid, _value: &SecretString) -> Result<(), SecretStoreError> {
        Ok(())
    }
    async fn delete(&self, _id: Uuid) -> Result<(), SecretStoreError> {
        Ok(())
    }
}

fn service(temp: &TempDir, profiles: Vec<ConnectionProfile>) -> AgentService {
    let context = AgentProjectContext::resolve(Some(temp.path())).unwrap();
    let resolver = CredentialResolver::new(
        Arc::new(EmptySecrets),
        LocalCredentialStore::new(temp.path().join("credential.key"), "test"),
    );
    AgentService::new(context, profiles, resolver).with_limits(2, 1024 * 1024)
}

fn sqlite_profile(path: &std::path::Path, name: &str) -> ConnectionProfile {
    import_connection_url(&format!("sqlite://{}", path.display()), Some(name))
        .unwrap()
        .profile
}

#[test]
fn connection_projection_never_exposes_sql_server_credentials() {
    let mut profile = import_connection_url("sqlserver://db.example.test/app", Some("mssql"))
        .unwrap()
        .profile;
    profile.user = Some("sa".into());
    profile.credential_policy = lazydb::profile::CredentialPolicy::Prompt;
    let temp = TempDir::new().unwrap();
    let service = service(&temp, vec![profile]);
    let serialized = serde_json::to_string(&service.connections()).unwrap();

    assert!(serialized.contains("db.example.test"));
    assert!(serialized.contains("sa"));
    assert!(!serialized.contains("password"));
    assert!(!serialized.contains("credential_policy"));
}

#[tokio::test]
async fn context_lists_current_project_before_global_and_excludes_other_project() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut current = import_connection_url(":memory:", Some("current"))
        .unwrap()
        .profile;
    current.access = ProfileAccess::Projects { roots: vec![root] };
    let global = import_connection_url(":memory:", Some("global"))
        .unwrap()
        .profile;
    let mut other = import_connection_url(":memory:", Some("other"))
        .unwrap()
        .profile;
    other.access = ProfileAccess::Projects {
        roots: vec![temp.path().join("other")],
    };

    let service = service(&temp, vec![current, global, other]);
    let names = service
        .connections()
        .into_iter()
        .map(|profile| profile.name)
        .collect::<Vec<_>>();
    assert_eq!(names, ["current", "global"]);
}

#[tokio::test]
async fn query_returns_typed_rows_and_target_identity() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    let db = temp.path().join("app.db");
    let profile = sqlite_profile(&db, "app-dev");
    let setup = lazydb::db::DatabaseConnection::connect(&profile, None)
        .await
        .unwrap();
    setup.execute("CREATE TABLE users (id INTEGER, name TEXT); INSERT INTO users VALUES (1, 'Ada'), (2, NULL);").await.unwrap();
    setup.close().await;

    let service = service(&temp, vec![profile]);
    let result = service
        .query(None, "SELECT id, name FROM users ORDER BY id")
        .await
        .unwrap();
    assert_eq!(result.target.connection.name, "app-dev");
    assert_eq!(
        result.target.connection.environment,
        lazydb::profile::Environment::Development
    );
    assert_eq!(result.outcome.row_count, 2);
    assert!(matches!(
        result.outcome.result_sets[0].rows[1][1],
        lazydb::db::value::CellValue::Null
    ));
    assert!(!result.outcome.truncated);
}

#[tokio::test]
async fn query_rejects_write_before_database_io() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    let db = temp.path().join("missing.db");
    let service = service(&temp, vec![sqlite_profile(&db, "app-dev")]);
    let error = service
        .query(None, "UPDATE users SET name = 'x'")
        .await
        .unwrap_err();
    assert!(error.message.contains("query rejected"));
    assert!(!db.exists());
}

#[tokio::test]
async fn execute_reports_server_policy_denial_before_database_io() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    let db = temp.path().join("missing.db");
    let service = service(&temp, vec![sqlite_profile(&db, "app-dev")]);

    let error = service
        .execute(None, "UPDATE users SET name = 'x'", WritePolicy::Deny)
        .await
        .unwrap_err();

    assert_eq!(error.code, AgentErrorCode::PolicyDenied);
    assert_eq!(
        error.message,
        "write rejected: the MCP server write policy is deny; restart it with --write-policy non-production for writable development or staging connections"
    );
    assert!(!error.message.contains("ServerWritePolicyDenied"));
    assert!(!db.exists());
}

#[tokio::test]
async fn execute_reports_read_only_profile_before_database_io() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    let db = temp.path().join("missing.db");
    let mut profile = sqlite_profile(&db, "app-dev");
    profile.read_only = true;
    profile.environment = Environment::Development;
    let service = service(&temp, vec![profile]);

    let error = service
        .execute(None, "UPDATE users SET name = 'x'", WritePolicy::All)
        .await
        .unwrap_err();

    assert_eq!(
        error.message,
        "write rejected: the selected connection is configured as read-only"
    );
    assert!(!db.exists());
}
