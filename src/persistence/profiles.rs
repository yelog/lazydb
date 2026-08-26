use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::profile::{
    CatalogScope, ConnectionProfile, ConnectionUrlFormat, CredentialPolicy, DatabaseKind,
    Environment, SslMode,
};

const PROFILE_FILE_VERSION: u16 = 3;

#[derive(Clone, Debug)]
pub struct ProfileStore {
    path: PathBuf,
}

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("profile I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("profile file is invalid: {0}")]
    Decode(#[from] toml::de::Error),
    #[error("profile serialization failed: {0}")]
    Encode(#[from] toml::ser::Error),
    #[error("profile file version {found} is not supported; expected version {expected}")]
    UnsupportedVersion { found: u16, expected: u16 },
    #[error("profile UUID {0} appears more than once")]
    DuplicateProfileId(Uuid),
    #[error("profile path has no parent directory")]
    MissingParent,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProfileFile {
    version: u16,
    profiles: Vec<ConnectionProfile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileFileV2 {
    #[serde(rename = "version")]
    _version: u16,
    profiles: Vec<ConnectionProfileV2>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionProfileV2 {
    id: Uuid,
    name: String,
    kind: DatabaseKind,
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    database: Option<String>,
    default_schema: Option<String>,
    sqlite_path: Option<PathBuf>,
    #[serde(default)]
    ssl_mode: SslMode,
    secret_ref: Option<String>,
    #[serde(default)]
    read_only: bool,
    #[serde(default)]
    environment: Environment,
    catalog_scope: CatalogScope,
}

impl From<ConnectionProfileV2> for ConnectionProfile {
    fn from(profile: ConnectionProfileV2) -> Self {
        Self {
            id: profile.id,
            name: profile.name,
            kind: profile.kind,
            url_format: ConnectionUrlFormat::default_for(profile.kind),
            host: profile.host,
            port: profile.port,
            user: profile.user,
            database: profile.database,
            default_schema: profile.default_schema,
            sqlite_path: profile.sqlite_path,
            ssl_mode: profile.ssl_mode,
            credential_policy: profile
                .secret_ref
                .map_or(CredentialPolicy::None, CredentialPolicy::Keyring),
            read_only: profile.read_only,
            environment: profile.environment,
            catalog_scope: profile.catalog_scope,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProfileFileHeader {
    version: u16,
}

impl ProfileStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Vec<ConnectionProfile>, PersistenceError> {
        let contents = match fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let header: ProfileFileHeader = toml::from_str(&contents)?;
        let mut profiles = match header.version {
            PROFILE_FILE_VERSION => toml::from_str::<ProfileFile>(&contents)?.profiles,
            2 => toml::from_str::<ProfileFileV2>(&contents)?
                .profiles
                .into_iter()
                .map(ConnectionProfile::from)
                .collect(),
            found => {
                return Err(PersistenceError::UnsupportedVersion {
                    found,
                    expected: PROFILE_FILE_VERSION,
                });
            }
        };
        for profile in &mut profiles {
            if !profile.url_format.is_compatible(profile.kind) {
                profile.url_format = ConnectionUrlFormat::default_for(profile.kind);
            }
        }
        validate_profile_ids(&profiles)?;
        Ok(profiles)
    }

    pub fn save(&self, profiles: &[ConnectionProfile]) -> Result<(), PersistenceError> {
        validate_profile_ids(profiles)?;
        let parent = self.path.parent().ok_or(PersistenceError::MissingParent)?;
        fs::create_dir_all(parent)?;
        set_private_dir_permissions(parent)?;

        let contents = toml::to_string_pretty(&ProfileFile {
            version: PROFILE_FILE_VERSION,
            profiles: profiles.to_vec(),
        })?;
        let file_name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("connections.toml");
        let temporary = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));

        let result = (|| -> Result<(), PersistenceError> {
            let mut options = OpenOptions::new();
            options.create_new(true).write(true);
            set_private_file_mode(&mut options);
            let mut file = options.open(&temporary)?;
            file.write_all(contents.as_bytes())?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)?;
            Ok(())
        })();

        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

fn validate_profile_ids(profiles: &[ConnectionProfile]) -> Result<(), PersistenceError> {
    let mut ids = HashSet::with_capacity(profiles.len());
    for profile in profiles {
        if !ids.insert(profile.id) {
            return Err(PersistenceError::DuplicateProfileId(profile.id));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_private_file_mode(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn set_private_file_mode(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn set_private_dir_permissions(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &Path) -> Result<(), std::io::Error> {
    Ok(())
}
