use std::{
    fs,
    path::{Path, PathBuf},
};

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
    #[error("failed to migrate application data: {0}")]
    Io(#[from] std::io::Error),
}

impl AppPaths {
    pub fn discover() -> Result<Self, PathError> {
        let dirs = ProjectDirs::from("dev", "lazydb", "lazydb").ok_or(PathError::Unavailable)?;
        let base_dirs = BaseDirs::new().ok_or(PathError::Unavailable)?;
        let config_dir = config_dir(&base_dirs);
        migrate_legacy_directory(dirs.config_dir(), &config_dir)?;
        Ok(Self {
            config_dir: config_dir.clone(),
            data_dir: dirs.data_dir().to_owned(),
            state_dir: config_dir,
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

fn migrate_legacy_directory(old_dir: &Path, new_dir: &Path) -> Result<(), std::io::Error> {
    if !cfg!(target_os = "macos") || old_dir == new_dir || !old_dir.exists() {
        return Ok(());
    }

    fs::create_dir_all(new_dir)?;
    set_private_dir_permissions(new_dir)?;
    for name in ["connections.toml", "credential.key", "workspace.toml"] {
        move_if_absent(&old_dir.join(name), &new_dir.join(name))?;
    }

    let old_sql = old_dir.join("sql");
    let new_sql = new_dir.join("sql");
    if old_sql.is_dir() {
        if !new_sql.exists() {
            fs::rename(old_sql, new_sql)?;
        } else {
            for entry in fs::read_dir(old_sql)? {
                let entry = entry?;
                let destination = new_sql.join(entry.file_name());
                move_if_absent(&entry.path(), &destination)?;
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_private_dir_permissions(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

fn move_if_absent(source: &Path, destination: &Path) -> Result<(), std::io::Error> {
    if source.exists() && !destination.exists() {
        fs::rename(source, destination)?;
    }
    Ok(())
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
