use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::db::catalog::{CatalogId, CatalogKind, QualifiedName};
use crate::model::relation::RelationView;
use crate::model::{execution_target::ExecutionTarget, transaction::TransactionMode};

const WORKSPACE_VERSION: u16 = 3;

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("workspace I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("workspace file is invalid: {0}")]
    Decode(#[from] toml::de::Error),
    #[error("workspace serialization failed: {0}")]
    Encode(#[from] toml::ser::Error),
    #[error("workspace version {found} is not supported; expected {expected}")]
    UnsupportedVersion { found: u16, expected: u16 },
    #[error("workspace is already locked by another LazyDB process")]
    Locked,
    #[error("workspace is invalid: {0}")]
    Invalid(String),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WorkspaceFile {
    pub version: u16,
    pub active_profile: Option<Uuid>,
    pub profiles: Vec<PersistedProfileWorkspace>,
}

#[derive(Clone, Debug, Deserialize)]
struct LegacyWorkspaceFile {
    #[serde(rename = "version")]
    _version: u16,
    active_console: Uuid,
    consoles: Vec<PersistedConsole>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PersistedConsole {
    pub id: Uuid,
    pub name: String,
    pub sql_file: PathBuf,
    pub target: Option<ExecutionTarget>,
    pub transaction_mode: TransactionMode,
    #[serde(default = "default_open")]
    pub open: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PersistedProfileWorkspace {
    pub profile_id: Uuid,
    pub active_tab: Option<Uuid>,
    pub consoles: Vec<PersistedConsole>,
    pub tabs: Vec<PersistedTab>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PersistedTab {
    Console { console_id: Uuid },
    Relation(PersistedRelationTab),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PersistedRelationTab {
    pub id: Uuid,
    pub object_id: CatalogId,
    pub qualified_name: QualifiedName,
    pub catalog_kind: CatalogKind,
    pub title: String,
    pub view: RelationView,
}

fn default_open() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceSnapshot {
    pub active_profile: Option<Uuid>,
    pub profiles: Vec<PersistedProfileWorkspace>,
    pub sql: Vec<(Uuid, String)>,
    // Kept only until the app snapshot code is migrated in a later task.
    pub active_console: Uuid,
    pub consoles: Vec<PersistedConsole>,
}

#[derive(Clone, Debug)]
pub struct WorkspaceStore {
    manifest: PathBuf,
    sql_dir: PathBuf,
}

pub struct WorkspaceLock {
    path: PathBuf,
    _file: File,
}

impl Drop for WorkspaceLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl WorkspaceStore {
    pub fn new(manifest: PathBuf, sql_dir: PathBuf) -> Self {
        Self { manifest, sql_dir }
    }

    pub fn lock(&self) -> Result<WorkspaceLock, WorkspaceError> {
        if let Some(parent) = self.manifest.parent() {
            fs::create_dir_all(parent)?;
        }
        let path = self.manifest.with_extension("lock");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    WorkspaceError::Locked
                } else {
                    WorkspaceError::Io(error)
                }
            })?;
        Ok(WorkspaceLock { path, _file: file })
    }

    pub fn load(&self) -> Result<Option<WorkspaceSnapshot>, WorkspaceError> {
        let contents = match fs::read_to_string(&self.manifest) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let version = toml::from_str::<toml::Value>(&contents)?
            .get("version")
            .and_then(toml::Value::as_integer)
            .and_then(|version| u16::try_from(version).ok())
            .ok_or_else(|| WorkspaceError::Invalid("workspace version is missing".into()))?;
        let (active_profile, active_console, profiles) = if version == WORKSPACE_VERSION {
            let file: WorkspaceFile = toml::from_str(&contents)?;
            (file.active_profile, Uuid::nil(), file.profiles)
        } else if matches!(version, 1 | 2) {
            let mut file: LegacyWorkspaceFile = toml::from_str(&contents)?;
            if version == 1 {
                for console in &mut file.consoles {
                    console.open = true;
                }
            }
            (None, file.active_console, migrate_legacy(file))
        } else {
            return Err(WorkspaceError::UnsupportedVersion {
                found: version,
                expected: WORKSPACE_VERSION,
            });
        };
        let sql = profiles
            .iter()
            .flat_map(|profile| profile.consoles.iter())
            .map(|console| {
                let path = self.sql_dir.join(&console.sql_file);
                let text = fs::read_to_string(path).unwrap_or_default();
                (console.id, text)
            })
            .collect();
        let snapshot = WorkspaceSnapshot {
            active_profile,
            profiles,
            sql,
            active_console,
            consoles: Vec::new(),
        };
        validate_snapshot(&snapshot)?;
        Ok(Some(snapshot))
    }

    pub fn save(&self, snapshot: &WorkspaceSnapshot) -> Result<(), WorkspaceError> {
        validate_snapshot(snapshot)?;
        fs::create_dir_all(&self.sql_dir)?;
        if let Some(parent) = self.manifest.parent() {
            fs::create_dir_all(parent)?;
        }
        for (id, text) in &snapshot.sql {
            let path = self.sql_dir.join(format!("{id}.sql"));
            let temporary = path.with_extension("sql.tmp");
            let mut file = File::create(&temporary)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            fs::rename(temporary, path)?;
        }
        let file = WorkspaceFile {
            version: WORKSPACE_VERSION,
            active_profile: snapshot.active_profile,
            profiles: snapshot.profiles.clone(),
        };
        let temporary = self.manifest.with_extension("toml.tmp");
        let mut manifest_file = File::create(&temporary)?;
        manifest_file.write_all(toml::to_string_pretty(&file)?.as_bytes())?;
        manifest_file.sync_all()?;
        fs::rename(temporary, &self.manifest)?;
        Ok(())
    }

    pub fn delete_sql_file(&self, id: Uuid) -> Result<(), WorkspaceError> {
        let path = self.sql_dir.join(format!("{id}.sql"));
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

fn migrate_legacy(file: LegacyWorkspaceFile) -> Vec<PersistedProfileWorkspace> {
    let mut profiles = Vec::<PersistedProfileWorkspace>::new();
    for console in file.consoles {
        let profile_id = console
            .target
            .as_ref()
            .map_or(Uuid::nil(), |target| target.profile_id);
        let profile_index = profiles
            .iter()
            .position(|profile| profile.profile_id == profile_id);
        let profile_index = profile_index.unwrap_or_else(|| {
            profiles.push(PersistedProfileWorkspace {
                profile_id,
                active_tab: None,
                consoles: Vec::new(),
                tabs: Vec::new(),
            });
            profiles.len() - 1
        });
        let profile = &mut profiles[profile_index];
        if console.id == file.active_console && console.open {
            profile.active_tab = Some(console.id);
        }
        if console.open {
            profile.tabs.push(PersistedTab::Console {
                console_id: console.id,
            });
        }
        profile.consoles.push(console);
    }
    profiles
}

pub fn validate_snapshot(snapshot: &WorkspaceSnapshot) -> Result<(), WorkspaceError> {
    let mut profile_ids = std::collections::HashSet::new();
    let mut console_ids = std::collections::HashSet::new();
    let mut relation_ids = std::collections::HashSet::new();
    for profile in &snapshot.profiles {
        if !profile_ids.insert(profile.profile_id) {
            return Err(WorkspaceError::Invalid("duplicate profile ID".into()));
        }
        if profile
            .active_tab
            .is_some_and(|id| !profile.tabs.iter().any(|tab| tab_id(tab) == id))
        {
            return Err(WorkspaceError::Invalid("active tab is not open".into()));
        }
        let profile_console_ids = profile
            .consoles
            .iter()
            .map(|console| console.id)
            .collect::<std::collections::HashSet<_>>();
        for console in &profile.consoles {
            if console.sql_file != PathBuf::from(format!("{}.sql", console.id)) {
                return Err(WorkspaceError::Invalid(format!(
                    "invalid SQL file for console {}",
                    console.id
                )));
            }
            if !console_ids.insert(console.id) {
                return Err(WorkspaceError::Invalid("duplicate console ID".into()));
            }
        }
        let mut open_console_ids = std::collections::HashSet::new();
        for tab in &profile.tabs {
            match tab {
                PersistedTab::Console { console_id } => {
                    if !profile_console_ids.contains(console_id) {
                        return Err(WorkspaceError::Invalid(
                            "console tab references a missing console".into(),
                        ));
                    }
                    if !open_console_ids.insert(*console_id) {
                        return Err(WorkspaceError::Invalid("duplicate open tab ID".into()));
                    }
                }
                PersistedTab::Relation(relation) => {
                    if !relation_ids.insert(relation.id) {
                        return Err(WorkspaceError::Invalid("duplicate tab ID".into()));
                    }
                    if console_ids.contains(&relation.id) {
                        return Err(WorkspaceError::Invalid(
                            "relation duplicates a console ID".into(),
                        ));
                    }
                    if relation.object_id.profile_id() != profile.profile_id {
                        return Err(WorkspaceError::Invalid(
                            "relation belongs to another profile".into(),
                        ));
                    }
                }
            }
        }
    }
    if snapshot
        .active_profile
        .is_some_and(|id| !profile_ids.contains(&id))
    {
        return Err(WorkspaceError::Invalid("active profile is missing".into()));
    }
    let mut sql_ids = std::collections::HashSet::new();
    for (id, _) in &snapshot.sql {
        if !sql_ids.insert(*id) || !console_ids.contains(id) {
            return Err(WorkspaceError::Invalid(
                "SQL entry does not name one known console".into(),
            ));
        }
    }
    if sql_ids.len() != console_ids.len() {
        return Err(WorkspaceError::Invalid(
            "every console must have exactly one SQL entry".into(),
        ));
    }
    Ok(())
}

fn tab_id(tab: &PersistedTab) -> Uuid {
    match tab {
        PersistedTab::Console { console_id } => *console_id,
        PersistedTab::Relation(relation) => relation.id,
    }
}
