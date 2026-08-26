use std::{collections::HashSet, path::PathBuf};

use percent_encoding::percent_decode_str;
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionProfile {
    pub id: Uuid,
    pub name: String,
    pub kind: DatabaseKind,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub database: Option<String>,
    pub default_schema: Option<String>,
    pub sqlite_path: Option<PathBuf>,
    #[serde(default)]
    pub ssl_mode: SslMode,
    pub secret_ref: Option<String>,
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
}

pub fn import_connection_url(
    input: &str,
    preferred_name: Option<&str>,
) -> Result<ImportedProfile, ProfileError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(ProfileError::Empty);
    }

    let normalized = input.strip_prefix("jdbc:").unwrap_or(input);
    if normalized == ":memory:"
        || normalized == "sqlite::memory:"
        || normalized.starts_with("sqlite::memory:?")
    {
        return Ok(sqlite_profile(
            None,
            true,
            preferred_name.unwrap_or("memory"),
            false,
        ));
    }

    if normalized.starts_with("sqlite:") || normalized.starts_with("file:") {
        return import_sqlite(normalized, preferred_name);
    }

    let url = Url::parse(normalized)?;
    match url.scheme().to_ascii_lowercase().as_str() {
        "postgres" | "postgresql" => import_server_url(url, DatabaseKind::Postgres, preferred_name),
        "mysql" => import_server_url(url, DatabaseKind::MySql, preferred_name),
        scheme => Err(ProfileError::UnsupportedScheme(scheme.to_owned())),
    }
}

fn import_server_url(
    url: Url,
    kind: DatabaseKind,
    preferred_name: Option<&str>,
) -> Result<ImportedProfile, ProfileError> {
    let host = url.host_str().ok_or(ProfileError::MissingHost)?.to_owned();
    let database = decode(url.path().trim_start_matches('/'));
    let database = (!database.is_empty()).then_some(database);
    let user = (!url.username().is_empty()).then(|| decode(url.username()));
    let password = url
        .password()
        .map(|value| SecretString::from(decode(value)));
    let mut default_schema = None;
    let mut ssl_mode = SslMode::Prefer;

    for (key, value) in url.query_pairs() {
        if key.eq_ignore_ascii_case("currentSchema") {
            default_schema = Some(value.into_owned());
        } else if key.eq_ignore_ascii_case("sslmode") {
            ssl_mode = parse_ssl_mode(&value, ssl_mode);
        } else if key.eq_ignore_ascii_case("useSSL") {
            ssl_mode = if value.eq_ignore_ascii_case("true") {
                SslMode::Require
            } else {
                SslMode::Disable
            };
        } else if key.eq_ignore_ascii_case("sslMode") {
            ssl_mode = parse_ssl_mode(&value, ssl_mode);
        }
    }

    let default_port = match kind {
        DatabaseKind::Postgres => 5432,
        DatabaseKind::MySql => 3306,
        DatabaseKind::Sqlite => unreachable!("server URL cannot be SQLite"),
    };
    let derived_name = database.as_deref().unwrap_or(&host);
    let catalog_scope = CatalogScope::for_profile(
        kind,
        database.as_deref().unwrap_or_default(),
        default_schema.as_deref(),
    );

    Ok(ImportedProfile {
        profile: ConnectionProfile {
            id: Uuid::new_v4(),
            name: choose_name(preferred_name, derived_name),
            kind,
            host: Some(host),
            port: Some(url.port().unwrap_or(default_port)),
            user,
            database,
            default_schema,
            sqlite_path: None,
            ssl_mode,
            secret_ref: None,
            read_only: false,
            environment: Environment::Development,
            catalog_scope,
        },
        transient_password: password,
    })
}

fn import_sqlite(
    input: &str,
    preferred_name: Option<&str>,
) -> Result<ImportedProfile, ProfileError> {
    let (path_and_prefix, query) = input.split_once('?').unwrap_or((input, ""));
    let raw_path = if let Some(rest) = path_and_prefix.strip_prefix("sqlite:") {
        rest.strip_prefix("//").unwrap_or(rest)
    } else {
        path_and_prefix
            .strip_prefix("file:")
            .unwrap_or(path_and_prefix)
    };
    let decoded = decode(raw_path);
    if decoded.is_empty() {
        return Err(ProfileError::MissingSqlitePath);
    }
    if decoded == ":memory:" {
        return Ok(sqlite_profile(
            None,
            true,
            preferred_name.unwrap_or("memory"),
            false,
        ));
    }

    let read_only = url::form_urlencoded::parse(query.as_bytes())
        .any(|(key, value)| key.eq_ignore_ascii_case("mode") && value.eq_ignore_ascii_case("ro"));
    let path = PathBuf::from(decoded);
    let derived_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("sqlite")
        .to_owned();
    let name = choose_name(preferred_name, &derived_name);

    Ok(sqlite_profile(Some(path), false, &name, read_only))
}

fn sqlite_profile(
    path: Option<PathBuf>,
    in_memory: bool,
    name: &str,
    read_only: bool,
) -> ImportedProfile {
    let database = if in_memory {
        Some(":memory:".to_owned())
    } else {
        path.as_ref()
            .map(|value| value.to_string_lossy().into_owned())
    };
    let catalog_scope = CatalogScope::for_profile(
        DatabaseKind::Sqlite,
        database.as_deref().unwrap_or(":memory:"),
        Some("main"),
    );

    ImportedProfile {
        profile: ConnectionProfile {
            id: Uuid::new_v4(),
            name: name.to_owned(),
            kind: DatabaseKind::Sqlite,
            host: None,
            port: None,
            user: None,
            database,
            default_schema: Some("main".to_owned()),
            sqlite_path: path,
            ssl_mode: SslMode::Disable,
            secret_ref: None,
            read_only,
            environment: Environment::Development,
            catalog_scope,
        },
        transient_password: None,
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

fn parse_ssl_mode(value: &str, fallback: SslMode) -> SslMode {
    match value.to_ascii_lowercase().replace('_', "-").as_str() {
        "disable" | "disabled" | "false" => SslMode::Disable,
        "prefer" | "preferred" => SslMode::Prefer,
        "require" | "required" | "true" => SslMode::Require,
        "verify-ca" | "verify-ca-certificate" => SslMode::VerifyCa,
        "verify-full" | "verify-identity" => SslMode::VerifyFull,
        _ => fallback,
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
