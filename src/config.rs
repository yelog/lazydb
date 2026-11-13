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
const MIN_REDIS_SCAN_COUNT_HINT: u32 = 1;
const MIN_REDIS_PAGE_SIZE: usize = 1;
const MIN_REDIS_STRING_PAGE_BYTES: usize = 1;
const MIN_REDIS_KEY_LIMIT: usize = 1;
const MIN_REDIS_KEY_BYTES: usize = 1;
const MIN_REDIS_VALUE_PAGE_BYTES: usize = 1;
const MIN_REDIS_VALUE_FORMAT_BYTES: usize = 1;
const MIN_REDIS_PREVIEW_DEBOUNCE_MS: u64 = 1;
const MIN_REDIS_READ_CONCURRENCY: usize = 1;
const MIN_REDIS_AUTO_FILL_REQUESTS: usize = 1;
const MIN_REDIS_AUTO_FILL_BUDGET_MS: u64 = 1;
const MIN_REDIS_INDEX_DISK_BYTES: u64 = 1;
const MIN_REDIS_METADATA_CACHE_CAPACITY: usize = 1;
const MIN_REDIS_METADATA_CACHE_TTL_SECONDS: u64 = 1;
const MIN_SSH_CONNECT_TIMEOUT_SECONDS: u64 = 1;
const MAX_SSH_CONNECT_TIMEOUT_SECONDS: u64 = 300;
const MIN_SSH_SERVER_ALIVE_INTERVAL_SECONDS: u64 = 1;
const MAX_SSH_SERVER_ALIVE_INTERVAL_SECONDS: u64 = 3_600;
const MIN_SSH_SERVER_ALIVE_COUNT_MAX: u32 = 1;
const MAX_SSH_SERVER_ALIVE_COUNT_MAX: u32 = 10;
const SUPPORTED_COMMANDS: &[&str] = &[
    "help",
    "omni",
    "quit",
    "terminal-selection",
    "notification-history",
    "open-sql-history",
    "update",
    "focus-next-pane",
    "focus-previous-pane",
    "run-statement",
    "run-buffer",
    "next-tab",
    "previous-tab",
    "move-tab-right",
    "move-tab-left",
    "close-tab",
    "open-dashboard",
    "open-explorer",
    "open-editors",
    "open-consoles",
    "run-leader-statement",
    "run-leader-buffer",
    "open-target-selector",
    "open-database-selector",
    "focus-pane-left",
    "focus-pane-down",
    "focus-pane-up",
    "focus-pane-right",
    "toggle-pane-maximized",
    "smart-toggle-pane-maximized",
    "reset-pane-sizes",
    "smart-focus-pane-left",
    "smart-focus-pane-down",
    "smart-focus-pane-up",
    "smart-focus-pane-right",
    "smart-resize-pane-left",
    "smart-resize-pane-down",
    "smart-resize-pane-up",
    "smart-resize-pane-right",
    "explorer-move-down",
    "explorer-move-up",
    "explorer-expand",
    "explorer-collapse",
    "results-move-left",
    "results-move-down",
    "results-move-up",
    "results-move-right",
    "results-first-page",
    "results-previous-page",
    "results-next-page",
    "results-last-page",
    "results-page-size",
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
    "explorer-create",
    "explorer-edit",
    "explorer-delete",
    "explorer-rename",
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
    #[error("redis settings must all be greater than zero")]
    InvalidRedisSettings,
    #[error(
        "connections.ssh settings must be bounded: connect_timeout_seconds 1..=300, server_alive_interval_seconds 1..=3600, and server_alive_count_max 1..=10"
    )]
    InvalidSshSettings,
    #[error("invalid keybinding for `{command}`: `{key}`")]
    InvalidKeybinding { command: String, key: String },
    #[error("keybinding `{key}` is assigned to both `{first}` and `{second}`")]
    ConflictingKeybindings {
        key: String,
        first: String,
        second: String,
    },
    #[error("invalid keybinding for `{command}`: `{key}` (token `{token}`: {reason})")]
    InvalidKeybindingDetail {
        command: String,
        key: String,
        token: String,
        reason: String,
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
    pub redis: RedisConfig,
    #[serde(default)]
    pub updates: UpdateConfig,
    pub keybindings: KeybindingConfig,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TerminalConfig {
    pub mouse: MouseMode,
    pub color: ColorMode,
    #[serde(default)]
    pub clipboard: ClipboardConfig,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ClipboardConfig {
    pub backend: ClipboardBackend,
    pub max_bytes: usize,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            backend: ClipboardBackend::System,
            max_bytes: 1_000_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ClipboardBackend {
    #[default]
    System,
    Osc52,
    Off,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UiConfig {
    pub icons: IconMode,
    pub motion: MotionMode,
    #[serde(default)]
    pub help_panel: HelpPanelView,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum HelpPanelView {
    #[default]
    Help,
    Omni,
}

impl HelpPanelView {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Help => "help",
            Self::Omni => "omni",
        }
    }
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
    #[serde(default)]
    pub ssh: SshConfig,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SshConfig {
    pub connect_timeout_seconds: u64,
    pub server_alive_interval_seconds: u64,
    pub server_alive_count_max: u32,
}

impl Default for SshConfig {
    fn default() -> Self {
        #[derive(Deserialize)]
        struct EmbeddedDefaults {
            connections: EmbeddedConnections,
        }

        #[derive(Deserialize)]
        struct EmbeddedConnections {
            ssh: SshConfig,
        }

        let defaults: EmbeddedDefaults = toml::from_str(DEFAULT_CONFIG_TOML)
            .expect("embedded default configuration must be valid");
        defaults.connections.ssh
    }
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
pub struct RedisConfig {
    pub scan_count_hint: u32,
    pub page_size: usize,
    pub string_page_bytes: usize,
    pub memory_key_limit: usize,
    pub memory_key_bytes: usize,
    pub value_page_bytes: usize,
    pub value_format_bytes: usize,
    pub preview_debounce_ms: u64,
    pub max_read_concurrency: usize,
    pub auto_fill_request_limit: usize,
    pub auto_fill_budget_ms: u64,
    pub index_disk_limit_bytes: u64,
    pub metadata_cache_capacity: usize,
    pub metadata_cache_ttl_seconds: u64,
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            scan_count_hint: 200,
            page_size: 200,
            string_page_bytes: 64 * 1024,
            memory_key_limit: 10_000,
            memory_key_bytes: 16 * 1024 * 1024,
            value_page_bytes: 1024 * 1024,
            value_format_bytes: 1024 * 1024,
            preview_debounce_ms: 100,
            max_read_concurrency: 4,
            auto_fill_request_limit: 8,
            auto_fill_budget_ms: 100,
            index_disk_limit_bytes: 1024 * 1024 * 1024,
            metadata_cache_capacity: 2_000,
            metadata_cache_ttl_seconds: 5,
        }
    }
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
    pub fn configured_sequences(&self, command: &str) -> &[Vec<KeyEvent>] {
        self.commands.get(command).map_or(&[], Vec::as_slice)
    }

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

    pub fn configured_for(&self, command: &str) -> bool {
        self.commands
            .get(command)
            .is_some_and(|sequences| !sequences.is_empty())
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

pub(crate) fn keybinding_display(event: &KeyEvent) -> String {
    let mut parts = Vec::new();
    for (modifier, name) in [
        (KeyModifiers::CONTROL, "Ctrl"),
        (KeyModifiers::ALT, "Alt"),
        (KeyModifiers::SHIFT, "Shift"),
        (KeyModifiers::SUPER, "Cmd"),
        (KeyModifiers::META, "Meta"),
        (KeyModifiers::HYPER, "Hyper"),
    ] {
        if event.modifiers.contains(modifier) {
            parts.push(name.to_owned());
        }
    }
    let key = match event.code {
        KeyCode::Char(' ') => "Space".to_owned(),
        KeyCode::Char(character) => character.to_string(),
        KeyCode::F(number) => format!("F{number}"),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Enter => "Enter".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::BackTab => "BackTab".into(),
        KeyCode::Backspace => "Backspace".into(),
        KeyCode::Delete => "Delete".into(),
        KeyCode::Insert => "Insert".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::PageUp => "PageUp".into(),
        KeyCode::PageDown => "PageDown".into(),
        _ => format!("{:?}", event.code),
    };
    parts.push(key);
    parts.join("+")
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
        let mut commands: BTreeMap<String, Vec<Vec<KeyEvent>>> = BTreeMap::new();
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
                if keys.is_empty() {
                    commands.remove(&command);
                    display.insert(command, Vec::new());
                    continue;
                }
                let sequences = keys
                    .iter()
                    .map(|key| {
                        if command == "smart-toggle-pane-maximized"
                            && key.split_whitespace().count() != 1
                        {
                            return Err(ConfigError::InvalidKeybindingDetail {
                                command: command.clone(),
                                key: key.clone(),
                                token: key.clone(),
                                reason: "smart maximize requires a single key chord".into(),
                            });
                        }
                        if key.split_whitespace().next().is_none() {
                            return Err(ConfigError::InvalidKeybinding {
                                command: command.clone(),
                                key: key.clone(),
                            });
                        }
                        key.split_whitespace()
                            .map(|part| {
                                parse_key(part).ok_or_else(|| {
                                    ConfigError::InvalidKeybindingDetail {
                                        command: command.clone(),
                                        key: key.clone(),
                                        token: part.to_owned(),
                                        reason: "unsupported key or modifier".into(),
                                    }
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
        for command in crate::input::panes::PaneCommand::ALL {
            for sequence in commands.get(command.name()).into_iter().flatten() {
                if sequence.len() >= 2
                    && sequence[0] == KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)
                    && sequence[1] == KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL)
                {
                    return Err(ConfigError::InvalidKeybinding {
                        command: command.name().to_owned(),
                        key: display
                            .get(command.name())
                            .and_then(|values| values.first())
                            .cloned()
                            .unwrap_or_default(),
                    });
                }
            }
        }
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
                    let key = if key.is_empty() {
                        display
                            .get(*second_command)
                            .and_then(|keys| keys.first())
                            .cloned()
                            .unwrap_or_default()
                    } else {
                        key
                    };
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
        if self.redis.scan_count_hint < MIN_REDIS_SCAN_COUNT_HINT
            || self.redis.page_size < MIN_REDIS_PAGE_SIZE
            || self.redis.string_page_bytes < MIN_REDIS_STRING_PAGE_BYTES
            || self.redis.memory_key_limit < MIN_REDIS_KEY_LIMIT
            || self.redis.memory_key_bytes < MIN_REDIS_KEY_BYTES
            || self.redis.value_page_bytes < MIN_REDIS_VALUE_PAGE_BYTES
            || self.redis.value_format_bytes < MIN_REDIS_VALUE_FORMAT_BYTES
            || self.redis.preview_debounce_ms < MIN_REDIS_PREVIEW_DEBOUNCE_MS
            || self.redis.max_read_concurrency < MIN_REDIS_READ_CONCURRENCY
            || self.redis.auto_fill_request_limit < MIN_REDIS_AUTO_FILL_REQUESTS
            || self.redis.auto_fill_budget_ms < MIN_REDIS_AUTO_FILL_BUDGET_MS
            || self.redis.index_disk_limit_bytes < MIN_REDIS_INDEX_DISK_BYTES
            || self.redis.metadata_cache_capacity < MIN_REDIS_METADATA_CACHE_CAPACITY
            || self.redis.metadata_cache_ttl_seconds < MIN_REDIS_METADATA_CACHE_TTL_SECONDS
        {
            return Err(ConfigError::InvalidRedisSettings);
        }
        if !(MIN_SSH_CONNECT_TIMEOUT_SECONDS..=MAX_SSH_CONNECT_TIMEOUT_SECONDS)
            .contains(&self.connections.ssh.connect_timeout_seconds)
            || !(MIN_SSH_SERVER_ALIVE_INTERVAL_SECONDS..=MAX_SSH_SERVER_ALIVE_INTERVAL_SECONDS)
                .contains(&self.connections.ssh.server_alive_interval_seconds)
            || !(MIN_SSH_SERVER_ALIVE_COUNT_MAX..=MAX_SSH_SERVER_ALIVE_COUNT_MAX)
                .contains(&self.connections.ssh.server_alive_count_max)
        {
            return Err(ConfigError::InvalidSshSettings);
        }
        self.keybindings.key_bindings()?;
        Ok(())
    }
}

fn parse_key(value: &str) -> Option<KeyEvent> {
    let mut modifiers = KeyModifiers::NONE;
    let mut key = value;
    let has_plus = value.contains('+');
    let has_hyphen = value.contains('-');
    if has_plus && has_hyphen {
        return None;
    }
    if (has_plus && value.matches('+').count() > 1)
        || (has_hyphen && value != "-" && value.matches('-').count() > 1)
    {
        let separator = if has_plus { '+' } else { '-' };
        if value.split(separator).any(|part| part.is_empty()) {
            return None;
        }
    }
    let separator = if has_plus {
        value.rsplit_once('+')
    } else if has_hyphen && value != "-" {
        value.rsplit_once('-')
    } else {
        None
    };
    if let Some((prefix, rest)) = separator {
        key = rest;
        let separator = if prefix.contains('+') { '+' } else { '-' };
        for modifier in prefix.split(separator) {
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" if !modifiers.contains(KeyModifiers::CONTROL) => {
                    modifiers |= KeyModifiers::CONTROL
                }
                "shift" if !modifiers.contains(KeyModifiers::SHIFT) => {
                    modifiers |= KeyModifiers::SHIFT
                }
                "alt" | "option" if !modifiers.contains(KeyModifiers::ALT) => {
                    modifiers |= KeyModifiers::ALT
                }
                "cmd" | "command" | "super" if !modifiers.contains(KeyModifiers::SUPER) => {
                    modifiers |= KeyModifiers::SUPER
                }
                "meta" if !modifiers.contains(KeyModifiers::META) => {
                    modifiers |= KeyModifiers::META
                }
                "hyper" if !modifiers.contains(KeyModifiers::HYPER) => {
                    modifiers |= KeyModifiers::HYPER
                }
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
        "plus" => KeyCode::Char('+'),
        "minus" => KeyCode::Char('-'),
        "esc" | "escape" => KeyCode::Esc,
        "enter" => KeyCode::Enter,
        "tab" if modifiers.contains(KeyModifiers::SHIFT) => {
            modifiers.remove(KeyModifiers::SHIFT);
            KeyCode::BackTab
        }
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
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

    use super::{
        AppConfig, ClipboardBackend, ConfigError, ConnectionAccessDefault, DEFAULT_CONFIG_TOML,
        HelpPanelView, SshConfig,
    };
    use crate::{cli::MotionMode, ui::icons::IconMode};

    #[test]
    fn cmd_ctrl_pane_binding_loads_and_preserves_other_defaults() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("settings.toml");
        fs::write(
            &path,
            "[keybindings.panes]\nfocus-pane-left = [\"Cmd+Ctrl+h\"]\n",
        )
        .unwrap();

        let config = AppConfig::load(path).unwrap();
        let bindings = config.keybindings.key_bindings().unwrap();

        assert!(bindings.matches(
            "focus-pane-left",
            KeyEvent::new(
                KeyCode::Char('h'),
                KeyModifiers::SUPER | KeyModifiers::CONTROL
            )
        ));
        assert!(bindings.matches("help", KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)));
    }

    #[test]
    fn pane_binding_modifier_aliases_and_special_keys_are_supported() {
        let mut config = AppConfig::default();
        config.keybindings.panes.insert(
            "focus-pane-left".into(),
            vec!["Super+Control+Left".into(), "Command-Control-Plus".into()],
        );
        let bindings = config.keybindings.key_bindings().unwrap();

        assert!(bindings.matches(
            "focus-pane-left",
            KeyEvent::new(KeyCode::Left, KeyModifiers::SUPER | KeyModifiers::CONTROL)
        ));
        assert!(bindings.matches(
            "focus-pane-left",
            KeyEvent::new(
                KeyCode::Char('+'),
                KeyModifiers::SUPER | KeyModifiers::CONTROL
            )
        ));
    }

    #[test]
    fn embedded_default_configuration_is_complete_and_valid() {
        let config = AppConfig::from_toml(DEFAULT_CONFIG_TOML).unwrap();

        assert_eq!(config.ui.icons, IconMode::NerdFont);
        assert_eq!(config.ui.help_panel, HelpPanelView::Help);
        assert_eq!(config.keybindings.sequence_timeout_ms, 750);
        assert_eq!(
            config.connections.default_access,
            ConnectionAccessDefault::Global
        );
        assert_eq!(config.connections.ssh, SshConfig::default());
        assert_eq!(config.connections.ssh.connect_timeout_seconds, 30);
        assert_eq!(config.connections.ssh.server_alive_interval_seconds, 30);
        assert_eq!(config.connections.ssh.server_alive_count_max, 3);
        assert_eq!(config.terminal.clipboard.backend, ClipboardBackend::System);
        assert_eq!(config.redis.scan_count_hint, 200);
        assert_eq!(config.redis.page_size, 200);
        assert_eq!(config.redis.memory_key_limit, 10_000);
        assert_eq!(config.redis.preview_debounce_ms, 100);
    }

    #[test]
    fn help_panel_setting_defaults_for_legacy_config_and_accepts_omni() {
        let legacy = DEFAULT_CONFIG_TOML.replace("help_panel = \"help\"\n", "");
        assert_eq!(
            AppConfig::from_toml(&legacy).unwrap().ui.help_panel,
            HelpPanelView::Help
        );
        let omni = DEFAULT_CONFIG_TOML.replace("help_panel = \"help\"", "help_panel = \"omni\"");
        assert_eq!(
            AppConfig::from_toml(&omni).unwrap().ui.help_panel,
            HelpPanelView::Omni
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
    fn missing_redis_section_uses_redis_defaults() {
        let (before, after) = DEFAULT_CONFIG_TOML.split_once("\n[redis]\n").unwrap();
        let after = after.split_once("\n[updates]\n").unwrap().1;
        let contents = format!("{before}\n[updates]\n{after}");
        let config = AppConfig::from_toml(&contents).unwrap();
        assert_eq!(config.redis, super::RedisConfig::default());
    }

    #[test]
    fn missing_ssh_section_uses_bounded_ssh_defaults() {
        let (before, after) = DEFAULT_CONFIG_TOML
            .split_once("\n[connections.ssh]\n")
            .unwrap();
        let after = after.split_once("\n[dashboard]\n").unwrap().1;
        let contents = format!("{before}\n[dashboard]\n{after}");

        let config = AppConfig::from_toml(&contents).unwrap();

        assert_eq!(config.connections.ssh, SshConfig::default());
    }

    #[test]
    fn partial_ssh_settings_are_merged_with_embedded_defaults() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("settings.toml");
        fs::write(&path, "[connections.ssh]\nconnect_timeout_seconds = 45\n").unwrap();

        let config = AppConfig::load(path).unwrap();

        assert_eq!(config.connections.ssh.connect_timeout_seconds, 45);
        assert_eq!(config.connections.ssh.server_alive_interval_seconds, 30);
        assert_eq!(config.connections.ssh.server_alive_count_max, 3);
    }

    #[test]
    fn legacy_settings_without_ssh_use_embedded_defaults() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("settings.toml");
        fs::write(&path, "[ui]\nmotion = \"reduced\"\n").unwrap();

        let config = AppConfig::load(path).unwrap();

        assert_eq!(config.connections.ssh, SshConfig::default());
    }

    #[test]
    fn ssh_settings_accept_the_documented_upper_bounds() {
        let contents = DEFAULT_CONFIG_TOML
            .replace(
                "connect_timeout_seconds = 30",
                "connect_timeout_seconds = 300",
            )
            .replace(
                "server_alive_interval_seconds = 30",
                "server_alive_interval_seconds = 3600",
            )
            .replace("server_alive_count_max = 3", "server_alive_count_max = 10");

        let config = AppConfig::from_toml(&contents).unwrap();

        assert_eq!(config.connections.ssh.connect_timeout_seconds, 300);
        assert_eq!(config.connections.ssh.server_alive_interval_seconds, 3_600);
        assert_eq!(config.connections.ssh.server_alive_count_max, 10);
    }

    #[test]
    fn ssh_settings_reject_zero_and_out_of_range_values() {
        for (from, to) in [
            (
                "connect_timeout_seconds = 30",
                "connect_timeout_seconds = 0",
            ),
            (
                "connect_timeout_seconds = 30",
                "connect_timeout_seconds = 301",
            ),
            (
                "server_alive_interval_seconds = 30",
                "server_alive_interval_seconds = 0",
            ),
            (
                "server_alive_interval_seconds = 30",
                "server_alive_interval_seconds = 3601",
            ),
            ("server_alive_count_max = 3", "server_alive_count_max = 0"),
            ("server_alive_count_max = 3", "server_alive_count_max = 11"),
        ] {
            let contents = DEFAULT_CONFIG_TOML.replace(from, to);
            assert!(
                matches!(
                    AppConfig::from_toml(&contents),
                    Err(ConfigError::InvalidSshSettings)
                ),
                "accepted {to}"
            );
        }
    }

    #[test]
    fn redis_settings_reject_zero_values() {
        let contents = DEFAULT_CONFIG_TOML.replace("scan_count_hint = 200", "scan_count_hint = 0");
        assert!(matches!(
            AppConfig::from_toml(&contents),
            Err(ConfigError::InvalidRedisSettings)
        ));
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
    fn clipboard_backend_is_recursively_overridden() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.toml");
        fs::write(
            &path,
            "[terminal.clipboard]\nbackend = \"osc52\"\nmax_bytes = 42\n",
        )
        .unwrap();
        let config = AppConfig::load(path).unwrap();
        assert_eq!(config.terminal.clipboard.backend, ClipboardBackend::Osc52);
        assert_eq!(config.terminal.clipboard.max_bytes, 42);
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
        assert!(matches!(
            error,
            ConfigError::InvalidKeybinding { .. } | ConfigError::InvalidKeybindingDetail { .. }
        ));
    }

    #[test]
    fn pane_binding_rejects_unknown_modifier_with_original_value() {
        let mut config = AppConfig::default();
        config
            .keybindings
            .panes
            .insert("focus-pane-left".into(), vec!["Cmd+Mispelled+h".into()]);

        let error = config.keybindings.key_bindings().unwrap_err();

        assert!(matches!(
            error,
            ConfigError::InvalidKeybindingDetail { command, key, token, .. }
                if command == "focus-pane-left"
                    && key == "Cmd+Mispelled+h"
                    && token == "Cmd+Mispelled+h"
        ));
    }

    #[test]
    fn all_pane_commands_are_known_and_empty_bindings_disable_them() {
        let mut config = AppConfig::default();
        for command in [
            "focus-pane-left",
            "focus-pane-down",
            "focus-pane-up",
            "focus-pane-right",
            "toggle-pane-maximized",
            "reset-pane-sizes",
        ] {
            config.keybindings.panes.insert(command.into(), Vec::new());
        }
        let bindings = config.keybindings.key_bindings().unwrap();
        for command in [
            "focus-pane-left",
            "focus-pane-down",
            "focus-pane-up",
            "focus-pane-right",
            "toggle-pane-maximized",
            "reset-pane-sizes",
        ] {
            assert!(
                !bindings.configured_for(command),
                "{command} remained enabled"
            );
            assert!(bindings.configured_sequences(command).is_empty());
        }
    }

    #[test]
    fn default_bindings_match_the_declared_events() {
        let bindings = AppConfig::default().keybindings.key_bindings().unwrap();

        assert!(bindings.matches("help", KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)));
        assert!(bindings.matches("omni", KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)));
        assert!(bindings.matches(
            "quit",
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
        ));
        assert!(bindings.matches(
            "terminal-selection",
            KeyEvent::new(
                KeyCode::Char('s'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )
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
        assert!(bindings.matches(
            "move-tab-right",
            KeyEvent::new(
                KeyCode::Char('n'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )
        ));
        assert!(bindings.matches(
            "move-tab-left",
            KeyEvent::new(
                KeyCode::Char('p'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )
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
