use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::model::{execution_target::ExecutionTarget, transaction::TransactionMode};

const WORKSPACE_VERSION: u16 = 1;

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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WorkspaceFile {
    pub version: u16,
    pub active_console: Uuid,
    pub consoles: Vec<PersistedConsole>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PersistedConsole {
    pub id: Uuid,
    pub name: String,
    pub sql_file: PathBuf,
    pub target: Option<ExecutionTarget>,
    pub transaction_mode: TransactionMode,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceSnapshot {
    pub active_console: Uuid,
    pub consoles: Vec<PersistedConsole>,
    pub sql: Vec<(Uuid, String)>,
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
        let file: WorkspaceFile = toml::from_str(&contents)?;
        if file.version != WORKSPACE_VERSION {
            return Err(WorkspaceError::UnsupportedVersion {
                found: file.version,
                expected: WORKSPACE_VERSION,
            });
        }
        let sql = file
            .consoles
            .iter()
            .map(|console| {
                let path = self.sql_dir.join(&console.sql_file);
                let text = fs::read_to_string(path).unwrap_or_default();
                (console.id, text)
            })
            .collect();
        Ok(Some(WorkspaceSnapshot {
            active_console: file.active_console,
            consoles: file.consoles,
            sql,
        }))
    }

    pub fn save(&self, snapshot: &WorkspaceSnapshot) -> Result<(), WorkspaceError> {
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
            active_console: snapshot.active_console,
            consoles: snapshot.consoles.clone(),
        };
        let temporary = self.manifest.with_extension("toml.tmp");
        let mut manifest_file = File::create(&temporary)?;
        manifest_file.write_all(toml::to_string_pretty(&file)?.as_bytes())?;
        manifest_file.sync_all()?;
        fs::rename(temporary, &self.manifest)?;
        Ok(())
    }
}
