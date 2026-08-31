use std::{fs, sync::Arc};

use async_trait::async_trait;
use lazydb::{
    agent::{context::AgentProjectContext, service::AgentService},
    db::DatabaseConnection,
    persistence::{
        credentials::CredentialResolver,
        local_credentials::LocalCredentialStore,
        secrets::{SecretStore, SecretStoreError},
    },
    profile::import_connection_url,
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

#[tokio::test]
async fn searches_sqlite_catalog_with_target_identity() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    let db = temp.path().join("catalog.db");
    let profile = import_connection_url(&format!("sqlite://{}", db.display()), Some("catalog"))
        .unwrap()
        .profile;
    let connection = DatabaseConnection::connect(&profile, None).await.unwrap();
    connection
        .execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);")
        .await
        .unwrap();
    connection.close().await;
    let service = AgentService::new(
        AgentProjectContext::resolve(Some(temp.path())).unwrap(),
        vec![profile],
        CredentialResolver::new(
            Arc::new(EmptySecrets),
            LocalCredentialStore::new(temp.path().join("key"), "test"),
        ),
    );

    let result = service
        .search_schema(None, "user".into(), 10)
        .await
        .unwrap();
    assert_eq!(result.target.connection.name, "catalog");
    assert!(result.hits.iter().any(|hit| hit.name == "users"));
}
