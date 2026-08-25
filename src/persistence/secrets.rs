use async_trait::async_trait;
use keyring::v1::{Entry, Error as KeyringError};
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;
use tokio::task;
use uuid::Uuid;

pub const KEYRING_SERVICE: &str = "dev.lazydb.lazydb";

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum SecretStoreError {
    #[error("native secret store is unavailable")]
    Unavailable,
    #[error("native secret store is locked")]
    Locked,
    #[error("native secret is missing")]
    Missing,
    #[error("native secret store operation failed")]
    Backend,
    #[error("invalid native secret reference")]
    InvalidReference,
}

#[async_trait]
pub trait SecretStore: Send + Sync {
    async fn available(&self) -> Result<(), SecretStoreError>;
    async fn get(&self, profile_id: Uuid) -> Result<Option<SecretString>, SecretStoreError>;
    async fn set(&self, profile_id: Uuid, password: &SecretString) -> Result<(), SecretStoreError>;
    async fn delete(&self, profile_id: Uuid) -> Result<(), SecretStoreError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NativeSecretStore;

#[async_trait]
impl SecretStore for NativeSecretStore {
    async fn available(&self) -> Result<(), SecretStoreError> {
        run_blocking(|| match Entry::store_status() {
            Ok(()) => Ok(()),
            Err(KeyringError::NoStorageAccess(_)) => Err(SecretStoreError::Locked),
            Err(_) => Err(SecretStoreError::Unavailable),
        })
        .await
    }

    async fn get(&self, profile_id: Uuid) -> Result<Option<SecretString>, SecretStoreError> {
        run_blocking(move || {
            let entry = entry(profile_id)?;
            match entry.get_password() {
                Ok(password) => Ok(Some(SecretString::from(password))),
                Err(KeyringError::NoEntry) => Ok(None),
                Err(error) => Err(classify_keyring_error(&error)),
            }
        })
        .await
    }

    async fn set(&self, profile_id: Uuid, password: &SecretString) -> Result<(), SecretStoreError> {
        let password = password.clone();
        run_blocking(move || {
            entry(profile_id)?
                .set_password(password.expose_secret())
                .map_err(|error| classify_keyring_error(&error))
        })
        .await
    }

    async fn delete(&self, profile_id: Uuid) -> Result<(), SecretStoreError> {
        run_blocking(move || match entry(profile_id)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(error) => Err(classify_keyring_error(&error)),
        })
        .await
    }
}

pub fn keyring_ref(profile_id: Uuid) -> String {
    format!("keyring:{KEYRING_SERVICE}/{profile_id}")
}

pub fn profile_id_from_ref(reference: &str) -> Result<Uuid, SecretStoreError> {
    let prefix = format!("keyring:{KEYRING_SERVICE}/");
    let account = reference
        .strip_prefix(&prefix)
        .ok_or(SecretStoreError::InvalidReference)?;
    let profile_id = Uuid::parse_str(account).map_err(|_| SecretStoreError::InvalidReference)?;

    if account != profile_id.to_string() {
        return Err(SecretStoreError::InvalidReference);
    }

    Ok(profile_id)
}

fn entry(profile_id: Uuid) -> Result<Entry, SecretStoreError> {
    Entry::new(KEYRING_SERVICE, &profile_id.to_string())
        .map_err(|error| classify_keyring_error(&error))
}

fn classify_keyring_error(error: &KeyringError) -> SecretStoreError {
    match error {
        KeyringError::NoStorageAccess(_) => SecretStoreError::Locked,
        KeyringError::NoEntry => SecretStoreError::Missing,
        KeyringError::NoDefaultStore => SecretStoreError::Unavailable,
        KeyringError::Invalid(parameter, _) if parameter == "platform" => {
            SecretStoreError::Unavailable
        }
        _ => SecretStoreError::Backend,
    }
}

async fn run_blocking<T, F>(operation: F) -> Result<T, SecretStoreError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, SecretStoreError> + Send + 'static,
{
    task::spawn_blocking(operation)
        .await
        .map_err(|_| SecretStoreError::Backend)?
}
