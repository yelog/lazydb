use std::path::{Path, PathBuf};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::env;

use directories::BaseDirs;
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
    #[error("multiple LazyDB data directories were found: {0}")]
    Ambiguous(String),
}

impl AppPaths {
    pub fn discover() -> Result<Self, PathError> {
        let base_dirs = BaseDirs::new().ok_or(PathError::Unavailable)?;
        let config_dir = discover_config_dir(&base_dirs)?;
        Ok(Self {
            config_dir: config_dir.clone(),
            data_dir: config_dir.clone(),
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

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
    }

    pub fn update_check_file(&self) -> PathBuf {
        self.state_dir.join("update-check.json")
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

    pub fn history_file(&self) -> PathBuf {
        self.state_dir.join("sql-history.sqlite3")
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn explicit_config_dir() -> Option<PathBuf> {
    if let Some(path) = env::var_os("LAZYDB_CONFIG_HOME").filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(path));
    }
    None
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn legacy_candidates(home: &Path) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        vec![
            home.join(".config/lazydb"),
            home.join(".local/share/lazydb"),
            home.join("Library/Application Support/dev.lazydb.lazydb"),
        ]
    }
    #[cfg(target_os = "linux")]
    {
        vec![
            home.join(".config/lazydb"),
            home.join(".local/share/lazydb"),
        ]
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn discover_config_dir(base_dirs: &BaseDirs) -> Result<PathBuf, PathError> {
    resolve_config_dir(
        base_dirs.home_dir(),
        explicit_config_dir(),
        legacy_candidates(base_dirs.home_dir()),
    )
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn discover_config_dir(base_dirs: &BaseDirs) -> Result<PathBuf, PathError> {
    Ok(base_dirs.config_dir().join("lazydb"))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn resolve_config_dir(
    home: &Path,
    explicit: Option<PathBuf>,
    legacy: Vec<PathBuf>,
) -> Result<PathBuf, PathError> {
    if let Some(path) = explicit {
        return Ok(if path.is_absolute() {
            path
        } else {
            home.join(path)
        });
    }

    let new_dir = home.join("lazydb");
    let mut existing = Vec::new();
    let mut identities = Vec::new();
    for path in std::iter::once(new_dir.clone()).chain(legacy) {
        if path.exists() && has_lazydb_data(&path) {
            let identity = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if !identities.iter().any(|seen| seen == &identity) {
                identities.push(identity);
                existing.push(path);
            }
        }
    }
    match existing.as_slice() {
        [] => Ok(new_dir),
        [path] => Ok(path.clone()),
        paths => Err(PathError::Ambiguous(
            paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        )),
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn has_lazydb_data(path: &Path) -> bool {
    [
        "connections.toml",
        "credential.key",
        "settings.toml",
        "workspace.toml",
        "sql-history.sqlite3",
        "install.json",
        "current",
        "releases",
        "sql",
    ]
    .iter()
    .any(|name| path.join(name).exists())
}

#[cfg(test)]
mod tests {
    use super::{discover_config_dir, resolve_config_dir};
    use directories::BaseDirs;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn config_directory_uses_cli_friendly_platform_path() {
        let base_dirs = BaseDirs::new().expect("test environment has a home directory");
        let path = discover_config_dir(&base_dirs).unwrap();

        if cfg!(any(target_os = "macos", target_os = "linux")) {
            if let Some(config_home) = std::env::var_os("LAZYDB_CONFIG_HOME") {
                assert_eq!(path, config_home);
            } else {
                assert!(
                    path == base_dirs.home_dir().join("lazydb")
                        || path == base_dirs.home_dir().join(".config/lazydb")
                        || path == base_dirs.home_dir().join(".local/share/lazydb")
                );
            }
        } else {
            assert_eq!(path, base_dirs.config_dir().join("lazydb"));
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn fresh_install_uses_home_lazydb_without_creating_it() {
        let directory = tempdir().unwrap();
        let home = directory.path().join("home");
        fs::create_dir(&home).unwrap();

        let selected = resolve_config_dir(
            &home,
            None,
            vec![
                home.join(".config/lazydb"),
                home.join(".local/share/lazydb"),
            ],
        )
        .unwrap();

        assert_eq!(selected, home.join("lazydb"));
        assert!(!selected.exists());
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn existing_legacy_root_wins_but_empty_root_does_not() {
        let directory = tempdir().unwrap();
        let home = directory.path().join("home");
        let old = home.join(".config/lazydb");
        fs::create_dir_all(&old).unwrap();

        assert_eq!(
            resolve_config_dir(&home, None, vec![old.clone()]).unwrap(),
            home.join("lazydb")
        );
        fs::write(old.join("settings.toml"), "[ui]\n").unwrap();
        assert_eq!(
            resolve_config_dir(&home, None, vec![old.clone()]).unwrap(),
            old
        );
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn new_root_wins_over_its_compatibility_symlink() {
        let directory = tempdir().unwrap();
        let home = directory.path().join("home");
        let new_root = home.join("lazydb");
        let old_root = home.join(".config/lazydb");
        fs::create_dir_all(&new_root).unwrap();
        fs::create_dir_all(old_root.parent().unwrap()).unwrap();
        fs::write(new_root.join("settings.toml"), "[ui]\n").unwrap();
        std::os::unix::fs::symlink(&new_root, &old_root).unwrap();

        assert_eq!(
            resolve_config_dir(&home, None, vec![old_root]).unwrap(),
            new_root
        );
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn multiple_legacy_roots_are_not_merged() {
        let directory = tempdir().unwrap();
        let home = directory.path().join("home");
        let config = home.join(".config/lazydb");
        let data = home.join(".local/share/lazydb");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(&data).unwrap();
        fs::write(config.join("settings.toml"), "[ui]\n").unwrap();
        fs::write(data.join("install.json"), "{}\n").unwrap();

        let error = resolve_config_dir(&home, None, vec![config, data]).unwrap_err();
        assert!(error.to_string().contains("multiple"));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn explicit_relative_root_is_resolved_under_home() {
        let directory = tempdir().unwrap();
        let home = directory.path().join("home");
        fs::create_dir_all(&home).unwrap();

        assert_eq!(
            resolve_config_dir(&home, Some(PathBuf::from("custom/lazydb")), vec![]).unwrap(),
            home.join("custom/lazydb")
        );
    }
}
