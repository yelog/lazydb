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
use crate::profile_compatibility::{
    ProfileLoadReport, ProfileUnavailableReason, UnavailableProfile,
};

const PROFILE_FILE_VERSION: u16 = 6;

#[derive(Clone, Debug)]
pub struct ProfileStore {
    path: PathBuf,
    credential_key_path: Option<PathBuf>,
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
    #[error("profile document is invalid: {0}")]
    InvalidStructure(String),
    #[error("profile lock failed: {0}")]
    Lock(#[from] super::profile_transaction::ProfileLockError),
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
        Self {
            path: resolve_profile_path(path),
            credential_key_path: None,
        }
    }

    pub fn with_credential_key_path(mut self, path: PathBuf) -> Self {
        self.credential_key_path = Some(path);
        self
    }

    pub fn credential_key_path(&self) -> PathBuf {
        self.credential_key_path.clone().unwrap_or_else(|| {
            self.path
                .parent()
                .map(|parent| parent.join("credential.key"))
                .unwrap_or_else(|| PathBuf::from("credential.key"))
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<ProfileCollection, PersistenceError> {
        Ok(self.load_report()?.collection)
    }

    pub fn load_report(&self) -> Result<ProfileLoadReport, PersistenceError> {
        let contents = match fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ProfileLoadReport::default());
            }
            Err(error) => return Err(error.into()),
        };
        let header: ProfileFileHeader = toml::from_str(&contents)?;
        let collection = match header.version {
            PROFILE_FILE_VERSION => {
                return self.load_current_report(&contents);
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
        Ok(ProfileLoadReport {
            collection,
            unavailable: Vec::new(),
        })
    }

    fn load_current_report(&self, contents: &str) -> Result<ProfileLoadReport, PersistenceError> {
        let document = toml::from_str::<toml::Value>(contents)?;
        let document = document.as_table().ok_or_else(|| {
            PersistenceError::InvalidStructure("profile document must be a table".to_owned())
        })?;
        let groups = document
            .get("groups")
            .cloned()
            .map(toml::Value::try_into)
            .transpose()?;
        let groups = groups.unwrap_or_default();
        let profiles = document
            .get("profiles")
            .and_then(toml::Value::as_array)
            .ok_or_else(|| {
                PersistenceError::InvalidStructure("profiles must be an array".to_owned())
            })?;

        let mut supported = Vec::new();
        let mut unavailable = Vec::new();
        for (index, value) in profiles.iter().enumerate() {
            match value.clone().try_into::<ConnectionProfile>() {
                Ok(profile) => supported.push(profile),
                Err(error) => {
                    let table = value.as_table();
                    let id = table
                        .and_then(|table| table.get("id"))
                        .and_then(toml::Value::as_str)
                        .and_then(|id| id.parse().ok());
                    let name = table
                        .and_then(|table| table.get("name"))
                        .and_then(toml::Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("Unavailable connection {}", index + 1));
                    let kind = table
                        .and_then(|table| table.get("kind"))
                        .and_then(toml::Value::as_str)
                        .map(str::to_owned);
                    let reason = if kind.as_deref().is_some_and(|kind| {
                        toml::Value::String(kind.to_owned())
                            .try_into::<DatabaseKind>()
                            .is_err()
                    }) {
                        ProfileUnavailableReason::UnsupportedKind
                    } else if error.to_string().contains("unknown field") {
                        ProfileUnavailableReason::UnsupportedConfiguration
                    } else {
                        ProfileUnavailableReason::InvalidConfiguration
                    };
                    unavailable.push(UnavailableProfile {
                        id,
                        name,
                        kind,
                        reason,
                    });
                }
            }
        }
        let collection = ProfileCollection {
            groups,
            profiles: supported,
        };
        validate_collection(&collection)?;
        Ok(ProfileLoadReport {
            collection,
            unavailable,
        })
    }

    pub fn save<T>(&self, input: T) -> Result<(), PersistenceError>
    where
        T: Into<ProfileCollection>,
    {
        let _lock = super::profile_transaction::ProfileLock::acquire(&self.path)?;
        self.save_unlocked(input)
    }

    /// Re-load and apply a profile mutation while holding the cross-process lock.
    /// The operation returns whether it changed the collection; unchanged
    /// operations do not rewrite the file.
    pub fn mutate<T>(
        &self,
        operation: impl FnOnce(&mut ProfileCollection) -> Result<(T, bool), String>,
    ) -> Result<T, ProfileMutationError> {
        let _lock = super::profile_transaction::ProfileLock::acquire(&self.path)?;
        let mut collection = self.load()?;
        let (result, changed) =
            operation(&mut collection).map_err(ProfileMutationError::Rejected)?;
        if changed {
            self.save_unlocked(collection)?;
        }
        Ok(result)
    }

    /// Commit a TUI snapshot as a three-way merge against the latest file.
    /// Concurrent changes to unrelated profiles are retained; changes to the
    /// same profile or group produce a conflict instead of a lost update.
    pub fn reconcile_save(
        &self,
        expected: &ProfileCollection,
        desired: ProfileCollection,
    ) -> Result<(), ProfileMutationError> {
        let _lock = super::profile_transaction::ProfileLock::acquire(&self.path)?;
        self.reconcile_save_with_lock(&_lock, expected, desired)
    }

    pub fn reconcile_save_with_lock(
        &self,
        lock: &super::profile_transaction::ProfileLock,
        expected: &ProfileCollection,
        desired: ProfileCollection,
    ) -> Result<(), ProfileMutationError> {
        if !lock.protects(&self.path)? {
            return Err(ProfileMutationError::Rejected(
                "profile_conflict: transaction lock protects another profile file".to_owned(),
            ));
        }
        let mut latest = self.load()?;
        let expected_profiles = expected
            .profiles
            .iter()
            .map(|profile| (profile.id, profile.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        let desired_profiles = desired
            .profiles
            .iter()
            .map(|profile| (profile.id, profile.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        let latest_profiles = latest
            .profiles
            .iter()
            .map(|profile| (profile.id, profile.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        let latest_disk_order = latest
            .profiles
            .iter()
            .map(|profile| profile.id)
            .collect::<Vec<_>>();

        for (id, old) in &expected_profiles {
            let new = desired_profiles.get(id);
            if new.is_some_and(|new| new == old) {
                continue;
            }
            let disk = latest_profiles.get(id);
            if disk != Some(old) && !(disk.is_none() && new.is_none()) {
                return Err(ProfileMutationError::Rejected(format!(
                    "profile_conflict: connection {id} changed in another process"
                )));
            }
            if let Some(new) = new {
                if let Some(current) = latest.profiles.iter_mut().find(|profile| profile.id == *id)
                {
                    *current = new.clone();
                }
            } else {
                latest.profiles.retain(|profile| profile.id != *id);
            }
        }

        for profile in &desired.profiles {
            if !expected_profiles.contains_key(&profile.id) {
                if latest
                    .profiles
                    .iter()
                    .any(|current| current.id == profile.id)
                {
                    return Err(ProfileMutationError::Rejected(format!(
                        "profile_conflict: connection {} already exists",
                        profile.id
                    )));
                }
                latest.profiles.push(profile.clone());
            }
        }

        let expected_groups = expected
            .groups
            .iter()
            .map(|group| (group.id, group.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        let desired_groups = desired
            .groups
            .iter()
            .map(|group| (group.id, group.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        let latest_groups = latest
            .groups
            .iter()
            .map(|group| (group.id, group.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        for (id, old) in &expected_groups {
            let new = desired_groups.get(id);
            if new.is_some_and(|new| new == old) {
                continue;
            }
            if latest_groups.get(id) != Some(old) {
                return Err(ProfileMutationError::Rejected(format!(
                    "profile_conflict: connection group {id} changed in another process"
                )));
            }
            if let Some(new) = new {
                if let Some(current) = latest.groups.iter_mut().find(|group| group.id == *id) {
                    *current = new.clone();
                }
            } else {
                latest.groups.retain(|group| group.id != *id);
            }
        }
        for group in &desired.groups {
            if !expected_groups.contains_key(&group.id) {
                if latest.groups.iter().any(|current| current.id == group.id) {
                    return Err(ProfileMutationError::Rejected(format!(
                        "profile_conflict: connection group {} already exists",
                        group.id
                    )));
                }
                latest.groups.push(group.clone());
            }
        }

        let expected_order = expected
            .profiles
            .iter()
            .map(|profile| profile.id)
            .collect::<Vec<_>>();
        let desired_order = desired
            .profiles
            .iter()
            .map(|profile| profile.id)
            .collect::<Vec<_>>();
        if expected_order != desired_order {
            let latest_existing_order = latest_disk_order
                .iter()
                .copied()
                .filter(|id| expected_profiles.contains_key(id))
                .collect::<Vec<_>>();
            let expected_existing_order = expected_order
                .iter()
                .copied()
                .filter(|id| latest_profiles.contains_key(id))
                .collect::<Vec<_>>();
            if latest_existing_order != expected_existing_order {
                return Err(ProfileMutationError::Rejected(
                    "profile_conflict: connection order changed in another process".to_owned(),
                ));
            }
            latest.profiles.sort_by_key(|profile| {
                desired_order
                    .iter()
                    .position(|id| *id == profile.id)
                    .unwrap_or(usize::MAX)
            });
        }

        self.save_unlocked(latest)?;
        Ok(())
    }

    pub async fn mutate_async<T, F, Fut>(&self, operation: F) -> Result<T, ProfileMutationError>
    where
        T: Send + 'static,
        F: FnOnce(ProfileCollection) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(ProfileCollection, T, bool), String>>
            + Send
            + 'static,
    {
        self.mutate_async_with_rollback(operation, || async { Ok(()) })
            .await
    }

    pub async fn mutate_async_with_rollback<T, F, Fut, R, RFut>(
        &self,
        operation: F,
        rollback: R,
    ) -> Result<T, ProfileMutationError>
    where
        T: Send + 'static,
        F: FnOnce(ProfileCollection) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(ProfileCollection, T, bool), String>>
            + Send
            + 'static,
        R: Fn() -> RFut + Send + Sync + 'static,
        RFut: std::future::Future<Output = Result<(), String>> + Send + 'static,
    {
        let store = self.clone();
        let lock_store = store.clone();
        let lock = tokio::task::spawn_blocking(move || {
            super::profile_transaction::ProfileLock::acquire(&lock_store.path)
        })
        .await
        .map_err(|error| ProfileMutationError::Worker(error.to_string()))??;
        let load_store = store.clone();
        let collection = tokio::task::spawn_blocking(move || load_store.load())
            .await
            .map_err(|error| ProfileMutationError::Worker(error.to_string()))??;
        let (collection, result, changed) = match operation(collection).await {
            Ok(result) => result,
            Err(message) => {
                let rollback_result = rollback().await;
                drop(lock);
                return match rollback_result {
                    Ok(()) => Err(ProfileMutationError::Rejected(message)),
                    Err(rollback_error) => Err(ProfileMutationError::Rejected(format!(
                        "credential_rollback_failed: {rollback_error}"
                    ))),
                };
            }
        };
        if changed {
            let save_result =
                match tokio::task::spawn_blocking(move || store.save_unlocked(collection)).await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(error)) => Err(ProfileMutationError::Persistence(error)),
                    Err(error) => Err(ProfileMutationError::Worker(error.to_string())),
                };
            if let Err(error) = save_result {
                let rollback_result = rollback().await;
                drop(lock);
                return match rollback_result {
                    Ok(()) => Err(error),
                    Err(rollback_error) => Err(ProfileMutationError::Rejected(format!(
                        "credential_rollback_failed: {rollback_error}"
                    ))),
                };
            }
        }
        drop(lock);
        Ok(result)
    }

    fn save_unlocked<T>(&self, input: T) -> Result<(), PersistenceError>
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

        let mut contents = toml::to_string_pretty(&ProfileFile {
            version: PROFILE_FILE_VERSION,
            groups: collection.groups.clone(),
            profiles,
        })?;
        if let Ok(existing) = fs::read_to_string(&self.path) {
            contents = preserve_unavailable_profiles(&existing, &contents)?;
        }
        let _: toml::Value = toml::from_str(&contents)?;
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

fn resolve_profile_path(path: PathBuf) -> PathBuf {
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|current| current.join(&path))
            .unwrap_or(path)
    };
    if let Ok(target) = absolute.canonicalize() {
        return target;
    }
    let (Some(parent), Some(file_name)) = (absolute.parent(), absolute.file_name()) else {
        return absolute;
    };
    match parent.canonicalize() {
        Ok(parent) => parent.join(file_name),
        Err(_) => absolute,
    }
}

#[derive(Debug, Error)]
pub enum ProfileMutationError {
    #[error("profile mutation rejected: {0}")]
    Rejected(String),
    #[error("profile lock failed: {0}")]
    Lock(#[from] super::profile_transaction::ProfileLockError),
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error("profile mutation worker failed: {0}")]
    Worker(String),
}

fn preserve_unavailable_profiles(
    existing: &str,
    generated: &str,
) -> Result<String, PersistenceError> {
    let existing_document = existing
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| PersistenceError::InvalidStructure(error.to_string()))?;
    // Older versions are migrated by `load_report`; don't retain their legacy
    // fields as though they were unavailable profiles from the current schema.
    if existing_document["version"]
        .as_integer()
        .is_some_and(|version| version < i64::from(PROFILE_FILE_VERSION))
    {
        return Ok(generated.to_owned());
    }
    let Some(existing_profiles) = existing_document["profiles"].as_array_of_tables() else {
        return Ok(generated.to_owned());
    };
    let mut document = generated
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| PersistenceError::InvalidStructure(error.to_string()))?;
    let mut generated_profiles = document["profiles"]
        .as_array_of_tables()
        .cloned()
        .unwrap_or_default();
    let mut unavailable = Vec::new();
    for old_profile in existing_profiles.iter() {
        let existing_id = old_profile["id"].as_str();
        if let Some(current) = existing_id.and_then(|id| {
            generated_profiles
                .iter_mut()
                .find(|table| table["id"].as_str() == Some(id))
        }) {
            merge_missing_profile_fields(current, old_profile);
            continue;
        }
        if profile_table_is_unavailable(old_profile) {
            unavailable.push(old_profile.clone());
        }
    }
    if !unavailable.is_empty() {
        for table in unavailable {
            generated_profiles.push(table);
        }
    }
    if !generated_profiles.is_empty() {
        document["profiles"] = generated_profiles.into();
    }
    let mut next_position = 0;
    normalize_table_positions(document.as_table_mut(), &mut next_position);
    Ok(document.to_string())
}

fn profile_table_is_unavailable(table: &toml_edit::Table) -> bool {
    let mut document = toml_edit::DocumentMut::new();
    let mut profile = table.clone();
    profile.set_position(1);
    document["profiles"] =
        toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::from_iter([profile]));
    let mut next_position = 0;
    normalize_table_positions(document.as_table_mut(), &mut next_position);
    toml::from_str::<toml::Value>(&document.to_string())
        .ok()
        .and_then(|document| document.get("profiles").cloned())
        .and_then(|profiles| profiles.as_array()?.first().cloned())
        .is_none_or(|profile| profile.try_into::<ConnectionProfile>().is_err())
}

fn normalize_table_positions(table: &mut toml_edit::Table, next_position: &mut isize) {
    table.set_position(*next_position);
    *next_position += 1;
    for (_, item) in table.iter_mut() {
        match item {
            toml_edit::Item::Table(child) => normalize_table_positions(child, next_position),
            toml_edit::Item::ArrayOfTables(children) => {
                for child in children.iter_mut() {
                    normalize_table_positions(child, next_position);
                }
            }
            _ => {}
        }
    }
}

fn merge_missing_profile_fields(current: &mut toml_edit::Table, previous: &toml_edit::Table) {
    for (key, previous_item) in previous.iter() {
        if matches!(key, "secret_ref" | "password" | "secret") {
            continue;
        }
        match (current.get_mut(key), previous_item) {
            (
                Some(toml_edit::Item::Table(current_table)),
                toml_edit::Item::Table(previous_table),
            ) => {
                merge_missing_profile_fields(current_table, previous_table);
            }
            (
                Some(toml_edit::Item::ArrayOfTables(current_tables)),
                toml_edit::Item::ArrayOfTables(previous_tables),
            ) => {
                if current_tables.is_empty() {
                    *current_tables = previous_tables.clone();
                }
            }
            (Some(_), _) => {}
            (None, _) => {
                current.insert(key, previous_item.clone());
            }
        }
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
