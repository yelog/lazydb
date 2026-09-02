use std::path::{Path, PathBuf};

use directories::{BaseDirs, ProjectDirs};
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
        let base_dirs = BaseDirs::new().ok_or(PathError::Unavailable)?;
        Ok(Self {
            config_dir: config_dir(&base_dirs),
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

    pub fn credential_key_file(&self) -> PathBuf {
        self.config_dir.join("credential.key")
    }

    pub fn workspace_file(&self) -> PathBuf {
        self.state_dir.join("workspace.toml")
    }

    pub fn workspace_sql_dir(&self) -> PathBuf {
        self.state_dir.join("sql")
    }
}

fn config_dir(base_dirs: &BaseDirs) -> PathBuf {
    if cfg!(target_os = "macos") {
        base_dirs.home_dir().join("lazydb")
    } else {
        base_dirs.config_dir().join("lazydb")
    }
}

#[cfg(test)]
mod tests {
    use super::config_dir;
    use directories::BaseDirs;

    #[test]
    fn config_directory_uses_cli_friendly_platform_path() {
        let base_dirs = BaseDirs::new().expect("test environment has a home directory");
        let path = config_dir(&base_dirs);

        if cfg!(target_os = "macos") {
            assert_eq!(path, base_dirs.home_dir().join("lazydb"));
        } else {
            assert_eq!(path, base_dirs.config_dir().join("lazydb"));
        }
    }
}
