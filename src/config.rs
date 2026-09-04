use std::{collections::BTreeMap, fs, path::PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
const MIN_UPDATE_CHECK_INTERVAL_HOURS: u64 = 1;
const SUPPORTED_COMMANDS: &[&str] = &[
    "help",
    "quit",
    "notification-history",
    "update",
    "focus-next-pane",
    "focus-previous-pane",
    "run-statement",
    "run-buffer",
    "next-tab",
    "previous-tab",
    "close-tab",
    "open-dashboard",
    "open-explorer",
    "open-editors",
    "run-leader-statement",
    "run-leader-buffer",
    "open-target-selector",
    "focus-pane-left",
    "focus-pane-down",
    "focus-pane-up",
    "focus-pane-right",
    "toggle-pane-maximized",
    "reset-pane-sizes",
    "explorer-move-down",
    "explorer-move-up",
    "explorer-expand",
    "explorer-collapse",
    "results-move-left",
    "results-move-down",
    "results-move-up",
    "results-move-right",
    "results-open-record",
    "results-copy-cell",
    "results-copy-row",
    "results-copy-row-headers",
    "results-toggle-view",
    "results-first-column",
    "results-last-column",
    "results-align-middle",
    "results-align-top",
    "results-align-bottom",
    "explorer-copy-selection",
    "explorer-find",
    "explorer-search",
    "explorer-new-profile",
    "explorer-refresh",
    "explorer-toggle",
];

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
    #[error("updates.check_interval_hours must be at least 1")]
    InvalidUpdateCheckInterval,
    #[error("invalid keybinding for `{command}`: `{key}`")]
    InvalidKeybinding { command: String, key: String },
    #[error("keybinding `{key}` is assigned to both `{first}` and `{second}`")]
    ConflictingKeybindings {
        key: String,
        first: String,
        second: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub version: u16,
    pub terminal: TerminalConfig,
    pub ui: UiConfig,
    pub execution: ExecutionConfig,
    #[serde(default)]
    pub connections: ConnectionsConfig,
    pub dashboard: DashboardConfig,
    #[serde(default)]
    pub updates: UpdateConfig,
    pub keybindings: KeybindingConfig,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TerminalConfig {
    pub mouse: MouseMode,
    pub color: ColorMode,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConnectionsConfig {
    pub default_access: ConnectionAccessDefault,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionAccessDefault {
    #[default]
    Global,
    Project,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DashboardConfig {
    pub refresh_interval_seconds: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UpdateConfig {
    pub check_on_startup: bool,
    pub check_interval_hours: u64,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            check_on_startup: true,
            check_interval_hours: 24,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct KeybindingConfig {
    pub preset: KeybindingPreset,
    pub sequence_timeout_ms: u64,
    #[serde(default)]
    pub global: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub leader: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub panes: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub explorer: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub results: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub editor: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub overlays: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyBindings {
    commands: BTreeMap<String, Vec<Vec<KeyEvent>>>,
    display: BTreeMap<String, Vec<String>>,
}

impl KeyBindings {
    pub fn matches(&self, command: &str, event: KeyEvent) -> bool {
        self.commands
            .get(command)
            .is_some_and(|sequences| sequences.iter().any(|sequence| sequence == &[event]))
    }

    pub fn matches_sequence(&self, command: &str, events: &[KeyEvent]) -> bool {
        self.commands
            .get(command)
            .is_some_and(|sequences| sequences.iter().any(|sequence| sequence == events))
    }

    pub fn matching_command(&self, events: &[KeyEvent]) -> Option<&str> {
        self.commands.iter().find_map(|(command, sequences)| {
            sequences
                .iter()
                .any(|sequence| sequence == events)
                .then_some(command.as_str())
        })
    }

    pub fn is_configured(&self, command: &str) -> bool {
        self.commands.contains_key(command)
    }

    pub fn has_any_prefix(&self, events: &[KeyEvent]) -> bool {
        self.commands
            .values()
            .flatten()
            .any(|sequence| sequence.len() > events.len() && sequence.starts_with(events))
    }

    pub fn has_sequence_prefix(&self, command: &str, events: &[KeyEvent]) -> bool {
        self.commands.get(command).is_some_and(|sequences| {
            sequences
                .iter()
                .any(|sequence| sequence.starts_with(events))
        })
    }

    pub fn sequence_for(&self, command: &str) -> Option<&[KeyEvent]> {
        self.commands
            .get(command)
            .and_then(|sequences| sequences.first().map(Vec::as_slice))
    }

    pub fn display_for(&self, command: &str) -> Option<String> {
        self.display.get(command).map(|keys| keys.join(", "))
    }
}

impl KeybindingConfig {
    pub fn key_bindings(&self) -> Result<KeyBindings, ConfigError> {
        let groups = [
            ("", &self.global),
            ("", &self.leader),
            ("", &self.panes),
            ("explorer-", &self.explorer),
            ("results-", &self.results),
            ("", &self.editor),
            ("", &self.overlays),
        ];
        let mut commands = BTreeMap::new();
        let mut display = BTreeMap::new();
        for (prefix, group) in groups {
            for (name, keys) in group {
                let command = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}{name}")
                };
                if !SUPPORTED_COMMANDS.contains(&command.as_str()) {
                    return Err(ConfigError::InvalidKeybinding {
                        command: command.clone(),
                        key: "unknown command".to_owned(),
                    });
                }
                let sequences = keys
                    .iter()
                    .map(|key| {
                        if key.split_whitespace().next().is_none() {
                            return Err(ConfigError::InvalidKeybinding {
                                command: command.clone(),
                                key: key.clone(),
                            });
                        }
                        key.split_whitespace()
                            .map(|part| {
                                parse_key(part).ok_or_else(|| ConfigError::InvalidKeybinding {
                                    command: command.clone(),
                                    key: key.clone(),
                                })
                            })
                            .collect()
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                commands.insert(command.clone(), sequences);
                display.insert(command, keys.clone());
            }
        }
        let entries = commands.iter().collect::<Vec<_>>();
        for (index, (first_command, first_sequences)) in entries.iter().enumerate() {
            for (second_command, second_sequences) in entries.iter().skip(index + 1) {
                if commands_share_context(first_command, second_command)
                    && first_sequences
                        .iter()
                        .any(|first| second_sequences.iter().any(|second| first == second))
                {
                    let key = display
                        .get(*first_command)
                        .and_then(|keys| keys.first())
                        .cloned()
                        .unwrap_or_default();
                    return Err(ConfigError::ConflictingKeybindings {
                        key,
                        first: (*first_command).clone(),
                        second: (*second_command).clone(),
                    });
                }
            }
        }
        Ok(KeyBindings { commands, display })
    }
}

fn commands_share_context(first: &str, second: &str) -> bool {
    let context = |command: &str| {
        if command.starts_with("explorer-") {
            Some("explorer")
        } else if command.starts_with("results-") {
            Some("results")
        } else {
            None
        }
    };
    context(first) == context(second)
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

    pub const fn dashboard_refresh_interval_millis(&self) -> u64 {
        self.dashboard
            .refresh_interval_seconds
            .saturating_mul(1_000)
    }

    pub const fn update_check_interval_hours(&self) -> u64 {
        self.updates.check_interval_hours
    }

    pub fn keybindings_for(&self, command: &str) -> &[String] {
        let (group, name) = if let Some(name) = command.strip_prefix("explorer-") {
            (&self.keybindings.explorer, name)
        } else if let Some(name) = command.strip_prefix("results-") {
            (&self.keybindings.results, name)
        } else {
            let groups = [
                &self.keybindings.global,
                &self.keybindings.leader,
                &self.keybindings.panes,
                &self.keybindings.editor,
                &self.keybindings.overlays,
            ];
            return groups
                .into_iter()
                .find_map(|group| group.get(command).map(Vec::as_slice))
                .unwrap_or(&[]);
        };
        group.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    fn validate(&self) -> Result<(), ConfigError> {
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
        if self.updates.check_interval_hours < MIN_UPDATE_CHECK_INTERVAL_HOURS {
            return Err(ConfigError::InvalidUpdateCheckInterval);
        }
        self.keybindings.key_bindings()?;
        Ok(())
    }
}

fn parse_key(value: &str) -> Option<KeyEvent> {
    let mut modifiers = KeyModifiers::NONE;
    let mut key = value;
    if let Some((prefix, rest)) = value.rsplit_once('-') {
        key = rest;
        for modifier in prefix.split('-') {
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
                "shift" => modifiers |= KeyModifiers::SHIFT,
                "alt" => modifiers |= KeyModifiers::ALT,
                _ => return None,
            }
        }
    }
    let key_name = key.to_ascii_lowercase();
    let code = match key_name.as_str() {
        "f1" => KeyCode::F(1),
        "f2" => KeyCode::F(2),
        "f3" => KeyCode::F(3),
        "f4" => KeyCode::F(4),
        "f5" => KeyCode::F(5),
        "f6" => KeyCode::F(6),
        "f7" => KeyCode::F(7),
        "f8" => KeyCode::F(8),
        "f9" => KeyCode::F(9),
        "f10" => KeyCode::F(10),
        "f11" => KeyCode::F(11),
        "f12" => KeyCode::F(12),
        "space" => KeyCode::Char(' '),
        "esc" | "escape" => KeyCode::Esc,
        "enter" => KeyCode::Enter,
        "tab" if modifiers.contains(KeyModifiers::SHIFT) => {
            modifiers.remove(KeyModifiers::SHIFT);
            KeyCode::BackTab
        }
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        _ => {
            let mut chars = key.chars();
            let character = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            KeyCode::Char(character)
        }
    };
    Some(KeyEvent::new(code, modifiers))
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
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::fs;

    use tempfile::TempDir;

    use super::{AppConfig, ConfigError, ConnectionAccessDefault, DEFAULT_CONFIG_TOML};
    use crate::{cli::MotionMode, ui::icons::IconMode};

    #[test]
    fn embedded_default_configuration_is_complete_and_valid() {
        let config = AppConfig::from_toml(DEFAULT_CONFIG_TOML).unwrap();

        assert_eq!(config.ui.icons, IconMode::NerdFont);
        assert_eq!(config.keybindings.sequence_timeout_ms, 750);
        assert_eq!(
            config.connections.default_access,
            ConnectionAccessDefault::Global
        );
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
            [connections]
            default_access = "project"
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
        assert_eq!(
            config.connections.default_access,
            ConnectionAccessDefault::Project
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

    #[test]
    fn invalid_keybinding_is_rejected() {
        let error = AppConfig::from_toml(
            r#"
            version = 1
            [terminal]
            mouse = "auto"
            color = "auto"
            [ui]
            icons = "nerd-font"
            motion = "full"
            [execution]
            confirmation = "risky"
            [dashboard]
            refresh_interval_seconds = 5
            [keybindings]
            preset = "vim"
            sequence_timeout_ms = 750
            [keybindings.global]
            help = ["F99"]
            "#,
        )
        .unwrap_err();
        assert!(matches!(error, ConfigError::InvalidKeybinding { .. }));
    }

    #[test]
    fn default_bindings_match_the_declared_events() {
        let bindings = AppConfig::default().keybindings.key_bindings().unwrap();

        assert!(bindings.matches("help", KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)));
        assert!(bindings.matches(
            "quit",
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
        ));
        assert!(bindings.matches(
            "focus-previous-pane",
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE)
        ));
        assert!(bindings.matches(
            "help",
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)
        ));
        assert!(!bindings.matches(
            "quit",
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)
        ));
        assert!(bindings.matches(
            "run-statement",
            KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE)
        ));
        assert!(bindings.matches(
            "run-buffer",
            KeyEvent::new(KeyCode::F(5), KeyModifiers::SHIFT)
        ));
        assert!(bindings.matches(
            "next-tab",
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL)
        ));
        assert!(bindings.matches_sequence(
            "next-tab",
            &[
                KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
            ]
        ));
        assert!(bindings.matches(
            "explorer-move-down",
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)
        ));
        assert!(bindings.matches(
            "results-move-right",
            KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE)
        ));
        assert!(bindings.matches_sequence(
            "open-dashboard",
            &[
                KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE),
            ]
        ));
    }

    #[test]
    fn conflicting_bindings_are_rejected() {
        let error = AppConfig::from_toml(
            r#"
            version = 1
            [terminal]
            mouse = "auto"
            color = "auto"
            [ui]
            icons = "nerd-font"
            motion = "full"
            [execution]
            confirmation = "risky"
            [dashboard]
            refresh_interval_seconds = 5
            [keybindings]
            preset = "vim"
            sequence_timeout_ms = 750
            [keybindings.global]
            help = ["F2"]
            quit = ["F2"]
            "#,
        )
        .unwrap_err();

        assert!(matches!(error, ConfigError::ConflictingKeybindings { .. }));
    }

    #[test]
    fn bindings_in_different_contexts_may_reuse_a_key() {
        let config = AppConfig::from_toml(
            r#"
            version = 1
            [terminal]
            mouse = "auto"
            color = "auto"
            [ui]
            icons = "nerd-font"
            motion = "full"
            [execution]
            confirmation = "risky"
            [dashboard]
            refresh_interval_seconds = 5
            [keybindings]
            preset = "vim"
            sequence_timeout_ms = 750
            [keybindings.explorer]
            move-down = ["j"]
            [keybindings.results]
            move-down = ["j"]
            "#,
        );

        assert!(config.is_ok());
    }

    #[test]
    fn default_keybindings_are_grouped_by_panel() {
        for section in [
            "[keybindings.global]",
            "[keybindings.leader]",
            "[keybindings.panes]",
            "[keybindings.explorer]",
            "[keybindings.results]",
            "[keybindings.editor]",
            "[keybindings.overlays]",
        ] {
            assert!(
                DEFAULT_CONFIG_TOML.contains(section),
                "missing default keybinding section: {section}"
            );
        }
    }
}
