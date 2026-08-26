use std::{collections::HashSet, path::PathBuf};

use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseKind {
    Postgres,
    MySql,
    Sqlite,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionUrlFormat {
    Postgres,
    #[default]
    PostgreSql,
    JdbcPostgreSql,
    MySql,
    JdbcMySql,
    Sqlite,
    FileUri,
    JdbcSqlite,
}

impl ConnectionUrlFormat {
    pub const fn default_for(kind: DatabaseKind) -> Self {
        match kind {
            DatabaseKind::Postgres => Self::PostgreSql,
            DatabaseKind::MySql => Self::MySql,
            DatabaseKind::Sqlite => Self::Sqlite,
        }
    }

    pub const fn is_compatible(self, kind: DatabaseKind) -> bool {
        matches!(
            (self, kind),
            (
                Self::Postgres | Self::PostgreSql | Self::JdbcPostgreSql,
                DatabaseKind::Postgres
            ) | (Self::MySql | Self::JdbcMySql, DatabaseKind::MySql)
                | (
                    Self::Sqlite | Self::FileUri | Self::JdbcSqlite,
                    DatabaseKind::Sqlite
                )
        )
    }

    pub const fn compatible_formats(kind: DatabaseKind) -> &'static [Self] {
        match kind {
            DatabaseKind::Postgres => &[Self::Postgres, Self::PostgreSql, Self::JdbcPostgreSql],
            DatabaseKind::MySql => &[Self::MySql, Self::JdbcMySql],
            DatabaseKind::Sqlite => &[Self::Sqlite, Self::FileUri, Self::JdbcSqlite],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct CatalogScope {
    pub databases: CatalogSelection<DatabaseScope>,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct DatabaseScope {
    pub name: String,
    pub schemas: CatalogSelection<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(tag = "mode", content = "items", rename_all = "snake_case")]
pub enum CatalogSelection<T> {
    All,
    Selected(Vec<T>),
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CatalogScopeValidationError {
    #[error("catalog scope must select at least one database")]
    EmptyDatabaseSelection,
    #[error("catalog scope contains an empty database name")]
    EmptyDatabaseName,
    #[error("catalog scope contains duplicate database name `{0}`")]
    DuplicateDatabase(String),
    #[error("catalog scope for database `{database}` must select at least one schema")]
    EmptySchemaSelection { database: String },
    #[error("catalog scope for database `{database}` contains an empty schema name")]
    EmptySchemaName { database: String },
    #[error("catalog scope for database `{database}` contains duplicate schema name `{schema}`")]
    DuplicateSchema { database: String, schema: String },
    #[error("default schema `{schema}` for database `{database}` is excluded by catalog scope")]
    DefaultSchemaExcluded { database: String, schema: String },
}

impl CatalogScope {
    pub fn for_profile(kind: DatabaseKind, database: &str, default_schema: Option<&str>) -> Self {
        let schemas = match (kind, default_schema) {
            (DatabaseKind::MySql, _) | (_, None) => CatalogSelection::All,
            (_, Some(schema)) => CatalogSelection::Selected(vec![schema.to_owned()]),
        };

        Self {
            databases: CatalogSelection::Selected(vec![DatabaseScope {
                name: database.to_owned(),
                schemas,
            }]),
        }
    }

    pub fn validate(
        &self,
        database: &str,
        default_schema: Option<&str>,
    ) -> Result<(), CatalogScopeValidationError> {
        if let CatalogSelection::Selected(databases) = &self.databases {
            if databases.is_empty() {
                return Err(CatalogScopeValidationError::EmptyDatabaseSelection);
            }

            let mut database_names = HashSet::new();
            for database_scope in databases {
                if database_scope.name.is_empty() {
                    return Err(CatalogScopeValidationError::EmptyDatabaseName);
                }
                if !database_names.insert(database_scope.name.as_str()) {
                    return Err(CatalogScopeValidationError::DuplicateDatabase(
                        database_scope.name.clone(),
                    ));
                }

                if let CatalogSelection::Selected(schemas) = &database_scope.schemas {
                    if schemas.is_empty() {
                        return Err(CatalogScopeValidationError::EmptySchemaSelection {
                            database: database_scope.name.clone(),
                        });
                    }

                    let mut schema_names = HashSet::new();
                    for schema in schemas {
                        if schema.is_empty() {
                            return Err(CatalogScopeValidationError::EmptySchemaName {
                                database: database_scope.name.clone(),
                            });
                        }
                        if !schema_names.insert(schema.as_str()) {
                            return Err(CatalogScopeValidationError::DuplicateSchema {
                                database: database_scope.name.clone(),
                                schema: schema.clone(),
                            });
                        }
                    }
                }
            }
        }

        if let Some(schema) = default_schema
            && !self.allows_schema(database, schema)
        {
            return Err(CatalogScopeValidationError::DefaultSchemaExcluded {
                database: database.to_owned(),
                schema: schema.to_owned(),
            });
        }

        Ok(())
    }

    pub fn allows_database(&self, database: &str) -> bool {
        match &self.databases {
            CatalogSelection::All => true,
            CatalogSelection::Selected(databases) => databases
                .iter()
                .any(|database_scope| database_scope.name == database),
        }
    }

    pub fn allows_schema(&self, database: &str, schema: &str) -> bool {
        match &self.databases {
            CatalogSelection::All => true,
            CatalogSelection::Selected(databases) => databases
                .iter()
                .find(|database_scope| database_scope.name == database)
                .is_some_and(|database_scope| match &database_scope.schemas {
                    CatalogSelection::All => true,
                    CatalogSelection::Selected(schemas) => {
                        schemas.iter().any(|selected| selected == schema)
                    }
                }),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SslMode {
    Disable,
    #[default]
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    #[default]
    Development,
    Staging,
    Production,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "policy", content = "reference", rename_all = "lowercase")]
pub enum CredentialPolicy {
    #[default]
    None,
    Prompt,
    Keyring(String),
}

impl CredentialPolicy {
    pub fn keyring_reference(&self) -> Option<&str> {
        match self {
            Self::Keyring(reference) => Some(reference),
            Self::None | Self::Prompt => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionProfile {
    pub id: Uuid,
    pub name: String,
    pub kind: DatabaseKind,
    #[serde(default)]
    pub url_format: ConnectionUrlFormat,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub database: Option<String>,
    pub default_schema: Option<String>,
    pub sqlite_path: Option<PathBuf>,
    #[serde(default)]
    pub ssl_mode: SslMode,
    #[serde(default)]
    pub credential_policy: CredentialPolicy,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub environment: Environment,
    pub catalog_scope: CatalogScope,
}

#[derive(Debug)]
pub struct ImportedProfile {
    pub profile: ConnectionProfile,
    pub transient_password: Option<SecretString>,
}

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("connection URL cannot be empty")]
    Empty,
    #[error("invalid connection URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("unsupported database scheme: {0}")]
    UnsupportedScheme(String),
    #[error("connection URL is missing a host")]
    MissingHost,
    #[error("SQLite URL is missing a database path")]
    MissingSqlitePath,
    #[error("connection URL contains unknown query parameter `{0}`")]
    UnknownQueryParameter(String),
    #[error("connection URL contains conflicting query parameter `{0}`")]
    ConflictingQueryParameter(String),
    #[error("connection URL has an invalid value for query parameter `{0}`")]
    InvalidQueryParameter(String),
    #[error("connection URL format is incompatible with the database driver")]
    IncompatibleFormat,
}

#[derive(Debug)]
pub struct ParsedConnectionUrl {
    pub kind: DatabaseKind,
    pub format: ConnectionUrlFormat,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub password: Option<SecretString>,
    pub database: Option<String>,
    pub default_schema: Option<String>,
    pub sqlite_path: Option<PathBuf>,
    pub sqlite_memory: bool,
    pub ssl_mode: SslMode,
    pub read_only: bool,
}

pub fn parse_connection_url(input: &str) -> Result<ParsedConnectionUrl, ProfileError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(ProfileError::Empty);
    }
    let (normalized, jdbc) = input
        .strip_prefix("jdbc:")
        .map_or((input, false), |value| (value, true));
    if normalized == ":memory:"
        || normalized == "sqlite::memory:"
        || normalized.starts_with("sqlite::memory:?")
        || normalized == "file::memory:"
        || normalized.starts_with("file::memory:?")
    {
        let format = if jdbc {
            ConnectionUrlFormat::JdbcSqlite
        } else if normalized.starts_with("file:") {
            ConnectionUrlFormat::FileUri
        } else {
            ConnectionUrlFormat::Sqlite
        };
        let query = normalized.split_once('?').map_or("", |(_, query)| query);
        let read_only = parse_sqlite_query(query)?;
        return Ok(ParsedConnectionUrl {
            kind: DatabaseKind::Sqlite,
            format,
            host: None,
            port: None,
            user: None,
            password: None,
            database: Some(":memory:".to_owned()),
            default_schema: Some("main".to_owned()),
            sqlite_path: None,
            sqlite_memory: true,
            ssl_mode: SslMode::Disable,
            read_only,
        });
    }
    if normalized.starts_with("sqlite:") || normalized.starts_with("file:") {
        return parse_sqlite_url(normalized, jdbc);
    }

    let url = Url::parse(normalized)?;
    let (kind, format) = match (url.scheme().to_ascii_lowercase().as_str(), jdbc) {
        ("postgres", false) => (DatabaseKind::Postgres, ConnectionUrlFormat::Postgres),
        ("postgresql", false) => (DatabaseKind::Postgres, ConnectionUrlFormat::PostgreSql),
        ("postgresql", true) => (DatabaseKind::Postgres, ConnectionUrlFormat::JdbcPostgreSql),
        ("mysql", false) => (DatabaseKind::MySql, ConnectionUrlFormat::MySql),
        ("mysql", true) => (DatabaseKind::MySql, ConnectionUrlFormat::JdbcMySql),
        (scheme, _) => return Err(ProfileError::UnsupportedScheme(scheme.to_owned())),
    };
    parse_server_url(url, kind, format)
}

pub fn import_connection_url(
    input: &str,
    preferred_name: Option<&str>,
) -> Result<ImportedProfile, ProfileError> {
    let parsed = parse_connection_url(input)?;
    let derived_name = if parsed.sqlite_memory {
        "memory".to_owned()
    } else if let Some(path) = &parsed.sqlite_path {
        path.file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("sqlite")
            .to_owned()
    } else {
        parsed
            .database
            .clone()
            .or_else(|| parsed.host.clone())
            .unwrap_or_else(|| "connection".to_owned())
    };
    let catalog_scope = CatalogScope::for_profile(
        parsed.kind,
        parsed.database.as_deref().unwrap_or_default(),
        parsed.default_schema.as_deref(),
    );
    Ok(ImportedProfile {
        profile: ConnectionProfile {
            id: Uuid::new_v4(),
            name: choose_name(preferred_name, &derived_name),
            kind: parsed.kind,
            url_format: parsed.format,
            host: parsed.host,
            port: parsed.port,
            user: parsed.user,
            database: parsed.database,
            default_schema: parsed.default_schema,
            sqlite_path: parsed.sqlite_path,
            ssl_mode: parsed.ssl_mode,
            credential_policy: CredentialPolicy::None,
            read_only: parsed.read_only,
            environment: Environment::Development,
            catalog_scope,
        },
        transient_password: parsed.password,
    })
}

fn parse_server_url(
    url: Url,
    kind: DatabaseKind,
    format: ConnectionUrlFormat,
) -> Result<ParsedConnectionUrl, ProfileError> {
    let host = url.host_str().ok_or(ProfileError::MissingHost)?.to_owned();
    let database = decode(url.path().trim_start_matches('/'));
    let database = (!database.is_empty()).then_some(database);
    let user = (!url.username().is_empty()).then(|| decode(url.username()));
    let password = url
        .password()
        .map(|value| SecretString::from(decode(value)));
    let mut default_schema = None;
    let mut ssl_mode = SslMode::Prefer;
    let mut read_only = false;
    let mut seen_schema = false;
    let mut seen_ssl = false;
    let mut seen_read_only = false;

    for (key, value) in url.query_pairs() {
        if key.eq_ignore_ascii_case("currentSchema") && kind == DatabaseKind::Postgres {
            reject_duplicate(&mut seen_schema, "currentSchema")?;
            default_schema = Some(value.into_owned());
        } else if key.eq_ignore_ascii_case("sslmode") && kind == DatabaseKind::Postgres {
            reject_duplicate(&mut seen_ssl, "ssl")?;
            ssl_mode = parse_ssl_mode(&value)
                .ok_or_else(|| ProfileError::InvalidQueryParameter("sslmode".into()))?;
        } else if key.eq_ignore_ascii_case("useSSL") && kind == DatabaseKind::MySql {
            reject_duplicate(&mut seen_ssl, "ssl")?;
            ssl_mode = if parse_bool(&value, "useSSL")? {
                SslMode::Require
            } else {
                SslMode::Disable
            };
        } else if key.eq_ignore_ascii_case("sslMode") && kind == DatabaseKind::MySql {
            reject_duplicate(&mut seen_ssl, "ssl")?;
            ssl_mode = parse_ssl_mode(&value)
                .ok_or_else(|| ProfileError::InvalidQueryParameter("sslMode".into()))?;
        } else if key.eq_ignore_ascii_case("readOnly") || key.eq_ignore_ascii_case("read_only") {
            reject_duplicate(&mut seen_read_only, "readOnly")?;
            read_only = parse_bool(&value, "readOnly")?;
        } else {
            return Err(ProfileError::UnknownQueryParameter(key.into_owned()));
        }
    }

    let default_port = match kind {
        DatabaseKind::Postgres => 5432,
        DatabaseKind::MySql => 3306,
        DatabaseKind::Sqlite => unreachable!("server URL cannot be SQLite"),
    };
    Ok(ParsedConnectionUrl {
        kind,
        format,
        host: Some(host),
        port: Some(url.port().unwrap_or(default_port)),
        user,
        password,
        database,
        default_schema,
        sqlite_path: None,
        sqlite_memory: false,
        ssl_mode,
        read_only,
    })
}

fn parse_sqlite_url(input: &str, jdbc: bool) -> Result<ParsedConnectionUrl, ProfileError> {
    let (path_and_prefix, query) = input.split_once('?').unwrap_or((input, ""));
    let raw_path = if let Some(rest) = path_and_prefix.strip_prefix("sqlite:") {
        rest.strip_prefix("//").unwrap_or(rest)
    } else {
        let rest = path_and_prefix
            .strip_prefix("file:")
            .unwrap_or(path_and_prefix);
        rest.strip_prefix("//").unwrap_or(rest)
    };
    let decoded = decode(raw_path);
    if decoded.is_empty() {
        return Err(ProfileError::MissingSqlitePath);
    }
    let read_only = parse_sqlite_query(query)?;
    let path = PathBuf::from(decoded);
    let database = path.to_string_lossy().into_owned();
    Ok(ParsedConnectionUrl {
        kind: DatabaseKind::Sqlite,
        format: if jdbc {
            ConnectionUrlFormat::JdbcSqlite
        } else if input.starts_with("file:") {
            ConnectionUrlFormat::FileUri
        } else {
            ConnectionUrlFormat::Sqlite
        },
        host: None,
        port: None,
        user: None,
        password: None,
        database: Some(database),
        default_schema: Some("main".to_owned()),
        sqlite_path: Some(path),
        sqlite_memory: false,
        ssl_mode: SslMode::Disable,
        read_only,
    })
}

const URL_COMPONENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b':')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

const SQLITE_PATH: &AsciiSet = &URL_COMPONENT.remove(b'/').remove(b':');
const QUERY_VALUE: &AsciiSet = &URL_COMPONENT.add(b'&').add(b'=').add(b'+');

pub fn format_connection_url(
    profile: &ConnectionProfile,
    format: ConnectionUrlFormat,
) -> Result<String, ProfileError> {
    if !format.is_compatible(profile.kind) {
        return Err(ProfileError::IncompatibleFormat);
    }
    if profile.kind == DatabaseKind::Sqlite {
        let memory =
            profile.sqlite_path.is_none() && profile.database.as_deref() == Some(":memory:");
        let value = if memory {
            ":memory:".to_owned()
        } else {
            let path = profile
                .sqlite_path
                .as_ref()
                .ok_or(ProfileError::MissingSqlitePath)?;
            utf8_percent_encode(&path.to_string_lossy(), SQLITE_PATH).to_string()
        };
        let prefix = match format {
            ConnectionUrlFormat::Sqlite => "sqlite:",
            ConnectionUrlFormat::FileUri => "file:",
            ConnectionUrlFormat::JdbcSqlite => "jdbc:sqlite:",
            _ => return Err(ProfileError::IncompatibleFormat),
        };
        let mut output = format!("{prefix}{value}");
        if profile.read_only {
            output.push_str("?mode=ro");
        }
        return Ok(output);
    }
    let scheme = match format {
        ConnectionUrlFormat::Postgres => "postgres",
        ConnectionUrlFormat::PostgreSql | ConnectionUrlFormat::JdbcPostgreSql => "postgresql",
        ConnectionUrlFormat::MySql | ConnectionUrlFormat::JdbcMySql => "mysql",
        _ => return Err(ProfileError::IncompatibleFormat),
    };
    let host = profile.host.as_deref().ok_or(ProfileError::MissingHost)?;
    let mut output = String::new();
    if matches!(
        format,
        ConnectionUrlFormat::JdbcPostgreSql | ConnectionUrlFormat::JdbcMySql
    ) {
        output.push_str("jdbc:");
    }
    output.push_str(scheme);
    output.push_str("://");
    if let Some(user) = profile.user.as_deref().filter(|value| !value.is_empty()) {
        output.push_str(&utf8_percent_encode(user, URL_COMPONENT).to_string());
        output.push('@');
    }
    if host.contains(':') && !host.starts_with('[') {
        output.push('[');
        output.push_str(host);
        output.push(']');
    } else {
        output.push_str(host);
    }
    if let Some(port) = profile.port {
        output.push(':');
        output.push_str(&port.to_string());
    }
    if let Some(database) = profile
        .database
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        output.push('/');
        output.push_str(&utf8_percent_encode(database, URL_COMPONENT).to_string());
    }
    let mut query = Vec::new();
    if profile.kind == DatabaseKind::Postgres {
        if let Some(schema) = profile
            .default_schema
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            query.push(format!(
                "currentSchema={}",
                utf8_percent_encode(schema, QUERY_VALUE)
            ));
        }
        query.push(format!("sslmode={}", ssl_mode_value(profile.ssl_mode)));
    } else {
        query.push(format!("sslMode={}", ssl_mode_value(profile.ssl_mode)));
    }
    if profile.read_only {
        query.push("readOnly=true".to_owned());
    }
    if !query.is_empty() {
        output.push('?');
        output.push_str(&query.join("&"));
    }
    Ok(output)
}

fn parse_sqlite_query(query: &str) -> Result<bool, ProfileError> {
    let mut seen_mode = false;
    let mut read_only = false;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if key.eq_ignore_ascii_case("mode") {
            reject_duplicate(&mut seen_mode, "mode")?;
            match value.to_ascii_lowercase().as_str() {
                "ro" => read_only = true,
                "rw" | "rwc" => read_only = false,
                _ => return Err(ProfileError::InvalidQueryParameter("mode".into())),
            }
        } else {
            return Err(ProfileError::UnknownQueryParameter(key.into_owned()));
        }
    }
    Ok(read_only)
}

fn reject_duplicate(seen: &mut bool, name: &str) -> Result<(), ProfileError> {
    if *seen {
        Err(ProfileError::ConflictingQueryParameter(name.to_owned()))
    } else {
        *seen = true;
        Ok(())
    }
}

fn parse_bool(value: &str, name: &str) -> Result<bool, ProfileError> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(ProfileError::InvalidQueryParameter(name.to_owned())),
    }
}

fn choose_name(preferred: Option<&str>, fallback: &str) -> String {
    preferred
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

fn decode(value: &str) -> String {
    percent_decode_str(value).decode_utf8_lossy().into_owned()
}

fn parse_ssl_mode(value: &str) -> Option<SslMode> {
    match value.to_ascii_lowercase().replace('_', "-").as_str() {
        "disable" | "disabled" | "false" => Some(SslMode::Disable),
        "prefer" | "preferred" => Some(SslMode::Prefer),
        "require" | "required" | "true" => Some(SslMode::Require),
        "verify-ca" | "verify-ca-certificate" => Some(SslMode::VerifyCa),
        "verify-full" | "verify-identity" => Some(SslMode::VerifyFull),
        _ => None,
    }
}

fn ssl_mode_value(value: SslMode) -> &'static str {
    match value {
        SslMode::Disable => "disable",
        SslMode::Prefer => "prefer",
        SslMode::Require => "require",
        SslMode::VerifyCa => "verify-ca",
        SslMode::VerifyFull => "verify-full",
    }
}

#[cfg(test)]
mod tests {
    use super::{CatalogScope, DatabaseKind, SslMode, import_connection_url};

    #[test]
    fn imports_postgres_jdbc_url_and_current_schema() {
        let imported = import_connection_url(
            "jdbc:postgresql://10.196.178.221:30345/moss?currentSchema=tools",
            None,
        )
        .unwrap();

        assert_eq!(imported.profile.kind, DatabaseKind::Postgres);
        assert_eq!(imported.profile.name, "moss");
        assert_eq!(imported.profile.host.as_deref(), Some("10.196.178.221"));
        assert_eq!(imported.profile.port, Some(30345));
        assert_eq!(imported.profile.database.as_deref(), Some("moss"));
        assert_eq!(imported.profile.default_schema.as_deref(), Some("tools"));
        assert_eq!(
            imported.profile.catalog_scope,
            CatalogScope::for_profile(DatabaseKind::Postgres, "moss", Some("tools"))
        );
    }

    #[test]
    fn imports_postgres_password_as_transient_secret() {
        let imported = import_connection_url(
            "postgres://alice:super-sensitive-password@db.example.com:5433/app?sslmode=require",
            Some("production"),
        )
        .unwrap();

        assert_eq!(imported.profile.user.as_deref(), Some("alice"));
        assert_eq!(imported.profile.port, Some(5433));
        assert_eq!(imported.profile.ssl_mode, SslMode::Require);
        assert!(imported.transient_password.is_some());
        assert!(!format!("{imported:?}").contains("super-sensitive-password"));
        assert!(
            !toml::to_string(&imported.profile)
                .unwrap()
                .contains("super-sensitive-password")
        );
    }

    #[test]
    fn imports_mysql_jdbc_ssl_settings() {
        let imported =
            import_connection_url("jdbc:mysql://db.example.com:3307/catalog?useSSL=true", None)
                .unwrap();

        assert_eq!(imported.profile.kind, DatabaseKind::MySql);
        assert_eq!(imported.profile.port, Some(3307));
        assert_eq!(imported.profile.database.as_deref(), Some("catalog"));
        assert_eq!(imported.profile.ssl_mode, SslMode::Require);
        assert_eq!(
            imported.profile.catalog_scope,
            CatalogScope::for_profile(DatabaseKind::MySql, "catalog", None)
        );
    }

    #[test]
    fn imports_database_default_ports() {
        let postgres = import_connection_url("postgres://db.example.com/app", None).unwrap();
        let mysql = import_connection_url("mysql://db.example.com/app", None).unwrap();

        assert_eq!(postgres.profile.port, Some(5432));
        assert_eq!(mysql.profile.port, Some(3306));
    }

    #[test]
    fn imports_sqlite_forms() {
        let absolute = import_connection_url("sqlite:///tmp/lazydb.db", None).unwrap();
        let relative = import_connection_url("sqlite://demo.db", None).unwrap();
        let file = import_connection_url("file:/tmp/file.db", None).unwrap();
        let memory = import_connection_url(":memory:", None).unwrap();

        assert_eq!(
            absolute.profile.catalog_scope,
            CatalogScope::for_profile(DatabaseKind::Sqlite, "/tmp/lazydb.db", Some("main"))
        );
        assert_eq!(
            memory.profile.catalog_scope,
            CatalogScope::for_profile(DatabaseKind::Sqlite, ":memory:", Some("main"))
        );
        assert_eq!(
            absolute.profile.sqlite_path.unwrap().to_string_lossy(),
            "/tmp/lazydb.db"
        );
        assert_eq!(
            relative.profile.sqlite_path.unwrap().to_string_lossy(),
            "demo.db"
        );
        assert_eq!(
            file.profile.sqlite_path.unwrap().to_string_lossy(),
            "/tmp/file.db"
        );
        assert_eq!(memory.profile.database.as_deref(), Some(":memory:"));
    }

    #[test]
    fn rejects_unknown_schemes() {
        let error = import_connection_url("oracle://db.example.com/app", None).unwrap_err();

        assert!(error.to_string().contains("unsupported database scheme"));
    }
}
