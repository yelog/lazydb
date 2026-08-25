use std::{fmt, path::PathBuf};

use secrecy::{ExposeSecret, SecretString, zeroize::Zeroizing};
use uuid::Uuid;

use crate::profile::{ConnectionProfile, DatabaseKind, Environment, SslMode};

use super::text_input::TextInput;

#[derive(Clone, Default)]
pub struct ProfileInput(SecretString);

impl ProfileInput {
    pub(crate) fn value(&self) -> &str {
        self.0.expose_secret()
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
    password_cursor: usize,
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
            password_cursor: 0,
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
            password_cursor: 0,
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
        let password = password.into();
        self.password_cursor = password.chars().count();
        self.password = SecretString::from(password);
    }

    pub fn password_len(&self) -> usize {
        self.password.expose_secret().chars().count()
    }

    pub fn insert(&mut self, field: ProfileField, character: char) {
        if field == ProfileField::Password {
            let mut password = Zeroizing::new(self.password.expose_secret().to_owned());
            let byte_index = character_byte_index(&password, self.password_cursor);
            password.insert(byte_index, character);
            self.password_cursor += 1;
            self.password = SecretString::from(std::mem::take(&mut *password));
        } else if let Some(input) = self.text_input_mut(field) {
            input.insert(character);
        }
    }

    pub fn paste(&mut self, field: ProfileField, text: &str) {
        if field == ProfileField::Password {
            let mut password = Zeroizing::new(self.password.expose_secret().to_owned());
            let byte_index = character_byte_index(&password, self.password_cursor);
            password.insert_str(byte_index, text);
            self.password_cursor += text.chars().count();
            self.password = SecretString::from(std::mem::take(&mut *password));
        } else if let Some(input) = self.text_input_mut(field) {
            input.paste(text);
        }
    }

    pub fn backspace(&mut self, field: ProfileField) {
        if field == ProfileField::Password {
            if self.password_cursor == 0 {
                return;
            }
            let mut password = Zeroizing::new(self.password.expose_secret().to_owned());
            let start = character_byte_index(&password, self.password_cursor - 1);
            let end = character_byte_index(&password, self.password_cursor);
            password.replace_range(start..end, "");
            self.password_cursor -= 1;
            self.password = SecretString::from(std::mem::take(&mut *password));
        } else if let Some(input) = self.text_input_mut(field) {
            input.backspace();
        }
    }

    pub fn delete(&mut self, field: ProfileField) {
        if field == ProfileField::Password {
            let mut password = Zeroizing::new(self.password.expose_secret().to_owned());
            let start = character_byte_index(&password, self.password_cursor);
            if start == password.len() {
                return;
            }
            let end = character_byte_index(&password, self.password_cursor + 1);
            password.replace_range(start..end, "");
            self.password = SecretString::from(std::mem::take(&mut *password));
        } else if let Some(input) = self.text_input_mut(field) {
            input.delete();
        }
    }

    pub fn move_left(&mut self, field: ProfileField) {
        if field == ProfileField::Password {
            self.password_cursor = self.password_cursor.saturating_sub(1);
        } else if let Some(input) = self.text_input_mut(field) {
            input.move_left();
        }
    }

    pub fn move_right(&mut self, field: ProfileField) {
        if field == ProfileField::Password {
            self.password_cursor = (self.password_cursor + 1).min(self.password_len());
        } else if let Some(input) = self.text_input_mut(field) {
            input.move_right();
        }
    }

    pub fn move_home(&mut self, field: ProfileField) {
        if field == ProfileField::Password {
            self.password_cursor = 0;
        } else if let Some(input) = self.text_input_mut(field) {
            input.move_home();
        }
    }

    pub fn move_end(&mut self, field: ProfileField) {
        if field == ProfileField::Password {
            self.password_cursor = self.password_len();
        } else if let Some(input) = self.text_input_mut(field) {
            input.move_end();
        }
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

    fn text_input_mut(&mut self, field: ProfileField) -> Option<&mut TextInput> {
        match field {
            ProfileField::Name => Some(&mut self.name),
            ProfileField::Host => Some(&mut self.host),
            ProfileField::Port => Some(&mut self.port),
            ProfileField::User => Some(&mut self.user),
            ProfileField::Database => Some(&mut self.database),
            ProfileField::Schema => Some(&mut self.schema),
            ProfileField::SqlitePath => Some(&mut self.sqlite_path),
            _ => None,
        }
    }

    fn set_kind(&mut self, kind: DatabaseKind) {
        if self.kind == kind {
            return;
        }

        let previous = self.kind;
        self.kind = kind;
        match kind {
            DatabaseKind::Postgres => {
                if self.host.value().trim().is_empty() {
                    self.host.set("localhost");
                }
                if self.port.value().trim().is_empty()
                    || (previous == DatabaseKind::MySql && self.port.value() == "3306")
                {
                    self.port.set("5432");
                }
                if self.schema.value().trim().is_empty() || self.schema.value() == "main" {
                    self.schema.set("public");
                }
                if previous == DatabaseKind::Sqlite && self.ssl_mode == SslMode::Disable {
                    self.ssl_mode = SslMode::Prefer;
                }
            }
            DatabaseKind::MySql => {
                if self.host.value().trim().is_empty() {
                    self.host.set("localhost");
                }
                if self.port.value().trim().is_empty()
                    || (previous == DatabaseKind::Postgres && self.port.value() == "5432")
                {
                    self.port.set("3306");
                }
                if self.schema.value() == "public" || self.schema.value() == "main" {
                    self.schema.set("");
                }
                if previous == DatabaseKind::Sqlite && self.ssl_mode == SslMode::Disable {
                    self.ssl_mode = SslMode::Prefer;
                }
            }
            DatabaseKind::Sqlite => self.ssl_mode = SslMode::Disable,
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

    pub fn move_selection(&mut self, delta: isize, profile_count: usize) {
        self.selected = move_bounded(self.selected, delta, profile_count);
    }

    pub fn move_field(&mut self, delta: isize) {
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

    pub fn cycle(&mut self, delta: i8) {
        let field = self.selected_field;
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        match field {
            ProfileField::Kind => draft.set_kind(cycle_value(
                draft.kind,
                &[
                    DatabaseKind::Postgres,
                    DatabaseKind::MySql,
                    DatabaseKind::Sqlite,
                ],
                delta,
            )),
            ProfileField::SslMode => {
                draft.ssl_mode = cycle_value(
                    draft.ssl_mode,
                    &[
                        SslMode::Disable,
                        SslMode::Prefer,
                        SslMode::Require,
                        SslMode::VerifyCa,
                        SslMode::VerifyFull,
                    ],
                    delta,
                );
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
            _ => return,
        }
        self.message = None;
    }

    pub fn toggle(&mut self) {
        let field = self.selected_field;
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        match field {
            ProfileField::ReadOnly => draft.read_only = !draft.read_only,
            ProfileField::RememberPassword => {
                draft.remember_password = !draft.remember_password;
            }
            ProfileField::SqliteMemory => draft.sqlite_memory = !draft.sqlite_memory,
            _ => return,
        }
        self.message = None;
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

fn character_byte_index(value: &str, character_index: usize) -> usize {
    value
        .char_indices()
        .nth(character_index)
        .map_or(value.len(), |(byte_index, _)| byte_index)
}

fn cycle_value<T: Copy + PartialEq>(current: T, values: &[T], delta: i8) -> T {
    let current = values
        .iter()
        .position(|value| *value == current)
        .unwrap_or(0);
    let next = (current as isize + isize::from(delta)).rem_euclid(values.len() as isize) as usize;
    values[next]
}

fn move_bounded(current: usize, delta: isize, count: usize) -> usize {
    if count == 0 {
        0
    } else {
        current
            .saturating_add_signed(delta)
            .min(count.saturating_sub(1))
    }
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
