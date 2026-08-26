use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::profile::ConnectionProfile;

const PROFILE_FILE_VERSION: u16 = 2;

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
        if header.version != PROFILE_FILE_VERSION {
            return Err(PersistenceError::UnsupportedVersion {
                found: header.version,
                expected: PROFILE_FILE_VERSION,
            });
        }
        let file: ProfileFile = toml::from_str(&contents)?;
        validate_profile_ids(&file.profiles)?;
        Ok(file.profiles)
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
