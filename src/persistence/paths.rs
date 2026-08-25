use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub state_dir: PathBuf,
}

#[derive(Debug, Error)]
pub enum PathError {
    #[error("the operating system did not provide application directories")]
    Unavailable,
}

impl AppPaths {
    pub fn discover() -> Result<Self, PathError> {
        let dirs = ProjectDirs::from("dev", "lazydb", "lazydb").ok_or(PathError::Unavailable)?;
        Ok(Self {
            config_dir: dirs.config_dir().to_owned(),
            data_dir: dirs.data_dir().to_owned(),
            state_dir: dirs
                .state_dir()
                .unwrap_or_else(|| dirs.data_local_dir())
                .to_owned(),
        })
    }

    pub fn for_test(root: &Path) -> Self {
        Self {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            state_dir: root.join("state"),
        }
    }

    pub fn profiles_file(&self) -> PathBuf {
        self.config_dir.join("connections.toml")
    }
}
