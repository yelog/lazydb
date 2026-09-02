use std::{fs, path::PathBuf};

use serde::Deserialize;
use thiserror::Error;

pub const DEFAULT_DASHBOARD_REFRESH_INTERVAL_SECONDS: u64 = 5;
const MIN_DASHBOARD_REFRESH_INTERVAL_SECONDS: u64 = 1;

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("settings file I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("settings file is invalid: {0}")]
    Decode(#[from] toml::de::Error),
    #[error(
        "dashboard.refresh_interval_seconds must be at least {MIN_DASHBOARD_REFRESH_INTERVAL_SECONDS}"
    )]
    InvalidDashboardRefreshInterval,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppSettings {
    pub dashboard_refresh_interval_seconds: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsFile {
    #[serde(default)]
    dashboard: DashboardSettings,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DashboardSettings {
    #[serde(default = "default_refresh_interval")]
    refresh_interval_seconds: u64,
}

impl Default for DashboardSettings {
    fn default() -> Self {
        Self {
            refresh_interval_seconds: default_refresh_interval(),
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            dashboard_refresh_interval_seconds: default_refresh_interval(),
        }
    }
}

impl AppSettings {
    pub fn load(path: PathBuf) -> Result<Self, SettingsError> {
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error.into()),
        };
        let file: SettingsFile = toml::from_str(&contents)?;
        if file.dashboard.refresh_interval_seconds < MIN_DASHBOARD_REFRESH_INTERVAL_SECONDS {
            return Err(SettingsError::InvalidDashboardRefreshInterval);
        }
        Ok(Self {
            dashboard_refresh_interval_seconds: file.dashboard.refresh_interval_seconds,
        })
    }

    pub const fn dashboard_refresh_interval_millis(self) -> u64 {
        self.dashboard_refresh_interval_seconds
            .saturating_mul(1_000)
    }
}

const fn default_refresh_interval() -> u64 {
    DEFAULT_DASHBOARD_REFRESH_INTERVAL_SECONDS
}

#[cfg(test)]
mod tests {
    use super::{AppSettings, DEFAULT_DASHBOARD_REFRESH_INTERVAL_SECONDS, SettingsError};
    use std::fs;

    #[test]
    fn missing_settings_use_five_second_default() {
        let settings =
            AppSettings::load("/tmp/lazydb-settings-does-not-exist.toml".into()).unwrap();
        assert_eq!(
            settings.dashboard_refresh_interval_seconds,
            DEFAULT_DASHBOARD_REFRESH_INTERVAL_SECONDS
        );
    }

    #[test]
    fn dashboard_refresh_interval_is_loaded_and_validated() {
        let path =
            std::env::temp_dir().join(format!("lazydb-settings-{}.toml", std::process::id()));
        fs::write(&path, "[dashboard]\nrefresh_interval_seconds = 8\n").unwrap();
        let settings = AppSettings::load(path.clone()).unwrap();
        assert_eq!(settings.dashboard_refresh_interval_millis(), 8_000);

        fs::write(&path, "[dashboard]\nrefresh_interval_seconds = 0\n").unwrap();
        assert!(matches!(
            AppSettings::load(path.clone()),
            Err(SettingsError::InvalidDashboardRefreshInterval)
        ));
        let _ = fs::remove_file(path);
    }
}
