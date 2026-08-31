use std::{fs, sync::Arc};

use async_trait::async_trait;
use lazydb::{
    agent::{
        context::AgentProjectContext,
        policy::{PolicyError, WritePolicy, authorize_query, authorize_write},
        service::AgentService,
    },
    persistence::{
        credentials::CredentialResolver,
        local_credentials::LocalCredentialStore,
        secrets::{SecretStore, SecretStoreError},
    },
    profile::{Environment, ProfileAccess, import_connection_url},
};
use secrecy::SecretString;
use tempfile::TempDir;
use uuid::Uuid;

struct EmptySecrets;
#[async_trait]
impl SecretStore for EmptySecrets {
    async fn available(&self) -> Result<(), SecretStoreError> {
        Ok(())
    }
    async fn get(&self, _: Uuid) -> Result<Option<SecretString>, SecretStoreError> {
        Ok(None)
    }
    async fn set(&self, _: Uuid, _: &SecretString) -> Result<(), SecretStoreError> {
        Ok(())
    }
    async fn delete(&self, _: Uuid) -> Result<(), SecretStoreError> {
        Ok(())
    }
}

#[test]
fn other_project_uuid_cannot_be_selected() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    let other_root = temp.path().join("other");
    fs::create_dir_all(&other_root).unwrap();
    let mut other = import_connection_url(":memory:", Some("other"))
        .unwrap()
        .profile;
    other.access = ProfileAccess::Projects {
        roots: vec![other_root],
    };
    let service = AgentService::new(
        AgentProjectContext::resolve(Some(temp.path())).unwrap(),
        vec![other.clone()],
        CredentialResolver::new(
            Arc::new(EmptySecrets),
            LocalCredentialStore::new(temp.path().join("key"), "test"),
        ),
    );
    let error = service.select(Some(&other.id.to_string())).unwrap_err();
    assert_eq!(
        error.code,
        lazydb::agent::selection::AgentErrorCode::NoVisibleConnections
    );
}

#[test]
fn sql_policy_rejects_nested_writes_and_transaction_control() {
    let mut profile = import_connection_url(":memory:", Some("test"))
        .unwrap()
        .profile;
    profile.environment = Environment::Development;
    assert_eq!(
        authorize_query(&profile, "WITH x AS (UPDATE t SET a = 1) SELECT * FROM x"),
        Err(PolicyError::ReadOnlyQueryRequired)
    );
    assert_eq!(
        authorize_query(&profile, "SELECT * FROM t FOR UPDATE"),
        Err(PolicyError::ReadOnlyQueryRequired)
    );
    assert_eq!(
        authorize_write(&profile, WritePolicy::All, "BEGIN"),
        Err(PolicyError::TransactionControlDisabled)
    );
    assert_eq!(
        authorize_query(&profile, "SELECT * INTO copy FROM source"),
        Err(PolicyError::ReadOnlyQueryRequired)
    );
}
