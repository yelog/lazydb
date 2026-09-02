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
    CatalogScope, ConnectionGroup, ConnectionProfile, ConnectionUrlFormat, CredentialPolicy,
    DatabaseKind, Environment, ProfileAccess, ProfileCollection, SslMode,
};

const PROFILE_FILE_VERSION: u16 = 6;

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
    #[error("connection group UUID {0} appears more than once")]
    DuplicateGroupId(Uuid),
    #[error("connection group name `{0}` appears more than once")]
    DuplicateGroupName(String),
    #[error("connection group `{0}` has an invalid name")]
    InvalidGroupName(String),
    #[error("profile {profile_id} references missing connection group {group_id}")]
    UnknownProfileGroup { profile_id: Uuid, group_id: Uuid },
    #[error("project root `{0}` must be absolute")]
    InvalidProjectRoot(PathBuf),
    #[error("project root `{0}` appears more than once")]
    DuplicateProjectRoot(PathBuf),
    #[error("profile path has no parent directory")]
    MissingParent,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProfileFile {
    version: u16,
    #[serde(default)]
    groups: Vec<ConnectionGroup>,
    profiles: Vec<ConnectionProfile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileFileV5 {
    #[serde(rename = "version")]
    _version: u16,
    profiles: Vec<ConnectionProfileV5>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionProfileV5 {
    id: Uuid,
    name: String,
    #[serde(default)]
    access: ProfileAccess,
    kind: DatabaseKind,
    #[serde(default)]
    url_format: ConnectionUrlFormat,
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    database: Option<String>,
    default_schema: Option<String>,
    sqlite_path: Option<PathBuf>,
    #[serde(default)]
    ssl_mode: SslMode,
    #[serde(default)]
    credential_policy: CredentialPolicy,
    #[serde(default)]
    read_only: bool,
    #[serde(default)]
    environment: Environment,
    catalog_scope: CatalogScope,
}

impl From<ConnectionProfileV5> for ConnectionProfile {
    fn from(profile: ConnectionProfileV5) -> Self {
        Self {
            id: profile.id,
            name: profile.name,
            access: profile.access,
            group_id: None,
            kind: profile.kind,
            url_format: profile.url_format,
            host: profile.host,
            port: profile.port,
            user: profile.user,
            database: profile.database,
            default_schema: profile.default_schema,
            sqlite_path: profile.sqlite_path,
            ssl_mode: profile.ssl_mode,
            credential_policy: profile.credential_policy,
            read_only: profile.read_only,
            environment: profile.environment,
            catalog_scope: profile.catalog_scope,
        }
    }
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
            access: ProfileAccess::Global,
            group_id: None,
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

    pub fn load(&self) -> Result<ProfileCollection, PersistenceError> {
        let contents = match fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ProfileCollection::default());
            }
            Err(error) => return Err(error.into()),
        };
        let header: ProfileFileHeader = toml::from_str(&contents)?;
        let collection = match header.version {
            PROFILE_FILE_VERSION => {
                let file = toml::from_str::<ProfileFile>(&contents)?;
                ProfileCollection {
                    groups: file.groups,
                    profiles: file.profiles,
                }
            }
            2 => ProfileCollection {
                groups: Vec::new(),
                profiles: toml::from_str::<ProfileFileV2>(&contents)?
                    .profiles
                    .into_iter()
                    .map(ConnectionProfile::from)
                    .collect(),
            },
            3..=5 => ProfileCollection {
                groups: Vec::new(),
                profiles: toml::from_str::<ProfileFileV5>(&contents)?
                    .profiles
                    .into_iter()
                    .map(ConnectionProfile::from)
                    .map(normalize_legacy_profile)
                    .collect(),
            },
            found => {
                return Err(PersistenceError::UnsupportedVersion {
                    found,
                    expected: PROFILE_FILE_VERSION,
                });
            }
        };
        let mut profiles = collection.profiles;
        for profile in &mut profiles {
            if let CredentialPolicy::Keyring(reference) = &profile.credential_policy {
                profile.credential_policy = CredentialPolicy::System(reference.clone());
            }
            if !profile.url_format.is_compatible(profile.kind) {
                profile.url_format = ConnectionUrlFormat::default_for(profile.kind);
            }
        }
        let collection = ProfileCollection {
            groups: collection.groups,
            profiles,
        };
        validate_collection(&collection)?;
        Ok(collection)
    }

    pub fn save<T>(&self, input: T) -> Result<(), PersistenceError>
    where
        T: Into<ProfileCollection>,
    {
        let collection: ProfileCollection = input.into();
        validate_collection(&collection)?;
        let mut profiles = collection.profiles.clone();
        for profile in &mut profiles {
            if let ProfileAccess::Projects { roots } = &mut profile.access {
                roots.sort();
            }
        }
        let parent = self.path.parent().ok_or(PersistenceError::MissingParent)?;
        fs::create_dir_all(parent)?;
        set_private_dir_permissions(parent)?;

        let contents = toml::to_string_pretty(&ProfileFile {
            version: PROFILE_FILE_VERSION,
            groups: collection.groups.clone(),
            profiles,
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

fn normalize_v3_profile(mut profile: ConnectionProfile) -> ConnectionProfile {
    if let CredentialPolicy::Keyring(reference) = profile.credential_policy {
        profile.credential_policy = CredentialPolicy::System(reference);
    }
    profile
}

fn normalize_legacy_profile(mut profile: ConnectionProfile) -> ConnectionProfile {
    profile.access = ProfileAccess::Global;
    normalize_v3_profile(profile)
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

fn validate_profile_access(profiles: &[ConnectionProfile]) -> Result<(), PersistenceError> {
    for profile in profiles {
        let ProfileAccess::Projects { roots } = &profile.access else {
            continue;
        };
        let mut unique = HashSet::with_capacity(roots.len());
        for root in roots {
            if !root.is_absolute() {
                return Err(PersistenceError::InvalidProjectRoot(root.clone()));
            }
            if !unique.insert(root) {
                return Err(PersistenceError::DuplicateProjectRoot(root.clone()));
            }
        }
    }
    Ok(())
}

fn validate_collection(collection: &ProfileCollection) -> Result<(), PersistenceError> {
    validate_profile_ids(&collection.profiles)?;
    validate_profile_access(&collection.profiles)?;

    let mut ids = HashSet::with_capacity(collection.groups.len());
    let mut names = HashSet::with_capacity(collection.groups.len());
    for group in &collection.groups {
        if !ids.insert(group.id) {
            return Err(PersistenceError::DuplicateGroupId(group.id));
        }
        let canonical = ConnectionGroup::new(group.id, &group.name)
            .map_err(|_| PersistenceError::InvalidGroupName(group.name.clone()))?;
        if canonical.name != group.name {
            return Err(PersistenceError::InvalidGroupName(group.name.clone()));
        }
        if !names.insert(group.normalized_name()) {
            return Err(PersistenceError::DuplicateGroupName(group.name.clone()));
        }
    }
    for profile in &collection.profiles {
        if let Some(group_id) = profile.group_id
            && !ids.contains(&group_id)
        {
            return Err(PersistenceError::UnknownProfileGroup {
                profile_id: profile.id,
                group_id,
            });
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
