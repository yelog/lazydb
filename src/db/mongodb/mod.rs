use std::time::Duration;
use tokio::time::Instant;

pub mod catalog;
pub mod management;
pub mod monitor;
pub mod mutation;
pub mod principal;
pub mod read;
pub mod transaction;
pub mod types;

use mongodb::{
    Client,
    options::{ClientOptions, Tls, TlsOptions},
};
use secrecy::{ExposeSecret, SecretString};

use crate::{
    db::{DatabaseError, ErrorCategory, ServerInfo},
    profile::{ConnectionProfile, DatabaseKind, MongoConnectionOptions, SslMode},
};

#[derive(Clone, Debug)]
pub struct MongoAdapter {
    client: Client,
    database: String,
}

impl MongoAdapter {
    pub fn client(&self) -> &Client {
        &self.client
    }

    pub(super) fn selected_database(&self) -> &str {
        &self.database
    }

    pub async fn list_database_names(&self) -> Result<Vec<String>, DatabaseError> {
        self.list_databases().await
    }
}

impl MongoAdapter {
    pub async fn connect(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
    ) -> Result<Self, DatabaseError> {
        let uri_options = profile.mongodb_options.as_ref().ok_or_else(|| {
            DatabaseError::configuration("MongoDB connection options are missing")
        })?;
        let database = profile
            .database
            .clone()
            .unwrap_or_else(|| "test".to_owned());
        if database.contains('\0') || database.contains('.') {
            return Err(DatabaseError::configuration(
                "MongoDB database name is invalid",
            ));
        }
        let uri = build_uri(profile, uri_options, password)?;
        let mut options = ClientOptions::parse(&uri)
            .await
            .map_err(|_| DatabaseError::configuration("MongoDB connection options are invalid"))?;
        if password.is_some() {
            options.credential = Some(
                mongodb::options::Credential::builder()
                    .username(profile.user.clone())
                    .password(password.map(|value| value.expose_secret().to_owned()))
                    .source(
                        uri_options
                            .uri_options
                            .iter()
                            .find(|(key, _)| key.eq_ignore_ascii_case("authSource"))
                            .map(|(_, value)| value.clone()),
                    )
                    .build(),
            );
        }
        options.connect_timeout = Some(Duration::from_secs(5));
        options.server_selection_timeout = Some(Duration::from_secs(10));
        match profile.ssl_mode {
            SslMode::Disable => options.tls = Some(Tls::Disabled),
            SslMode::Prefer => {}
            SslMode::Require => {
                options.tls = Some(TlsOptions::builder().build().into());
            }
            SslMode::VerifyCa | SslMode::VerifyFull => {
                let ca_file = uri_options
                    .uri_options
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case("tlsCAFile"))
                    .map(|(_, ca_file)| std::path::PathBuf::from(ca_file));
                let tls = TlsOptions::builder().ca_file_path(ca_file).build();
                options.tls = Some(tls.into());
            }
        }
        let client = Client::with_options(options)
            .map_err(|_| DatabaseError::configuration("MongoDB client configuration is invalid"))?;
        client
            .database(&database)
            .run_command(mongodb::bson::doc! { "ping": 1 })
            .await
            .map_err(map_error)?;
        Ok(Self { client, database })
    }

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        let version = self
            .client
            .database(&self.database)
            .run_command(mongodb::bson::doc! { "buildInfo": 1 })
            .await
            .ok()
            .and_then(|reply| reply.get_str("version").ok().map(str::to_owned))
            .unwrap_or_else(|| "unknown".to_owned());
        Ok(ServerInfo {
            kind: DatabaseKind::MongoDb,
            version,
            database: self.database.clone(),
            current_user: None,
        })
    }

    pub async fn replace_document(
        &self,
        namespace: &types::MongoNamespace,
        id: mongodb::bson::Bson,
        expected: &mongodb::bson::Document,
        replacement: mongodb::bson::Document,
    ) -> Result<(), DatabaseError> {
        namespace.collection.as_deref().ok_or_else(|| {
            DatabaseError::configuration("MongoDB collection namespace is required")
        })?;
        if !self
            .compare_and_replace_document(namespace, id, expected, replacement)
            .await?
        {
            return Err(DatabaseError::configuration(
                "document changed during replacement",
            ));
        }
        Ok(())
    }

    pub async fn insert_documents(
        &self,
        namespace: &types::MongoNamespace,
        documents: Vec<mongodb::bson::Document>,
    ) -> Result<u64, DatabaseError> {
        match self.insert_documents_detailed(namespace, documents).await? {
            mutation::MongoMutationOutcome::Confirmed { affected } => Ok(affected),
            mutation::MongoMutationOutcome::Partial { affected, .. } => Ok(affected),
            mutation::MongoMutationOutcome::Unknown { .. } => Err(DatabaseError {
                category: ErrorCategory::Network,
                code: Some("mongodb_write_outcome_unknown".into()),
                message:
                    "MongoDB insert outcome is unknown; inspect the collection before retrying"
                        .into(),
                diagnostic: None,
            }),
        }
    }

    pub async fn insert_documents_detailed(
        &self,
        namespace: &types::MongoNamespace,
        documents: Vec<mongodb::bson::Document>,
    ) -> Result<mutation::MongoMutationOutcome, DatabaseError> {
        if documents.is_empty() || documents.len() > 500 {
            return Err(DatabaseError::configuration(
                "insert accepts 1..=500 documents",
            ));
        }
        let collection = namespace.collection.as_deref().ok_or_else(|| {
            DatabaseError::configuration("MongoDB collection namespace is required")
        })?;
        let count = documents.len() as u64;
        let result = self
            .client
            .database(&namespace.database)
            .collection::<mongodb::bson::Document>(collection)
            .insert_many(documents)
            .await;
        match result {
            Ok(result) => Ok(mutation::MongoMutationOutcome::Confirmed {
                affected: result.inserted_ids.len() as u64,
            }),
            Err(error) if mutation::error_is_definitely_rejected(&error) => Err(map_error(error)),
            Err(error) => match error.kind.as_ref() {
                mongodb::error::ErrorKind::InsertMany(bulk)
                    if bulk.write_concern_error.is_none() && bulk.write_errors.is_some() =>
                {
                    let errors = bulk.write_errors.as_ref().expect("checked above");
                    let failed = errors
                        .iter()
                        .map(|error| error.index)
                        .collect::<std::collections::HashSet<_>>();
                    let affected = count.saturating_sub(failed.len() as u64);
                    if affected == 0 {
                        Ok(mutation::MongoMutationOutcome::Confirmed { affected: 0 })
                    } else if affected < count {
                        Ok(mutation::MongoMutationOutcome::Partial {
                            affected,
                            requested: count,
                        })
                    } else {
                        Ok(mutation::MongoMutationOutcome::Unknown { requested: count })
                    }
                }
                _ => Ok(mutation::MongoMutationOutcome::Unknown { requested: count }),
            },
        }
    }

    pub async fn delete_document(
        &self,
        namespace: &types::MongoNamespace,
        id: mongodb::bson::Bson,
        expected: &mongodb::bson::Document,
    ) -> Result<bool, DatabaseError> {
        match self
            .delete_document_detailed(namespace, id, expected)
            .await?
        {
            mutation::MongoMutationOutcome::Confirmed { affected } => Ok(affected == 1),
            mutation::MongoMutationOutcome::Partial { .. }
            | mutation::MongoMutationOutcome::Unknown { .. } => Err(unknown_write_error("delete")),
        }
    }

    pub async fn delete_document_detailed(
        &self,
        namespace: &types::MongoNamespace,
        id: mongodb::bson::Bson,
        expected: &mongodb::bson::Document,
    ) -> Result<mutation::MongoMutationOutcome, DatabaseError> {
        let collection = namespace.collection.as_deref().ok_or_else(|| {
            DatabaseError::configuration("MongoDB collection namespace is required")
        })?;
        let filter = exact_document_guard(id, expected);
        let result = self
            .client
            .database(&namespace.database)
            .collection::<mongodb::bson::Document>(collection)
            .delete_one(filter)
            .await;
        match result {
            Ok(result) => Ok(mutation::MongoMutationOutcome::Confirmed {
                affected: result.deleted_count,
            }),
            Err(error) if matches!(*error.kind, mongodb::error::ErrorKind::Command(ref command) if command.code == 112) => {
                Ok(mutation::MongoMutationOutcome::Confirmed { affected: 0 })
            }
            Err(error) if mutation::error_is_definitely_rejected(&error) => Err(map_error(error)),
            Err(_error) => Ok(mutation::MongoMutationOutcome::Unknown { requested: 1 }),
        }
    }

    pub async fn compare_and_replace_document(
        &self,
        namespace: &types::MongoNamespace,
        id: mongodb::bson::Bson,
        expected: &mongodb::bson::Document,
        replacement: mongodb::bson::Document,
    ) -> Result<bool, DatabaseError> {
        match self
            .compare_and_replace_document_detailed(namespace, id, expected, replacement)
            .await?
        {
            mutation::MongoMutationOutcome::Confirmed { affected } => Ok(affected == 1),
            mutation::MongoMutationOutcome::Partial { .. }
            | mutation::MongoMutationOutcome::Unknown { .. } => {
                Err(unknown_write_error("replacement"))
            }
        }
    }

    pub async fn compare_and_replace_document_detailed(
        &self,
        namespace: &types::MongoNamespace,
        id: mongodb::bson::Bson,
        expected: &mongodb::bson::Document,
        replacement: mongodb::bson::Document,
    ) -> Result<mutation::MongoMutationOutcome, DatabaseError> {
        let collection = namespace.collection.as_deref().ok_or_else(|| {
            DatabaseError::configuration("MongoDB collection namespace is required")
        })?;
        let filter = exact_document_guard(id, expected);
        let result = self
            .client
            .database(&namespace.database)
            .collection::<mongodb::bson::Document>(collection)
            .replace_one(filter, replacement)
            .await;
        match result {
            Ok(result) => Ok(
                match mutation::compare_and_set_outcome(
                    result.matched_count,
                    Some(result.modified_count),
                ) {
                    mutation::MongoCompareAndSetOutcome::Applied => {
                        mutation::MongoMutationOutcome::Confirmed { affected: 1 }
                    }
                    mutation::MongoCompareAndSetOutcome::Conflict => {
                        mutation::MongoMutationOutcome::Confirmed { affected: 0 }
                    }
                    mutation::MongoCompareAndSetOutcome::Unknown => {
                        mutation::MongoMutationOutcome::Unknown { requested: 1 }
                    }
                },
            ),
            Err(error) if matches!(*error.kind, mongodb::error::ErrorKind::Command(ref command) if command.code == 112) => {
                Ok(mutation::MongoMutationOutcome::Confirmed { affected: 0 })
            }
            Err(error) if mutation::error_is_definitely_rejected(&error) => Err(map_error(error)),
            Err(_error) => Ok(mutation::MongoMutationOutcome::Unknown { requested: 1 }),
        }
    }

    pub async fn close(self) {
        self.close_until(Instant::now() + Duration::from_secs(1))
            .await;
    }

    pub(crate) async fn close_until(self, deadline: Instant) {
        if tokio::time::timeout_at(deadline, self.client.shutdown())
            .await
            .is_err()
        {
            tracing::debug!("MongoDB client shutdown deadline reached; remote cleanup unconfirmed");
        }
    }
}

fn build_uri(
    profile: &ConnectionProfile,
    config: &MongoConnectionOptions,
    password: Option<&SecretString>,
) -> Result<String, DatabaseError> {
    if config.seeds.is_empty() {
        return Err(DatabaseError::configuration("MongoDB host is required"));
    }
    if config.srv != (profile.url_format == crate::profile::ConnectionUrlFormat::MongoDbSrv) {
        return Err(DatabaseError::configuration(
            "MongoDB URI format does not match its seed options",
        ));
    }
    if config.srv && (config.seeds.len() != 1 || config.seeds[0].port.is_some()) {
        return Err(DatabaseError::configuration(
            "MongoDB SRV requires one seed without a port",
        ));
    }
    let scheme = if config.srv {
        "mongodb+srv://"
    } else {
        "mongodb://"
    };
    let mut uri = scheme.to_owned();
    if let Some(user) = profile.user.as_deref().filter(|user| !user.is_empty()) {
        uri.push_str(&encode_uri_component(user));
        if let Some(password) = password {
            uri.push(':');
            uri.push_str(&encode_uri_component(password.expose_secret()));
        }
        uri.push('@');
    } else if password.is_some() {
        return Err(DatabaseError::configuration(
            "MongoDB password requires a username",
        ));
    }
    for (index, seed) in config.seeds.iter().enumerate() {
        if index > 0 {
            uri.push(',');
        }
        if seed.host.is_empty() || seed.host.contains('/') || seed.host.contains('@') {
            return Err(DatabaseError::configuration("MongoDB seed host is invalid"));
        }
        if seed.host.contains(':') {
            uri.push('[');
            uri.push_str(&seed.host);
            uri.push(']');
        } else {
            uri.push_str(&seed.host);
        }
        if let Some(port) = seed.port {
            uri.push(':');
            uri.push_str(&port.to_string());
        }
    }
    if let Some(database) = profile
        .database
        .as_deref()
        .filter(|database| !database.is_empty())
    {
        uri.push('/');
        uri.push_str(&encode_uri_component(database));
    }
    if !config.uri_options.is_empty() {
        uri.push('?');
        for (index, (key, value)) in config.uri_options.iter().enumerate() {
            if index > 0 {
                uri.push('&');
            }
            uri.push_str(&encode_uri_component(key));
            uri.push('=');
            uri.push_str(&encode_uri_component(value));
        }
    }
    Ok(uri)
}

fn encode_uri_component(input: &str) -> String {
    percent_encoding::utf8_percent_encode(input, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn exact_document_guard(
    id: mongodb::bson::Bson,
    expected: &mongodb::bson::Document,
) -> mongodb::bson::Document {
    mongodb::bson::doc! {
        "_id": {"$eq": id},
        "$expr": {"$eq": ["$$ROOT", {"$literal": expected.clone()}]}
    }
}

fn unknown_write_error(operation: &str) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Network,
        code: Some("mongodb_write_outcome_unknown".to_owned()),
        message: format!(
            "MongoDB {operation} outcome is unknown; inspect the collection before retrying"
        ),
        diagnostic: None,
    }
}

fn map_error(error: mongodb::error::Error) -> DatabaseError {
    let safe = error.to_string().to_ascii_lowercase();
    let category = if safe.contains("not authorized") || safe.contains("unauthorized") {
        ErrorCategory::Permission
    } else if safe.contains("authentication failed") || safe.contains("auth error") {
        ErrorCategory::Authentication
    } else if safe.contains("invalid") || safe.contains("parse") {
        ErrorCategory::Configuration
    } else {
        ErrorCategory::Network
    };
    DatabaseError {
        category,
        code: Some("mongodb_error".to_owned()),
        message: "MongoDB operation failed; inspect the error code and connection settings"
            .to_owned(),
        diagnostic: None,
    }
}

#[cfg(test)]
mod tests {
    use super::build_uri;
    use crate::profile::{
        CatalogScope, ConnectionProfile, ConnectionUrlFormat, CredentialPolicy, DatabaseKind,
        Environment, MongoConnectionOptions, MongoSeed, ProfileAccess, SslMode,
    };
    use secrecy::SecretString;
    use uuid::Uuid;

    fn profile(options: MongoConnectionOptions) -> ConnectionProfile {
        ConnectionProfile {
            id: Uuid::new_v4(),
            name: "mongo".to_owned(),
            access: ProfileAccess::Global,
            group_id: None,
            kind: DatabaseKind::MongoDb,
            url_format: if options.srv {
                ConnectionUrlFormat::MongoDbSrv
            } else {
                ConnectionUrlFormat::MongoDb
            },
            host: options.seeds.first().map(|seed| seed.host.clone()),
            port: options.seeds.first().and_then(|seed| seed.port),
            user: Some("user@name".to_owned()),
            database: Some("app".to_owned()),
            default_schema: None,
            sqlite_path: None,
            duckdb_path: None,
            mongodb_options: Some(options),
            ssl_mode: SslMode::Prefer,
            credential_policy: CredentialPolicy::None,
            read_only: false,
            environment: Environment::Development,
            catalog_scope: CatalogScope::for_profile(DatabaseKind::MongoDb, "app", None),
        }
    }

    #[test]
    fn uri_preserves_seed_order_options_and_escapes_credentials_without_logging_them() {
        let options = MongoConnectionOptions {
            seeds: vec![
                MongoSeed {
                    host: "db-a".to_owned(),
                    port: Some(27017),
                },
                MongoSeed {
                    host: "db-b".to_owned(),
                    port: Some(27018),
                },
            ],
            srv: false,
            uri_options: vec![("replicaSet".to_owned(), "rs 0".to_owned())],
        };
        let profile = profile(options);
        let password = SecretString::from("p@ss:word");
        let uri = build_uri(
            &profile,
            profile.mongodb_options.as_ref().unwrap(),
            Some(&password),
        )
        .unwrap();
        assert!(uri.starts_with("mongodb://user%40name:p%40ss%3Aword@db-a:27017,db-b:27018/app?"));
        assert!(uri.contains("replicaSet=rs%200"));
        assert!(!format!("{profile:?}").contains("p@ss:word"));
    }

    #[test]
    fn srv_uri_omits_port_and_invalid_seed_config_fails_closed() {
        let options = MongoConnectionOptions {
            seeds: vec![MongoSeed {
                host: "cluster.example.test".to_owned(),
                port: None,
            }],
            srv: true,
            uri_options: Vec::new(),
        };
        let srv_profile = profile(options);
        assert_eq!(
            build_uri(
                &srv_profile,
                srv_profile.mongodb_options.as_ref().unwrap(),
                None
            )
            .unwrap(),
            "mongodb+srv://user%40name@cluster.example.test/app"
        );

        let invalid = MongoConnectionOptions {
            seeds: vec![MongoSeed {
                host: "cluster.example.test".to_owned(),
                port: Some(27017),
            }],
            srv: true,
            uri_options: Vec::new(),
        };
        let invalid_profile = profile(invalid);
        assert!(
            build_uri(
                &invalid_profile,
                invalid_profile.mongodb_options.as_ref().unwrap(),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn absent_standard_port_uses_driver_default_without_emitting_empty_host_port() {
        let options = MongoConnectionOptions {
            seeds: vec![MongoSeed {
                host: "db.example.test".to_owned(),
                port: None,
            }],
            srv: false,
            uri_options: Vec::new(),
        };
        let mut profile = profile(options);
        profile.port = None;
        assert_eq!(
            build_uri(&profile, profile.mongodb_options.as_ref().unwrap(), None).unwrap(),
            "mongodb://user%40name@db.example.test/app"
        );
    }
}
