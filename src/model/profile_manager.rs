use std::{fmt, path::PathBuf};

use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;

use crate::profile::{ConnectionProfile, DatabaseKind, Environment, SslMode};

use super::text_input::TextInput;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileManagerPage {
    List,
    Form,
    ConfirmDelete,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ProfileField {
    Kind,
    Name,
    Host,
    Port,
    User,
    Password,
    Database,
    Schema,
    SslMode,
    Environment,
    ReadOnly,
    RememberPassword,
    SqliteMemory,
    SqlitePath,
    Test,
    Save,
    SaveAndConnect,
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

#[derive(Clone)]
pub enum CredentialUpdate {
    Preserve,
    Session(SecretString),
    Remember(SecretString),
    Forget,
}

impl fmt::Debug for CredentialUpdate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Preserve => formatter.write_str("Preserve"),
            Self::Session(_) => formatter.write_str("Session([REDACTED])"),
            Self::Remember(_) => formatter.write_str("Remember([REDACTED])"),
            Self::Forget => formatter.write_str("Forget"),
        }
    }
}

#[derive(Clone)]
pub struct ProfileSubmission {
    pub profile: ConnectionProfile,
    pub credential: CredentialUpdate,
}

impl fmt::Debug for ProfileSubmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProfileSubmission")
            .field("profile", &self.profile)
            .field("credential", &self.credential)
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

impl fmt::Display for ProfileValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProfileValidationError {}

#[derive(Clone)]
pub struct ProfileDraft {
    profile_id: Uuid,
    pub kind: DatabaseKind,
    pub name: TextInput,
    pub host: TextInput,
    pub port: TextInput,
    pub user: TextInput,
    password: SecretString,
    pub database: TextInput,
    pub schema: TextInput,
    pub ssl_mode: SslMode,
    pub environment: Environment,
    pub read_only: bool,
    pub remember_password: bool,
    pub sqlite_memory: bool,
    pub sqlite_path: TextInput,
    pub original_secret_ref: Option<String>,
    pub has_stored_credential: bool,
    include_databases: Vec<String>,
    include_schemas: Vec<String>,
}

impl ProfileDraft {
    pub fn new(kind: DatabaseKind) -> Self {
        let (host, port, schema, ssl_mode) = match kind {
            DatabaseKind::Postgres => ("localhost", "5432", "public", SslMode::Prefer),
            DatabaseKind::MySql => ("localhost", "3306", "", SslMode::Prefer),
            DatabaseKind::Sqlite => ("", "", "main", SslMode::Disable),
        };

        Self {
            profile_id: Uuid::new_v4(),
            kind,
            name: TextInput::default(),
            host: TextInput::from(host),
            port: TextInput::from(port),
            user: TextInput::default(),
            password: SecretString::from(String::new()),
            database: TextInput::default(),
            schema: TextInput::from(schema),
            ssl_mode,
            environment: Environment::Development,
            read_only: false,
            remember_password: false,
            sqlite_memory: false,
            sqlite_path: TextInput::default(),
            original_secret_ref: None,
            has_stored_credential: false,
            include_databases: Vec::new(),
            include_schemas: Vec::new(),
        }
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

        Self {
            profile_id: profile.id,
            kind: profile.kind,
            name: TextInput::from(profile.name.clone()),
            host: TextInput::from(profile.host.clone().unwrap_or_default()),
            port: TextInput::from(
                profile
                    .port
                    .map(|port| port.to_string())
                    .unwrap_or_default(),
            ),
            user: TextInput::from(profile.user.clone().unwrap_or_default()),
            password: SecretString::from(String::new()),
            database: TextInput::from(profile.database.clone().unwrap_or_default()),
            schema: TextInput::from(profile.default_schema.clone().unwrap_or_default()),
            ssl_mode: profile.ssl_mode,
            environment: profile.environment,
            read_only: profile.read_only,
            remember_password: profile.secret_ref.is_some(),
            sqlite_memory,
            sqlite_path: TextInput::from(sqlite_path),
            original_secret_ref: profile.secret_ref.clone(),
            has_stored_credential,
            include_databases: profile.include_databases.clone(),
            include_schemas: profile.include_schemas.clone(),
        }
    }

    pub fn profile_id(&self) -> Uuid {
        self.profile_id
    }

    pub fn password(&self) -> &SecretString {
        &self.password
    }

    pub fn set_password(&mut self, password: impl Into<String>) {
        self.password = SecretString::from(password.into());
    }

    pub fn password_len(&self) -> usize {
        self.password.expose_secret().chars().count()
    }

    pub fn visible_fields(&self) -> &'static [ProfileField] {
        match (self.kind, self.sqlite_memory) {
            (DatabaseKind::Postgres | DatabaseKind::MySql, _) => &SERVER_FIELDS,
            (DatabaseKind::Sqlite, false) => &SQLITE_FILE_FIELDS,
            (DatabaseKind::Sqlite, true) => &SQLITE_MEMORY_FIELDS,
        }
    }

    pub fn validate(
        &self,
        profiles: &[ConnectionProfile],
    ) -> Result<ProfileSubmission, ProfileValidationError> {
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

        let (host, port, user, database, default_schema, sqlite_path, ssl_mode) = match self.kind {
            DatabaseKind::Postgres | DatabaseKind::MySql => {
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
                    optional(&self.schema),
                    None,
                    self.ssl_mode,
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
                )
            }
        };

        let credential = self.credential_update();
        let secret_ref = if matches!(credential, CredentialUpdate::Preserve) {
            self.original_secret_ref.clone()
        } else {
            None
        };

        Ok(ProfileSubmission {
            profile: ConnectionProfile {
                id: self.profile_id,
                name,
                kind: self.kind,
                host,
                port,
                user,
                database,
                default_schema,
                sqlite_path,
                ssl_mode,
                secret_ref,
                read_only: self.read_only,
                environment: self.environment,
                include_databases: self.include_databases.clone(),
                include_schemas: self.include_schemas.clone(),
            },
            credential,
        })
    }

    fn credential_update(&self) -> CredentialUpdate {
        if self.kind == DatabaseKind::Sqlite {
            return if self.original_secret_ref.is_some() {
                CredentialUpdate::Forget
            } else {
                CredentialUpdate::Preserve
            };
        }

        if !self.password.expose_secret().is_empty() {
            return if self.remember_password {
                CredentialUpdate::Remember(self.password.clone())
            } else {
                CredentialUpdate::Session(self.password.clone())
            };
        }

        if self.original_secret_ref.is_some() && !self.remember_password {
            CredentialUpdate::Forget
        } else {
            CredentialUpdate::Preserve
        }
    }
}

impl fmt::Debug for ProfileDraft {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProfileDraft")
            .field("profile_id", &self.profile_id)
            .field("kind", &self.kind)
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
            .field("remember_password", &self.remember_password)
            .field("sqlite_memory", &self.sqlite_memory)
            .field("sqlite_path", &self.sqlite_path)
            .field("original_secret_ref", &self.original_secret_ref)
            .field("has_stored_credential", &self.has_stored_credential)
            .field("include_databases", &self.include_databases)
            .field("include_schemas", &self.include_schemas)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct ProfileManagerState {
    pub page: ProfileManagerPage,
    pub selected: usize,
    pub draft: Option<ProfileDraft>,
    pub selected_field: ProfileField,
    pub operation: Option<ProfileOperation>,
    pub message: Option<String>,
    pub request_generation: u64,
    pub opened_automatically: bool,
}

impl ProfileManagerState {
    pub fn new(opened_automatically: bool) -> Self {
        Self {
            page: ProfileManagerPage::List,
            selected: 0,
            draft: None,
            selected_field: ProfileField::Kind,
            operation: None,
            message: None,
            request_generation: 0,
            opened_automatically,
        }
    }

    pub fn start_new(&mut self, kind: DatabaseKind) {
        self.page = ProfileManagerPage::Form;
        self.draft = Some(ProfileDraft::new(kind));
        self.selected_field = ProfileField::Kind;
        self.operation = None;
        self.message = None;
    }

    pub fn start_edit(&mut self, profile: &ConnectionProfile, has_stored_credential: bool) {
        self.page = ProfileManagerPage::Form;
        self.draft = Some(ProfileDraft::edit(profile, has_stored_credential));
        self.selected_field = ProfileField::Kind;
        self.operation = None;
        self.message = None;
    }

    pub fn visible_fields(&self) -> &'static [ProfileField] {
        self.draft
            .as_ref()
            .map_or(&[], ProfileDraft::visible_fields)
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

fn optional(input: &TextInput) -> Option<String> {
    let value = input.value().trim();
    (!value.is_empty()).then(|| value.to_owned())
}

const SERVER_FIELDS: [ProfileField; 16] = [
    ProfileField::Kind,
    ProfileField::Name,
    ProfileField::Host,
    ProfileField::Port,
    ProfileField::User,
    ProfileField::Password,
    ProfileField::Database,
    ProfileField::Schema,
    ProfileField::SslMode,
    ProfileField::Environment,
    ProfileField::ReadOnly,
    ProfileField::RememberPassword,
    ProfileField::Test,
    ProfileField::Save,
    ProfileField::SaveAndConnect,
    ProfileField::Cancel,
];

const SQLITE_FILE_FIELDS: [ProfileField; 9] = [
    ProfileField::Kind,
    ProfileField::Name,
    ProfileField::SqliteMemory,
    ProfileField::SqlitePath,
    ProfileField::ReadOnly,
    ProfileField::Test,
    ProfileField::Save,
    ProfileField::SaveAndConnect,
    ProfileField::Cancel,
];

const SQLITE_MEMORY_FIELDS: [ProfileField; 8] = [
    ProfileField::Kind,
    ProfileField::Name,
    ProfileField::SqliteMemory,
    ProfileField::ReadOnly,
    ProfileField::Test,
    ProfileField::Save,
    ProfileField::SaveAndConnect,
    ProfileField::Cancel,
];
