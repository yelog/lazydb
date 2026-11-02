use std::{collections::HashSet, ops::Deref, path::PathBuf};

use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use crate::persistence::local_credentials::EncryptedCredential;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseKind {
    Postgres,
    CockroachDb,
    MySql,
    MariaDb,
    Oracle,
    SqlServer,
    Sqlite,
    DuckDb,
    Redis,
    Valkey,
    MongoDb,
    ClickHouse,
    Elasticsearch,
    OpenSearch,
    TiDb,
    Snowflake,
}

impl DatabaseKind {
    pub const fn is_search_rest(self) -> bool {
        matches!(self, Self::Elasticsearch | Self::OpenSearch)
    }

    pub const fn is_redis_compatible(self) -> bool {
        matches!(self, Self::Redis | Self::Valkey)
    }

    pub const fn is_postgres_compatible(self) -> bool {
        matches!(self, Self::Postgres | Self::CockroachDb)
    }

    pub fn is_relational(self) -> bool {
        matches!(
            self,
            Self::Postgres
                | Self::CockroachDb
                | Self::MySql
                | Self::MariaDb
                | Self::TiDb
                | Self::Oracle
                | Self::SqlServer
                | Self::Sqlite
                | Self::ClickHouse
                | Self::DuckDb
                | Self::Snowflake
        )
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionUrlFormat {
    #[serde(rename = "cockroachdb")]
    CockroachDb,
    Postgres,
    #[default]
    PostgreSql,
    JdbcPostgreSql,
    MySql,
    JdbcMySql,
    MariaDb,
    JdbcOracle,
    SqlServer,
    MsSql,
    JdbcSqlServer,
    Sqlite,
    FileUri,
    JdbcSqlite,
    DuckDb,
    Redis,
    RedisTls,
    Valkey,
    ValkeyTls,
    MongoDb,
    MongoDbSrv,
    ClickHouse,
    ClickHouseTls,
    Elasticsearch,
    ElasticsearchTls,
    OpenSearch,
    OpenSearchTls,
    #[serde(rename = "tidb")]
    TiDb,
    Snowflake,
}

impl ConnectionUrlFormat {
    pub const fn default_for(kind: DatabaseKind) -> Self {
        match kind {
            DatabaseKind::Postgres => Self::PostgreSql,
            DatabaseKind::CockroachDb => Self::CockroachDb,
            DatabaseKind::MySql => Self::MySql,
            DatabaseKind::MariaDb => Self::MariaDb,
            DatabaseKind::TiDb => Self::TiDb,
            DatabaseKind::Oracle => Self::JdbcOracle,
            DatabaseKind::SqlServer => Self::SqlServer,
            DatabaseKind::Sqlite => Self::Sqlite,
            DatabaseKind::DuckDb => Self::DuckDb,
            DatabaseKind::Redis => Self::Redis,
            DatabaseKind::Valkey => Self::Valkey,
            DatabaseKind::MongoDb => Self::MongoDb,
            DatabaseKind::ClickHouse => Self::ClickHouse,
            DatabaseKind::Elasticsearch => Self::Elasticsearch,
            DatabaseKind::OpenSearch => Self::OpenSearch,
            DatabaseKind::Snowflake => Self::Snowflake,
        }
    }

    pub const fn is_compatible(self, kind: DatabaseKind) -> bool {
        matches!(
            (self, kind),
            (
                Self::Postgres | Self::PostgreSql | Self::JdbcPostgreSql,
                DatabaseKind::Postgres
            ) | (
                Self::CockroachDb | Self::Postgres | Self::PostgreSql | Self::JdbcPostgreSql,
                DatabaseKind::CockroachDb
            ) | (Self::MySql | Self::JdbcMySql, DatabaseKind::MySql)
                | (Self::MariaDb, DatabaseKind::MariaDb)
                | (Self::TiDb, DatabaseKind::TiDb)
                | (Self::JdbcOracle, DatabaseKind::Oracle)
                | (
                    Self::SqlServer | Self::MsSql | Self::JdbcSqlServer,
                    DatabaseKind::SqlServer
                )
                | (
                    Self::Sqlite | Self::FileUri | Self::JdbcSqlite,
                    DatabaseKind::Sqlite
                )
                | (Self::Redis | Self::RedisTls, DatabaseKind::Redis)
                | (Self::Valkey | Self::ValkeyTls, DatabaseKind::Valkey)
                | (Self::MongoDb | Self::MongoDbSrv, DatabaseKind::MongoDb)
                | (
                    Self::ClickHouse | Self::ClickHouseTls,
                    DatabaseKind::ClickHouse
                )
                | (
                    Self::Elasticsearch | Self::ElasticsearchTls,
                    DatabaseKind::Elasticsearch
                )
                | (
                    Self::OpenSearch | Self::OpenSearchTls,
                    DatabaseKind::OpenSearch
                )
                | (Self::DuckDb, DatabaseKind::DuckDb)
                | (Self::Snowflake, DatabaseKind::Snowflake)
        )
    }

    pub const fn compatible_formats(kind: DatabaseKind) -> &'static [Self] {
        match kind {
            DatabaseKind::Postgres => &[Self::Postgres, Self::PostgreSql, Self::JdbcPostgreSql],
            DatabaseKind::CockroachDb => &[
                Self::CockroachDb,
                Self::Postgres,
                Self::PostgreSql,
                Self::JdbcPostgreSql,
            ],
            DatabaseKind::MySql => &[Self::MySql, Self::JdbcMySql],
            DatabaseKind::MariaDb => &[Self::MariaDb],
            DatabaseKind::TiDb => &[Self::TiDb],
            DatabaseKind::Oracle => &[Self::JdbcOracle],
            DatabaseKind::SqlServer => &[Self::SqlServer, Self::MsSql, Self::JdbcSqlServer],
            DatabaseKind::Sqlite => &[Self::Sqlite, Self::FileUri, Self::JdbcSqlite],
            DatabaseKind::DuckDb => &[Self::DuckDb],
            DatabaseKind::Redis => &[Self::Redis, Self::RedisTls],
            DatabaseKind::Valkey => &[Self::Valkey, Self::ValkeyTls],
            DatabaseKind::MongoDb => &[Self::MongoDb, Self::MongoDbSrv],
            DatabaseKind::ClickHouse => &[Self::ClickHouse, Self::ClickHouseTls],
            DatabaseKind::Elasticsearch => &[Self::Elasticsearch, Self::ElasticsearchTls],
            DatabaseKind::OpenSearch => &[Self::OpenSearch, Self::OpenSearchTls],
            DatabaseKind::Snowflake => &[Self::Snowflake],
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
        if kind.is_redis_compatible() {
            return Self {
                databases: CatalogSelection::All,
            };
        }
        // Search REST drivers expose a cluster through a virtual database namespace.
        // This name never restricts which indices can be queried.
        let database = if kind.is_search_rest() && database.trim().is_empty() {
            "default"
        } else {
            database
        };
        let schemas = match (kind, default_schema) {
            (
                DatabaseKind::MySql
                | DatabaseKind::TiDb
                | DatabaseKind::ClickHouse
                | DatabaseKind::Elasticsearch
                | DatabaseKind::OpenSearch,
                _,
            )
            | (_, None) => CatalogSelection::All,
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

        let _ = (database, default_schema);

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

    pub fn with_attached_databases(
        mut self,
        current_database: &str,
        attached: impl IntoIterator<Item = String>,
    ) -> Self {
        let CatalogSelection::Selected(databases) = &mut self.databases else {
            return self;
        };
        let Some(current) = databases
            .iter()
            .find(|database| database.name == current_database)
            .cloned()
        else {
            return self;
        };
        for name in attached {
            if name != current_database && !databases.iter().any(|database| database.name == name) {
                databases.push(DatabaseScope {
                    schemas: if name == "temp" {
                        CatalogSelection::All
                    } else {
                        current.schemas.clone()
                    },
                    name,
                });
            }
        }
        self
    }

    pub fn with_discovered_databases(
        mut self,
        current_database: &str,
        discovered: impl IntoIterator<Item = String>,
    ) -> Self {
        if let CatalogSelection::Selected(databases) = &mut self.databases {
            let schema_scope = databases
                .iter()
                .find(|database| database.name == current_database)
                .map(|database| database.schemas.clone())
                .unwrap_or(CatalogSelection::All);
            for name in discovered {
                if name != current_database
                    && !databases.iter().any(|database| database.name == name)
                {
                    databases.push(DatabaseScope {
                        name: name.clone(),
                        schemas: if name == "temp" {
                            CatalogSelection::All
                        } else {
                            schema_scope.clone()
                        },
                    });
                }
            }
        }
        self
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
#[serde(tag = "policy", content = "reference", rename_all = "snake_case")]
pub enum CredentialPolicy {
    #[default]
    None,
    Prompt,
    LocalEncrypted(EncryptedCredential),
    System(String),
    /// Legacy v3 spelling. Profile loading normalizes this to `System`.
    Keyring(String),
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum ProfileAccess {
    #[default]
    Global,
    Projects {
        roots: Vec<PathBuf>,
    },
}

impl ProfileAccess {
    pub fn contains_project(&self, root: &std::path::Path) -> bool {
        matches!(self, Self::Projects { roots } if roots.iter().any(|candidate| candidate == root))
    }

    pub fn add_project(&mut self, root: PathBuf) {
        let Self::Projects { roots } = self else {
            return;
        };
        if !roots.contains(&root) {
            roots.push(root);
            roots.sort();
        }
    }

    pub fn remove_project(&mut self, root: &std::path::Path) {
        if let Self::Projects { roots } = self {
            roots.retain(|candidate| candidate != root);
        }
    }
}

pub const MAX_CONNECTION_GROUP_NAME_CHARS: usize = 64;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionGroup {
    pub id: Uuid,
    pub name: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProfileCollection {
    pub groups: Vec<ConnectionGroup>,
    pub profiles: Vec<ConnectionProfile>,
}

impl From<Vec<ConnectionProfile>> for ProfileCollection {
    fn from(profiles: Vec<ConnectionProfile>) -> Self {
        Self {
            groups: Vec::new(),
            profiles,
        }
    }
}

impl From<&[ConnectionProfile]> for ProfileCollection {
    fn from(profiles: &[ConnectionProfile]) -> Self {
        Self::from(profiles.to_vec())
    }
}

impl<const N: usize> From<&[ConnectionProfile; N]> for ProfileCollection {
    fn from(profiles: &[ConnectionProfile; N]) -> Self {
        Self::from(profiles.as_slice())
    }
}

impl From<&ProfileCollection> for ProfileCollection {
    fn from(collection: &ProfileCollection) -> Self {
        collection.clone()
    }
}

impl Deref for ProfileCollection {
    type Target = [ConnectionProfile];

    fn deref(&self) -> &Self::Target {
        &self.profiles
    }
}

impl PartialEq<Vec<ConnectionProfile>> for ProfileCollection {
    fn eq(&self, other: &Vec<ConnectionProfile>) -> bool {
        self.profiles == *other
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ConnectionGroupNameError {
    #[error("group name cannot be empty")]
    Empty,
    #[error("group name cannot exceed {MAX_CONNECTION_GROUP_NAME_CHARS} characters")]
    TooLong,
}

impl ConnectionGroup {
    pub fn new(id: Uuid, name: impl Into<String>) -> Result<Self, ConnectionGroupNameError> {
        let name = name.into().trim().to_owned();
        if name.is_empty() {
            return Err(ConnectionGroupNameError::Empty);
        }
        if name.chars().count() > MAX_CONNECTION_GROUP_NAME_CHARS {
            return Err(ConnectionGroupNameError::TooLong);
        }
        Ok(Self { id, name })
    }

    pub fn normalized_name(&self) -> String {
        self.name.to_lowercase()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PasswordStorageChoice {
    #[default]
    LocalEncrypted,
    System,
}

impl CredentialPolicy {
    pub const fn has_persisted_credential(&self) -> bool {
        matches!(
            self,
            Self::LocalEncrypted(_) | Self::System(_) | Self::Keyring(_)
        )
    }

    pub const fn storage_choice(&self) -> Option<PasswordStorageChoice> {
        match self {
            Self::LocalEncrypted(_) => Some(PasswordStorageChoice::LocalEncrypted),
            Self::System(_) | Self::Keyring(_) => Some(PasswordStorageChoice::System),
            Self::None | Self::Prompt => None,
        }
    }

    pub fn keyring_reference(&self) -> Option<&str> {
        match self {
            Self::System(reference) | Self::Keyring(reference) => Some(reference),
            Self::None | Self::Prompt | Self::LocalEncrypted(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ElasticsearchAuth {
    #[default]
    Basic,
    ApiKey,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ElasticsearchConnectionOptions {
    #[serde(default)]
    pub auth: ElasticsearchAuth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_cert_path: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenSearchAuth {
    #[default]
    Basic,
    None,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenSearchConnectionOptions {
    #[serde(default)]
    pub auth: OpenSearchAuth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_cert_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionProfile {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub access: ProfileAccess,
    #[serde(default)]
    pub group_id: Option<Uuid>,
    pub kind: DatabaseKind,
    #[serde(default)]
    pub url_format: ConnectionUrlFormat,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub database: Option<String>,
    pub default_schema: Option<String>,
    pub sqlite_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duckdb_path: Option<PathBuf>,
    #[serde(default)]
    pub ssl_mode: SslMode,
    #[serde(default)]
    pub credential_policy: CredentialPolicy,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub environment: Environment,
    pub catalog_scope: CatalogScope,
    #[serde(default)]
    pub mongodb_options: Option<MongoConnectionOptions>,
    #[serde(default)]
    pub pg_options: Option<PgConnectionOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elasticsearch_options: Option<ElasticsearchConnectionOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opensearch_options: Option<OpenSearchConnectionOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snowflake_options: Option<SnowflakeConnectionOptions>,
}

#[derive(Debug)]
pub struct ImportedProfile {
    pub profile: ConnectionProfile,
    pub transient_password: Option<SecretString>,
    pub read_only_explicit: bool,
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
    #[error("connection URL is missing a port")]
    MissingPort,
    #[error("connection URL is missing an Oracle service name")]
    MissingOracleService,
    #[error("SQLite URL is missing a database path")]
    MissingSqlitePath,
    #[error("DuckDB URL is missing a database path")]
    MissingDuckDbPath,
    #[error("connection URL contains unknown query parameter `{0}`")]
    UnknownQueryParameter(String),
    #[error("connection URL contains conflicting query parameter `{0}`")]
    ConflictingQueryParameter(String),
    #[error("connection URL has an invalid value for query parameter `{0}`")]
    InvalidQueryParameter(String),
    #[error(
        "connection property `{0}` is unsupported; use SQL Server username/password authentication with an explicit TCP host and port"
    )]
    UnsupportedProperty(String),
    #[error("connection URL format is incompatible with the database driver")]
    IncompatibleFormat,
}

fn parse_duckdb_url(input: &str) -> Result<Option<ParsedConnectionUrl>, ProfileError> {
    let Some(rest) = input
        .get(..7)
        .filter(|prefix| prefix.eq_ignore_ascii_case("duckdb:"))
        .map(|_| &input[7..])
    else {
        return Ok(None);
    };

    let (path_and_authority, query) = rest.split_once('?').unwrap_or((rest, ""));
    let path_and_authority =
        path_and_authority
            .strip_prefix("//")
            .map_or(path_and_authority, |value| {
                if value.starts_with('/') || is_windows_filesystem_path(value) {
                    value
                } else {
                    path_and_authority
                }
            });
    let decoded_windows_path = percent_decode_str(path_and_authority).decode_utf8().ok();
    let windows_path = decoded_windows_path
        .as_deref()
        .is_some_and(is_windows_filesystem_path);
    let path = if path_and_authority == ":memory:" {
        path_and_authority
    } else if windows_path {
        decoded_windows_path.as_deref().unwrap()
    } else if let Some(path) = path_and_authority.strip_prefix("//") {
        if path.starts_with('/') {
            path
        } else {
            return Err(ProfileError::InvalidQueryParameter("path".into()));
        }
    } else {
        if path_and_authority.starts_with('/') {
            path_and_authority
        } else if path_and_authority
            .split('/')
            .next()
            .is_some_and(|part| part.contains('.'))
        {
            return Err(ProfileError::InvalidQueryParameter("path".into()));
        } else {
            path_and_authority
        }
    };
    let memory = path == ":memory:";
    if path.is_empty() {
        return Err(ProfileError::MissingDuckDbPath);
    }

    let mut read_only = false;
    let mut read_only_seen = false;
    let mut schema = "main".to_owned();
    let mut schema_seen = false;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if key.eq_ignore_ascii_case("read_only") {
            if read_only_seen {
                return Err(ProfileError::ConflictingQueryParameter("read_only".into()));
            }
            read_only_seen = true;
            read_only = match value.as_ref() {
                "true" => true,
                "false" => false,
                _ => return Err(ProfileError::InvalidQueryParameter("read_only".into())),
            };
        } else if key.eq_ignore_ascii_case("schema") {
            if schema_seen {
                return Err(ProfileError::ConflictingQueryParameter("schema".into()));
            }
            schema_seen = true;
            schema = value.into_owned();
            if schema.is_empty() {
                return Err(ProfileError::InvalidQueryParameter("schema".into()));
            }
        } else {
            return Err(ProfileError::UnknownQueryParameter(key.into_owned()));
        }
    }
    if memory && read_only {
        return Err(ProfileError::InvalidQueryParameter("read_only".into()));
    }

    let decoded_path = if windows_path {
        std::borrow::Cow::Owned(path.to_owned())
    } else {
        percent_decode_str(path)
            .decode_utf8()
            .map_err(|_| ProfileError::InvalidQueryParameter("path".into()))?
    };
    let duckdb_path = (!memory).then(|| PathBuf::from(decoded_path.into_owned()));
    let database = duckdb_path
        .as_ref()
        .and_then(|path| path.file_stem())
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("memory")
        .to_owned();
    Ok(Some(ParsedConnectionUrl {
        kind: DatabaseKind::DuckDb,
        format: ConnectionUrlFormat::DuckDb,
        host: None,
        port: None,
        user: None,
        password: None,
        database: Some(database),
        default_schema: Some(schema),
        sqlite_path: None,
        sqlite_memory: false,
        duckdb_path,
        ssl_mode: SslMode::Disable,
        read_only,
        read_only_explicit: read_only_seen,
        mongodb_options: None,
        pg_options: None,
        snowflake_options: None,
    }))
}

fn is_windows_filesystem_path(value: &str) -> bool {
    let value = value.strip_prefix("\\\\?\\").unwrap_or(value);
    if value.starts_with("\\\\") || value.starts_with("UNC\\") {
        return true;
    }
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
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
    pub duckdb_path: Option<PathBuf>,
    pub ssl_mode: SslMode,
    pub read_only: bool,
    pub read_only_explicit: bool,
    pub mongodb_options: Option<MongoConnectionOptions>,
    pub pg_options: Option<PgConnectionOptions>,
    pub snowflake_options: Option<SnowflakeConnectionOptions>,
}

/// PostgreSQL protocol options. Certificate material remains in the referenced
/// files; profiles and URL conversion only retain their paths.
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PgConnectionOptions {
    #[serde(default)]
    pub ssl_root_cert: Option<PathBuf>,
    #[serde(default)]
    pub ssl_client_cert: Option<PathBuf>,
    #[serde(default)]
    pub ssl_client_key: Option<PathBuf>,
    #[serde(default)]
    pub options: Option<String>,
}

/// SQL API session options. The access token is kept in the shared credential store.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SnowflakeConnectionOptions {
    pub warehouse: Option<String>,
    pub role: Option<String>,
    pub token_type: String,
}

impl Default for SnowflakeConnectionOptions {
    fn default() -> Self {
        Self {
            warehouse: None,
            role: None,
            token_type: "PROGRAMMATIC_ACCESS_TOKEN".to_owned(),
        }
    }
}

impl SnowflakeConnectionOptions {
    pub fn validate(&self) -> Result<(), ProfileError> {
        if !matches!(
            self.token_type.as_str(),
            "PROGRAMMATIC_ACCESS_TOKEN" | "OAUTH" | "KEYPAIR_JWT"
        ) {
            return Err(ProfileError::InvalidQueryParameter("token_type".into()));
        }
        for (name, value) in [("warehouse", &self.warehouse), ("role", &self.role)] {
            if value.as_ref().is_some_and(|value| value.trim().is_empty()) {
                return Err(ProfileError::InvalidQueryParameter(name.into()));
            }
        }
        Ok(())
    }
}

/// MongoDB options which cannot be represented by the shared single-host
/// connection fields. Credentials are deliberately excluded.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MongoConnectionOptions {
    pub seeds: Vec<MongoSeed>,
    pub srv: bool,
    pub uri_options: Vec<(String, String)>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MongoSeed {
    pub host: String,
    pub port: Option<u16>,
}

pub fn parse_connection_url(input: &str) -> Result<ParsedConnectionUrl, ProfileError> {
    parse_connection_url_with_driver(input, None)
}

/// Parse a URL using an explicit driver identity, including PostgreSQL URLs
/// supplied for CockroachDB. Defaults are chosen after resolving that identity.
pub fn parse_connection_url_with_driver(
    input: &str,
    driver: Option<DatabaseKind>,
) -> Result<ParsedConnectionUrl, ProfileError> {
    let parsed = parse_connection_url_inner(input, driver)?;
    if driver.is_some_and(|driver| driver != parsed.kind) {
        return Err(ProfileError::IncompatibleFormat);
    }
    Ok(parsed)
}

fn parse_connection_url_inner(
    input: &str,
    driver: Option<DatabaseKind>,
) -> Result<ParsedConnectionUrl, ProfileError> {
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
            snowflake_options: None,
            duckdb_path: None,
            ssl_mode: SslMode::Disable,
            read_only,
            read_only_explicit: url::form_urlencoded::parse(query.as_bytes())
                .any(|(key, _)| key.eq_ignore_ascii_case("mode")),
            mongodb_options: None,
            pg_options: None,
        });
    }
    if normalized.starts_with("sqlite:") || normalized.starts_with("file:") {
        return parse_sqlite_url(normalized, jdbc);
    }
    if let Some(parsed) = parse_duckdb_url(normalized)? {
        return Ok(parsed);
    }
    if jdbc
        && normalized
            .get(..12)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("sqlserver://"))
    {
        return parse_jdbc_sql_server_url(normalized);
    }
    if jdbc && normalized.to_ascii_lowercase().starts_with("oracle:thin:@") {
        return parse_jdbc_oracle_url(normalized);
    }
    if normalized.to_ascii_lowercase().starts_with("mongodb://")
        || normalized
            .to_ascii_lowercase()
            .starts_with("mongodb+srv://")
    {
        return parse_mongodb_url(normalized);
    }

    if let Some(parsed) = parse_duckdb_url(normalized)? {
        return Ok(parsed);
    }
    let url = Url::parse(normalized)?;
    let (kind, format) = match (url.scheme().to_ascii_lowercase().as_str(), jdbc) {
        ("cockroachdb", false) => (DatabaseKind::CockroachDb, ConnectionUrlFormat::CockroachDb),
        ("postgres", false) => (DatabaseKind::Postgres, ConnectionUrlFormat::Postgres),
        ("postgresql", false) => (DatabaseKind::Postgres, ConnectionUrlFormat::PostgreSql),
        ("postgresql", true) => (DatabaseKind::Postgres, ConnectionUrlFormat::JdbcPostgreSql),
        ("mysql", false) => (DatabaseKind::MySql, ConnectionUrlFormat::MySql),
        ("mysql", true) => (DatabaseKind::MySql, ConnectionUrlFormat::JdbcMySql),
        ("mariadb", false) => (DatabaseKind::MariaDb, ConnectionUrlFormat::MariaDb),
        ("tidb", false) => (DatabaseKind::TiDb, ConnectionUrlFormat::TiDb),
        ("sqlserver", false) => (DatabaseKind::SqlServer, ConnectionUrlFormat::SqlServer),
        ("mssql", false) => (DatabaseKind::SqlServer, ConnectionUrlFormat::MsSql),
        ("redis", false) => (DatabaseKind::Redis, ConnectionUrlFormat::Redis),
        ("rediss", false) => (DatabaseKind::Redis, ConnectionUrlFormat::RedisTls),
        ("valkey", false) => (DatabaseKind::Valkey, ConnectionUrlFormat::Valkey),
        ("valkeys", false) => (DatabaseKind::Valkey, ConnectionUrlFormat::ValkeyTls),
        ("snowflake", false) => (DatabaseKind::Snowflake, ConnectionUrlFormat::Snowflake),
        ("clickhouse", false) => (DatabaseKind::ClickHouse, ConnectionUrlFormat::ClickHouse),
        ("clickhouses", false) => (DatabaseKind::ClickHouse, ConnectionUrlFormat::ClickHouseTls),
        ("elasticsearch", false) => (
            DatabaseKind::Elasticsearch,
            ConnectionUrlFormat::Elasticsearch,
        ),
        ("elasticsearchs", false) => (
            DatabaseKind::Elasticsearch,
            ConnectionUrlFormat::ElasticsearchTls,
        ),
        ("opensearch", false) => (DatabaseKind::OpenSearch, ConnectionUrlFormat::OpenSearch),
        ("opensearchs", false) => (DatabaseKind::OpenSearch, ConnectionUrlFormat::OpenSearchTls),
        (scheme, _) => return Err(ProfileError::UnsupportedScheme(scheme.to_owned())),
    };
    let kind = match driver {
        Some(driver) if format.is_compatible(driver) => driver,
        Some(_) => return Err(ProfileError::IncompatibleFormat),
        None => kind,
    };
    parse_server_url(url, kind, format)
}

fn parse_mongodb_url(input: &str) -> Result<ParsedConnectionUrl, ProfileError> {
    let (srv, rest) = if input
        .get(..14)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("mongodb+srv://"))
    {
        (true, &input[14..])
    } else if input
        .get(..10)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("mongodb://"))
    {
        (false, &input[10..])
    } else {
        return Err(ProfileError::UnsupportedScheme("mongodb".into()));
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let suffix = &rest[authority_end..];
    if authority.is_empty() || suffix.contains('#') {
        return Err(ProfileError::MissingHost);
    }
    let (credentials, seeds) = match authority.rsplit_once('@') {
        Some((credentials, seeds)) => (Some(credentials), seeds),
        None => (None, authority),
    };
    if seeds.is_empty() {
        return Err(ProfileError::MissingHost);
    }
    let mut user = None;
    let mut password = None;
    if let Some(credentials) = credentials {
        let (raw_user, raw_password) = credentials.split_once(':').unwrap_or((credentials, ""));
        if raw_user.is_empty() {
            return Err(ProfileError::InvalidQueryParameter("username".into()));
        }
        user = Some(decode(raw_user));
        if credentials.contains(':') {
            password = Some(SecretString::from(decode(raw_password)));
        }
    }
    let mut parsed_seeds = Vec::new();
    for seed in seeds.split(',') {
        if seed.is_empty() {
            return Err(ProfileError::MissingHost);
        }
        let (host, port) = if seed.starts_with('[') {
            let end = seed.find(']').ok_or(ProfileError::MissingHost)?;
            let host = &seed[1..end];
            let suffix = &seed[end + 1..];
            let port = if suffix.is_empty() {
                None
            } else {
                Some(parse_mongo_port(
                    suffix.strip_prefix(':').ok_or(ProfileError::MissingHost)?,
                )?)
            };
            (host, port)
        } else {
            match seed.rsplit_once(':') {
                Some((host, port)) if !host.contains(':') => (host, Some(parse_mongo_port(port)?)),
                _ => (seed, None),
            }
        };
        if host.is_empty() || (srv && (!parsed_seeds.is_empty() || port.is_some())) {
            return Err(ProfileError::InvalidQueryParameter("host".into()));
        }
        parsed_seeds.push(MongoSeed {
            host: host.to_owned(),
            port,
        });
    }
    let path_query = suffix.strip_prefix('/').unwrap_or(suffix);
    let (database_raw, query) = path_query.split_once('?').unwrap_or((path_query, ""));
    let database = (!database_raw.is_empty()).then(|| decode(database_raw));
    let mut options = Vec::new();
    let mut singleton = HashSet::new();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode_str(key)
            .decode_utf8()
            .map_err(|_| ProfileError::InvalidQueryParameter("option".into()))?
            .into_owned();
        let value = percent_decode_str(value)
            .decode_utf8()
            .map_err(|_| ProfileError::InvalidQueryParameter(key.clone()))?
            .into_owned();
        if key.eq_ignore_ascii_case("password")
            || key.eq_ignore_ascii_case("tlsCertificateKeyFilePassword")
            || key.to_ascii_lowercase().contains("token")
        {
            return Err(ProfileError::UnsupportedProperty(key));
        }
        let repeatable = key.eq_ignore_ascii_case("readPreferenceTags");
        if !repeatable && !singleton.insert(key.to_ascii_lowercase()) {
            return Err(ProfileError::ConflictingQueryParameter(key));
        }
        if !is_mongodb_uri_option(&key) {
            return Err(ProfileError::UnknownQueryParameter(key));
        }
        options.push((key, value));
    }
    if srv
        && options.iter().any(|(key, value)| {
            key.eq_ignore_ascii_case("tls") && value.eq_ignore_ascii_case("false")
        })
    {
        return Err(ProfileError::ConflictingQueryParameter("tls".into()));
    }
    let host = parsed_seeds.first().map(|seed| seed.host.clone());
    let port = parsed_seeds.first().and_then(|seed| seed.port);
    Ok(ParsedConnectionUrl {
        kind: DatabaseKind::MongoDb,
        format: if srv {
            ConnectionUrlFormat::MongoDbSrv
        } else {
            ConnectionUrlFormat::MongoDb
        },
        host,
        port,
        user,
        password,
        database,
        default_schema: None,
        sqlite_path: None,
        sqlite_memory: false,
        snowflake_options: None,
        duckdb_path: None,
        ssl_mode: options
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("tls") || key.eq_ignore_ascii_case("ssl"))
            .map(|(_, value)| {
                if parse_bool(value, "tls").unwrap_or(false) {
                    SslMode::Require
                } else {
                    SslMode::Disable
                }
            })
            .unwrap_or(if srv {
                SslMode::Require
            } else {
                SslMode::Prefer
            }),
        read_only: false,
        read_only_explicit: false,
        mongodb_options: Some(MongoConnectionOptions {
            seeds: parsed_seeds,
            srv,
            uri_options: options,
        }),
        pg_options: None,
    })
}

fn parse_mongo_port(value: &str) -> Result<u16, ProfileError> {
    value
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| ProfileError::InvalidQueryParameter("port".into()))
}

fn is_mongodb_uri_option(key: &str) -> bool {
    [
        "authSource",
        "authMechanism",
        "replicaSet",
        "directConnection",
        "appName",
        "readPreference",
        "readPreferenceTags",
        "retryReads",
        "retryWrites",
        "w",
        "wTimeoutMS",
        "connectTimeoutMS",
        "serverSelectionTimeoutMS",
        "socketTimeoutMS",
        "tls",
        "ssl",
        "tlsCAFile",
        "tlsCertificateKeyFile",
        "tlsAllowInvalidCertificates",
        "tlsAllowInvalidHostnames",
        "compressors",
        "zlibCompressionLevel",
        "loadBalanced",
        "maxPoolSize",
        "minPoolSize",
        "maxIdleTimeMS",
        "heartbeatFrequencyMS",
    ]
    .iter()
    .any(|known| known.eq_ignore_ascii_case(key))
}

fn parse_jdbc_oracle_url(input: &str) -> Result<ParsedConnectionUrl, ProfileError> {
    let (input, query) = input.split_once('?').unwrap_or((input, ""));
    let address = input
        .strip_prefix("oracle:thin:@")
        .ok_or_else(|| ProfileError::UnsupportedScheme("oracle".to_owned()))?;
    let (host_port, service) = address.split_once('/').ok_or(ProfileError::MissingHost)?;
    let (host, port) = host_port
        .rsplit_once(':')
        .ok_or(ProfileError::MissingHost)?;
    let port = port
        .parse::<u16>()
        .map_err(|_| ProfileError::InvalidQueryParameter("port".into()))?;
    if host.is_empty() || service.is_empty() || port == 0 {
        return Err(ProfileError::MissingHost);
    }
    let mut user = None;
    let mut password = None;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "user" => user = Some(value.into_owned()),
            "password" => password = Some(SecretString::from(value.into_owned())),
            _ => return Err(ProfileError::UnknownQueryParameter(key.into_owned())),
        }
    }
    Ok(ParsedConnectionUrl {
        kind: DatabaseKind::Oracle,
        format: ConnectionUrlFormat::JdbcOracle,
        host: Some(host.to_owned()),
        port: Some(port),
        user,
        password,
        database: Some(service.to_owned()),
        default_schema: None,
        sqlite_path: None,
        sqlite_memory: false,
        snowflake_options: None,
        duckdb_path: None,
        ssl_mode: SslMode::Prefer,
        read_only: false,
        read_only_explicit: false,
        mongodb_options: None,
        pg_options: None,
    })
}

pub fn import_connection_url(
    input: &str,
    preferred_name: Option<&str>,
) -> Result<ImportedProfile, ProfileError> {
    import_connection_url_with_driver(input, preferred_name, None)
}

pub fn import_connection_url_with_driver(
    input: &str,
    preferred_name: Option<&str>,
    driver: Option<DatabaseKind>,
) -> Result<ImportedProfile, ProfileError> {
    let parsed = parse_connection_url_with_driver(input, driver)?;
    let derived_name = if parsed.kind == DatabaseKind::DuckDb && parsed.duckdb_path.is_none() {
        "memory".to_owned()
    } else if let Some(path) = &parsed.duckdb_path {
        path.file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("duckdb")
            .to_owned()
    } else if parsed.sqlite_memory {
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
    let scope_database = if parsed.kind == DatabaseKind::DuckDb {
        parsed.database.as_deref().unwrap_or("memory")
    } else {
        parsed.database.as_deref().unwrap_or_default()
    };
    let catalog_scope = CatalogScope::for_profile(
        parsed.kind,
        scope_database,
        parsed.default_schema.as_deref(),
    );
    Ok(ImportedProfile {
        profile: ConnectionProfile {
            id: Uuid::new_v4(),
            name: choose_name(preferred_name, &derived_name),
            access: ProfileAccess::Global,
            group_id: None,
            kind: parsed.kind,
            url_format: parsed.format,
            host: parsed.host,
            port: parsed.port,
            user: parsed.user,
            database: parsed.database,
            default_schema: parsed.default_schema,
            sqlite_path: parsed.sqlite_path,
            duckdb_path: parsed.duckdb_path,
            ssl_mode: parsed.ssl_mode,
            credential_policy: CredentialPolicy::None,
            read_only: parsed.read_only,
            environment: Environment::Development,
            catalog_scope,
            mongodb_options: parsed.mongodb_options,
            elasticsearch_options: None,
            opensearch_options: None,
            pg_options: parsed.pg_options,
            snowflake_options: parsed.snowflake_options,
        },
        transient_password: parsed.password,
        read_only_explicit: parsed.read_only_explicit,
    })
}

fn validate_snowflake_endpoint(
    host: &str,
    port: u16,
    ssl_mode: SslMode,
) -> Result<(), ProfileError> {
    if !host.contains('.')
        || host.contains(['/', '@', '?', '#', ':'])
        || host.chars().any(char::is_whitespace)
    {
        return Err(ProfileError::InvalidQueryParameter("host".into()));
    }
    if port != 443 {
        return Err(ProfileError::InvalidQueryParameter("port".into()));
    }
    if ssl_mode != SslMode::VerifyFull {
        return Err(ProfileError::InvalidQueryParameter("sslMode".into()));
    }
    Ok(())
}

fn parse_server_url(
    url: Url,
    kind: DatabaseKind,
    format: ConnectionUrlFormat,
) -> Result<ParsedConnectionUrl, ProfileError> {
    let host = url.host_str().ok_or(ProfileError::MissingHost)?.to_owned();
    let database = decode(url.path().trim_start_matches('/'));
    let database = if kind.is_redis_compatible() {
        let database = if database.is_empty() { "0" } else { &database };
        let number = database
            .parse::<u32>()
            .map_err(|_| ProfileError::InvalidQueryParameter("database".into()))?;
        Some(number.to_string())
    } else if kind == DatabaseKind::ClickHouse {
        Some(
            if database.is_empty() {
                "default"
            } else {
                &database
            }
            .to_owned(),
        )
    } else if kind == DatabaseKind::CockroachDb {
        Some(if database.is_empty() {
            "defaultdb".to_owned()
        } else {
            database
        })
    } else {
        (!database.is_empty()).then_some(database)
    };
    let user = if matches!(kind, DatabaseKind::ClickHouse | DatabaseKind::TiDb) {
        Some(if url.username().is_empty() {
            if kind == DatabaseKind::TiDb {
                "root"
            } else {
                "default"
            }
            .to_owned()
        } else {
            decode(url.username())
        })
    } else {
        (!url.username().is_empty()).then(|| decode(url.username()))
    };
    let password = url
        .password()
        .map(|value| SecretString::from(decode(value)));
    let mut default_schema = (kind == DatabaseKind::Snowflake).then(|| "PUBLIC".to_owned());
    let mut snowflake_options =
        (kind == DatabaseKind::Snowflake).then(SnowflakeConnectionOptions::default);
    let mut seen_snowflake_options = HashSet::new();
    if kind.is_search_rest() {
        if url.port() == Some(0) {
            return Err(ProfileError::InvalidQueryParameter("port".into()));
        }
        if url.fragment().is_some() {
            return Err(ProfileError::InvalidQueryParameter("fragment".into()));
        }
    }
    let mut ssl_mode = if kind == DatabaseKind::Snowflake {
        SslMode::VerifyFull
    } else if matches!(
        kind,
        DatabaseKind::ClickHouse | DatabaseKind::Elasticsearch | DatabaseKind::OpenSearch
    ) {
        if matches!(
            format,
            ConnectionUrlFormat::ClickHouseTls
                | ConnectionUrlFormat::ElasticsearchTls
                | ConnectionUrlFormat::OpenSearchTls
        ) {
            SslMode::VerifyFull
        } else {
            SslMode::Disable
        }
    } else if kind.is_redis_compatible() {
        if matches!(
            format,
            ConnectionUrlFormat::RedisTls | ConnectionUrlFormat::ValkeyTls
        ) {
            SslMode::Require
        } else {
            SslMode::Disable
        }
    } else {
        SslMode::Prefer
    };
    let mut read_only = false;
    let mut seen_schema = false;
    let mut seen_ssl = false;
    let mut seen_read_only = false;
    let mut read_only_explicit = false;
    let mut encrypt = None;
    let mut trust_server_certificate = None;
    let mut pg_options = PgConnectionOptions::default();

    for (key, value) in url.query_pairs() {
        if kind == DatabaseKind::Snowflake
            && ["schema", "warehouse", "role", "token_type"]
                .iter()
                .any(|name| key.eq_ignore_ascii_case(name))
        {
            let key = key.to_ascii_lowercase();
            if !seen_snowflake_options.insert(key.clone()) {
                return Err(ProfileError::ConflictingQueryParameter(key));
            }
            if value.trim().is_empty() {
                return Err(ProfileError::InvalidQueryParameter(key));
            }
            let options = snowflake_options.as_mut().expect("Snowflake options");
            match key.as_str() {
                "schema" => default_schema = Some(value.into_owned()),
                "warehouse" => options.warehouse = Some(value.into_owned()),
                "role" => options.role = Some(value.into_owned()),
                "token_type" => options.token_type = value.into_owned(),
                _ => unreachable!(),
            }
        } else if (key.eq_ignore_ascii_case("currentSchema")
            || key.eq_ignore_ascii_case("schema") && kind == DatabaseKind::SqlServer)
            && (kind.is_postgres_compatible() || kind == DatabaseKind::SqlServer)
        {
            reject_duplicate(&mut seen_schema, "currentSchema")?;
            default_schema = Some(value.into_owned());
        } else if key.eq_ignore_ascii_case("sslmode") && kind.is_postgres_compatible() {
            reject_duplicate(&mut seen_ssl, "ssl")?;
            ssl_mode = parse_ssl_mode(&value)
                .ok_or_else(|| ProfileError::InvalidQueryParameter("sslmode".into()))?;
        } else if kind.is_postgres_compatible()
            && matches!(
                key.to_ascii_lowercase().as_str(),
                "sslrootcert" | "sslcert" | "sslkey" | "options"
            )
        {
            let key = key.to_ascii_lowercase();
            match key.as_str() {
                "sslrootcert" => {
                    reject_option_duplicate(&pg_options.ssl_root_cert, &key)?;
                    pg_options.ssl_root_cert = Some(PathBuf::from(value.into_owned()));
                }
                "sslcert" => {
                    reject_option_duplicate(&pg_options.ssl_client_cert, &key)?;
                    pg_options.ssl_client_cert = Some(PathBuf::from(value.into_owned()));
                }
                "sslkey" => {
                    reject_option_duplicate(&pg_options.ssl_client_key, &key)?;
                    pg_options.ssl_client_key = Some(PathBuf::from(value.into_owned()));
                }
                "options" => {
                    reject_option_duplicate(&pg_options.options, &key)?;
                    pg_options.options = Some(value.into_owned());
                }
                _ => unreachable!("recognized PostgreSQL option"),
            }
        } else if key.eq_ignore_ascii_case("useSSL")
            && matches!(
                kind,
                DatabaseKind::MySql | DatabaseKind::MariaDb | DatabaseKind::TiDb
            )
        {
            reject_duplicate(&mut seen_ssl, "ssl")?;
            ssl_mode = if parse_bool(&value, "useSSL")? {
                SslMode::Require
            } else {
                SslMode::Disable
            };
        } else if key.eq_ignore_ascii_case("sslMode")
            && matches!(
                kind,
                DatabaseKind::MySql | DatabaseKind::MariaDb | DatabaseKind::TiDb
            )
        {
            reject_duplicate(&mut seen_ssl, "ssl")?;
            ssl_mode = parse_ssl_mode(&value)
                .ok_or_else(|| ProfileError::InvalidQueryParameter("sslMode".into()))?;
        } else if (key.eq_ignore_ascii_case("sslMode") || key.eq_ignore_ascii_case("sslmode"))
            && kind == DatabaseKind::ClickHouse
        {
            reject_duplicate(&mut seen_ssl, "sslMode")?;
            ssl_mode = parse_ssl_mode(&value)
                .ok_or_else(|| ProfileError::InvalidQueryParameter("sslMode".into()))?;
        } else if key.eq_ignore_ascii_case("encrypt") && kind == DatabaseKind::SqlServer {
            reject_option_duplicate(&encrypt, "encrypt")?;
            encrypt = Some(parse_bool(&value, "encrypt")?);
        } else if key.eq_ignore_ascii_case("trustServerCertificate")
            && kind == DatabaseKind::SqlServer
        {
            reject_option_duplicate(&trust_server_certificate, "trustServerCertificate")?;
            trust_server_certificate = Some(parse_bool(&value, "trustServerCertificate")?);
        } else if key.eq_ignore_ascii_case("readOnly") || key.eq_ignore_ascii_case("read_only") {
            reject_duplicate(&mut seen_read_only, "readOnly")?;
            read_only_explicit = true;
            read_only = parse_bool(&value, "readOnly")?;
        } else if kind == DatabaseKind::SqlServer && is_unsupported_sql_server_property(&key) {
            return Err(ProfileError::UnsupportedProperty(key.into_owned()));
        } else {
            return Err(ProfileError::UnknownQueryParameter(key.into_owned()));
        }
    }
    if kind == DatabaseKind::SqlServer {
        ssl_mode = sql_server_ssl_mode(encrypt, trust_server_certificate)?;
    }
    if kind == DatabaseKind::ClickHouse
        && ((format == ConnectionUrlFormat::ClickHouseTls) == (ssl_mode == SslMode::Disable))
    {
        return Err(ProfileError::ConflictingQueryParameter("sslMode".into()));
    }

    if kind == DatabaseKind::Snowflake {
        validate_snowflake_endpoint(&host, url.port().unwrap_or(443), ssl_mode)?;
        if database
            .as_deref()
            .is_none_or(|database| database.is_empty() || database.contains('/'))
        {
            return Err(ProfileError::InvalidQueryParameter("database".into()));
        }
        if url.fragment().is_some() {
            return Err(ProfileError::InvalidQueryParameter("fragment".into()));
        }
        snowflake_options
            .as_ref()
            .expect("Snowflake options")
            .validate()?;
    }
    let default_port = match kind {
        DatabaseKind::Snowflake => 443,
        DatabaseKind::Postgres => 5432,
        DatabaseKind::CockroachDb => 26257,
        DatabaseKind::MySql | DatabaseKind::MariaDb => 3306,
        DatabaseKind::TiDb => 4000,
        DatabaseKind::Oracle => 1521,
        DatabaseKind::SqlServer => 1433,
        DatabaseKind::Sqlite | DatabaseKind::DuckDb => {
            unreachable!("file database URL cannot be a server URL")
        }
        DatabaseKind::Redis | DatabaseKind::Valkey => 6379,
        DatabaseKind::MongoDb => unreachable!("MongoDB has its own multi-seed URL parser"),
        DatabaseKind::Elasticsearch | DatabaseKind::OpenSearch => 9200,
        DatabaseKind::ClickHouse => match format {
            ConnectionUrlFormat::ClickHouseTls if url.port().is_none() => 8443,
            _ => 8123,
        },
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
        duckdb_path: None,
        ssl_mode,
        read_only,
        read_only_explicit,
        mongodb_options: None,
        pg_options: (pg_options != PgConnectionOptions::default()).then_some(pg_options),
        snowflake_options,
    })
}

fn parse_jdbc_sql_server_url(input: &str) -> Result<ParsedConnectionUrl, ProfileError> {
    let rest = &input[12..];
    let (server, properties) = rest.split_once(';').unwrap_or((rest, ""));
    if server.contains('\\') {
        return Err(ProfileError::UnsupportedProperty("instanceName".into()));
    }
    let endpoint = Url::parse(&format!("sqlserver://{server}"))?;
    let host = endpoint
        .host_str()
        .ok_or(ProfileError::MissingHost)?
        .to_owned();
    if !matches!(endpoint.path(), "" | "/")
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
    {
        return Err(ProfileError::InvalidQueryParameter("serverName".into()));
    }

    let mut database = None;
    let mut user = None;
    let mut password = None;
    let mut default_schema = None;
    let mut encrypt = None;
    let mut trust_server_certificate = None;
    let mut read_only = None;
    for (key, value) in parse_jdbc_properties(properties)? {
        if key.eq_ignore_ascii_case("databaseName") {
            reject_option_duplicate(&database, "databaseName")?;
            database = Some(value);
        } else if key.eq_ignore_ascii_case("user") {
            reject_option_duplicate(&user, "user")?;
            user = Some(value);
        } else if key.eq_ignore_ascii_case("password") {
            reject_option_duplicate(&password, "password")?;
            password = Some(SecretString::from(value));
        } else if key.eq_ignore_ascii_case("currentSchema") {
            reject_option_duplicate(&default_schema, "currentSchema")?;
            default_schema = Some(value);
        } else if key.eq_ignore_ascii_case("encrypt") {
            reject_option_duplicate(&encrypt, "encrypt")?;
            encrypt = Some(parse_bool(&value, "encrypt")?);
        } else if key.eq_ignore_ascii_case("trustServerCertificate") {
            reject_option_duplicate(&trust_server_certificate, "trustServerCertificate")?;
            trust_server_certificate = Some(parse_bool(&value, "trustServerCertificate")?);
        } else if key.eq_ignore_ascii_case("readOnly") {
            reject_option_duplicate(&read_only, "readOnly")?;
            read_only = Some(parse_bool(&value, "readOnly")?);
        } else if is_unsupported_sql_server_property(&key) {
            return Err(ProfileError::UnsupportedProperty(key));
        } else {
            return Err(ProfileError::UnknownQueryParameter(key));
        }
    }

    Ok(ParsedConnectionUrl {
        kind: DatabaseKind::SqlServer,
        format: ConnectionUrlFormat::JdbcSqlServer,
        host: Some(host),
        port: Some(endpoint.port().unwrap_or(1433)),
        user,
        password,
        database,
        default_schema,
        sqlite_path: None,
        sqlite_memory: false,
        snowflake_options: None,
        duckdb_path: None,
        ssl_mode: sql_server_ssl_mode(encrypt, trust_server_certificate)?,
        read_only: read_only.unwrap_or(false),
        read_only_explicit: read_only.is_some(),
        mongodb_options: None,
        pg_options: None,
    })
}

fn parse_jdbc_properties(input: &str) -> Result<Vec<(String, String)>, ProfileError> {
    let mut properties = Vec::new();
    let mut start = 0;
    let bytes = input.as_bytes();
    while start < bytes.len() {
        let Some(relative_equals) = input[start..].find('=') else {
            return Err(ProfileError::InvalidQueryParameter("JDBC property".into()));
        };
        let equals = start + relative_equals;
        let key = &input[start..equals];
        if key.is_empty() || key.contains(';') {
            return Err(ProfileError::InvalidQueryParameter("JDBC property".into()));
        }
        let value_start = equals + 1;
        let (value, next) = if bytes.get(value_start) == Some(&b'{') {
            let mut value = String::new();
            let mut cursor = value_start + 1;
            loop {
                match bytes.get(cursor) {
                    Some(b'}') if bytes.get(cursor + 1) == Some(&b'}') => {
                        value.push('}');
                        cursor += 2;
                    }
                    Some(b'}') => {
                        cursor += 1;
                        if cursor < bytes.len() && bytes[cursor] != b';' {
                            return Err(ProfileError::InvalidQueryParameter(key.to_owned()));
                        }
                        break (value, cursor.saturating_add(1));
                    }
                    Some(_) => {
                        let character = input[cursor..].chars().next().unwrap();
                        value.push(character);
                        cursor += character.len_utf8();
                    }
                    None => return Err(ProfileError::InvalidQueryParameter(key.to_owned())),
                }
            }
        } else {
            let end = input[value_start..]
                .find(';')
                .map_or(bytes.len(), |offset| value_start + offset);
            (input[value_start..end].to_owned(), end.saturating_add(1))
        };
        properties.push((key.to_owned(), value));
        start = next;
    }
    Ok(properties)
}

fn is_unsupported_sql_server_property(key: &str) -> bool {
    [
        "integratedSecurity",
        "authentication",
        "instanceName",
        "accessToken",
        "accessTokenCallbackClass",
    ]
    .iter()
    .any(|candidate| key.eq_ignore_ascii_case(candidate))
}

fn sql_server_ssl_mode(
    encrypt: Option<bool>,
    trust_server_certificate: Option<bool>,
) -> Result<SslMode, ProfileError> {
    match (encrypt, trust_server_certificate) {
        (None, None) => Ok(SslMode::Prefer),
        (None, Some(_)) | (Some(false), Some(true)) => Err(
            ProfileError::ConflictingQueryParameter("trustServerCertificate".into()),
        ),
        (Some(false), None | Some(false)) => Ok(SslMode::Disable),
        (Some(true), Some(true)) => Ok(SslMode::Require),
        (Some(true), None | Some(false)) => Ok(SslMode::VerifyFull),
    }
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
        snowflake_options: None,
        duckdb_path: None,
        ssl_mode: SslMode::Disable,
        read_only,
        read_only_explicit: url::form_urlencoded::parse(query.as_bytes())
            .any(|(key, _)| key.eq_ignore_ascii_case("mode")),
        mongodb_options: None,
        pg_options: None,
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
    if profile.kind == DatabaseKind::MongoDb {
        return format_mongodb_url(profile, format);
    }
    if profile.kind == DatabaseKind::DuckDb {
        let path = match profile.duckdb_path.as_ref() {
            Some(path) => utf8_percent_encode(&path.to_string_lossy(), SQLITE_PATH).to_string(),
            None => ":memory:".to_owned(),
        };
        let mut query = Vec::new();
        if profile.read_only {
            query.push("read_only=true".to_owned());
        }
        if let Some(schema) = profile.default_schema.as_deref()
            && schema != "main"
        {
            query.push(format!(
                "schema={}",
                utf8_percent_encode(schema, QUERY_VALUE)
            ));
        }
        let suffix = if query.is_empty() {
            String::new()
        } else {
            format!("?{}", query.join("&"))
        };
        return Ok(format!("duckdb:{path}{suffix}"));
    }
    if format == ConnectionUrlFormat::JdbcSqlServer {
        return format_jdbc_sql_server_url(profile);
    }
    if format == ConnectionUrlFormat::JdbcOracle {
        let host = profile.host.as_deref().ok_or(ProfileError::MissingHost)?;
        let port = profile.port.ok_or(ProfileError::MissingPort)?;
        let service = profile
            .database
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or(ProfileError::MissingOracleService)?;
        let mut output = format!("jdbc:oracle:thin:@{host}:{port}/{service}");
        if let Some(user) = profile.user.as_deref().filter(|value| !value.is_empty()) {
            output.push_str(&format!("?user={}", utf8_percent_encode(user, QUERY_VALUE)));
        }
        return Ok(output);
    }
    let scheme = match format {
        ConnectionUrlFormat::CockroachDb => "cockroachdb",
        ConnectionUrlFormat::Postgres => "postgres",
        ConnectionUrlFormat::PostgreSql | ConnectionUrlFormat::JdbcPostgreSql => "postgresql",
        ConnectionUrlFormat::MySql | ConnectionUrlFormat::JdbcMySql => "mysql",
        ConnectionUrlFormat::MariaDb => "mariadb",
        ConnectionUrlFormat::TiDb => "tidb",
        ConnectionUrlFormat::SqlServer | ConnectionUrlFormat::JdbcSqlServer => "sqlserver",
        ConnectionUrlFormat::MsSql => "mssql",
        ConnectionUrlFormat::Redis => "redis",
        ConnectionUrlFormat::RedisTls => "rediss",
        ConnectionUrlFormat::Valkey => "valkey",
        ConnectionUrlFormat::ValkeyTls => "valkeys",
        ConnectionUrlFormat::Snowflake => "snowflake",
        ConnectionUrlFormat::ClickHouse => "clickhouse",
        ConnectionUrlFormat::ClickHouseTls => "clickhouses",
        ConnectionUrlFormat::Elasticsearch => "elasticsearch",
        ConnectionUrlFormat::ElasticsearchTls => "elasticsearchs",
        ConnectionUrlFormat::OpenSearch => "opensearch",
        ConnectionUrlFormat::OpenSearchTls => "opensearchs",
        _ => return Err(ProfileError::IncompatibleFormat),
    };
    let host = profile.host.as_deref().ok_or(ProfileError::MissingHost)?;
    let mut output = String::new();
    if matches!(
        format,
        ConnectionUrlFormat::JdbcPostgreSql
            | ConnectionUrlFormat::JdbcMySql
            | ConnectionUrlFormat::JdbcSqlServer
    ) {
        output.push_str("jdbc:");
    }
    output.push_str(scheme);
    output.push_str("://");
    if let Some(user) = profile
        .user
        .as_deref()
        .filter(|value| !value.is_empty())
        .filter(|_| profile.kind != DatabaseKind::OpenSearch)
        .filter(|_| {
            profile.kind != DatabaseKind::Elasticsearch
                || profile
                    .elasticsearch_options
                    .as_ref()
                    .is_none_or(|options| options.auth == ElasticsearchAuth::Basic)
        })
    {
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
    if profile.kind == DatabaseKind::Snowflake {
        validate_snowflake_endpoint(host, profile.port.unwrap_or(443), profile.ssl_mode)?;
        if profile
            .database
            .as_deref()
            .is_none_or(|database| database.trim().is_empty() || database.contains('/'))
        {
            return Err(ProfileError::InvalidQueryParameter("database".into()));
        }
        let fallback = SnowflakeConnectionOptions::default();
        let options = profile.snowflake_options.as_ref().unwrap_or(&fallback);
        options.validate()?;
        for (key, value) in [
            (
                "schema",
                profile.default_schema.as_deref().or(Some("PUBLIC")),
            ),
            ("warehouse", options.warehouse.as_deref()),
            ("role", options.role.as_deref()),
            ("token_type", Some(options.token_type.as_str())),
        ] {
            if let Some(value) = value {
                if value.trim().is_empty() {
                    return Err(ProfileError::InvalidQueryParameter(key.into()));
                }
                query.push(format!("{key}={}", utf8_percent_encode(value, QUERY_VALUE)));
            }
        }
    } else if profile.kind == DatabaseKind::Oracle {
        if let Some(user) = profile.user.as_deref().filter(|value| !value.is_empty()) {
            query.push(format!("user={}", utf8_percent_encode(user, QUERY_VALUE)));
        }
    } else if profile.kind.is_postgres_compatible() {
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
        if let Some(options) = &profile.pg_options {
            for (key, path) in [
                ("sslrootcert", &options.ssl_root_cert),
                ("sslcert", &options.ssl_client_cert),
                ("sslkey", &options.ssl_client_key),
            ] {
                if let Some(path) = path {
                    query.push(format!(
                        "{key}={}",
                        utf8_percent_encode(&path.to_string_lossy(), QUERY_VALUE)
                    ));
                }
            }
            if let Some(options) = &options.options {
                query.push(format!(
                    "options={}",
                    utf8_percent_encode(options, QUERY_VALUE)
                ));
            }
        }
    } else if matches!(
        profile.kind,
        DatabaseKind::MySql | DatabaseKind::MariaDb | DatabaseKind::TiDb
    ) {
        query.push(format!("sslMode={}", ssl_mode_value(profile.ssl_mode)));
    } else if profile.kind.is_redis_compatible() {
        // The redis/rediss and valkey/valkeys schemes are the TLS source of truth.
    } else if profile.kind.is_search_rest() {
        let expected_ssl_mode = if matches!(
            format,
            ConnectionUrlFormat::ElasticsearchTls | ConnectionUrlFormat::OpenSearchTls
        ) {
            SslMode::VerifyFull
        } else {
            SslMode::Disable
        };
        if profile.ssl_mode != expected_ssl_mode {
            return Err(ProfileError::ConflictingQueryParameter("ssl".to_owned()));
        }
        if profile.port == Some(0) {
            return Err(ProfileError::InvalidQueryParameter("port".into()));
        }
        if profile
            .default_schema
            .as_deref()
            .is_some_and(|schema| !schema.is_empty())
        {
            return Err(ProfileError::UnknownQueryParameter("schema".to_owned()));
        }
    } else if profile.kind == DatabaseKind::ClickHouse {
        if (format == ConnectionUrlFormat::ClickHouseTls) == (profile.ssl_mode == SslMode::Disable)
        {
            return Err(ProfileError::ConflictingQueryParameter("ssl".to_owned()));
        }
        if format == ConnectionUrlFormat::ClickHouseTls {
            query.push(format!("sslMode={}", ssl_mode_value(profile.ssl_mode)));
        }
    } else {
        if let Some(schema) = profile
            .default_schema
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            query.push(format!(
                "schema={}",
                utf8_percent_encode(schema, QUERY_VALUE)
            ));
        }
        append_sql_server_tls_query(&mut query, profile.ssl_mode);
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

fn format_mongodb_url(
    profile: &ConnectionProfile,
    format: ConnectionUrlFormat,
) -> Result<String, ProfileError> {
    let fallback_options = MongoConnectionOptions {
        seeds: vec![MongoSeed {
            host: profile.host.clone().ok_or(ProfileError::MissingHost)?,
            port: if format == ConnectionUrlFormat::MongoDbSrv {
                None
            } else {
                Some(profile.port.unwrap_or(27017))
            },
        }],
        srv: format == ConnectionUrlFormat::MongoDbSrv,
        uri_options: Vec::new(),
    };
    let options = profile
        .mongodb_options
        .as_ref()
        .unwrap_or(&fallback_options);
    let scheme = match format {
        ConnectionUrlFormat::MongoDb => "mongodb://",
        ConnectionUrlFormat::MongoDbSrv => "mongodb+srv://",
        _ => return Err(ProfileError::IncompatibleFormat),
    };
    if options.srv != (format == ConnectionUrlFormat::MongoDbSrv) {
        return Err(ProfileError::IncompatibleFormat);
    }
    let seeds = if options.seeds.is_empty() {
        &fallback_options.seeds
    } else {
        &options.seeds
    };
    let mut output = scheme.to_owned();
    if let Some(user) = profile.user.as_deref().filter(|user| !user.is_empty()) {
        output.push_str(&utf8_percent_encode(user, URL_COMPONENT).to_string());
        output.push('@');
    }
    for (index, seed) in seeds.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        if seed.host.contains(':') {
            output.push('[');
            output.push_str(&seed.host);
            output.push(']');
        } else {
            output.push_str(&seed.host);
        }
        if let Some(port) = seed.port {
            output.push(':');
            output.push_str(&port.to_string());
        }
    }
    if let Some(database) = profile.database.as_deref() {
        output.push('/');
        output.push_str(&utf8_percent_encode(database, URL_COMPONENT).to_string());
    }
    if !options.uri_options.is_empty() {
        output.push('?');
        for (index, (key, value)) in options.uri_options.iter().enumerate() {
            if index > 0 {
                output.push('&');
            }
            output.push_str(&utf8_percent_encode(key, QUERY_VALUE).to_string());
            output.push('=');
            output.push_str(&utf8_percent_encode(value, QUERY_VALUE).to_string());
        }
    }
    Ok(output)
}

fn format_jdbc_sql_server_url(profile: &ConnectionProfile) -> Result<String, ProfileError> {
    let host = profile.host.as_deref().ok_or(ProfileError::MissingHost)?;
    let mut output = "jdbc:sqlserver://".to_owned();
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
        append_jdbc_property(&mut output, "databaseName", database);
    }
    if let Some(user) = profile.user.as_deref().filter(|value| !value.is_empty()) {
        append_jdbc_property(&mut output, "user", user);
    }
    if let Some(schema) = profile
        .default_schema
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        append_jdbc_property(&mut output, "currentSchema", schema);
    }
    match profile.ssl_mode {
        SslMode::Disable => append_jdbc_property(&mut output, "encrypt", "false"),
        SslMode::Prefer => {}
        SslMode::Require => {
            append_jdbc_property(&mut output, "encrypt", "true");
            append_jdbc_property(&mut output, "trustServerCertificate", "true");
        }
        SslMode::VerifyCa | SslMode::VerifyFull => {
            append_jdbc_property(&mut output, "encrypt", "true");
            append_jdbc_property(&mut output, "trustServerCertificate", "false");
        }
    }
    if profile.read_only {
        append_jdbc_property(&mut output, "readOnly", "true");
    }
    Ok(output)
}

fn append_sql_server_tls_query(query: &mut Vec<String>, ssl_mode: SslMode) {
    match ssl_mode {
        SslMode::Disable => query.push("encrypt=false".to_owned()),
        SslMode::Prefer => {}
        SslMode::Require => {
            query.push("encrypt=true".to_owned());
            query.push("trustServerCertificate=true".to_owned());
        }
        SslMode::VerifyCa | SslMode::VerifyFull => {
            query.push("encrypt=true".to_owned());
            query.push("trustServerCertificate=false".to_owned());
        }
    }
}

fn append_jdbc_property(output: &mut String, key: &str, value: &str) {
    output.push(';');
    output.push_str(key);
    output.push('=');
    if value.contains([';', '=']) || value.starts_with('{') || value.ends_with('}') {
        output.push('{');
        output.push_str(&value.replace('}', "}}"));
        output.push('}');
    } else {
        output.push_str(value);
    }
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

fn reject_option_duplicate<T>(value: &Option<T>, name: &str) -> Result<(), ProfileError> {
    if value.is_some() {
        Err(ProfileError::ConflictingQueryParameter(name.to_owned()))
    } else {
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
    use super::{
        CatalogScope, DatabaseKind, SQLITE_PATH, SslMode, format_connection_url,
        import_connection_url,
    };
    use percent_encoding::utf8_percent_encode;
    use std::path::Path;

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
    fn attached_database_scope_inherits_schema_restriction_without_widening_it() {
        let scope = CatalogScope::for_profile(DatabaseKind::DuckDb, "primary", Some("main"))
            .with_attached_databases("primary", ["primary".into(), "attached".into()]);
        assert!(scope.allows_database("primary"));
        assert!(scope.allows_database("attached"));
        assert!(!scope.allows_database("unknown"));
        assert!(scope.allows_schema("attached", "main"));
        assert!(!scope.allows_schema("attached", "other"));
        let scope = CatalogScope::for_profile(DatabaseKind::DuckDb, "primary", Some("main"))
            .with_discovered_databases("primary", ["temp".into()]);
        assert!(scope.allows_database("temp"));
        assert!(scope.allows_schema("temp", "main"));
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
    fn imports_and_exports_duckdb_paths_and_options() {
        let imported = import_connection_url(
            "duckdb:/tmp/analytics%20data.duckdb?read_only=true&schema=raw",
            None,
        )
        .unwrap();
        assert_eq!(imported.profile.kind, DatabaseKind::DuckDb);
        assert_eq!(
            imported.profile.duckdb_path.as_deref(),
            Some(std::path::Path::new("/tmp/analytics data.duckdb"))
        );
        assert_eq!(imported.profile.database.as_deref(), Some("analytics data"));
        assert_eq!(imported.profile.default_schema.as_deref(), Some("raw"));
        assert!(imported.profile.read_only);
        let exported =
            format_connection_url(&imported.profile, imported.profile.url_format).unwrap();
        assert_eq!(
            exported,
            "duckdb:/tmp/analytics%20data.duckdb?read_only=true&schema=raw"
        );

        let memory = import_connection_url("duckdb::memory:", None).unwrap();
        assert_eq!(memory.profile.kind, DatabaseKind::DuckDb);
        assert!(memory.profile.duckdb_path.is_none());
        assert_eq!(memory.profile.database.as_deref(), Some("memory"));
        assert_eq!(
            format_connection_url(&memory.profile, memory.profile.url_format).unwrap(),
            "duckdb::memory:"
        );

        for path in ["duckdb:relative/data.duckdb", "duckdb:/absolute.duckdb"] {
            let profile = import_connection_url(path, None).unwrap().profile;
            let exported = format_connection_url(&profile, profile.url_format).unwrap();
            assert_eq!(exported, path);
        }

        for (path, canonical) in [
            (
                "duckdb:目录/a%25b%23c%3Fd.duckdb",
                "duckdb:%E7%9B%AE%E5%BD%95/a%25b%23c%3Fd.duckdb",
            ),
            (
                "duckdb:/tmp/目录/a%25b%23c%3Fd.duckdb",
                "duckdb:/tmp/%E7%9B%AE%E5%BD%95/a%25b%23c%3Fd.duckdb",
            ),
            (
                "duckdb:C%3A/Users/example/data.duckdb",
                "duckdb:C:/Users/example/data.duckdb",
            ),
        ] {
            let profile = import_connection_url(path, None).unwrap().profile;
            assert_eq!(
                format_connection_url(&profile, profile.url_format).unwrap(),
                canonical
            );
        }

        for path in [
            r"C:\Users\runner\AppData\Local\Temp\duckdb file.duckdb",
            r"\\?\C:\Users\runner\AppData\Local\Temp\duckdb file.duckdb",
            r"\\?\UNC\server\share\duckdb file.duckdb",
        ] {
            let encoded: String = utf8_percent_encode(path, SQLITE_PATH).to_string();
            let imported = import_connection_url(&format!("duckdb:{encoded}"), None).unwrap();
            assert_eq!(
                imported.profile.duckdb_path.as_deref(),
                Some(Path::new(path))
            );
        }
    }

    #[test]
    fn rejects_invalid_duckdb_options_and_remote_urls() {
        for url in [
            "duckdb:/tmp/data.duckdb?unknown=true",
            "duckdb:/tmp/data.duckdb?read_only=yes",
            "duckdb:/tmp/data.duckdb?schema=a&schema=b",
            "duckdb:remote.example/data.duckdb",
            "duckdb::memory:?read_only=true",
        ] {
            assert!(import_connection_url(url, None).is_err(), "accepted {url}");
        }
    }

    #[test]
    fn older_profile_payload_without_duckdb_path_remains_readable() {
        let mut value = serde_json::to_value(
            import_connection_url("sqlite:/tmp/demo.db", None)
                .unwrap()
                .profile,
        )
        .unwrap();
        value.as_object_mut().unwrap().remove("duckdb_path");
        let profile: super::ConnectionProfile = serde_json::from_value(value).unwrap();
        assert_eq!(profile.kind, DatabaseKind::Sqlite);
        assert!(profile.duckdb_path.is_none());
    }

    #[test]
    fn rejects_unknown_schemes() {
        let error = import_connection_url("oracle://db.example.com/app", None).unwrap_err();

        assert!(error.to_string().contains("unsupported database scheme"));
    }
}
