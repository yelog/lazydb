use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use lazydb::{
    persistence::{
        credentials::{CredentialResolutionError, CredentialResolver},
        local_credentials::LocalCredentialStore,
        secrets::{SecretStore, SecretStoreError},
    },
    profile::{CredentialPolicy, import_connection_url},
};
use secrecy::{ExposeSecret, SecretString};
use tempfile::TempDir;
use uuid::Uuid;

#[derive(Default)]
struct FakeSecretStore {
    values: Mutex<HashMap<Uuid, SecretString>>,
}

#[async_trait]
impl SecretStore for FakeSecretStore {
    async fn available(&self) -> Result<(), SecretStoreError> {
        Ok(())
    }
    async fn get(&self, id: Uuid) -> Result<Option<SecretString>, SecretStoreError> {
        Ok(self.values.lock().unwrap().get(&id).cloned())
    }
    async fn set(&self, id: Uuid, value: &SecretString) -> Result<(), SecretStoreError> {
        self.values.lock().unwrap().insert(id, value.clone());
        Ok(())
    }
    async fn delete(&self, id: Uuid) -> Result<(), SecretStoreError> {
        self.values.lock().unwrap().remove(&id);
        Ok(())
    }
}

fn resolver(temp: &TempDir, secrets: Arc<FakeSecretStore>) -> CredentialResolver {
    CredentialResolver::new(
        secrets,
        LocalCredentialStore::new(temp.path().join("credential.key"), "test"),
    )
}

#[tokio::test]
async fn resolves_passwordless_and_rejects_interactive_profiles() {
    let temp = TempDir::new().unwrap();
    let secrets = Arc::new(FakeSecretStore::default());
    let resolver = resolver(&temp, secrets);
    let passwordless = import_connection_url(":memory:", Some("none"))
        .unwrap()
        .profile;
    assert!(
        resolver
            .resolve_headless(&passwordless)
            .await
            .unwrap()
            .is_none()
    );

    let mut prompt = import_connection_url("postgres://localhost/app", Some("prompt"))
        .unwrap()
        .profile;
    prompt.credential_policy = CredentialPolicy::Prompt;
    assert_eq!(
        resolver.resolve_headless(&prompt).await.unwrap_err(),
        CredentialResolutionError::InteractionRequired
    );
}

#[tokio::test]
async fn decrypts_local_credentials_and_rejects_wrong_profile_reference() {
    let temp = TempDir::new().unwrap();
    let secrets = Arc::new(FakeSecretStore::default());
    let local = LocalCredentialStore::new(temp.path().join("credential.key"), "test");
    let mut profile = import_connection_url("postgres://localhost/app", Some("local"))
        .unwrap()
        .profile;
    profile.credential_policy = CredentialPolicy::LocalEncrypted(
        local
            .encrypt(profile.id, &SecretString::from("secret"))
            .unwrap(),
    );
    let resolver = CredentialResolver::new(secrets, local);
    assert_eq!(
        resolver
            .resolve_headless(&profile)
            .await
            .unwrap()
            .unwrap()
            .expose_secret(),
        "secret"
    );

    profile.credential_policy = CredentialPolicy::System(
        "keyring:dev.lazydb.lazydb/00000000-0000-0000-0000-000000000000".into(),
    );
    assert_eq!(
        resolver.resolve_headless(&profile).await.unwrap_err(),
        CredentialResolutionError::InvalidReference
    );
}
