use std::{fs, path::PathBuf};

use serde::Deserialize;
use thiserror::Error;

use crate::{
    cli::{ColorMode, ConfirmationPolicy, MotionMode, MouseMode},
    ui::icons::IconMode,
};

pub const CONFIG_VERSION: u16 = 1;
pub const DEFAULT_CONFIG_TOML: &str = include_str!("../config/default.toml");
const MIN_DASHBOARD_REFRESH_INTERVAL_SECONDS: u64 = 1;
const MIN_KEY_SEQUENCE_TIMEOUT_MS: u64 = 1;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("settings file I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("settings file is invalid: {0}")]
    Decode(#[from] toml::de::Error),
    #[error("settings merge failed: {0}")]
    Merge(String),
    #[error("configuration version {found} is unsupported; expected {expected}")]
    UnsupportedVersion { found: u16, expected: u16 },
    #[error(
        "dashboard.refresh_interval_seconds must be at least {MIN_DASHBOARD_REFRESH_INTERVAL_SECONDS}"
    )]
    InvalidDashboardRefreshInterval,
    #[error("keybindings.sequence_timeout_ms must be at least {MIN_KEY_SEQUENCE_TIMEOUT_MS}")]
    InvalidKeySequenceTimeout,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub version: u16,
    pub terminal: TerminalConfig,
    pub ui: UiConfig,
    pub execution: ExecutionConfig,
    pub dashboard: DashboardConfig,
    pub keybindings: KeybindingConfig,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TerminalConfig {
    pub mouse: MouseMode,
    pub color: ColorMode,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UiConfig {
    pub icons: IconMode,
    pub motion: MotionMode,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ExecutionConfig {
    pub confirmation: ConfirmationPolicy,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DashboardConfig {
    pub refresh_interval_seconds: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct KeybindingConfig {
    pub preset: KeybindingPreset,
    pub sequence_timeout_ms: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum KeybindingPreset {
    Vim,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self::from_toml(DEFAULT_CONFIG_TOML).expect("embedded default configuration must be valid")
    }
}

impl AppConfig {
    pub fn load(path: PathBuf) -> Result<Self, ConfigError> {
        let mut merged = toml::from_str::<toml::Value>(DEFAULT_CONFIG_TOML).map_err(|error| {
            ConfigError::Merge(format!(
                "embedded default configuration is invalid: {error}"
            ))
        })?;
        match fs::read_to_string(path) {
            Ok(contents) => {
                let overrides = toml::from_str::<toml::Value>(&contents)?;
                merge_values(&mut merged, overrides);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let config: Self = merged.try_into()?;
        config.validate()?;
        Ok(config)
    }

    pub fn from_toml(contents: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(contents)?;
        config.validate()?;
        Ok(config)
    }

    pub fn apply_cli_overrides(
        &mut self,
        mouse: Option<MouseMode>,
        color: Option<ColorMode>,
        icons: Option<IconMode>,
        motion: Option<MotionMode>,
        confirmation: Option<ConfirmationPolicy>,
    ) {
        if let Some(mouse) = mouse {
            self.terminal.mouse = mouse;
        }
        if let Some(color) = color {
            self.terminal.color = color;
        }
        if let Some(icons) = icons {
            self.ui.icons = icons;
        }
        if let Some(motion) = motion {
            self.ui.motion = motion;
        }
        if let Some(confirmation) = confirmation {
            self.execution.confirmation = confirmation;
        }
    }

    pub const fn dashboard_refresh_interval_millis(self) -> u64 {
        self.dashboard
            .refresh_interval_seconds
            .saturating_mul(1_000)
    }

    fn validate(self) -> Result<(), ConfigError> {
        if self.version != CONFIG_VERSION {
            return Err(ConfigError::UnsupportedVersion {
                found: self.version,
                expected: CONFIG_VERSION,
            });
        }
        if self.dashboard.refresh_interval_seconds < MIN_DASHBOARD_REFRESH_INTERVAL_SECONDS {
            return Err(ConfigError::InvalidDashboardRefreshInterval);
        }
        if self.keybindings.sequence_timeout_ms < MIN_KEY_SEQUENCE_TIMEOUT_MS {
            return Err(ConfigError::InvalidKeySequenceTimeout);
        }
        Ok(())
    }
}

fn merge_values(base: &mut toml::Value, overrides: toml::Value) {
    match (base, overrides) {
        (toml::Value::Table(base), toml::Value::Table(overrides)) => {
            for (key, value) in overrides {
                if let Some(base_value) = base.get_mut(&key) {
                    merge_values(base_value, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (base, overrides) => *base = overrides,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{AppConfig, ConfigError, DEFAULT_CONFIG_TOML};
    use crate::{cli::MotionMode, ui::icons::IconMode};

    #[test]
    fn embedded_default_configuration_is_complete_and_valid() {
        let config = AppConfig::from_toml(DEFAULT_CONFIG_TOML).unwrap();

        assert_eq!(config.ui.icons, IconMode::NerdFont);
        assert_eq!(config.keybindings.sequence_timeout_ms, 750);
    }

    #[test]
    fn missing_user_file_uses_embedded_defaults() {
        let temp = TempDir::new().unwrap();

        assert_eq!(
            AppConfig::load(temp.path().join("settings.toml")).unwrap(),
            AppConfig::default()
        );
    }

    #[test]
    fn user_file_only_overrides_explicit_values() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.toml");
        fs::write(&path, "[ui]\nmotion = \"reduced\"\n").unwrap();

        let config = AppConfig::load(path).unwrap();

        assert_eq!(config.ui.motion, MotionMode::Reduced);
        assert_eq!(config.ui.icons, IconMode::NerdFont);
        assert_eq!(config.dashboard.refresh_interval_seconds, 5);
    }

    #[test]
    fn explicit_cli_values_override_user_settings() {
        let mut config = AppConfig::from_toml(
            r#"
            version = 1
            [terminal]
            mouse = "off"
            color = "never"
            [ui]
            icons = "ascii"
            motion = "off"
            [execution]
            confirmation = "always"
            [dashboard]
            refresh_interval_seconds = 2
            [keybindings]
            preset = "vim"
            sequence_timeout_ms = 100
            "#,
        )
        .unwrap();

        config.apply_cli_overrides(
            Some(crate::cli::MouseMode::On),
            Some(crate::cli::ColorMode::Always),
            Some(IconMode::Unicode),
            Some(MotionMode::Reduced),
            Some(crate::cli::ConfirmationPolicy::RiskyOnly),
        );

        assert_eq!(config.terminal.mouse, crate::cli::MouseMode::On);
        assert_eq!(config.terminal.color, crate::cli::ColorMode::Always);
        assert_eq!(config.ui.icons, IconMode::Unicode);
        assert_eq!(config.ui.motion, MotionMode::Reduced);
        assert_eq!(
            config.execution.confirmation,
            crate::cli::ConfirmationPolicy::RiskyOnly
        );
        assert_eq!(config.dashboard.refresh_interval_seconds, 2);
    }

    #[test]
    fn unknown_fields_and_invalid_values_are_rejected() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.toml");
        fs::write(&path, "[ui]\niconz = \"ascii\"\n").unwrap();
        assert!(matches!(
            AppConfig::load(path.clone()),
            Err(ConfigError::Decode(_))
        ));

        fs::write(&path, "[keybindings]\nsequence_timeout_ms = 0\n").unwrap();
        assert!(matches!(
            AppConfig::load(path),
            Err(ConfigError::InvalidKeySequenceTimeout)
        ));
    }
}
