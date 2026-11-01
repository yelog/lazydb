use std::{
    collections::HashSet,
    collections::hash_map::DefaultHasher,
    fmt,
    hash::{Hash, Hasher},
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::db::descriptor::{DatabaseCategory, descriptor, drivers_in};
use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;

const URL_QUERY_VALUE: &percent_encoding::AsciiSet =
    &percent_encoding::CONTROLS.add(b'&').add(b'=').add(b'+');
const URL_COMPONENT: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'<')
    .add(b'>')
    .add(b'`');
const URL_PARSE_DEBOUNCE: Duration = Duration::from_millis(300);

use crate::{
    db::{
        ServerInfo,
        catalog::{CatalogCapabilities, CatalogDiscovery},
    },
    persistence::secrets::SecretStoreAvailability,
    profile::{
        CatalogScope, CatalogScopeValidationError, CatalogSelection, ConnectionProfile,
        ConnectionUrlFormat, CredentialPolicy, DatabaseKind, DatabaseScope, Environment,
        ParsedConnectionUrl, PasswordStorageChoice, ProfileAccess, SslMode, format_connection_url,
        parse_connection_url, parse_connection_url_with_driver,
    },
};

use super::{secret_text_input::SecretTextInput, text_input::TextInput};

#[derive(Clone, Default)]
pub struct ProfileInput(SecretString);

impl ProfileInput {
    pub(crate) fn value(&self) -> &str {
        self.0.expose_secret()
    }

    pub(crate) fn character(&self) -> Option<char> {
        let mut characters = self.value().chars();
        let character = characters.next()?;
        characters.next().is_none().then_some(character)
    }
}

impl fmt::Debug for ProfileInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl PartialEq for ProfileInput {
    fn eq(&self, other: &Self) -> bool {
        self.value() == other.value()
    }
}

impl Eq for ProfileInput {}

impl From<char> for ProfileInput {
    fn from(value: char) -> Self {
        Self(SecretString::from(value.to_string()))
    }
}

impl From<String> for ProfileInput {
    fn from(value: String) -> Self {
        Self(SecretString::from(value))
    }
}

impl From<&str> for ProfileInput {
    fn from(value: &str) -> Self {
        Self(SecretString::from(value.to_owned()))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileManagerPage {
    Form,
    Scope,
    ConfirmDelete,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ProfileField {
    DatabaseCategory,
    Kind,
    UrlFormat,
    Url,
    Name,
    Host,
    Port,
    User,
    Password,
    Database,
    Schema,
    VisibleObjects,
    SslMode,
    ElasticsearchAuth,
    CaCertPath,
    Environment,
    ReadOnly,
    PasswordStorage,
    SqliteMemory,
    SqlitePath,
    DuckDbPath,
    Test,
    Save,
    Cancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileOperation {
    Testing,
    Saving,
    SavingAndConnecting,
    Deleting,
    Connecting,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileMessageLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileMessage {
    pub level: ProfileMessageLevel,
    pub text: String,
}

impl ProfileMessage {
    pub fn new(level: ProfileMessageLevel, text: impl Into<String>) -> Self {
        Self {
            level,
            text: text.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogScopeMode {
    Derived,
    Explicit,
}

pub const DRIVER_ORDER: [DatabaseKind; crate::db::descriptor::DRIVERS.len()] = {
    let drivers = crate::db::descriptor::DRIVERS;
    let mut order = [DatabaseKind::Postgres; crate::db::descriptor::DRIVERS.len()];
    let mut index = 0;
    while index < drivers.len() {
        order[index] = drivers[index].kind;
        index += 1;
    }
    order
};

#[derive(Clone)]
pub enum CredentialUpdate {
    Preserve,
    Session(SecretString),
    Remember(SecretString),
    LocalEncrypted(SecretString),
    System(SecretString),
    Forget,
}

impl fmt::Debug for CredentialUpdate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Preserve => formatter.write_str("Preserve"),
            Self::Session(_) => formatter.write_str("Session([REDACTED])"),
            Self::Remember(_) => formatter.write_str("Remember([REDACTED])"),
            Self::LocalEncrypted(_) => formatter.write_str("LocalEncrypted([REDACTED])"),
            Self::System(_) => formatter.write_str("System([REDACTED])"),
            Self::Forget => formatter.write_str("Forget"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DiscoveryFingerprint(u64);

impl DiscoveryFingerprint {
    pub fn for_profile(
        profile: &ConnectionProfile,
        credential_present: bool,
        credential_revision: u64,
    ) -> Self {
        let mut hasher = DefaultHasher::new();
        (profile.kind as u8).hash(&mut hasher);
        profile.host.hash(&mut hasher);
        profile.port.hash(&mut hasher);
        profile.user.hash(&mut hasher);
        profile.database.hash(&mut hasher);
        profile.default_schema.hash(&mut hasher);
        profile.snowflake_options.hash(&mut hasher);
        (profile.ssl_mode as u8).hash(&mut hasher);
        profile.elasticsearch_options.hash(&mut hasher);
        profile.pg_options.hash(&mut hasher);
        profile.sqlite_path.hash(&mut hasher);
        (profile.kind == DatabaseKind::Sqlite
            && profile.sqlite_path.is_none()
            && profile.database.as_deref() == Some(":memory:"))
        .hash(&mut hasher);
        credential_present.hash(&mut hasher);
        credential_revision.hash(&mut hasher);
        Self(hasher.finish())
    }
}

#[derive(Clone)]
pub struct ProfileSubmission {
    pub profile: ConnectionProfile,
    pub credential: CredentialUpdate,
    pub discovery_fingerprint: DiscoveryFingerprint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProfileChange {
    pub connection_settings_changed: bool,
    pub catalog_scope_changed: bool,
    pub display_only_changed: bool,
    pub credentials_changed: bool,
}

impl ProfileSubmission {
    pub fn new(
        profile: ConnectionProfile,
        credential: CredentialUpdate,
        credential_revision: u64,
    ) -> Self {
        let credential_present = match &credential {
            CredentialUpdate::Preserve => {
                !matches!(profile.credential_policy, CredentialPolicy::None)
            }
            CredentialUpdate::Session(_)
            | CredentialUpdate::Remember(_)
            | CredentialUpdate::LocalEncrypted(_)
            | CredentialUpdate::System(_) => true,
            CredentialUpdate::Forget => false,
        };
        let discovery_fingerprint =
            DiscoveryFingerprint::for_profile(&profile, credential_present, credential_revision);
        Self {
            profile,
            credential,
            discovery_fingerprint,
        }
    }
}

impl fmt::Debug for ProfileSubmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProfileSubmission")
            .field("profile", &self.profile)
            .field("credential", &self.credential)
            .field("discovery_fingerprint", &self.discovery_fingerprint)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileValidationError {
    pub field: ProfileField,
    pub message: String,
}

impl ProfileValidationError {
    fn new(field: ProfileField, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

fn url_generation_error(error: crate::profile::ProfileError) -> ProfileValidationError {
    let (field, message) = match error {
        crate::profile::ProfileError::MissingHost => (ProfileField::Host, "host is required"),
        crate::profile::ProfileError::MissingPort => (ProfileField::Port, "port is required"),
        crate::profile::ProfileError::MissingOracleService => {
            (ProfileField::Database, "service name is required")
        }
        crate::profile::ProfileError::MissingSqlitePath => (
            ProfileField::SqlitePath,
            "SQLite path is required when memory mode is disabled",
        ),
        crate::profile::ProfileError::MissingDuckDbPath => {
            (ProfileField::DuckDbPath, "DuckDB path is required")
        }
        _ => (ProfileField::Url, "connection URL could not be generated"),
    };
    ProfileValidationError::new(field, message)
}

impl fmt::Display for ProfileValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProfileValidationError {}

#[derive(Clone, Eq, PartialEq)]
pub struct ProfileCatalogDiscovery {
    pub fingerprint: DiscoveryFingerprint,
    pub server: ServerInfo,
    pub capabilities: CatalogCapabilities,
    /// `None` means this database kind has no SQL catalog discovery.
    /// `Some(Err(_))` means discovery was attempted and failed.
    pub discovery: Option<Result<CatalogDiscovery, String>>,
}

impl fmt::Debug for ProfileCatalogDiscovery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProfileCatalogDiscovery")
            .field("fingerprint", &self.fingerprint)
            .field("server_kind", &self.server.kind)
            .field("capabilities", &self.capabilities)
            .field(
                "discovery",
                &match &self.discovery {
                    None => "not_applicable",
                    Some(Ok(_)) => "available",
                    Some(Err(_)) => "warning",
                },
            )
            .finish()
    }
}

#[derive(Clone, Default, Eq, PartialEq)]
pub enum CatalogDiscoveryState {
    #[default]
    NotRequested,
    Fresh(ProfileCatalogDiscovery),
    Stale(ProfileCatalogDiscovery),
}

impl fmt::Debug for CatalogDiscoveryState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRequested => formatter.write_str("NotRequested"),
            Self::Fresh(snapshot) => formatter.debug_tuple("Fresh").field(snapshot).finish(),
            Self::Stale(snapshot) => formatter.debug_tuple("Stale").field(snapshot).finish(),
        }
    }
}

#[derive(Clone)]
pub struct ProfileDraft {
    profile_id: Uuid,
    pub access: ProfileAccess,
    pub kind: DatabaseKind,
    recent_drivers: [Option<DatabaseKind>; 2],
    automatic_port: Option<String>,
    pub url_format: ConnectionUrlFormat,
    url: SecretTextInput,
    url_selection: Option<(usize, usize)>,
    url_pending: bool,
    url_parse_deadline: Option<Instant>,
    url_error: Option<String>,
    url_generation_error: Option<ProfileValidationError>,
    pub name: TextInput,
    pub host: TextInput,
    pub port: TextInput,
    pub user: TextInput,
    password: SecretTextInput,
    pub database: TextInput,
    pub schema: TextInput,
    pub ssl_mode: SslMode,
    pub elasticsearch_auth: crate::profile::ElasticsearchAuth,
    pub ca_cert_path: TextInput,
    credential_reset: bool,
    pub environment: Environment,
    pub read_only: bool,
    pub password_storage: PasswordStorageChoice,
    pub sqlite_memory: bool,
    pub sqlite_path: TextInput,
    pub mongodb_options: Option<crate::profile::MongoConnectionOptions>,
    pub pg_options: Option<crate::profile::PgConnectionOptions>,
    pub snowflake_options: Option<crate::profile::SnowflakeConnectionOptions>,
    pub duckdb_path: TextInput,
    pub original_credential_policy: CredentialPolicy,
    pub has_stored_credential: bool,
    pub catalog_scope: CatalogScope,
    pub catalog_scope_mode: CatalogScopeMode,
    pub catalog_discovery: CatalogDiscoveryState,
    pub discovery_fingerprint: Option<DiscoveryFingerprint>,
    credential_revision: u64,
    pub system_credential_availability: SecretStoreAvailability,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopeRow {
    pub id: String,
    pub name: String,
    pub selection: ScopeSelectionState,
    pub read_only: bool,
    pub unavailable: bool,
    pub database: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeSelectionState {
    Unchecked,
    Partial,
    Checked,
}

impl ProfileDraft {
    pub fn category(&self) -> DatabaseCategory {
        descriptor(self.kind).category
    }

    pub fn select_category(&mut self, category: DatabaseCategory) {
        if self.category() == category {
            return;
        }
        let index = |category| usize::from(category == DatabaseCategory::NonRelational);
        self.recent_drivers[index(self.category())] = Some(self.kind);
        let kind = self.recent_drivers[index(category)]
            .filter(|kind| descriptor(*kind).category == category)
            .unwrap_or_else(|| drivers_in(category).next().expect("nonempty category").kind);
        self.set_kind(kind);
    }

    pub fn new(kind: DatabaseKind) -> Self {
        let (host, port, schema, ssl_mode) = match kind {
            DatabaseKind::Snowflake => ("", "443", "PUBLIC", SslMode::VerifyFull),
            DatabaseKind::Postgres => ("localhost", "5432", "public", SslMode::Prefer),
            DatabaseKind::CockroachDb => ("localhost", "26257", "public", SslMode::Prefer),
            DatabaseKind::MySql | DatabaseKind::MariaDb => {
                ("localhost", "3306", "", SslMode::Prefer)
            }
            DatabaseKind::TiDb => ("localhost", "4000", "", SslMode::Prefer),
            DatabaseKind::Oracle => ("localhost", "1521", "", SslMode::Prefer),
            DatabaseKind::SqlServer => ("localhost", "1433", "dbo", SslMode::Prefer),
            DatabaseKind::Sqlite | DatabaseKind::DuckDb => ("", "", "main", SslMode::Disable),
            DatabaseKind::Redis | DatabaseKind::Valkey => {
                ("localhost", "6379", "", SslMode::Disable)
            }
            DatabaseKind::MongoDb => ("localhost", "27017", "", SslMode::Prefer),
            DatabaseKind::ClickHouse => ("localhost", "8123", "", SslMode::Disable),
            DatabaseKind::Elasticsearch => ("localhost", "9200", "", SslMode::Disable),
        };

        let mut draft = Self {
            profile_id: Uuid::new_v4(),
            access: ProfileAccess::Global,
            kind,
            url_format: ConnectionUrlFormat::default_for(kind),
            recent_drivers: [None, None],
            automatic_port: (kind != DatabaseKind::DuckDb).then(|| port.to_owned()),
            url: SecretTextInput::default(),
            url_selection: None,
            url_pending: false,
            url_parse_deadline: None,
            url_error: None,
            url_generation_error: None,
            name: TextInput::default(),
            host: TextInput::from(host),
            port: TextInput::from(port),
            user: TextInput::default(),
            password: SecretTextInput::default(),
            database: TextInput::default(),
            schema: TextInput::from(schema),
            ssl_mode,
            elasticsearch_auth: Default::default(),
            ca_cert_path: TextInput::default(),
            credential_reset: false,
            environment: Environment::Development,
            read_only: false,
            password_storage: PasswordStorageChoice::LocalEncrypted,
            sqlite_memory: false,
            sqlite_path: TextInput::default(),
            mongodb_options: None,
            pg_options: None,
            snowflake_options: (kind == DatabaseKind::Snowflake)
                .then(crate::profile::SnowflakeConnectionOptions::default),
            duckdb_path: TextInput::default(),
            original_credential_policy: CredentialPolicy::None,
            has_stored_credential: false,
            catalog_scope: CatalogScope::for_profile(kind, "", Some(schema)),
            catalog_scope_mode: CatalogScopeMode::Derived,
            catalog_discovery: CatalogDiscoveryState::NotRequested,
            discovery_fingerprint: None,
            credential_revision: 0,
            system_credential_availability: SecretStoreAvailability::Unavailable,
        };
        if kind == DatabaseKind::TiDb {
            draft.user.set("root");
        }
        if kind == DatabaseKind::CockroachDb {
            draft.database.set("defaultdb");
            draft.sync_derived_catalog_scope();
        } else if kind.is_redis_compatible() {
            draft.database.set("0");
        } else if kind == DatabaseKind::ClickHouse {
            draft.database.set("default");
            draft.user.set("default");
        }
        if kind == DatabaseKind::MongoDb {
            draft.mongodb_options = Some(crate::profile::MongoConnectionOptions {
                seeds: vec![crate::profile::MongoSeed {
                    host: host.to_owned(),
                    port: Some(27017),
                }],
                srv: false,
                uri_options: Vec::new(),
            });
        }
        if kind == DatabaseKind::Sqlite {
            draft.sqlite_memory = false;
        }
        draft.refresh_url();
        draft
    }

    pub fn edit(profile: &ConnectionProfile, has_stored_credential: bool) -> Self {
        let sqlite_memory = profile.kind == DatabaseKind::Sqlite
            && profile.sqlite_path.is_none()
            && profile.database.as_deref() == Some(":memory:");
        let sqlite_path = profile
            .sqlite_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .or_else(|| {
                (!sqlite_memory && profile.kind == DatabaseKind::Sqlite)
                    .then(|| profile.database.clone())
                    .flatten()
            })
            .unwrap_or_default();
        let derived_scope = CatalogScope::for_profile(
            profile.kind,
            profile.database.as_deref().unwrap_or_default(),
            profile.default_schema.as_deref(),
        );

        let mut draft = Self {
            profile_id: profile.id,
            access: profile.access.clone(),
            kind: profile.kind,
            recent_drivers: [None, None],
            automatic_port: None,
            url_format: if profile.url_format.is_compatible(profile.kind) {
                profile.url_format
            } else {
                ConnectionUrlFormat::default_for(profile.kind)
            },
            url: SecretTextInput::default(),
            url_selection: None,
            url_pending: false,
            url_parse_deadline: None,
            url_error: None,
            url_generation_error: None,
            name: TextInput::from(profile.name.clone()),
            host: TextInput::from(profile.host.clone().unwrap_or_default()),
            port: TextInput::from(
                profile
                    .port
                    .map(|port| port.to_string())
                    .unwrap_or_default(),
            ),
            mongodb_options: profile.mongodb_options.clone(),
            pg_options: profile.pg_options.clone(),
            snowflake_options: profile.snowflake_options.clone(),
            user: TextInput::from(profile.user.clone().unwrap_or_default()),
            password: SecretTextInput::default(),
            database: TextInput::from(profile.database.clone().unwrap_or_default()),
            schema: TextInput::from(profile.default_schema.clone().unwrap_or_default()),
            ssl_mode: profile.ssl_mode,
            elasticsearch_auth: profile
                .elasticsearch_options
                .as_ref()
                .map(|options| options.auth)
                .unwrap_or_default(),
            ca_cert_path: TextInput::from(
                profile
                    .elasticsearch_options
                    .as_ref()
                    .and_then(|options| options.ca_cert_path.as_ref())
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
            credential_reset: false,
            environment: profile.environment,
            read_only: profile.read_only,
            password_storage: match &profile.credential_policy {
                CredentialPolicy::System(_) | CredentialPolicy::Keyring(_) => {
                    PasswordStorageChoice::System
                }
                _ => PasswordStorageChoice::LocalEncrypted,
            },
            sqlite_memory,
            sqlite_path: TextInput::from(sqlite_path),
            duckdb_path: TextInput::from(
                profile
                    .duckdb_path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
            original_credential_policy: profile.credential_policy.clone(),
            has_stored_credential,
            catalog_scope: profile.catalog_scope.clone(),
            catalog_scope_mode: if profile.catalog_scope == derived_scope {
                CatalogScopeMode::Derived
            } else {
                CatalogScopeMode::Explicit
            },
            catalog_discovery: CatalogDiscoveryState::NotRequested,
            discovery_fingerprint: None,
            credential_revision: 0,
            system_credential_availability: SecretStoreAvailability::Unavailable,
        };
        draft.refresh_url();
        draft
    }

    pub fn profile_id(&self) -> Uuid {
        self.profile_id
    }

    pub fn password(&self) -> &SecretString {
        self.password.secret()
    }

    pub fn url_display(&self) -> String {
        let display = redact_url_password(self.url.value()).0;
        redact_url_query_passwords(&display)
    }

    pub fn oracle_url_preview(&self) -> Option<String> {
        if self.kind != DatabaseKind::Oracle
            || self.url_pending
            || self.url_generation_error.is_none()
        {
            return None;
        }

        let host =
            non_empty(&self.host).map_or_else(|| "<host>".to_owned(), |value| value.to_owned());
        let port = match self.port.value().trim().parse::<u16>() {
            Ok(port) if port > 0 => port.to_string(),
            Ok(_) | Err(_) if self.port.value().trim().is_empty() => "<port>".to_owned(),
            Ok(_) | Err(_) => "<invalid-port>".to_owned(),
        };
        let service = non_empty(&self.database).map_or_else(
            || "<service-name>".to_owned(),
            |value| percent_encoding::utf8_percent_encode(value, URL_COMPONENT).to_string(),
        );
        let mut preview = format!("jdbc:oracle:thin:@{host}:{port}/{service}");
        if let Some(user) = non_empty(&self.user) {
            preview.push_str("?user=");
            preview.push_str(
                &percent_encoding::utf8_percent_encode(user, URL_QUERY_VALUE).to_string(),
            );
        }
        Some(preview)
    }

    pub fn url_cursor(&self) -> usize {
        let raw = self.url.value();
        let (display, password_ranges) = redact_url_password(raw);
        if password_ranges.is_empty() {
            return self.url.cursor().min(display.chars().count());
        }
        let raw_cursor = self.url.cursor().min(raw.chars().count());
        let redacted_len = "[REDACTED]".chars().count();
        let mut adjustment = 0_isize;
        for (start, end) in password_ranges {
            if raw_cursor <= start {
                break;
            }
            if raw_cursor <= end {
                return (start as isize + adjustment + redacted_len as isize) as usize;
            }
            adjustment += redacted_len as isize - (end - start) as isize;
        }
        (raw_cursor as isize + adjustment) as usize
    }

    pub fn url_selection_range(&self) -> Option<std::ops::Range<usize>> {
        let (start, end) = self.url_selection?;
        (start.min(end) < start.max(end)).then_some(start.min(end)..start.max(end))
    }

    pub fn begin_url_selection(&mut self, cursor: usize) {
        let cursor = cursor.min(self.url_display().chars().count());
        self.url_selection = Some((cursor, cursor));
    }

    pub fn extend_url_selection(&mut self, cursor: usize) {
        let cursor = cursor.min(self.url_display().chars().count());
        let start = self
            .url_selection
            .map_or(self.url_cursor(), |(_, start)| start);
        self.url_selection = Some((start, cursor));
    }

    pub fn clear_url_selection(&mut self) {
        self.url_selection = None;
    }

    pub fn selected_url_display(&self) -> Option<String> {
        let range = self.url_selection_range()?;
        Some(
            self.url_display()
                .chars()
                .skip(range.start)
                .take(range.end - range.start)
                .collect(),
        )
    }

    pub fn url_is_pending(&self) -> bool {
        self.url_pending
    }

    pub fn url_parse_is_due_at(&self, now: Instant) -> bool {
        self.url_pending
            && self
                .url_parse_deadline
                .is_some_and(|deadline| now >= deadline)
    }

    pub(crate) fn parse_url_if_due_at(&mut self, now: Instant) -> bool {
        if !self.url_parse_is_due_at(now) {
            return false;
        }

        let result = self.parse_url();
        match result {
            Ok(parsed) => {
                self.apply_parsed_url(parsed);
                true
            }
            Err(error) => {
                self.url_pending = false;
                self.url_parse_deadline = None;
                self.url_error = Some(error.to_string());
                true
            }
        }
    }

    pub fn url_error(&self) -> Option<&str> {
        self.url_error.as_deref()
    }

    pub fn url_generation_error(&self) -> Option<&ProfileValidationError> {
        self.url_generation_error.as_ref()
    }

    pub fn commit_url(&mut self) -> Result<(), ProfileValidationError> {
        if !self.url_pending {
            return self.url_error.as_ref().map_or(Ok(()), |error| {
                Err(ProfileValidationError::new(ProfileField::Url, error))
            });
        }
        let parsed = self.parse_url().map_err(|error| {
            let message = error.to_string();
            self.url_error = Some(message.clone());
            ProfileValidationError::new(ProfileField::Url, message)
        })?;

        self.apply_parsed_url(parsed);
        self.refresh_url();
        Ok(())
    }

    fn parse_url(&self) -> Result<ParsedConnectionUrl, crate::profile::ProfileError> {
        // Generic PostgreSQL URLs do not identify the server product. Preserve
        // an explicitly selected CockroachDB driver while allowing other URLs
        // to continue selecting their own driver.
        if self.kind == DatabaseKind::CockroachDb {
            let parsed = parse_connection_url(self.url.value())?;
            if parsed.kind.is_postgres_compatible() {
                return parse_connection_url_with_driver(
                    self.url.value(),
                    Some(DatabaseKind::CockroachDb),
                );
            }
            return Ok(parsed);
        }
        parse_connection_url(self.url.value())
    }

    fn apply_parsed_url(&mut self, parsed: ParsedConnectionUrl) {
        self.recent_drivers[usize::from(self.category() == DatabaseCategory::NonRelational)] =
            Some(self.kind);
        // URL credentials are always Basic Auth, never an API key.
        if self.kind == DatabaseKind::Elasticsearch
            && (parsed.kind != self.kind || parsed.user.is_some() || parsed.password.is_some())
        {
            self.set_elasticsearch_auth(crate::profile::ElasticsearchAuth::Basic);
        }
        if parsed.kind != DatabaseKind::Elasticsearch {
            self.ca_cert_path.set("");
        }
        self.kind = parsed.kind;
        self.automatic_port = match parsed.kind {
            DatabaseKind::ClickHouse if parsed.port == Some(8123) => Some("8123".to_owned()),
            DatabaseKind::ClickHouse if parsed.port == Some(8443) => Some("8443".to_owned()),
            _ => None,
        };
        self.url_format = parsed.format;
        self.mongodb_options = parsed.mongodb_options.clone();
        if parsed.kind.is_postgres_compatible() {
            self.pg_options = parsed.pg_options;
        }
        self.snowflake_options = parsed.snowflake_options.clone();
        self.host.set(parsed.host.unwrap_or_default());
        self.port
            .set(parsed.port.map(|port| port.to_string()).unwrap_or_default());
        self.user.set(parsed.user.unwrap_or_default());
        self.database.set(parsed.database.unwrap_or_default());
        self.schema.set(parsed.default_schema.unwrap_or_default());
        self.ssl_mode = parsed.ssl_mode;
        self.read_only = parsed.read_only;
        self.sqlite_memory = parsed.sqlite_memory;
        self.sqlite_path.set(
            parsed
                .sqlite_path
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        if let Some(password) = parsed.password {
            self.set_password(password.expose_secret().to_owned());
        }
        self.url_pending = false;
        self.url_parse_deadline = None;
        self.url_error = None;
        self.invalidate_catalog_discovery();
        self.sync_derived_catalog_scope();
    }

    pub fn set_password(&mut self, password: impl Into<String>) {
        let password = password.into();
        self.password.set(password);
        self.credential_changed();
    }

    pub fn password_len(&self) -> usize {
        self.password.value().chars().count()
    }

    pub fn insert(&mut self, field: ProfileField, character: char) {
        if field == ProfileField::Url {
            if self.url.insert(character) {
                self.mark_url_edited();
            }
        } else if field == ProfileField::Password {
            if !self.password.insert(character) {
                return;
            }
        } else if let Some(input) = self.text_input_mut(field) {
            input.insert(character);
        }
        self.connection_field_changed(field);
    }

    pub fn paste(&mut self, field: ProfileField, text: &str) {
        if field == ProfileField::Url {
            if self.url.paste(text) {
                self.mark_url_edited();
            }
        } else if field == ProfileField::Password {
            if !self.password.paste(text) {
                return;
            }
        } else if let Some(input) = self.text_input_mut(field) {
            input.paste(text);
        }
        self.connection_field_changed(field);
    }

    pub fn backspace(&mut self, field: ProfileField) {
        if field == ProfileField::Url {
            if self.url.backspace() {
                self.mark_url_edited();
            }
        } else if field == ProfileField::Password {
            if !self.password.backspace() {
                return;
            }
        } else if let Some(input) = self.text_input_mut(field) {
            input.backspace();
        }
        self.connection_field_changed(field);
    }

    pub fn delete_previous_word(&mut self, field: ProfileField) {
        if field == ProfileField::Url {
            if self.url.delete_previous_word() {
                self.mark_url_edited();
            }
        } else if field == ProfileField::Password {
            if !self.password.delete_previous_word() {
                return;
            }
        } else if let Some(input) = self.text_input_mut(field) {
            input.delete_previous_word();
        }
        self.connection_field_changed(field);
    }

    pub fn delete_to_start(&mut self, field: ProfileField) {
        if field == ProfileField::Url {
            if self.url.delete_to_start() {
                self.mark_url_edited();
            }
        } else if field == ProfileField::Password {
            if !self.password.delete_to_start() {
                return;
            }
        } else if let Some(input) = self.text_input_mut(field) {
            input.delete_to_start();
        }
        self.connection_field_changed(field);
    }

    pub fn delete(&mut self, field: ProfileField) {
        if field == ProfileField::Url {
            if self.url.delete() {
                self.mark_url_edited();
            }
        } else if field == ProfileField::Password {
            if !self.password.delete() {
                return;
            }
        } else if let Some(input) = self.text_input_mut(field) {
            input.delete();
        }
        self.connection_field_changed(field);
    }

    pub fn begin_catalog_discovery(&mut self, fingerprint: DiscoveryFingerprint) {
        self.discovery_fingerprint = Some(fingerprint);
    }

    pub fn visible_objects_summary(&self) -> String {
        match &self.catalog_scope.databases {
            CatalogSelection::All => "all databases".to_owned(),
            CatalogSelection::Selected(items) => format!(
                "{} database{}",
                items.len(),
                if items.len() == 1 { "" } else { "s" }
            ),
        }
    }

    pub fn invalidate_discovery_for_test(&mut self) {
        self.invalidate_catalog_discovery();
    }

    pub fn apply_catalog_discovery(&mut self, snapshot: ProfileCatalogDiscovery) -> bool {
        if self.discovery_fingerprint != Some(snapshot.fingerprint) {
            return false;
        }
        self.catalog_discovery = CatalogDiscoveryState::Fresh(snapshot);
        true
    }

    fn invalidate_catalog_discovery(&mut self) {
        let current = std::mem::take(&mut self.catalog_discovery);
        self.catalog_discovery = match current {
            CatalogDiscoveryState::Fresh(snapshot) => CatalogDiscoveryState::Stale(snapshot),
            other => other,
        };
        self.discovery_fingerprint = None;
    }

    fn connection_field_changed(&mut self, field: ProfileField) {
        if field == ProfileField::Port {
            self.automatic_port = None;
        }
        if field == ProfileField::Password {
            self.credential_changed();
        } else if matches!(
            field,
            ProfileField::Host
                | ProfileField::Port
                | ProfileField::User
                | ProfileField::CaCertPath
                | ProfileField::Database
                | ProfileField::Schema
                | ProfileField::SqlitePath
                | ProfileField::DuckDbPath
        ) {
            self.invalidate_catalog_discovery();
        }
        if self.kind == DatabaseKind::MongoDb
            && matches!(field, ProfileField::Host | ProfileField::Port)
            && self.mongodb_options.is_some()
        {
            // The host and port fields are a summary of the first seed for an
            // imported multi-seed URI. Editing either explicitly switches to
            // the simple single-seed form so the displayed URL cannot lie.
            let srv = self.url_format == ConnectionUrlFormat::MongoDbSrv;
            let host = self.host.value().trim().to_owned();
            let port = self
                .port
                .value()
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|port| *port > 0);
            self.mongodb_options = Some(crate::profile::MongoConnectionOptions {
                seeds: if host.is_empty() {
                    Vec::new()
                } else {
                    vec![crate::profile::MongoSeed {
                        host,
                        port: if srv { None } else { port },
                    }]
                },
                srv,
                uri_options: self
                    .mongodb_options
                    .as_ref()
                    .map(|options| options.uri_options.clone())
                    .unwrap_or_default(),
            });
        }
        if matches!(
            field,
            ProfileField::Database
                | ProfileField::Schema
                | ProfileField::SqlitePath
                | ProfileField::DuckDbPath
        ) {
            self.sync_derived_catalog_scope();
        }
        if field != ProfileField::Url && field != ProfileField::Password {
            self.refresh_url();
        }
    }

    fn credential_changed(&mut self) {
        self.credential_revision = self.credential_revision.saturating_add(1);
        self.invalidate_catalog_discovery();
    }

    pub fn move_left(&mut self, field: ProfileField) {
        if field == ProfileField::Url {
            self.url.move_left();
        } else if field == ProfileField::Password {
            self.password.move_left();
        } else if let Some(input) = self.text_input_mut(field) {
            input.move_left();
        }
    }

    pub fn set_cursor(&mut self, field: ProfileField, cursor: usize) {
        if field == ProfileField::Url {
            self.url.set_cursor(cursor);
        } else if field == ProfileField::Password {
            self.password.set_cursor(cursor);
        } else if let Some(input) = self.text_input_mut(field) {
            input.set_cursor(cursor);
        }
    }

    pub fn move_right(&mut self, field: ProfileField) {
        if field == ProfileField::Url {
            self.url.move_right();
        } else if field == ProfileField::Password {
            self.password.move_right();
        } else if let Some(input) = self.text_input_mut(field) {
            input.move_right();
        }
    }

    pub fn move_home(&mut self, field: ProfileField) {
        if field == ProfileField::Url {
            self.url.move_home();
        } else if field == ProfileField::Password {
            self.password.move_home();
        } else if let Some(input) = self.text_input_mut(field) {
            input.move_home();
        }
    }

    pub fn move_end(&mut self, field: ProfileField) {
        if field == ProfileField::Url {
            self.url.move_end();
        } else if field == ProfileField::Password {
            self.password.move_end();
        } else if let Some(input) = self.text_input_mut(field) {
            input.move_end();
        }
    }

    pub fn visible_fields(&self) -> &'static [ProfileField] {
        match (self.kind, self.sqlite_memory) {
            (DatabaseKind::Postgres | DatabaseKind::CockroachDb, _) => &POSTGRES_FIELDS,
            (DatabaseKind::MySql, _) => &MYSQL_FIELDS,
            (DatabaseKind::SqlServer | DatabaseKind::Snowflake, _) => &POSTGRES_FIELDS,
            (DatabaseKind::Sqlite, false) => &SQLITE_FILE_FIELDS,
            (DatabaseKind::Sqlite, true) => &SQLITE_MEMORY_FIELDS,
            (DatabaseKind::DuckDb, _) => &DUCKDB_FIELDS,
            (DatabaseKind::MariaDb | DatabaseKind::TiDb, _) => &MYSQL_FIELDS,
            (DatabaseKind::Oracle, _) => &ORACLE_FIELDS,
            (DatabaseKind::Redis | DatabaseKind::Valkey, _) => &REDIS_FIELDS,
            (DatabaseKind::MongoDb, _) => &MONGODB_FIELDS,
            (DatabaseKind::ClickHouse, _) => &MYSQL_FIELDS,
            (DatabaseKind::Elasticsearch, _)
                if self.elasticsearch_auth == crate::profile::ElasticsearchAuth::ApiKey =>
            {
                &ELASTICSEARCH_API_KEY_FIELDS
            }
            (DatabaseKind::Elasticsearch, _) => &ELASTICSEARCH_FIELDS,
        }
    }

    pub fn password_storage_choices(&self) -> &'static [PasswordStorageChoice] {
        if self.kind == DatabaseKind::Sqlite {
            &[]
        } else if matches!(
            self.system_credential_availability,
            SecretStoreAvailability::Available | SecretStoreAvailability::Locked
        ) || matches!(
            self.original_credential_policy,
            CredentialPolicy::System(_) | CredentialPolicy::Keyring(_)
        ) {
            &PASSWORD_STORAGE_CHOICES
        } else {
            &PASSWORD_STORAGE_LOCAL_ONLY
        }
    }

    pub fn validate(
        &self,
        profiles: &[ConnectionProfile],
    ) -> Result<ProfileSubmission, ProfileValidationError> {
        if self.url_pending {
            return Err(ProfileValidationError::new(
                ProfileField::Url,
                "connection URL has uncommitted changes",
            ));
        }
        if let Some(error) = &self.url_error {
            return Err(ProfileValidationError::new(ProfileField::Url, error));
        }
        let name = required(&self.name, ProfileField::Name, "profile name is required")?;
        let normalized_name = name.to_lowercase();
        if profiles.iter().any(|profile| {
            profile.id != self.profile_id && profile.name.trim().to_lowercase() == normalized_name
        }) {
            return Err(ProfileValidationError::new(
                ProfileField::Name,
                "profile name already exists",
            ));
        }

        let (
            host,
            port,
            user,
            database,
            default_schema,
            sqlite_path,
            ssl_mode,
            mongodb_options,
            duckdb_path,
        ) = match self.kind {
            DatabaseKind::Postgres
            | DatabaseKind::CockroachDb
            | DatabaseKind::MySql
            | DatabaseKind::MariaDb
            | DatabaseKind::TiDb
            | DatabaseKind::Oracle
            | DatabaseKind::SqlServer
            | DatabaseKind::Snowflake => {
                let host = required(&self.host, ProfileField::Host, "host is required")?;
                let port = self.port.value().trim().parse::<u16>().map_err(|_| {
                    ProfileValidationError::new(
                        ProfileField::Port,
                        "port must be an integer from 1 to 65535",
                    )
                })?;
                if port == 0 {
                    return Err(ProfileValidationError::new(
                        ProfileField::Port,
                        "port must be an integer from 1 to 65535",
                    ));
                }
                let database = required(
                    &self.database,
                    ProfileField::Database,
                    "database is required",
                )?;

                (
                    Some(host),
                    Some(port),
                    optional(&self.user),
                    Some(database),
                    matches!(
                        self.kind,
                        DatabaseKind::Postgres
                            | DatabaseKind::CockroachDb
                            | DatabaseKind::SqlServer
                            | DatabaseKind::Snowflake
                    )
                    .then(|| {
                        optional(&self.schema).or_else(|| {
                            (self.kind == DatabaseKind::Snowflake).then(|| "PUBLIC".to_owned())
                        })
                    })
                    .flatten(),
                    None,
                    self.ssl_mode,
                    self.mongodb_options.clone(),
                    None,
                )
            }
            DatabaseKind::Sqlite => {
                let path = if self.sqlite_memory {
                    None
                } else {
                    Some(PathBuf::from(required(
                        &self.sqlite_path,
                        ProfileField::SqlitePath,
                        "SQLite path is required when memory mode is disabled",
                    )?))
                };
                let database = path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_else(|| ":memory:".to_owned());

                (
                    None,
                    None,
                    None,
                    Some(database),
                    Some("main".to_owned()),
                    path,
                    SslMode::Disable,
                    self.mongodb_options.clone(),
                    None,
                )
            }
            DatabaseKind::DuckDb => {
                let raw = self.duckdb_path.value().trim();
                if raw.is_empty() && self.read_only {
                    return Err(ProfileValidationError::new(
                        ProfileField::ReadOnly,
                        "in-memory DuckDB cannot be read-only",
                    ));
                }
                let path = (!raw.is_empty()).then(|| PathBuf::from(raw));
                let database = path
                    .as_ref()
                    .and_then(|path| path.file_stem())
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("memory")
                    .to_owned();
                (
                    None,
                    None,
                    None,
                    Some(database),
                    Some(optional(&self.schema).unwrap_or_else(|| "main".into())),
                    path.clone(),
                    SslMode::Disable,
                    None,
                    path,
                )
            }
            DatabaseKind::Redis | DatabaseKind::Valkey => {
                let database = required(
                    &self.database,
                    ProfileField::Database,
                    "Key-value database number is required",
                )?
                .parse::<u32>()
                .map_err(|_| {
                    ProfileValidationError::new(
                        ProfileField::Database,
                        "Database number must be a non-negative integer",
                    )
                })?;
                let port = self.port.value().trim().parse::<u16>().map_err(|_| {
                    ProfileValidationError::new(
                        ProfileField::Port,
                        "port must be an integer from 1 to 65535",
                    )
                })?;
                let host = required(&self.host, ProfileField::Host, "host is required")?;
                (
                    Some(host),
                    Some(port),
                    optional(&self.user),
                    Some(database.to_string()),
                    None,
                    None,
                    self.ssl_mode,
                    self.mongodb_options.clone(),
                    None,
                )
            }
            DatabaseKind::MongoDb => {
                let host = required(&self.host, ProfileField::Host, "host is required")?;
                let port = if self.url_format == ConnectionUrlFormat::MongoDbSrv {
                    None
                } else {
                    Some(
                        self.port
                            .value()
                            .trim()
                            .parse::<u16>()
                            .ok()
                            .filter(|port| *port > 0)
                            .ok_or_else(|| {
                                ProfileValidationError::new(
                                    ProfileField::Port,
                                    "port must be an integer from 1 to 65535",
                                )
                            })?,
                    )
                };
                let uri_options = self
                    .mongodb_options
                    .as_ref()
                    .map(|options| options.uri_options.clone())
                    .unwrap_or_default();
                let srv = self.url_format == ConnectionUrlFormat::MongoDbSrv;
                let profile_options = self
                    .mongodb_options
                    .as_ref()
                    .filter(|options| {
                        options.srv == srv
                            && options
                                .seeds
                                .first()
                                .is_some_and(|seed| seed.host == host && seed.port == port)
                    })
                    .cloned()
                    .unwrap_or_else(|| crate::profile::MongoConnectionOptions {
                        seeds: vec![crate::profile::MongoSeed {
                            host: host.clone(),
                            port,
                        }],
                        srv,
                        uri_options,
                    });
                (
                    Some(host),
                    port,
                    optional(&self.user),
                    optional(&self.database),
                    None,
                    None,
                    self.ssl_mode,
                    Some(profile_options),
                    None,
                )
            }
            DatabaseKind::Elasticsearch => {
                let host = required(&self.host, ProfileField::Host, "host is required")?;
                let port = self
                    .port
                    .value()
                    .trim()
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port > 0)
                    .ok_or_else(|| {
                        ProfileValidationError::new(
                            ProfileField::Port,
                            "port must be an integer from 1 to 65535",
                        )
                    })?;
                if !self.url_format.is_compatible(self.kind) {
                    return Err(ProfileValidationError::new(
                        ProfileField::UrlFormat,
                        "connection URL format is incompatible with Elasticsearch",
                    ));
                }
                let expected_ssl_mode = if self.url_format == ConnectionUrlFormat::ElasticsearchTls
                {
                    SslMode::VerifyFull
                } else {
                    SslMode::Disable
                };
                if self.ssl_mode != expected_ssl_mode {
                    return Err(ProfileValidationError::new(
                        ProfileField::SslMode,
                        "Elasticsearch URL format requires disabled TLS or full certificate verification",
                    ));
                }
                if optional(&self.ca_cert_path).is_some() && self.ssl_mode != SslMode::VerifyFull {
                    return Err(ProfileValidationError::new(
                        ProfileField::CaCertPath,
                        "CA certificate requires HTTPS with full certificate verification",
                    ));
                }
                if self.elasticsearch_auth == crate::profile::ElasticsearchAuth::ApiKey
                    && self.password_len() == 0
                    && !self.has_stored_credential
                    && (self.credential_reset
                        || !matches!(self.original_credential_policy, CredentialPolicy::Prompt))
                {
                    return Err(ProfileValidationError::new(
                        ProfileField::Password,
                        "encoded API key is required",
                    ));
                }
                (
                    Some(host),
                    Some(port),
                    (self.elasticsearch_auth == crate::profile::ElasticsearchAuth::Basic)
                        .then(|| optional(&self.user))
                        .flatten(),
                    optional(&self.database),
                    None,
                    None,
                    self.ssl_mode,
                    None,
                    None,
                )
            }
            DatabaseKind::ClickHouse => {
                let host = required(&self.host, ProfileField::Host, "host is required")?;
                let port = self
                    .port
                    .value()
                    .trim()
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port > 0)
                    .ok_or_else(|| {
                        ProfileValidationError::new(
                            ProfileField::Port,
                            "port must be an integer from 1 to 65535",
                        )
                    })?;
                let database = required(
                    &self.database,
                    ProfileField::Database,
                    "database is required",
                )?;
                let tls = self.url_format == ConnectionUrlFormat::ClickHouseTls;
                if tls == (self.ssl_mode == SslMode::Disable) {
                    return Err(ProfileValidationError::new(
                        ProfileField::SslMode,
                        "ClickHouse URL format and SSL mode must select the same transport",
                    ));
                }
                (
                    Some(host),
                    Some(port),
                    Some(if self.user.value().trim().is_empty() {
                        "default".to_owned()
                    } else {
                        self.user.value().trim().to_owned()
                    }),
                    Some(database),
                    Some(self.database.value().to_owned()),
                    None,
                    self.ssl_mode,
                    None,
                    None,
                )
            }
        };

        if self.kind == DatabaseKind::Snowflake {
            // The URL formatter validates the fixed HTTPS endpoint and token options.
            let profile = self.connection_profile_for_url()?;
            format_connection_url(&profile, ConnectionUrlFormat::Snowflake).map_err(|error| {
                ProfileValidationError::new(ProfileField::Url, error.to_string())
            })?;
        }
        let credential = self.credential_update();
        let credential_policy = match &credential {
            CredentialUpdate::Preserve => self.original_credential_policy.clone(),
            CredentialUpdate::Session(_) | CredentialUpdate::Remember(_) => {
                CredentialPolicy::Prompt
            }
            CredentialUpdate::LocalEncrypted(_) => CredentialPolicy::Prompt,
            CredentialUpdate::System(_) => CredentialPolicy::Prompt,
            CredentialUpdate::Forget => CredentialPolicy::None,
        };
        let catalog_scope = match self.catalog_scope_mode {
            CatalogScopeMode::Derived => CatalogScope::for_profile(
                self.kind,
                database.as_deref().unwrap_or_default(),
                default_schema.as_deref(),
            ),
            CatalogScopeMode::Explicit => self.catalog_scope.clone(),
        };
        catalog_scope
            .validate(
                database.as_deref().unwrap_or_default(),
                default_schema.as_deref(),
            )
            .map_err(|error| {
                ProfileValidationError::new(catalog_scope_error_field(&error), error.to_string())
            })?;

        Ok(ProfileSubmission::new(
            ConnectionProfile {
                id: self.profile_id,
                name,
                access: self.access.clone(),
                group_id: None,
                kind: self.kind,
                url_format: self.url_format,
                host,
                port,
                user,
                database,
                default_schema,
                sqlite_path,
                mongodb_options,
                elasticsearch_options: self.elasticsearch_options(),
                pg_options: self.pg_options.clone(),
                snowflake_options: self.snowflake_options.clone(),
                duckdb_path,
                ssl_mode,
                credential_policy,
                read_only: self.read_only,
                environment: self.environment,
                catalog_scope,
            },
            credential,
            self.credential_revision,
        ))
    }

    fn elasticsearch_options(&self) -> Option<crate::profile::ElasticsearchConnectionOptions> {
        (self.kind == DatabaseKind::Elasticsearch).then(|| {
            crate::profile::ElasticsearchConnectionOptions {
                auth: self.elasticsearch_auth,
                ca_cert_path: optional(&self.ca_cert_path).map(PathBuf::from),
            }
        })
    }

    pub fn set_elasticsearch_auth(&mut self, auth: crate::profile::ElasticsearchAuth) {
        if self.elasticsearch_auth == auth {
            return;
        }
        self.elasticsearch_auth = auth;
        // A credential belongs to its authentication mechanism and cannot be reused across modes.
        self.password = SecretTextInput::default();
        self.has_stored_credential = false;
        self.credential_reset = true;
        self.credential_changed();
        self.refresh_url();
    }

    fn credential_update(&self) -> CredentialUpdate {
        if self.kind == DatabaseKind::Sqlite {
            return if !matches!(self.original_credential_policy, CredentialPolicy::None) {
                CredentialUpdate::Forget
            } else {
                CredentialUpdate::Preserve
            };
        }

        if !self.password.value().is_empty() {
            return match self.password_storage {
                PasswordStorageChoice::LocalEncrypted => {
                    CredentialUpdate::LocalEncrypted(self.password.secret().clone())
                }
                PasswordStorageChoice::System => {
                    CredentialUpdate::System(self.password.secret().clone())
                }
            };
        }

        if self.credential_reset {
            CredentialUpdate::Forget
        } else {
            CredentialUpdate::Preserve
        }
    }

    fn sync_derived_catalog_scope(&mut self) {
        if self.catalog_scope_mode != CatalogScopeMode::Derived {
            return;
        }
        let (database, default_schema) = match self.kind {
            DatabaseKind::Postgres
            | DatabaseKind::CockroachDb
            | DatabaseKind::SqlServer
            | DatabaseKind::Snowflake => (
                self.database.value(),
                optional(&self.schema).or_else(|| {
                    (self.kind == DatabaseKind::Snowflake).then(|| "PUBLIC".to_owned())
                }),
            ),
            DatabaseKind::MySql | DatabaseKind::MariaDb | DatabaseKind::TiDb => {
                (self.database.value(), None)
            }
            DatabaseKind::Oracle => (self.database.value(), None),
            DatabaseKind::Sqlite => (
                if self.sqlite_memory {
                    ":memory:"
                } else {
                    self.sqlite_path.value()
                },
                Some("main".to_owned()),
            ),
            DatabaseKind::DuckDb => (
                self.duckdb_path.value(),
                Some(optional(&self.schema).unwrap_or_else(|| "main".into())),
            ),
            DatabaseKind::Redis | DatabaseKind::Valkey => (self.database.value(), None),
            DatabaseKind::MongoDb | DatabaseKind::Elasticsearch => (self.database.value(), None),
            DatabaseKind::ClickHouse => (
                self.database.value(),
                Some(self.database.value().to_owned()),
            ),
        };
        self.catalog_scope =
            CatalogScope::for_profile(self.kind, database, default_schema.as_deref());
    }

    fn text_input_mut(&mut self, field: ProfileField) -> Option<&mut TextInput> {
        match field {
            ProfileField::Name => Some(&mut self.name),
            ProfileField::Host => Some(&mut self.host),
            ProfileField::Port => Some(&mut self.port),
            ProfileField::User => Some(&mut self.user),
            ProfileField::Database => Some(&mut self.database),
            ProfileField::Schema => Some(&mut self.schema),
            ProfileField::SqlitePath => Some(&mut self.sqlite_path),
            ProfileField::DuckDbPath => Some(&mut self.duckdb_path),
            ProfileField::CaCertPath => Some(&mut self.ca_cert_path),
            _ => None,
        }
    }

    pub(crate) fn mouse_selection_input_mut(
        &mut self,
        field: ProfileField,
    ) -> Option<&mut TextInput> {
        self.text_input_mut(field)
    }

    pub(crate) fn mouse_selection_input(&self, field: ProfileField) -> Option<&TextInput> {
        match field {
            ProfileField::Name => Some(&self.name),
            ProfileField::Host => Some(&self.host),
            ProfileField::Port => Some(&self.port),
            ProfileField::User => Some(&self.user),
            ProfileField::Database => Some(&self.database),
            ProfileField::Schema => Some(&self.schema),
            ProfileField::SqlitePath => Some(&self.sqlite_path),
            ProfileField::DuckDbPath => Some(&self.duckdb_path),
            ProfileField::CaCertPath => Some(&self.ca_cert_path),
            _ => None,
        }
    }

    pub fn undo(&mut self, field: ProfileField) {
        let changed = match field {
            ProfileField::Url => self.url.undo(),
            ProfileField::Password => self.password.undo(),
            _ => self.text_input_mut(field).is_some_and(|input| input.undo()),
        };
        if changed {
            if field == ProfileField::Url {
                self.mark_url_edited();
            }
            self.connection_field_changed(field);
        }
    }

    pub fn redo(&mut self, field: ProfileField) {
        let changed = match field {
            ProfileField::Url => self.url.redo(),
            ProfileField::Password => self.password.redo(),
            _ => self.text_input_mut(field).is_some_and(|input| input.redo()),
        };
        if changed {
            if field == ProfileField::Url {
                self.mark_url_edited();
            }
            self.connection_field_changed(field);
        }
    }

    pub fn finish_edit_group(&mut self, field: ProfileField) {
        match field {
            ProfileField::Url => self.url.finish_edit_group(),
            ProfileField::Password => self.password.finish_edit_group(),
            _ => {
                if let Some(input) = self.text_input_mut(field) {
                    input.finish_edit_group();
                }
            }
        }
    }

    fn set_kind(&mut self, kind: DatabaseKind) {
        if self.kind == kind {
            return;
        }

        self.invalidate_catalog_discovery();
        if self.kind == DatabaseKind::Elasticsearch {
            self.set_elasticsearch_auth(crate::profile::ElasticsearchAuth::Basic);
            self.ca_cert_path.set("");
        }
        let previous = self.kind;
        self.recent_drivers[usize::from(self.category() == DatabaseCategory::NonRelational)] =
            Some(previous);
        let cross_category = self.category() != descriptor(kind).category;
        let port_is_automatic = self.automatic_port.as_deref() == Some(self.port.value());
        if let Some(target_port) = default_port(kind)
            && (self.port.value().trim().is_empty() || port_is_automatic)
        {
            self.port.set(target_port.to_string());
            self.automatic_port = Some(target_port.to_string());
        }
        if cross_category {
            self.catalog_scope_mode = CatalogScopeMode::Derived;
        }
        self.kind = kind;
        self.url_format = if kind.is_redis_compatible() {
            redis_compatible_url_format(kind, self.ssl_mode)
        } else {
            ConnectionUrlFormat::default_for(kind)
        };
        match kind {
            DatabaseKind::Snowflake => {
                self.ssl_mode = SslMode::VerifyFull;
                if self.schema.value().trim().is_empty()
                    || matches!(self.schema.value(), "public" | "main" | "dbo")
                {
                    self.schema.set("PUBLIC");
                }
                self.snowflake_options.get_or_insert_with(Default::default);
            }
            DatabaseKind::Postgres | DatabaseKind::CockroachDb => {
                if kind == DatabaseKind::CockroachDb && self.database.value().trim().is_empty() {
                    self.database.set("defaultdb");
                }
                if self.host.value().trim().is_empty() {
                    self.host.set("localhost");
                }
                if self.schema.value().trim().is_empty() || self.schema.value() == "main" {
                    self.schema.set("public");
                }
                if previous == DatabaseKind::Sqlite && self.ssl_mode == SslMode::Disable {
                    self.ssl_mode = SslMode::Prefer;
                }
            }
            DatabaseKind::MySql | DatabaseKind::MariaDb | DatabaseKind::TiDb => {
                if kind == DatabaseKind::TiDb && self.user.value().trim().is_empty() {
                    self.user.set("root");
                }
                if self.host.value().trim().is_empty() {
                    self.host.set("localhost");
                }
                if self.schema.value() == "public" || self.schema.value() == "main" {
                    self.schema.set("");
                }
                if previous == DatabaseKind::Sqlite && self.ssl_mode == SslMode::Disable {
                    self.ssl_mode = SslMode::Prefer;
                }
            }
            DatabaseKind::SqlServer => {
                if self.host.value().trim().is_empty() {
                    self.host.set("localhost");
                }
                if self.schema.value().trim().is_empty()
                    || self.schema.value() == "public"
                    || self.schema.value() == "main"
                {
                    self.schema.set("dbo");
                }
                if previous == DatabaseKind::Sqlite && self.ssl_mode == SslMode::Disable {
                    self.ssl_mode = SslMode::Prefer;
                }
            }
            DatabaseKind::Sqlite => self.ssl_mode = SslMode::Disable,
            DatabaseKind::DuckDb => {
                self.ssl_mode = SslMode::Disable;
                self.host.set("");
                self.port.set("");
                self.user.set("");
            }
            DatabaseKind::Redis | DatabaseKind::Valkey => {
                if self.host.value().trim().is_empty() {
                    self.host.set("localhost");
                }
                if self.database.value().trim().parse::<u32>().is_err() {
                    self.database.set("0");
                }
                if matches!(
                    self.ssl_mode,
                    SslMode::Prefer | SslMode::VerifyCa | SslMode::VerifyFull
                ) {
                    self.ssl_mode = SslMode::Disable;
                }
            }
            DatabaseKind::MongoDb => {
                if self.host.value().trim().is_empty() {
                    self.host.set("localhost");
                }
                if self.port.value().trim().is_empty() {
                    self.port.set("27017");
                    self.automatic_port = Some("27017".to_owned());
                }
                if self.ssl_mode == SslMode::Disable {
                    self.ssl_mode = SslMode::Prefer;
                }
                if self.mongodb_options.is_none() {
                    self.mongodb_options = Some(crate::profile::MongoConnectionOptions {
                        seeds: vec![crate::profile::MongoSeed {
                            host: self.host.value().to_owned(),
                            port: Some(27017),
                        }],
                        srv: false,
                        uri_options: Vec::new(),
                    });
                }
            }
            DatabaseKind::ClickHouse => {
                if self.host.value().trim().is_empty() {
                    self.host.set("localhost");
                }
                if self.port.value().trim().is_empty() {
                    self.port.set("8123");
                    self.automatic_port = Some("8123".to_owned());
                }
                if self.database.value().trim().is_empty() {
                    self.database.set("default");
                }
                if self.user.value().trim().is_empty() {
                    self.user.set("default");
                }
                if previous.is_redis_compatible() {
                    self.ssl_mode = SslMode::Disable;
                }
            }
            DatabaseKind::Elasticsearch => {
                if self.host.value().trim().is_empty() {
                    self.host.set("localhost");
                }
                self.schema.set("");
                self.ssl_mode = SslMode::Disable;
            }
            DatabaseKind::Oracle => {
                if self.host.value().trim().is_empty() {
                    self.host.set("localhost");
                }
                if self.schema.value() == "public" || self.schema.value() == "main" {
                    self.schema.set("");
                }
                if previous == DatabaseKind::Sqlite && self.ssl_mode == SslMode::Disable {
                    self.ssl_mode = SslMode::Prefer;
                }
            }
        }
        self.sync_derived_catalog_scope();
        self.refresh_url();
    }

    fn mark_url_edited(&mut self) {
        self.mark_url_edited_at(Instant::now());
    }

    fn mark_url_edited_at(&mut self, now: Instant) {
        self.url_pending = true;
        self.url_parse_deadline = Some(now + URL_PARSE_DEBOUNCE);
        self.url_error = None;
        self.url_generation_error = None;
        self.url_selection = None;
    }

    fn refresh_url(&mut self) {
        let profile = match self.connection_profile_for_url() {
            Ok(profile) => profile,
            Err(error) => {
                self.url.set("");
                self.url_selection = None;
                self.url_pending = false;
                self.url_parse_deadline = None;
                self.url_error = None;
                self.url_generation_error = Some(error);
                return;
            }
        };
        let url = match format_connection_url(&profile, self.url_format) {
            Ok(url) => url,
            Err(error) => {
                self.url.set("");
                self.url_selection = None;
                self.url_pending = false;
                self.url_parse_deadline = None;
                self.url_error = None;
                self.url_generation_error = Some(url_generation_error(error));
                return;
            }
        };
        self.url.set(url);
        self.url_selection = None;
        self.url_pending = false;
        self.url_parse_deadline = None;
        self.url_error = None;
        self.url_generation_error = None;
    }

    fn connection_profile_for_url(&self) -> Result<ConnectionProfile, ProfileValidationError> {
        let (host, port, user, database, default_schema, sqlite_path) = match self.kind {
            DatabaseKind::DuckDb => {
                let raw = optional(&self.duckdb_path);
                if raw.is_none() && self.read_only {
                    return Err(ProfileValidationError::new(
                        ProfileField::ReadOnly,
                        "in-memory DuckDB cannot be read-only",
                    ));
                }
                let path = raw.map(PathBuf::from);
                let database = path
                    .as_ref()
                    .and_then(|path| path.file_stem())
                    .and_then(|name| name.to_str())
                    .unwrap_or("memory")
                    .to_owned();
                (
                    None,
                    None,
                    None,
                    Some(database),
                    Some(optional(&self.schema).unwrap_or_else(|| "main".into())),
                    path,
                )
            }
            DatabaseKind::Postgres
            | DatabaseKind::CockroachDb
            | DatabaseKind::MySql
            | DatabaseKind::MariaDb
            | DatabaseKind::TiDb
            | DatabaseKind::SqlServer
            | DatabaseKind::Snowflake => {
                let host = optional(&self.host).ok_or_else(|| {
                    ProfileValidationError::new(ProfileField::Host, "host is required")
                })?;
                let port = self.port.value().trim().parse::<u16>().map_err(|_| {
                    ProfileValidationError::new(
                        ProfileField::Port,
                        "port must be an integer from 1 to 65535",
                    )
                })?;
                if port == 0 {
                    return Err(ProfileValidationError::new(
                        ProfileField::Port,
                        "port must be an integer from 1 to 65535",
                    ));
                }
                (
                    Some(host),
                    Some(port),
                    optional(&self.user),
                    optional(&self.database),
                    matches!(
                        self.kind,
                        DatabaseKind::Postgres
                            | DatabaseKind::CockroachDb
                            | DatabaseKind::SqlServer
                            | DatabaseKind::Snowflake
                    )
                    .then(|| {
                        optional(&self.schema).or_else(|| {
                            (self.kind == DatabaseKind::Snowflake).then(|| "PUBLIC".to_owned())
                        })
                    })
                    .flatten(),
                    None,
                )
            }
            DatabaseKind::ClickHouse => {
                let host = optional(&self.host).ok_or_else(|| {
                    ProfileValidationError::new(ProfileField::Host, "host is required")
                })?;
                let port = self.port.value().trim().parse::<u16>().map_err(|_| {
                    ProfileValidationError::new(
                        ProfileField::Port,
                        "port must be an integer from 1 to 65535",
                    )
                })?;
                if port == 0 {
                    return Err(ProfileValidationError::new(
                        ProfileField::Port,
                        "port must be an integer from 1 to 65535",
                    ));
                }
                let database = required(
                    &self.database,
                    ProfileField::Database,
                    "database is required",
                )?;
                (
                    Some(host),
                    Some(port),
                    optional(&self.user).or_else(|| Some("default".to_owned())),
                    Some(database.clone()),
                    Some(database),
                    None,
                )
            }
            DatabaseKind::Redis | DatabaseKind::Valkey => {
                let host = required(&self.host, ProfileField::Host, "host is required")?;
                let port = self.port.value().trim().parse::<u16>().map_err(|_| {
                    ProfileValidationError::new(
                        ProfileField::Port,
                        "port must be an integer from 1 to 65535",
                    )
                })?;
                if port == 0 {
                    return Err(ProfileValidationError::new(
                        ProfileField::Port,
                        "port must be an integer from 1 to 65535",
                    ));
                }
                let database = self.database.value().trim().parse::<u32>().map_err(|_| {
                    ProfileValidationError::new(
                        ProfileField::Database,
                        "Database number must be a non-negative integer",
                    )
                })?;
                (
                    Some(host),
                    Some(port),
                    optional(&self.user),
                    Some(database.to_string()),
                    None,
                    None,
                )
            }
            DatabaseKind::Oracle => {
                let port = self.port.value().trim().parse::<u16>().map_err(|_| {
                    ProfileValidationError::new(
                        ProfileField::Port,
                        "port must be an integer from 1 to 65535",
                    )
                })?;
                if port == 0 {
                    return Err(ProfileValidationError::new(
                        ProfileField::Port,
                        "port must be an integer from 1 to 65535",
                    ));
                }
                (
                    Some(optional(&self.host).ok_or_else(|| {
                        ProfileValidationError::new(ProfileField::Host, "host is required")
                    })?),
                    Some(port),
                    optional(&self.user),
                    Some(optional(&self.database).ok_or_else(|| {
                        ProfileValidationError::new(
                            ProfileField::Database,
                            "service name is required",
                        )
                    })?),
                    None,
                    None,
                )
            }
            DatabaseKind::Sqlite => {
                let path = (!self.sqlite_memory)
                    .then(|| optional(&self.sqlite_path).map(PathBuf::from))
                    .flatten();
                if !self.sqlite_memory && path.is_none() {
                    return Err(ProfileValidationError::new(
                        ProfileField::SqlitePath,
                        "SQLite path is required when memory mode is disabled",
                    ));
                }
                (
                    None,
                    None,
                    None,
                    Some(if self.sqlite_memory {
                        ":memory:".to_owned()
                    } else {
                        path.as_ref().unwrap().to_string_lossy().into_owned()
                    }),
                    Some("main".to_owned()),
                    path,
                )
            }
            DatabaseKind::Elasticsearch => {
                let host = required(&self.host, ProfileField::Host, "host is required")?;
                let port = self
                    .port
                    .value()
                    .trim()
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port > 0)
                    .ok_or_else(|| {
                        ProfileValidationError::new(
                            ProfileField::Port,
                            "port must be an integer from 1 to 65535",
                        )
                    })?;
                (
                    Some(host),
                    Some(port),
                    optional(&self.user),
                    optional(&self.database),
                    None,
                    None,
                )
            }
            DatabaseKind::MongoDb => {
                let host = optional(&self.host).ok_or_else(|| {
                    ProfileValidationError::new(ProfileField::Host, "host is required")
                })?;
                let port = if self.url_format == ConnectionUrlFormat::MongoDbSrv {
                    None
                } else {
                    Some(
                        self.port
                            .value()
                            .trim()
                            .parse::<u16>()
                            .ok()
                            .filter(|port| *port > 0)
                            .ok_or_else(|| {
                                ProfileValidationError::new(
                                    ProfileField::Port,
                                    "port must be an integer from 1 to 65535",
                                )
                            })?,
                    )
                };
                (
                    Some(host),
                    port,
                    optional(&self.user),
                    optional(&self.database),
                    None,
                    None,
                )
            }
        };
        Ok(ConnectionProfile {
            id: self.profile_id,
            name: String::new(),
            access: self.access.clone(),
            group_id: None,
            kind: self.kind,
            url_format: self.url_format,
            host,
            port,
            user,
            database,
            default_schema,
            sqlite_path,
            mongodb_options: self.mongodb_options.clone(),
            elasticsearch_options: self.elasticsearch_options(),
            pg_options: self.pg_options.clone(),
            snowflake_options: self.snowflake_options.clone(),
            duckdb_path: (self.kind == DatabaseKind::DuckDb)
                .then(|| optional(&self.duckdb_path).map(PathBuf::from))
                .flatten(),
            ssl_mode: self.ssl_mode,
            credential_policy: CredentialPolicy::None,
            read_only: self.read_only,
            environment: self.environment,
            catalog_scope: self.catalog_scope.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_parse_deadline_is_trailing_and_not_due_early() {
        let start = Instant::now();
        let mut draft = ProfileDraft::new(DatabaseKind::Postgres);

        draft.mark_url_edited_at(start);
        assert!(!draft.url_parse_is_due_at(start + Duration::from_millis(299)));
        assert!(draft.url_parse_is_due_at(start + Duration::from_millis(300)));

        let later = start + Duration::from_millis(100);
        draft.mark_url_edited_at(later);
        assert!(!draft.url_parse_is_due_at(start + Duration::from_millis(300)));
        assert!(draft.url_parse_is_due_at(later + Duration::from_millis(300)));
    }

    #[test]
    fn generated_urls_are_not_left_pending_for_auto_parse() {
        let draft = ProfileDraft::new(DatabaseKind::Postgres);

        assert!(!draft.url_is_pending());
        assert!(!draft.url_parse_is_due_at(Instant::now()));
    }

    #[test]
    fn due_url_parse_updates_fields_without_rewriting_the_editor() {
        let mut draft = ProfileDraft::new(DatabaseKind::Postgres);
        while draft.url_cursor() > 0 {
            draft.backspace(ProfileField::Url);
        }
        draft.paste(
            ProfileField::Url,
            "postgresql://alice:secret@db.example:5440/app?sslmode=require",
        );
        draft.move_left(ProfileField::Url);
        let url = draft.url_display();
        let cursor = draft.url_cursor();

        assert!(draft.parse_url_if_due_at(Instant::now() + Duration::from_secs(1)));
        assert_eq!(draft.url_display(), url);
        assert_eq!(draft.url_cursor(), cursor);
        assert_eq!(draft.host.value(), "db.example");
        assert_eq!(draft.port.value(), "5440");
        assert_eq!(draft.password().expose_secret(), "secret");
        assert!(!draft.url_is_pending());
        assert!(draft.validate(&[]).is_err());
    }

    #[test]
    fn due_invalid_url_keeps_structured_fields_and_stops_retrying() {
        let mut draft = ProfileDraft::new(DatabaseKind::Postgres);
        let host = draft.host.value().to_owned();
        draft.move_home(ProfileField::Url);
        draft.paste(ProfileField::Url, "not-a-url");

        assert!(draft.parse_url_if_due_at(Instant::now() + Duration::from_secs(1)));
        assert_eq!(draft.host.value(), host);
        assert!(draft.url_error().is_some());
        assert!(!draft.url_is_pending());
        assert!(!draft.parse_url_if_due_at(Instant::now() + Duration::from_secs(2)));
    }
}

impl fmt::Debug for ProfileDraft {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProfileDraft")
            .field("profile_id", &self.profile_id)
            .field("kind", &self.kind)
            .field("url_format", &self.url_format)
            .field("url", &"[REDACTED]")
            .field("name", &self.name)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &"[REDACTED]")
            .field("database", &self.database)
            .field("schema", &self.schema)
            .field("ssl_mode", &self.ssl_mode)
            .field("environment", &self.environment)
            .field("read_only", &self.read_only)
            .field("password_storage", &self.password_storage)
            .field("sqlite_memory", &self.sqlite_memory)
            .field("sqlite_path", &self.sqlite_path)
            .field(
                "original_credential_policy",
                &self.original_credential_policy,
            )
            .field("has_stored_credential", &self.has_stored_credential)
            .field("catalog_scope", &self.catalog_scope)
            .field("catalog_scope_mode", &self.catalog_scope_mode)
            .field("catalog_discovery", &self.catalog_discovery)
            .field("discovery_fingerprint", &self.discovery_fingerprint)
            .field("credential_revision", &self.credential_revision)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct ProfileManagerState {
    pub page: ProfileManagerPage,
    pub draft: Option<ProfileDraft>,
    pub delete_profile_id: Option<Uuid>,
    pub delete_focus: ProfileDeleteFocus,
    pub selected_field: ProfileField,
    pub operation: Option<ProfileOperation>,
    pub message: Option<ProfileMessage>,
    pub request_generation: u64,
    pub opened_automatically: bool,
    pub scope_selected_row: Option<String>,
    pub scope_expanded_databases: HashSet<String>,
    pub scope_viewport: usize,
    pub scope_warning: Option<String>,
    pub scope_discovery_request: Option<(u64, DiscoveryFingerprint)>,
    pub system_credential_availability: SecretStoreAvailability,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProfileDeleteFocus {
    #[default]
    Cancel,
    Delete,
}

const SCOPE_VIEWPORT_CAPACITY: usize = 29;

impl ProfileManagerState {
    pub fn set_message(&mut self, level: ProfileMessageLevel, message: impl Into<String>) {
        self.message = Some(ProfileMessage::new(level, message));
    }

    pub fn new(opened_automatically: bool) -> Self {
        Self {
            page: ProfileManagerPage::Form,
            draft: None,
            delete_profile_id: None,
            delete_focus: ProfileDeleteFocus::Cancel,
            selected_field: ProfileField::Kind,
            operation: None,
            message: None,
            request_generation: 0,
            opened_automatically,
            scope_selected_row: None,
            scope_expanded_databases: HashSet::new(),
            scope_viewport: 0,
            scope_warning: None,
            scope_discovery_request: None,
            system_credential_availability: SecretStoreAvailability::Unavailable,
        }
    }

    pub fn start_new(&mut self, kind: DatabaseKind) {
        self.page = ProfileManagerPage::Form;
        self.draft = Some(ProfileDraft::new(kind));
        self.selected_field = ProfileField::Kind;
        self.operation = None;
        self.message = None;
        self.scope_warning = None;
        self.scope_discovery_request = None;
    }

    pub fn start_edit(&mut self, profile: &ConnectionProfile, has_stored_credential: bool) {
        self.page = ProfileManagerPage::Form;
        self.draft = Some(ProfileDraft::edit(profile, has_stored_credential));
        self.selected_field = ProfileField::DatabaseCategory;
        self.operation = None;
        self.message = None;
        self.scope_warning = None;
        self.scope_discovery_request = None;
    }

    pub fn open_scope_picker(&mut self) {
        if let Some(draft) = self.draft.as_mut() {
            draft.sync_derived_catalog_scope();
            self.page = ProfileManagerPage::Scope;
            self.scope_expanded_databases = selected_database_names(&draft.catalog_scope);
            self.scope_selected_row = self
                .scope_rows_for_render()
                .first()
                .map(|row| row.id.clone());
            if let Some(database) = self
                .scope_selected_row
                .as_deref()
                .and_then(scope_database_id)
            {
                self.scope_expanded_databases.insert(database.to_owned());
            }
            self.scope_viewport = 0;
            self.scope_warning = self.scope_unavailable_warning();
        }
    }

    pub fn begin_scope_discovery(&mut self, request_id: u64, fingerprint: DiscoveryFingerprint) {
        if let Some(draft) = self.draft.as_mut() {
            draft.begin_catalog_discovery(fingerprint);
        }
        self.scope_discovery_request = Some((request_id, fingerprint));
        self.scope_warning = Some("Discovering databases and schemas...".into());
    }

    pub fn matches_scope_discovery(
        &self,
        request_id: u64,
        fingerprint: DiscoveryFingerprint,
    ) -> bool {
        self.scope_discovery_request == Some((request_id, fingerprint))
    }

    pub fn finish_scope_discovery(&mut self) {
        self.scope_discovery_request = None;
        self.scope_warning = self.scope_unavailable_warning();
    }

    pub fn fail_scope_discovery(&mut self, message: String) {
        self.scope_discovery_request = None;
        self.scope_warning = Some(format!(
            "Catalog discovery failed: {message}; saved selections are shown"
        ));
    }

    pub const fn scope_discovery_loading(&self) -> bool {
        self.scope_discovery_request.is_some()
    }

    pub fn close_scope_picker(&mut self) {
        self.page = ProfileManagerPage::Form;
        self.selected_field = ProfileField::VisibleObjects;
    }

    pub fn move_scope_selection(&mut self, delta: isize) {
        if let Some(database) = self
            .scope_selected_row
            .as_deref()
            .filter(|id| !id.contains(":schema:"))
            .and_then(scope_database_id)
        {
            self.scope_expanded_databases.insert(database.to_owned());
        }
        let rows = self.scope_rows_for_render();
        if rows.is_empty() {
            self.scope_selected_row = None;
            self.scope_viewport = 0;
            return;
        }
        let current = self
            .scope_selected_row
            .as_ref()
            .and_then(|id| rows.iter().position(|row| &row.id == id))
            .unwrap_or(0);
        let next = (current as isize + delta).clamp(0, rows.len() as isize - 1) as usize;
        self.scope_selected_row = Some(rows[next].id.clone());
        if next < self.scope_viewport {
            self.scope_viewport = next;
        } else if next >= self.scope_viewport + SCOPE_VIEWPORT_CAPACITY {
            self.scope_viewport = next + 1 - SCOPE_VIEWPORT_CAPACITY;
        }
    }

    pub fn set_scope_viewport_for_test(&mut self, viewport: usize) {
        self.scope_viewport = viewport;
    }

    pub fn scope_warning(&self) -> Option<&str> {
        self.scope_warning.as_deref()
    }

    pub fn scope_row(&self, id: &str) -> Option<ScopeRow> {
        self.scope_rows_for_render()
            .into_iter()
            .find(|row| row.id == id)
    }

    pub fn toggle_scope_row(&mut self, id: &str) -> bool {
        if self.scope_discovery_loading() {
            return false;
        }
        let Some(row) = self.scope_row(id) else {
            return false;
        };
        if row.read_only {
            return false;
        }
        let discovered = self.discovered_scope();
        let Some(draft) = self.draft.as_mut() else {
            return false;
        };
        let changed = toggle_scope(
            &mut draft.catalog_scope,
            draft.kind,
            &row,
            discovered.as_ref(),
        );
        if changed {
            draft.catalog_scope_mode = CatalogScopeMode::Explicit;
            if row.database
                && row.selection != ScopeSelectionState::Checked
                && let Some(database) = scope_database_id(&row.id)
            {
                self.scope_expanded_databases.insert(database.to_owned());
            }
            self.scope_warning = self.scope_unavailable_warning();
        }
        changed
    }

    fn discovered_scope(&self) -> Option<crate::db::catalog::CatalogDiscovery> {
        self.draft
            .as_ref()
            .and_then(|draft| match &draft.catalog_discovery {
                CatalogDiscoveryState::Fresh(snapshot) | CatalogDiscoveryState::Stale(snapshot) => {
                    snapshot
                        .discovery
                        .as_ref()
                        .and_then(|result| result.as_ref().ok().cloned())
                }
                CatalogDiscoveryState::NotRequested => None,
            })
    }

    pub fn scope_rows_for_render(&self) -> Vec<ScopeRow> {
        let Some(draft) = self.draft.as_ref() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        let selected = match &draft.catalog_scope.databases {
            CatalogSelection::All => None,
            CatalogSelection::Selected(items) => Some(items),
        };
        let discovery = match &draft.catalog_discovery {
            CatalogDiscoveryState::Fresh(snapshot) | CatalogDiscoveryState::Stale(snapshot) => {
                snapshot.discovery.as_ref()
            }
            CatalogDiscoveryState::NotRequested => None,
        };
        let discovered = discovery.and_then(|result| result.as_ref().ok());
        let names =
            selected
                .into_iter()
                .flat_map(|items| items.iter().map(|item| item.name.as_str()))
                .chain(discovered.into_iter().flat_map(|discovery| {
                    discovery.databases.iter().map(|item| item.name.as_str())
                }))
                .collect::<HashSet<_>>();
        for database in names {
            let schemas = discovered
                .and_then(|discovery| {
                    discovery
                        .databases
                        .iter()
                        .find(|item| item.name == database)
                        .map(|item| item.schemas.clone())
                })
                .unwrap_or_default();
            rows.push(ScopeRow {
                id: format!("database:{database}"),
                name: database.to_owned(),
                selection: database_selection_state(
                    &draft.catalog_scope,
                    database,
                    &schemas,
                    discovered.is_some(),
                ),
                read_only: false,
                unavailable: discovery.is_none_or(|result| {
                    result.as_ref().map_or(true, |items| {
                        !items.databases.iter().any(|item| item.name == database)
                    })
                }),
                database: true,
            });
            if self.scope_expanded_databases.contains(database)
                || self.scope_selected_row.as_deref() == Some(&format!("database:{database}"))
            {
                let selected_schemas = selected_schema(&draft.catalog_scope, database);
                if matches!(
                    draft.kind,
                    DatabaseKind::MySql | DatabaseKind::MariaDb | DatabaseKind::TiDb
                ) {
                    rows.push(ScopeRow {
                        id: format!("database:{database}:schema:{database}"),
                        name: database.to_owned(),
                        selection: if database_selected(&draft.catalog_scope, database) {
                            ScopeSelectionState::Checked
                        } else {
                            ScopeSelectionState::Unchecked
                        },
                        read_only: true,
                        unavailable: discovery.is_none_or(|result| {
                            result.as_ref().map_or(true, |items| {
                                !items.databases.iter().any(|item| item.name == database)
                            })
                        }),
                        database: false,
                    });
                }
                let custom_schemas =
                    selected_schemas.map_or_else(Vec::new, |selection| match selection {
                        CatalogSelection::Selected(items) => items.clone(),
                        CatalogSelection::All => Vec::new(),
                    });
                let schema_names = schemas
                    .into_iter()
                    .chain(custom_schemas)
                    .collect::<HashSet<_>>();
                for schema in schema_names {
                    rows.push(ScopeRow {
                        id: format!("database:{database}:schema:{schema}"),
                        name: schema.clone(),
                        selection: if schema_selected(&draft.catalog_scope, database, &schema) {
                            ScopeSelectionState::Checked
                        } else {
                            ScopeSelectionState::Unchecked
                        },
                        read_only: matches!(
                            draft.kind,
                            DatabaseKind::MySql | DatabaseKind::MariaDb | DatabaseKind::TiDb
                        ),
                        unavailable: discovery.is_none_or(|result| {
                            result.as_ref().map_or(true, |items| {
                                !items
                                    .databases
                                    .iter()
                                    .find(|item| item.name == database)
                                    .is_some_and(|item| {
                                        item.schemas.iter().any(|name| name == &schema)
                                    })
                            })
                        }),
                        database: false,
                    });
                }
            }
        }
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        rows
    }

    fn scope_unavailable_warning(&self) -> Option<String> {
        let draft = self.draft.as_ref()?;
        match &draft.catalog_discovery {
            CatalogDiscoveryState::Stale(snapshot) => snapshot
                .discovery
                .as_ref()
                .and_then(|result| result.as_ref().err())
                .map_or_else(
                    || Some("Discovery is stale; saved selections were preserved".into()),
                    |error| Some(format!("Catalog discovery warning: {error}")),
                ),
            CatalogDiscoveryState::NotRequested => {
                Some("Discovery unavailable; saved selections are shown".into())
            }
            CatalogDiscoveryState::Fresh(snapshot) => snapshot
                .discovery
                .as_ref()
                .and_then(|result| result.as_ref().err())
                .map(|error| format!("Catalog discovery warning: {error}"))
                .or_else(|| {
                    snapshot
                        .discovery
                        .as_ref()
                        .and_then(|result| result.as_ref().ok())
                        .and_then(|discovery| {
                            (!discovery.warnings.is_empty()).then(|| {
                                format!(
                                    "Catalog discovery warning: {}",
                                    discovery.warnings.join("; ")
                                )
                            })
                        })
                }),
        }
    }

    pub fn visible_fields(&self) -> &'static [ProfileField] {
        self.draft
            .as_ref()
            .map_or(&[], ProfileDraft::visible_fields)
    }

    pub fn set_system_credential_availability(&mut self, availability: SecretStoreAvailability) {
        self.system_credential_availability = availability;
        if let Some(draft) = self.draft.as_mut() {
            draft.system_credential_availability = availability;
        }
    }

    pub fn password_storage_choices(&self) -> &'static [PasswordStorageChoice] {
        self.draft
            .as_ref()
            .map_or(&[], ProfileDraft::password_storage_choices)
    }

    pub fn move_field(&mut self, delta: isize) {
        if self.selected_field == ProfileField::Url
            && let Err(error) = self.commit_url()
        {
            self.set_message(ProfileMessageLevel::Error, error.message);
            return;
        }
        if let Some(draft) = self.draft.as_mut() {
            draft.finish_edit_group(self.selected_field);
        }
        let fields = self.visible_fields();
        if fields.is_empty() {
            return;
        }
        let current = fields
            .iter()
            .position(|field| *field == self.selected_field)
            .unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(fields.len() as isize) as usize;
        self.selected_field = fields[next];
    }

    pub fn focus_field(&mut self, field: ProfileField) {
        if self.selected_field == ProfileField::Url
            && field != ProfileField::Url
            && let Err(error) = self.commit_url()
        {
            self.set_message(ProfileMessageLevel::Error, error.message);
            return;
        }
        if field != self.selected_field
            && let Some(draft) = self.draft.as_mut()
        {
            draft.finish_edit_group(self.selected_field);
        }
        if self.visible_fields().contains(&field) {
            self.selected_field = field;
        }
    }

    pub fn insert(&mut self, character: char) {
        let field = self.selected_field;
        if let Some(draft) = self.draft.as_mut() {
            draft.insert(field, character);
            self.message = None;
        }
    }

    pub fn paste(&mut self, text: &str) {
        let field = self.selected_field;
        if let Some(draft) = self.draft.as_mut() {
            draft.paste(field, text);
            self.message = None;
        }
    }

    pub fn backspace(&mut self) {
        let field = self.selected_field;
        if let Some(draft) = self.draft.as_mut() {
            draft.backspace(field);
            self.message = None;
        }
    }

    pub fn delete_previous_word(&mut self) {
        let field = self.selected_field;
        if let Some(draft) = self.draft.as_mut() {
            draft.delete_previous_word(field);
            self.message = None;
        }
    }

    pub fn delete_to_start(&mut self) {
        let field = self.selected_field;
        if let Some(draft) = self.draft.as_mut() {
            draft.delete_to_start(field);
            self.message = None;
        }
    }

    pub fn delete(&mut self) {
        let field = self.selected_field;
        if let Some(draft) = self.draft.as_mut() {
            draft.delete(field);
            self.message = None;
        }
    }

    pub fn move_cursor_left(&mut self) {
        let field = self.selected_field;
        if let Some(draft) = self.draft.as_mut() {
            draft.move_left(field);
        }
    }

    pub fn move_cursor_right(&mut self) {
        let field = self.selected_field;
        if let Some(draft) = self.draft.as_mut() {
            draft.move_right(field);
        }
    }

    pub fn move_cursor_home(&mut self) {
        let field = self.selected_field;
        if let Some(draft) = self.draft.as_mut() {
            draft.move_home(field);
        }
    }

    pub fn move_cursor_end(&mut self) {
        let field = self.selected_field;
        if let Some(draft) = self.draft.as_mut() {
            draft.move_end(field);
        }
    }

    pub fn set_cursor(&mut self, field: ProfileField, cursor: usize) {
        if let Some(draft) = self.draft.as_mut() {
            draft.set_cursor(field, cursor);
        }
    }

    pub fn undo(&mut self) {
        let field = self.selected_field;
        if let Some(draft) = self.draft.as_mut() {
            draft.undo(field);
            self.message = None;
        }
    }

    pub fn redo(&mut self) {
        let field = self.selected_field;
        if let Some(draft) = self.draft.as_mut() {
            draft.redo(field);
            self.message = None;
        }
    }

    pub fn cycle(&mut self, delta: i8) {
        if self.operation.is_some() {
            return;
        }
        let field = self.selected_field;
        let password_choices = self.password_storage_choices();
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        match field {
            ProfileField::DatabaseCategory => {
                draft.select_category(cycle_value(draft.category(), &DatabaseCategory::ALL, delta))
            }
            ProfileField::Kind => {
                let kinds = drivers_in(draft.category())
                    .map(|driver| driver.kind)
                    .collect::<Vec<_>>();
                draft.set_kind(cycle_value(draft.kind, &kinds, delta));
            }
            ProfileField::UrlFormat => {
                draft.url_format = cycle_value(
                    draft.url_format,
                    ConnectionUrlFormat::compatible_formats(draft.kind),
                    delta,
                );
                if draft.kind == DatabaseKind::Elasticsearch {
                    draft.ssl_mode = if draft.url_format == ConnectionUrlFormat::ElasticsearchTls {
                        SslMode::VerifyFull
                    } else {
                        SslMode::Disable
                    };
                }
                draft.refresh_url();
            }
            ProfileField::ElasticsearchAuth => {
                draft.set_elasticsearch_auth(cycle_value(
                    draft.elasticsearch_auth,
                    &[
                        crate::profile::ElasticsearchAuth::Basic,
                        crate::profile::ElasticsearchAuth::ApiKey,
                    ],
                    delta,
                ));
            }
            ProfileField::SslMode => {
                let choices: &[SslMode] = if draft.kind == DatabaseKind::Elasticsearch {
                    &[SslMode::Disable, SslMode::VerifyFull]
                } else if draft.kind == DatabaseKind::Snowflake {
                    &[SslMode::VerifyFull]
                } else if draft.kind.is_redis_compatible() {
                    &[SslMode::Disable, SslMode::Require]
                } else {
                    &[
                        SslMode::Disable,
                        SslMode::Prefer,
                        SslMode::Require,
                        SslMode::VerifyCa,
                        SslMode::VerifyFull,
                    ]
                };
                draft.ssl_mode = cycle_value(draft.ssl_mode, choices, delta);
                if draft.kind.is_redis_compatible() {
                    draft.url_format = redis_compatible_url_format(draft.kind, draft.ssl_mode);
                }
                if draft.kind == DatabaseKind::Elasticsearch {
                    draft.url_format = if draft.ssl_mode == SslMode::Disable {
                        ConnectionUrlFormat::Elasticsearch
                    } else {
                        ConnectionUrlFormat::ElasticsearchTls
                    };
                }
                draft.invalidate_catalog_discovery();
                draft.refresh_url();
            }
            ProfileField::Environment => {
                draft.environment = cycle_value(
                    draft.environment,
                    &[
                        Environment::Development,
                        Environment::Staging,
                        Environment::Production,
                    ],
                    delta,
                );
            }
            ProfileField::PasswordStorage => {
                draft.password_storage =
                    cycle_value(draft.password_storage, password_choices, delta);
                draft.credential_changed();
            }
            _ => return,
        }
        self.message = None;
    }

    pub fn select_driver(&mut self, kind: DatabaseKind) {
        if self.operation.is_some() {
            return;
        }
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        draft.set_kind(kind);
        self.message = None;
    }

    pub fn toggle(&mut self) {
        let field = self.selected_field;
        let password_choices = self.password_storage_choices();
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        match field {
            ProfileField::ReadOnly => draft.read_only = !draft.read_only,
            ProfileField::PasswordStorage => {
                draft.password_storage = cycle_value(draft.password_storage, password_choices, 1);
                draft.credential_changed();
            }
            ProfileField::SqliteMemory => {
                draft.sqlite_memory = !draft.sqlite_memory;
                draft.invalidate_catalog_discovery();
                draft.sync_derived_catalog_scope();
            }
            _ => return,
        }
        if matches!(field, ProfileField::ReadOnly | ProfileField::SqliteMemory) {
            draft.refresh_url();
        }
        self.message = None;
    }

    pub fn commit_url(&mut self) -> Result<(), ProfileValidationError> {
        let result = self.draft.as_mut().map_or(Ok(()), ProfileDraft::commit_url);
        match &result {
            Ok(()) => self.message = None,
            Err(error) => {
                self.selected_field = ProfileField::Url;
                self.set_message(ProfileMessageLevel::Error, error.message.clone());
            }
        }
        result
    }
}

impl Default for ProfileManagerState {
    fn default() -> Self {
        Self::new(false)
    }
}

fn required(
    input: &TextInput,
    field: ProfileField,
    message: &'static str,
) -> Result<String, ProfileValidationError> {
    let value = input.value().trim();
    if value.is_empty() {
        Err(ProfileValidationError::new(field, message))
    } else {
        Ok(value.to_owned())
    }
}

fn non_empty(input: &TextInput) -> Option<&str> {
    let value = input.value().trim();
    (!value.is_empty()).then_some(value)
}

fn default_port(kind: DatabaseKind) -> Option<u16> {
    match kind {
        DatabaseKind::Postgres => Some(5432),
        DatabaseKind::CockroachDb => Some(26257),
        DatabaseKind::MySql | DatabaseKind::MariaDb => Some(3306),
        DatabaseKind::TiDb => Some(4000),
        DatabaseKind::Oracle => Some(1521),
        DatabaseKind::SqlServer => Some(1433),
        DatabaseKind::Sqlite | DatabaseKind::DuckDb => None,
        DatabaseKind::Redis | DatabaseKind::Valkey => Some(6379),
        DatabaseKind::MongoDb => Some(27017),
        DatabaseKind::ClickHouse => Some(8123),
        DatabaseKind::Elasticsearch => Some(9200),
        DatabaseKind::Snowflake => Some(443),
    }
}

fn redis_compatible_url_format(kind: DatabaseKind, ssl_mode: SslMode) -> ConnectionUrlFormat {
    match (kind, ssl_mode) {
        (DatabaseKind::Redis, SslMode::Disable | SslMode::Prefer) => ConnectionUrlFormat::Redis,
        (DatabaseKind::Redis, _) => ConnectionUrlFormat::RedisTls,
        (DatabaseKind::Valkey, SslMode::Disable | SslMode::Prefer) => ConnectionUrlFormat::Valkey,
        (DatabaseKind::Valkey, _) => ConnectionUrlFormat::ValkeyTls,
        _ => unreachable!("expected a Redis-compatible database kind"),
    }
}

fn optional(input: &TextInput) -> Option<String> {
    let value = input.value().trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn catalog_scope_error_field(error: &CatalogScopeValidationError) -> ProfileField {
    match error {
        CatalogScopeValidationError::EmptyDatabaseSelection
        | CatalogScopeValidationError::EmptyDatabaseName
        | CatalogScopeValidationError::DuplicateDatabase(_) => ProfileField::Database,
        CatalogScopeValidationError::EmptySchemaSelection { .. }
        | CatalogScopeValidationError::EmptySchemaName { .. }
        | CatalogScopeValidationError::DuplicateSchema { .. }
        | CatalogScopeValidationError::DefaultSchemaExcluded { .. } => ProfileField::Schema,
    }
}

fn cycle_value<T: Copy + PartialEq>(current: T, values: &[T], delta: i8) -> T {
    let current = values
        .iter()
        .position(|value| *value == current)
        .unwrap_or(0);
    let next = (current as isize + isize::from(delta)).rem_euclid(values.len() as isize) as usize;
    values[next]
}

fn database_selected(scope: &CatalogScope, name: &str) -> bool {
    matches!(&scope.databases, CatalogSelection::All)
        || matches!(&scope.databases, CatalogSelection::Selected(items) if items.iter().any(|item| item.name == name))
}

fn selected_database_names(scope: &CatalogScope) -> HashSet<String> {
    match &scope.databases {
        CatalogSelection::All => HashSet::new(),
        CatalogSelection::Selected(items) => items.iter().map(|item| item.name.clone()).collect(),
    }
}

fn scope_database_id(id: &str) -> Option<&str> {
    id.strip_prefix("database:")
        .and_then(|value| value.split(":schema:").next())
        .filter(|database| !database.is_empty())
}

fn schema_selected(scope: &CatalogScope, database: &str, schema: &str) -> bool {
    match selected_schema(scope, database) {
        None => matches!(scope.databases, CatalogSelection::All),
        Some(CatalogSelection::All) => true,
        Some(CatalogSelection::Selected(items)) => items.iter().any(|item| item == schema),
    }
}

fn database_selection_state(
    scope: &CatalogScope,
    database: &str,
    discovered_schemas: &[String],
    discovery_available: bool,
) -> ScopeSelectionState {
    if matches!(scope.databases, CatalogSelection::All) {
        return ScopeSelectionState::Checked;
    }
    let Some(selection) = selected_schema(scope, database) else {
        return ScopeSelectionState::Unchecked;
    };
    match selection {
        CatalogSelection::All => ScopeSelectionState::Checked,
        CatalogSelection::Selected(items) => {
            if items.is_empty() {
                ScopeSelectionState::Unchecked
            } else if discovery_available
                && !discovered_schemas.is_empty()
                && items.len() == discovered_schemas.len()
                && discovered_schemas
                    .iter()
                    .all(|schema| items.contains(schema))
            {
                ScopeSelectionState::Checked
            } else {
                ScopeSelectionState::Partial
            }
        }
    }
}

fn selected_schema<'a>(
    scope: &'a CatalogScope,
    database: &str,
) -> Option<&'a CatalogSelection<String>> {
    match &scope.databases {
        CatalogSelection::All => None,
        CatalogSelection::Selected(items) => items
            .iter()
            .find(|item| item.name == database)
            .map(|item| &item.schemas),
    }
}

fn toggle_scope(
    scope: &mut CatalogScope,
    kind: DatabaseKind,
    row: &ScopeRow,
    discovered: Option<&crate::db::catalog::CatalogDiscovery>,
) -> bool {
    materialize_all_databases(scope, discovered);
    let Some((database, schema)) = row
        .id
        .strip_prefix("database:")
        .and_then(|value| value.split_once(":schema:"))
    else {
        let Some(database) = row.id.strip_prefix("database:") else {
            return false;
        };
        if database.is_empty() {
            return false;
        }
        if row.selection == ScopeSelectionState::Checked {
            remove_database(scope, database);
        } else {
            select_database(scope, database);
        }
        return true;
    };
    if matches!(
        kind,
        DatabaseKind::MySql | DatabaseKind::MariaDb | DatabaseKind::TiDb
    ) {
        return false;
    }
    let discovered_schemas = discovered
        .and_then(|discovery| {
            discovery
                .databases
                .iter()
                .find(|item| item.name == database)
        })
        .map(|item| item.schemas.as_slice())
        .unwrap_or_default();
    if schema_selected(scope, database, schema) {
        let remaining = match selected_schema(scope, database) {
            Some(CatalogSelection::All) | None => discovered_schemas
                .iter()
                .filter(|candidate| candidate.as_str() != schema)
                .cloned()
                .collect::<Vec<_>>(),
            Some(CatalogSelection::Selected(items)) => items
                .iter()
                .filter(|candidate| candidate.as_str() != schema)
                .cloned()
                .collect::<Vec<_>>(),
        };
        if remaining.is_empty() {
            remove_database(scope, database);
        } else {
            set_database_schemas(scope, database, CatalogSelection::Selected(remaining));
        }
    } else {
        let mut selected = match selected_schema(scope, database) {
            Some(CatalogSelection::Selected(items)) => items.clone(),
            Some(CatalogSelection::All) | None => Vec::new(),
        };
        selected.push(schema.to_owned());
        selected.sort();
        selected.dedup();
        set_database_schemas(scope, database, CatalogSelection::Selected(selected));
    }
    true
}

fn select_database(scope: &mut CatalogScope, database: &str) {
    match &mut scope.databases {
        CatalogSelection::All => {}
        CatalogSelection::Selected(items) => {
            if let Some(item) = items.iter_mut().find(|item| item.name == database) {
                item.schemas = CatalogSelection::All;
            } else {
                items.push(DatabaseScope {
                    name: database.to_owned(),
                    schemas: CatalogSelection::All,
                });
                items.sort_by(|left, right| left.name.cmp(&right.name));
            }
        }
    }
}

fn remove_database(scope: &mut CatalogScope, database: &str) {
    match &mut scope.databases {
        CatalogSelection::All => {
            scope.databases = CatalogSelection::Selected(Vec::new());
        }
        CatalogSelection::Selected(items) => items.retain(|item| item.name != database),
    }
}

fn set_database_schemas(
    scope: &mut CatalogScope,
    database: &str,
    schemas: CatalogSelection<String>,
) {
    if matches!(scope.databases, CatalogSelection::All) {
        scope.databases = CatalogSelection::Selected(Vec::new());
    }
    let CatalogSelection::Selected(items) = &mut scope.databases else {
        return;
    };
    if let Some(item) = items.iter_mut().find(|item| item.name == database) {
        item.schemas = schemas;
    } else {
        items.push(DatabaseScope {
            name: database.to_owned(),
            schemas,
        });
        items.sort_by(|left, right| left.name.cmp(&right.name));
    }
}

fn materialize_all_databases(
    scope: &mut CatalogScope,
    discovered: Option<&crate::db::catalog::CatalogDiscovery>,
) {
    if !matches!(scope.databases, CatalogSelection::All) {
        return;
    }
    let Some(discovered) = discovered else {
        return;
    };
    scope.databases = CatalogSelection::Selected(
        discovered
            .databases
            .iter()
            .map(|database| DatabaseScope {
                name: database.name.clone(),
                schemas: CatalogSelection::All,
            })
            .collect(),
    );
}

fn redact_url_password(value: &str) -> (String, Vec<(usize, usize)>) {
    if value
        .get(..17)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("jdbc:sqlserver://"))
    {
        let lowercase = value.to_ascii_lowercase();
        let mut byte_ranges = Vec::new();
        let mut search_start = 0;
        while let Some(relative_start) = lowercase[search_start..].find(";password=") {
            let property_start = search_start + relative_start;
            let password_start = property_start + ";password=".len();
            let password_end = if value.as_bytes().get(password_start) == Some(&b'{') {
                let mut cursor = password_start + 1;
                loop {
                    match value.as_bytes().get(cursor) {
                        Some(b'}') if value.as_bytes().get(cursor + 1) == Some(&b'}') => {
                            cursor += 2
                        }
                        Some(b'}') => break cursor + 1,
                        Some(_) => cursor += value[cursor..].chars().next().unwrap().len_utf8(),
                        None => break value.len(),
                    }
                }
            } else {
                value[password_start..]
                    .find(';')
                    .map_or(value.len(), |offset| password_start + offset)
            };
            byte_ranges.push((password_start, password_end));
            search_start = password_end.max(password_start + 1);
        }
        if !byte_ranges.is_empty() {
            let mut redacted = String::with_capacity(value.len());
            let mut copied = 0;
            let mut character_ranges = Vec::with_capacity(byte_ranges.len());
            for (start, end) in byte_ranges {
                redacted.push_str(&value[copied..start]);
                redacted.push_str("[REDACTED]");
                character_ranges
                    .push((value[..start].chars().count(), value[..end].chars().count()));
                copied = end;
            }
            redacted.push_str(&value[copied..]);
            return (redacted, character_ranges);
        }
    }
    redact_authority_password(value)
}

fn redact_authority_password(value: &str) -> (String, Vec<(usize, usize)>) {
    let Some(scheme_end) = value.find("://") else {
        return (value.to_owned(), Vec::new());
    };
    let authority_start = scheme_end + 3;
    let authority_end = value[authority_start..]
        .find(['/', '?', '#'])
        .map_or(value.len(), |offset| authority_start + offset);
    let authority = &value[authority_start..authority_end];
    let Some(at) = authority.rfind('@') else {
        return (value.to_owned(), Vec::new());
    };
    let Some(colon) = authority[..at].find(':') else {
        return (value.to_owned(), Vec::new());
    };
    let password_start = authority_start + colon + 1;
    let password_end = authority_start + at;
    let password_chars = (
        value[..password_start].chars().count(),
        value[..password_end].chars().count(),
    );
    (
        format!(
            "{}[REDACTED]{}",
            &value[..password_start],
            &value[password_end..]
        ),
        vec![password_chars],
    )
}

fn redact_url_query_passwords(value: &str) -> String {
    crate::security::redact_url_query_credentials(value)
}

const POSTGRES_FIELDS: [ProfileField; 18] = [
    ProfileField::DatabaseCategory,
    ProfileField::Kind,
    ProfileField::Name,
    ProfileField::Host,
    ProfileField::Port,
    ProfileField::Database,
    ProfileField::Schema,
    ProfileField::VisibleObjects,
    ProfileField::User,
    ProfileField::Password,
    ProfileField::PasswordStorage,
    ProfileField::SslMode,
    ProfileField::Environment,
    ProfileField::ReadOnly,
    ProfileField::Url,
    ProfileField::Test,
    ProfileField::Save,
    ProfileField::Cancel,
];

const ORACLE_FIELDS: [ProfileField; 17] = [
    ProfileField::DatabaseCategory,
    ProfileField::Kind,
    ProfileField::Name,
    ProfileField::Host,
    ProfileField::Port,
    ProfileField::Database,
    ProfileField::VisibleObjects,
    ProfileField::User,
    ProfileField::Password,
    ProfileField::PasswordStorage,
    ProfileField::SslMode,
    ProfileField::Environment,
    ProfileField::ReadOnly,
    ProfileField::Url,
    ProfileField::Test,
    ProfileField::Save,
    ProfileField::Cancel,
];

const ELASTICSEARCH_FIELDS: [ProfileField; 19] = [
    ProfileField::DatabaseCategory,
    ProfileField::Kind,
    ProfileField::Name,
    ProfileField::Host,
    ProfileField::Port,
    ProfileField::Database,
    ProfileField::VisibleObjects,
    ProfileField::ElasticsearchAuth,
    ProfileField::User,
    ProfileField::Password,
    ProfileField::PasswordStorage,
    ProfileField::SslMode,
    ProfileField::CaCertPath,
    ProfileField::Environment,
    ProfileField::ReadOnly,
    ProfileField::Url,
    ProfileField::Test,
    ProfileField::Save,
    ProfileField::Cancel,
];

const ELASTICSEARCH_API_KEY_FIELDS: [ProfileField; 18] = [
    ProfileField::DatabaseCategory,
    ProfileField::Kind,
    ProfileField::Name,
    ProfileField::Host,
    ProfileField::Port,
    ProfileField::Database,
    ProfileField::VisibleObjects,
    ProfileField::ElasticsearchAuth,
    ProfileField::Password,
    ProfileField::PasswordStorage,
    ProfileField::SslMode,
    ProfileField::CaCertPath,
    ProfileField::Environment,
    ProfileField::ReadOnly,
    ProfileField::Url,
    ProfileField::Test,
    ProfileField::Save,
    ProfileField::Cancel,
];

const MYSQL_FIELDS: [ProfileField; 17] = [
    ProfileField::DatabaseCategory,
    ProfileField::Kind,
    ProfileField::Name,
    ProfileField::Host,
    ProfileField::Port,
    ProfileField::Database,
    ProfileField::VisibleObjects,
    ProfileField::User,
    ProfileField::Password,
    ProfileField::PasswordStorage,
    ProfileField::SslMode,
    ProfileField::Environment,
    ProfileField::ReadOnly,
    ProfileField::Url,
    ProfileField::Test,
    ProfileField::Save,
    ProfileField::Cancel,
];

const SQLITE_FILE_FIELDS: [ProfileField; 11] = [
    ProfileField::DatabaseCategory,
    ProfileField::Kind,
    ProfileField::Name,
    ProfileField::SqliteMemory,
    ProfileField::SqlitePath,
    ProfileField::VisibleObjects,
    ProfileField::ReadOnly,
    ProfileField::Url,
    ProfileField::Test,
    ProfileField::Save,
    ProfileField::Cancel,
];

const SQLITE_MEMORY_FIELDS: [ProfileField; 10] = [
    ProfileField::DatabaseCategory,
    ProfileField::Kind,
    ProfileField::Name,
    ProfileField::SqliteMemory,
    ProfileField::VisibleObjects,
    ProfileField::ReadOnly,
    ProfileField::Url,
    ProfileField::Test,
    ProfileField::Save,
    ProfileField::Cancel,
];

const DUCKDB_FIELDS: [ProfileField; 11] = [
    ProfileField::DatabaseCategory,
    ProfileField::Kind,
    ProfileField::Name,
    ProfileField::DuckDbPath,
    ProfileField::Schema,
    ProfileField::VisibleObjects,
    ProfileField::ReadOnly,
    ProfileField::Url,
    ProfileField::Test,
    ProfileField::Save,
    ProfileField::Cancel,
];

const REDIS_FIELDS: [ProfileField; 16] = [
    ProfileField::DatabaseCategory,
    ProfileField::Kind,
    ProfileField::Name,
    ProfileField::Host,
    ProfileField::Port,
    ProfileField::Database,
    ProfileField::User,
    ProfileField::Password,
    ProfileField::PasswordStorage,
    ProfileField::SslMode,
    ProfileField::Environment,
    ProfileField::ReadOnly,
    ProfileField::Url,
    ProfileField::Test,
    ProfileField::Save,
    ProfileField::Cancel,
];

const MONGODB_FIELDS: [ProfileField; 15] = [
    ProfileField::DatabaseCategory,
    ProfileField::Kind,
    ProfileField::Name,
    ProfileField::Host,
    ProfileField::Port,
    ProfileField::Database,
    ProfileField::User,
    ProfileField::Password,
    ProfileField::PasswordStorage,
    ProfileField::SslMode,
    ProfileField::Environment,
    ProfileField::ReadOnly,
    ProfileField::Url,
    ProfileField::Test,
    ProfileField::Cancel,
];

const PASSWORD_STORAGE_CHOICES: [PasswordStorageChoice; 2] = [
    PasswordStorageChoice::LocalEncrypted,
    PasswordStorageChoice::System,
];
const PASSWORD_STORAGE_LOCAL_ONLY: [PasswordStorageChoice; 1] =
    [PasswordStorageChoice::LocalEncrypted];
