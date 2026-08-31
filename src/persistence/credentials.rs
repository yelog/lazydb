use std::sync::Arc;

use secrecy::SecretString;
use thiserror::Error;

use super::{
    local_credentials::LocalCredentialStore,
    secrets::{SecretStore, SecretStoreError, profile_id_from_ref},
};
use crate::profile::{ConnectionProfile, CredentialPolicy};

#[derive(Debug, Error, Eq, PartialEq)]
pub enum CredentialResolutionError {
    #[error("interactive password input is required")]
    InteractionRequired,
    #[error("stored password is missing")]
    Missing,
    #[error("stored password is unavailable")]
    Unavailable,
    #[error("stored password reference is invalid")]
    InvalidReference,
    #[error("stored password could not be decrypted")]
    Decryption,
    #[error("credential store operation failed")]
    Store,
}

pub struct CredentialResolver {
    secret_store: Arc<dyn SecretStore>,
    local_store: LocalCredentialStore,
}

impl CredentialResolver {
    pub fn new(secret_store: Arc<dyn SecretStore>, local_store: LocalCredentialStore) -> Self {
        Self {
            secret_store,
            local_store,
        }
    }

    pub async fn resolve_headless(
        &self,
        profile: &ConnectionProfile,
    ) -> Result<Option<SecretString>, CredentialResolutionError> {
        match &profile.credential_policy {
            CredentialPolicy::None => Ok(None),
            CredentialPolicy::Prompt => Err(CredentialResolutionError::InteractionRequired),
            CredentialPolicy::LocalEncrypted(credential) => self
                .local_store
                .decrypt(profile.id, credential)
                .map(Some)
                .map_err(|_| CredentialResolutionError::Decryption),
            CredentialPolicy::System(_) | CredentialPolicy::Keyring(_) => {
                validate_secret_reference(profile)?;
                match self.secret_store.get(profile.id).await {
                    Ok(Some(password)) => Ok(Some(password)),
                    Ok(None) | Err(SecretStoreError::Missing) => {
                        Err(CredentialResolutionError::Missing)
                    }
                    Err(SecretStoreError::Locked | SecretStoreError::Unavailable) => {
                        Err(CredentialResolutionError::Unavailable)
                    }
                    Err(_) => Err(CredentialResolutionError::Store),
                }
            }
        }
    }
}

fn validate_secret_reference(profile: &ConnectionProfile) -> Result<(), CredentialResolutionError> {
    let Some(reference) = profile.credential_policy.keyring_reference() else {
        return Ok(());
    };
    let referenced_profile =
        profile_id_from_ref(reference).map_err(|_| CredentialResolutionError::InvalidReference)?;
    if referenced_profile != profile.id {
        return Err(CredentialResolutionError::InvalidReference);
    }
    Ok(())
}
