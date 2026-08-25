use std::{collections::HashMap, sync::Mutex};

use async_trait::async_trait;
use lazydb::persistence::secrets::{
    KEYRING_SERVICE, SecretStore, SecretStoreError, keyring_ref, profile_id_from_ref,
};
use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;

#[test]
fn keyring_references_round_trip_profile_ids() {
    let id = Uuid::new_v4();
    let reference = keyring_ref(id);

    assert_eq!(reference, format!("keyring:{KEYRING_SERVICE}/{id}"));
    assert_eq!(profile_id_from_ref(&reference).unwrap(), id);
}

#[test]
fn keyring_references_require_the_exact_canonical_format() {
    let id = Uuid::new_v4();
    let invalid_references = [
        "env:password".to_owned(),
        format!("keyring:other.service/{id}"),
        format!("keyring:{KEYRING_SERVICE}"),
        format!("keyring:{KEYRING_SERVICE}/{}", id.simple()),
        format!(
            "keyring:{KEYRING_SERVICE}/{}",
            id.to_string().to_uppercase()
        ),
        format!("keyring:{KEYRING_SERVICE}/{id}/extra"),
    ];

    for reference in invalid_references {
        assert_eq!(
            profile_id_from_ref(&reference),
            Err(SecretStoreError::InvalidReference),
            "accepted invalid reference: {reference}"
        );
    }
}

#[derive(Default)]
struct FakeSecretStore {
    values: Mutex<HashMap<Uuid, SecretString>>,
}

#[async_trait]
impl SecretStore for FakeSecretStore {
    async fn available(&self) -> Result<(), SecretStoreError> {
        Ok(())
    }

    async fn get(&self, profile_id: Uuid) -> Result<Option<SecretString>, SecretStoreError> {
        Ok(self.values.lock().unwrap().get(&profile_id).cloned())
    }

    async fn set(&self, profile_id: Uuid, password: &SecretString) -> Result<(), SecretStoreError> {
        self.values
            .lock()
            .unwrap()
            .insert(profile_id, password.clone());
        Ok(())
    }

    async fn delete(&self, profile_id: Uuid) -> Result<(), SecretStoreError> {
        self.values.lock().unwrap().remove(&profile_id);
        Ok(())
    }
}

#[tokio::test]
async fn secret_store_contract_supports_a_fake_without_native_access() {
    let store = FakeSecretStore::default();
    let id = Uuid::new_v4();
    let value = "not-in-the-reference";
    let secret = SecretString::from(value.to_owned());

    store.available().await.unwrap();
    assert!(store.get(id).await.unwrap().is_none());

    store.set(id, &secret).await.unwrap();
    assert_eq!(store.get(id).await.unwrap().unwrap().expose_secret(), value);
    assert!(!keyring_ref(id).contains(value));

    store.delete(id).await.unwrap();
    assert!(store.get(id).await.unwrap().is_none());
    store.delete(id).await.unwrap();
}
