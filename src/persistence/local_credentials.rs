use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::rand_core::RngCore,
    aead::{Aead, KeyInit, OsRng, Payload},
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

const KEY_VERSION: u8 = 1;
const KEY_SIZE: usize = 32;
const KEY_FILE_SIZE: usize = 1 + KEY_SIZE;
const CREDENTIAL_VERSION: u16 = 1;
const NONCE_SIZE: usize = 24;
const MAX_CIPHERTEXT_SIZE: usize = 16 * 1024;
const ASSOCIATED_DATA_PREFIX: &str = "lazydb-credential-v1";

#[derive(Debug, Error)]
pub enum LocalCredentialError {
    #[error("local credential I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("local credential key is invalid")]
    InvalidKey,
    #[error("local credential payload is invalid")]
    InvalidPayload,
    #[error("local credential format version {0} is unsupported")]
    UnsupportedVersion(u16),
    #[error("local credential authentication failed")]
    Authentication,
    #[error("local credential encryption failed")]
    Encryption,
}

#[derive(Clone)]
pub struct LocalCredentialKeyStore {
    path: PathBuf,
}

impl LocalCredentialKeyStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load_or_create(&self) -> Result<[u8; KEY_SIZE], LocalCredentialError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
            set_private_dir_permissions(parent)?;
        }

        let mut key_file = [0u8; KEY_FILE_SIZE];
        OsRng
            .try_fill_bytes(&mut key_file[1..])
            .map_err(|_| LocalCredentialError::InvalidKey)?;
        key_file[0] = KEY_VERSION;

        let result = (|| -> Result<(), LocalCredentialError> {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            set_private_file_mode(&mut options);
            let mut file = options.open(&self.path)?;
            file.write_all(&key_file)?;
            file.sync_all()?;
            Ok(())
        })();

        match result {
            Ok(()) => decode_key(&key_file),
            Err(LocalCredentialError::Io(error))
                if error.kind() == io::ErrorKind::AlreadyExists =>
            {
                self.load()
            }
            Err(error) => Err(error),
        }
    }

    pub fn load(&self) -> Result<[u8; KEY_SIZE], LocalCredentialError> {
        let bytes = fs::read(&self.path)?;
        decode_key(&bytes)
    }
}

#[derive(Clone)]
pub struct LocalCredentialStore {
    key_store: LocalCredentialKeyStore,
    service: String,
}

impl LocalCredentialStore {
    pub fn new(key_path: PathBuf, service: impl Into<String>) -> Self {
        Self {
            key_store: LocalCredentialKeyStore::new(key_path),
            service: service.into(),
        }
    }

    pub fn from_paths(
        paths: &crate::persistence::paths::AppPaths,
        service: impl Into<String>,
    ) -> Self {
        Self::new(paths.credential_key_file(), service)
    }

    pub fn key_path(&self) -> &Path {
        self.key_store.path()
    }

    pub fn encrypt(
        &self,
        profile_id: Uuid,
        password: &SecretString,
    ) -> Result<EncryptedCredential, LocalCredentialError> {
        let key = self.key_store.load_or_create()?;
        let cipher = XChaCha20Poly1305::new((&key).into());
        let mut nonce = [0u8; NONCE_SIZE];
        OsRng
            .try_fill_bytes(&mut nonce)
            .map_err(|_| LocalCredentialError::Encryption)?;
        #[allow(deprecated)]
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: password.expose_secret().as_bytes(),
                    aad: self.associated_data(profile_id).as_bytes(),
                },
            )
            .map_err(|_| LocalCredentialError::Encryption)?;
        Ok(EncryptedCredential {
            version: CREDENTIAL_VERSION,
            nonce: BASE64.encode(nonce),
            ciphertext: BASE64.encode(ciphertext),
        })
    }

    pub fn decrypt(
        &self,
        profile_id: Uuid,
        credential: &EncryptedCredential,
    ) -> Result<SecretString, LocalCredentialError> {
        if credential.version != CREDENTIAL_VERSION {
            return Err(LocalCredentialError::UnsupportedVersion(credential.version));
        }
        let nonce = decode_bounded::<NONCE_SIZE>(&credential.nonce)?;
        let ciphertext = BASE64
            .decode(&credential.ciphertext)
            .map_err(|_| LocalCredentialError::InvalidPayload)?;
        if ciphertext.len() > MAX_CIPHERTEXT_SIZE {
            return Err(LocalCredentialError::InvalidPayload);
        }
        let key = self.key_store.load()?;
        let cipher = XChaCha20Poly1305::new((&key).into());
        #[allow(deprecated)]
        let password = cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: ciphertext.as_ref(),
                    aad: self.associated_data(profile_id).as_bytes(),
                },
            )
            .map_err(|_| LocalCredentialError::Authentication)?;
        String::from_utf8(password)
            .map(SecretString::from)
            .map_err(|_| LocalCredentialError::InvalidPayload)
    }

    fn associated_data(&self, profile_id: Uuid) -> String {
        format!("{ASSOCIATED_DATA_PREFIX}\0{}\0{profile_id}", self.service)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EncryptedCredential {
    pub version: u16,
    pub nonce: String,
    pub ciphertext: String,
}

fn decode_key(bytes: &[u8]) -> Result<[u8; KEY_SIZE], LocalCredentialError> {
    if bytes.len() != KEY_FILE_SIZE || bytes[0] != KEY_VERSION {
        return Err(LocalCredentialError::InvalidKey);
    }
    let mut key = [0u8; KEY_SIZE];
    key.copy_from_slice(&bytes[1..]);
    Ok(key)
}

fn decode_bounded<const N: usize>(encoded: &str) -> Result<[u8; N], LocalCredentialError> {
    let bytes = BASE64
        .decode(encoded)
        .map_err(|_| LocalCredentialError::InvalidPayload)?;
    let array: [u8; N] = bytes
        .try_into()
        .map_err(|_| LocalCredentialError::InvalidPayload)?;
    Ok(array)
}

#[cfg(unix)]
fn set_private_file_mode(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn set_private_file_mode(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn set_private_dir_permissions(path: &Path) -> Result<(), io::Error> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &Path) -> Result<(), io::Error> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, LocalCredentialStore) {
        let dir = tempdir().unwrap();
        let store = LocalCredentialStore::new(dir.path().join("credential.key"), "test-service");
        (dir, store)
    }

    #[test]
    fn key_is_created_once_and_reloaded() {
        let (_dir, store) = store();
        let first = store.key_store.load_or_create().unwrap();
        let second = store.key_store.load_or_create().unwrap();
        assert_eq!(first, second);
        assert_eq!(fs::read(store.key_path()).unwrap().len(), KEY_FILE_SIZE);
    }

    #[test]
    fn invalid_existing_key_is_not_replaced() {
        let (_dir, store) = store();
        fs::write(store.key_path(), b"invalid").unwrap();
        assert!(matches!(
            store.key_store.load_or_create(),
            Err(LocalCredentialError::InvalidKey)
        ));
        assert_eq!(fs::read(store.key_path()).unwrap(), b"invalid");
    }

    #[cfg(unix)]
    #[test]
    fn key_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, store) = store();
        store.key_store.load_or_create().unwrap();
        assert_eq!(
            fs::metadata(store.key_path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn encryption_round_trip_is_random_and_profile_bound() {
        let (_dir, store) = store();
        let id = Uuid::new_v4();
        let password = SecretString::from("not-for-logs");
        let first = store.encrypt(id, &password).unwrap();
        let second = store.encrypt(id, &password).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            store.decrypt(id, &first).unwrap().expose_secret(),
            "not-for-logs"
        );
        assert!(matches!(
            store.decrypt(Uuid::new_v4(), &first),
            Err(LocalCredentialError::Authentication)
        ));
        assert!(!first.ciphertext.contains("not-for-logs"));
    }

    #[test]
    fn tampering_and_wrong_service_fail_authentication() {
        let (_dir, store) = store();
        let id = Uuid::new_v4();
        let password = SecretString::from("secret");
        let mut credential = store.encrypt(id, &password).unwrap();
        credential.ciphertext.push('A');
        assert!(matches!(
            store.decrypt(id, &credential),
            Err(LocalCredentialError::Authentication | LocalCredentialError::InvalidPayload)
        ));
        let other = LocalCredentialStore::new(store.key_path().to_owned(), "other-service");
        let credential = store.encrypt(id, &password).unwrap();
        assert!(matches!(
            other.decrypt(id, &credential),
            Err(LocalCredentialError::Authentication)
        ));
    }
}
