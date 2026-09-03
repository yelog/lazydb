use std::{
    collections::{HashMap, HashSet},
    fmt,
    ops::{Deref, DerefMut},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use futures_util::TryStreamExt;
use secrecy::{ExposeSecret, SecretString};
use tiberius::{
    AuthMethod, Client, ColumnData, ColumnType, Config, EncryptionLevel, FromSqlOwned, Query,
    QueryItem, Row,
};
use tokio::{net::TcpStream, sync::OwnedSemaphorePermit};
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};
use uuid::Uuid;

use super::mutation::{InputValue, MutationResult, RelationMutation, RelationMutationRequest};
use super::transaction::{TransactionBackend, TransactionError};
use super::{
    DatabaseError, ErrorCategory, RelationPreview, ServerInfo,
    catalog::{
        CatalogCapabilities, CatalogCount, CatalogDiscovery, CatalogEntry, CatalogGroupSummary,
        CatalogId, CatalogKind, CatalogMetadata, CatalogPage, CatalogRequest, CatalogRequestKey,
        CatalogSearchHit, CatalogSearchPage, CatalogSearchRequest, CatalogTarget, ColumnMetadata,
        ColumnMetadataCapabilities, ConstraintMembership, ConstraintMetadata, DdlProvenance,
        DiscoveredDatabase, IndexMetadata, NamespaceModel, ObjectGroup, OptionalMetadata,
        QualifiedName, RelationDdl, finalize_keyset_page,
    },
    catalog_drop::{CatalogDropError, CatalogDropPlan, CatalogDropRequest},
    catalog_mutation::CatalogMutationCapabilities,
    ddl::{DdlSection, assemble_ddl},
    monitor::{MonitorMetadata, MonitorSnapshot, ProcessSnapshot},
    query::{ColumnMeta, QueryOutcome, QueryStats, RELATION_PREVIEW_LIMIT, ResultSet},
    value::CellValue,
};
use crate::{
    identity::ConnectionIdentity,
    profile::{CatalogScope, ConnectionProfile, DatabaseKind, SslMode},
};
use futures_util::future::BoxFuture;

const DEFAULT_MAX_CONNECTIONS: usize = 4;
const UNSUPPORTED_MESSAGE: &str = "SQL Server support is not implemented yet";
pub const DASHBOARD_UNSUPPORTED_CODE: &str = "sql_server_dashboard_unsupported";
pub const DASHBOARD_UNSUPPORTED_MESSAGE: &str = "SQL Server dashboard metrics are not supported";
pub const PROCESS_METRICS_UNSUPPORTED_CODE: &str = "sql_server_process_metrics_unsupported";
pub const PROCESS_METRICS_UNSUPPORTED_MESSAGE: &str =
    "SQL Server process metrics are not supported";
const PROBE_SQL: &str = "SELECT CAST(SERVERPROPERTY('ProductVersion') AS nvarchar(128)) AS [version], DB_NAME() AS [current_database]";
const DATABASES_SQL: &str = "SELECT [name] FROM sys.databases WHERE [state] = 0 AND HAS_DBACCESS([name]) = 1 ORDER BY [name]";
const UNSUPPORTED_PREVIEW_LEN: usize = 64;

pub(crate) type TdsClient = Client<Compat<TcpStream>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MsSqlTlsMode {
    Plaintext,
    RequiredUnverified,
    RequiredVerified,
}

#[derive(Clone)]
pub(crate) struct MsSqlConnectSettings {
    host: String,
    port: u16,
    user: String,
    password: SecretString,
    database: String,
    tls_mode: MsSqlTlsMode,
    read_only: bool,
}

impl fmt::Debug for MsSqlConnectSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MsSqlConnectSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &"<redacted>")
            .field("database", &self.database)
            .field("tls_mode", &self.tls_mode)
            .field("read_only", &self.read_only)
            .finish()
    }
}

#[allow(dead_code)]
impl MsSqlConnectSettings {
    pub(crate) fn from_profile(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
    ) -> Result<Self, DatabaseError> {
        if profile.kind != DatabaseKind::SqlServer {
            return Err(DatabaseError::configuration("profile is not SQL Server"));
        }
        if profile.sqlite_path.is_some() {
            return Err(DatabaseError::configuration(
                "SQL Server profile cannot contain a SQLite database path",
            ));
        }

        let required = |value: &Option<String>, field: &str| {
            value
                .as_deref()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| {
                    DatabaseError::configuration(format!("SQL Server profile has no {field}"))
                })
        };

        let host = required(&profile.host, "host")?;
        let port = profile
            .port
            .ok_or_else(|| DatabaseError::configuration("SQL Server profile has no port"))?;
        let user = required(&profile.user, "user")?;
        let database = required(&profile.database, "database")?;
        let password = password
            .cloned()
            .ok_or_else(|| DatabaseError::configuration("SQL Server profile has no password"))?;

        // Tiberius cannot safely express "verified TLS, otherwise plaintext". Prefer is
        // therefore fail-closed and behaves like VerifyFull. Its Rustls backend also has no
        // CA-only mode, so VerifyCa performs both chain and hostname verification.
        let tls_mode = match profile.ssl_mode {
            SslMode::Disable => MsSqlTlsMode::Plaintext,
            SslMode::Require => MsSqlTlsMode::RequiredUnverified,
            SslMode::Prefer | SslMode::VerifyCa | SslMode::VerifyFull => {
                MsSqlTlsMode::RequiredVerified
            }
        };

        Ok(Self {
            host,
            port,
            user,
            password,
            database,
            tls_mode,
            read_only: profile.read_only,
        })
    }

    fn for_database(&self, database: impl Into<String>) -> Self {
        let mut settings = self.clone();
        settings.database = database.into();
        settings
    }

    fn tiberius_config(&self) -> Config {
        let mut config = Config::new();
        config.host(&self.host);
        config.port(self.port);
        config.database(&self.database);
        config.application_name("lazydb");
        config.readonly(self.read_only);
        config.authentication(AuthMethod::sql_server(
            &self.user,
            self.password.expose_secret(),
        ));

        match self.tls_mode {
            // NotSupported requests plaintext; a server that requires encryption can still
            // negotiate TLS. EncryptionLevel::Off would always encrypt login credentials.
            MsSqlTlsMode::Plaintext => config.encryption(EncryptionLevel::NotSupported),
            MsSqlTlsMode::RequiredUnverified => {
                config.encryption(EncryptionLevel::Required);
                config.trust_cert();
            }
            MsSqlTlsMode::RequiredVerified => config.encryption(EncryptionLevel::Required),
        }

        config
    }

    async fn connect(&self) -> Result<TdsClient, DatabaseError> {
        let config = self.tiberius_config();
        let tcp = TcpStream::connect(config.get_addr())
            .await
            .map_err(network_error)?;
        tcp.set_nodelay(true).map_err(network_error)?;
        Client::connect(config, tcp.compat_write())
            .await
            .map_err(|error| tiberius_error(error, ErrorCategory::Network))
    }
}

pub(crate) struct LeaseSlot<T> {
    client: Option<T>,
    idle: Arc<Mutex<Vec<T>>>,
    permit: Option<OwnedSemaphorePermit>,
    closed: Arc<AtomicBool>,
    reusable: bool,
}

impl<T> LeaseSlot<T> {
    #[allow(dead_code)]
    pub(crate) fn mark_reusable(&mut self) {
        self.reusable = true;
    }
}

impl<T> Deref for LeaseSlot<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.client
            .as_ref()
            .expect("lease always contains a client")
    }
}

impl<T> DerefMut for LeaseSlot<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.client
            .as_mut()
            .expect("lease always contains a client")
    }
}

impl<T> Drop for LeaseSlot<T> {
    fn drop(&mut self) {
        if self.reusable {
            let mut idle = self.idle.lock().unwrap_or_else(|error| error.into_inner());
            if !self.closed.load(Ordering::Acquire)
                && let Some(client) = self.client.take()
            {
                idle.push(client);
            }
        }
        // Dirty and closed clients are dropped before the permit is released.
        self.client.take();
        self.permit.take();
    }
}

pub(crate) type MsSqlLease = LeaseSlot<TdsClient>;

#[derive(Clone)]
pub(crate) struct MsSqlConnectionPool {
    settings: Arc<MsSqlConnectSettings>,
    idle: Arc<Mutex<Vec<TdsClient>>>,
    permits: Arc<tokio::sync::Semaphore>,
    closed: Arc<AtomicBool>,
}

#[allow(dead_code)]
impl MsSqlConnectionPool {
    fn new(settings: Arc<MsSqlConnectSettings>) -> Self {
        Self::with_max_connections(settings, DEFAULT_MAX_CONNECTIONS)
    }

    fn with_max_connections(settings: Arc<MsSqlConnectSettings>, max_connections: usize) -> Self {
        assert!(
            max_connections > 0,
            "pool must allow at least one connection"
        );
        Self {
            settings,
            idle: Arc::new(Mutex::new(Vec::new())),
            permits: Arc::new(tokio::sync::Semaphore::new(max_connections)),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    async fn checkout(&self) -> Result<MsSqlLease, DatabaseError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(pool_closed_error());
        }

        let permit = Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_| pool_closed_error())?;
        let idle_client = {
            let mut idle = self.idle.lock().unwrap_or_else(|error| error.into_inner());
            if self.closed.load(Ordering::Acquire) {
                return Err(pool_closed_error());
            }
            idle.pop()
        };
        let client = match idle_client {
            Some(client) => client,
            None => self.settings.connect().await?,
        };

        if self.closed.load(Ordering::Acquire) {
            drop(client);
            return Err(pool_closed_error());
        }

        Ok(LeaseSlot {
            client: Some(client),
            idle: Arc::clone(&self.idle),
            permit: Some(permit),
            closed: Arc::clone(&self.closed),
            reusable: false,
        })
    }

    async fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.permits.close();
        let clients = {
            let mut idle = self.idle.lock().unwrap_or_else(|error| error.into_inner());
            std::mem::take(&mut *idle)
        };
        for client in clients {
            let _ = client.close().await;
        }
    }
}

#[derive(Clone)]
pub struct MsSqlAdapter {
    settings: Arc<MsSqlConnectSettings>,
    pools: Arc<tokio::sync::Mutex<HashMap<String, MsSqlConnectionPool>>>,
    closed: Arc<AtomicBool>,
    connection_id: Uuid,
    catalog_scope: CatalogScope,
}

impl fmt::Debug for MsSqlAdapter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MsSqlAdapter")
            .field("connection_id", &self.connection_id)
            .field("catalog_scope", &self.catalog_scope)
            .finish_non_exhaustive()
    }
}

impl MsSqlAdapter {
    pub fn catalog_mutation_capabilities() -> CatalogMutationCapabilities {
        CatalogMutationCapabilities::default()
    }

    pub async fn transaction_backend(&self) -> Result<MsSqlTransactionBackend, DatabaseError> {
        let client = self
            .settings
            .for_database(self.settings.database.clone())
            .connect()
            .await?;
        Ok(MsSqlTransactionBackend {
            client: Some(client),
            settings: self.settings.as_ref().clone(),
            database: self.settings.database.clone(),
            depth: 0,
        })
    }

    pub async fn connect(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
    ) -> Result<Self, DatabaseError> {
        let adapter = Self::from_profile(profile, password)?;
        if let Err(error) = adapter.probe().await {
            adapter.close().await;
            return Err(error);
        }
        Ok(adapter)
    }

    pub(crate) fn from_profile(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
    ) -> Result<Self, DatabaseError> {
        let settings = Arc::new(MsSqlConnectSettings::from_profile(profile, password)?);
        Ok(Self {
            settings,
            pools: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            closed: Arc::new(AtomicBool::new(false)),
            connection_id: profile.id,
            catalog_scope: profile.catalog_scope.clone(),
        })
    }

    async fn pool_for_database(
        &self,
        database: &str,
    ) -> Result<MsSqlConnectionPool, DatabaseError> {
        if !self.catalog_scope.allows_database(database) {
            return Err(DatabaseError::configuration(
                "SQL Server database is outside the configured catalog scope",
            ));
        }
        self.pool_for_database_unchecked(database).await
    }

    async fn pool_for_database_unchecked(
        &self,
        database: &str,
    ) -> Result<MsSqlConnectionPool, DatabaseError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(pool_closed_error());
        }
        let mut pools = self.pools.lock().await;
        if self.closed.load(Ordering::Acquire) {
            return Err(pool_closed_error());
        }
        Ok(pools
            .entry(database.to_owned())
            .or_insert_with(|| {
                MsSqlConnectionPool::new(Arc::new(self.settings.for_database(database)))
            })
            .clone())
    }

    pub fn catalog_capabilities() -> CatalogCapabilities {
        CatalogCapabilities {
            namespace_model: NamespaceModel::DatabaseAndSchema,
            top_level_groups: vec![
                ObjectGroup::Tables,
                ObjectGroup::Views,
                ObjectGroup::Functions,
                ObjectGroup::Procedures,
                ObjectGroup::Sequences,
                ObjectGroup::Triggers,
            ],
            column_metadata: ColumnMetadataCapabilities {
                type_family: true,
                default_expression: true,
                identity: true,
                generated_expression: true,
                numeric_precision_and_scale: true,
                character_length: true,
                collation: true,
                ..ColumnMetadataCapabilities::default()
            },
            supports_lazy_children: false,
        }
    }

    pub fn plan_catalog_drop(
        request: CatalogDropRequest,
        entry: &CatalogEntry,
    ) -> Result<CatalogDropPlan, CatalogDropError> {
        request.validate()?;
        let sql = match entry.kind {
            CatalogKind::Table => format!(
                "DROP TABLE {}",
                mssql_top_level_name(entry, CatalogKind::Table)?
            ),
            CatalogKind::View => format!(
                "DROP VIEW {}",
                mssql_top_level_name(entry, CatalogKind::View)?
            ),
            CatalogKind::Sequence => format!(
                "DROP SEQUENCE {}",
                mssql_top_level_name(entry, CatalogKind::Sequence)?
            ),
            CatalogKind::Function => format!(
                "DROP FUNCTION {}",
                mssql_top_level_name(entry, CatalogKind::Function)?
            ),
            CatalogKind::Procedure => format!(
                "DROP PROCEDURE {}",
                mssql_top_level_name(entry, CatalogKind::Procedure)?
            ),
            CatalogKind::Trigger => format!("DROP TRIGGER {}", mssql_trigger_name(entry)?),
            CatalogKind::Index => {
                let (relation, _) = mssql_child_name(entry, CatalogKind::Index)?;
                format!(
                    "DROP INDEX {} ON {}",
                    quote_identifier(&entry.qualified_name.object),
                    relation
                )
            }
            kind => {
                return Err(CatalogDropError::Unsupported {
                    kind,
                    reason: match kind {
                        CatalogKind::Column => {
                            "SQL Server column drops are unsupported by the catalog planner"
                        }
                        CatalogKind::PrimaryKey
                        | CatalogKind::UniqueConstraint
                        | CatalogKind::ForeignKey
                        | CatalogKind::CheckConstraint => {
                            "SQL Server constraint drops are unsupported by the catalog planner"
                        }
                        _ => UNSUPPORTED_MESSAGE,
                    }
                    .to_owned(),
                });
            }
        };
        CatalogDropPlan::new(request, entry, sql)
    }

    pub async fn load_monitor_snapshot(&self) -> Result<MonitorSnapshot, DatabaseError> {
        unsupported_operation(DASHBOARD_UNSUPPORTED_CODE, DASHBOARD_UNSUPPORTED_MESSAGE)
    }

    pub async fn load_monitor_metadata(&self) -> Result<MonitorMetadata, DatabaseError> {
        unsupported_operation(DASHBOARD_UNSUPPORTED_CODE, DASHBOARD_UNSUPPORTED_MESSAGE)
    }

    pub async fn load_process_snapshot(&self) -> Result<ProcessSnapshot, DatabaseError> {
        unsupported_operation(
            PROCESS_METRICS_UNSUPPORTED_CODE,
            PROCESS_METRICS_UNSUPPORTED_MESSAGE,
        )
    }

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        let pool = self.pool_for_database(&self.settings.database).await?;
        let mut rows = query_rows(&pool, PROBE_SQL).await?;
        if rows.len() != 1 {
            return Err(decode_error(
                "SQL Server probe returned an unexpected row count",
            ));
        }
        let row = rows.pop().expect("row count was checked");
        let version = required_string(&row, "version")?;
        let database = required_string(&row, "current_database")?;
        if !supports_server_version(&version) {
            return Err(DatabaseError {
                category: ErrorCategory::Unsupported,
                code: Some("sql_server_version_unsupported".to_owned()),
                message: crate::security::sanitize_terminal_text(&format!(
                    "SQL Server 2012 or newer is required; server version is {version}"
                )),
            });
        }
        Ok(ServerInfo {
            kind: DatabaseKind::SqlServer,
            version,
            database,
            current_user: None,
        })
    }

    pub async fn discover_catalog_scope(&self) -> Result<CatalogDiscovery, DatabaseError> {
        let initial_pool = self.pool_for_database(&self.settings.database).await?;
        let databases = query_strings(&initial_pool, DATABASES_SQL, "name").await?;
        let mut discovered = Vec::with_capacity(databases.len());
        let mut warnings = Vec::new();

        for database in databases {
            let schemas = match self.pool_for_database_unchecked(&database).await {
                Ok(pool) => {
                    let sql = format!(
                        "SELECT [name] FROM {}.sys.schemas WHERE [name] NOT IN ('guest', 'INFORMATION_SCHEMA', 'sys') ORDER BY [name]",
                        quote_identifier(&database)
                    );
                    match query_strings(&pool, &sql, "name").await {
                        Ok(schemas) => schemas,
                        Err(error) => {
                            warnings.push(discovery_warning(&database, &error));
                            Vec::new()
                        }
                    }
                }
                Err(error) => {
                    warnings.push(discovery_warning(&database, &error));
                    Vec::new()
                }
            };
            discovered.push(DiscoveredDatabase {
                name: database,
                schemas,
            });
        }

        Ok(CatalogDiscovery {
            databases: discovered,
            warnings,
        })
    }

    pub async fn load_catalog_page(
        &self,
        request: &CatalogRequest,
    ) -> Result<CatalogPage, DatabaseError> {
        request
            .validate_for_profile(self.connection_id)
            .map_err(DatabaseError::invalid_catalog_request)?;
        validate_catalog_scope(&request.scope)?;
        match &request.key.target {
            CatalogTarget::Databases => self.load_database_page(request).await,
            CatalogTarget::Schemas { database } => self.load_schema_page(request, database).await,
            CatalogTarget::Groups { schema } => self.load_group_page(request, schema).await,
            CatalogTarget::Objects { schema, group } => {
                if *group == ObjectGroup::Triggers {
                    let (database, schema_name) = schema_names(schema, &request.key.target)?;
                    self.load_trigger_page(request, schema, &database, &schema_name)
                        .await
                } else {
                    self.load_object_page(request, schema, *group).await
                }
            }
            CatalogTarget::RelationChildren { relation } => {
                self.load_relation_children_page(request, relation).await
            }
        }
    }

    async fn load_trigger_page(
        &self,
        request: &CatalogRequest,
        schema_id: &CatalogId,
        database: &str,
        schema: &str,
    ) -> Result<CatalogPage, DatabaseError> {
        let pool = self.pool_for_database(database).await?;
        let sql = format!(
            "SELECT tr.[name], tr.[object_id], parent.[name] AS [relation_name], parent.[object_id] AS [relation_id], parent.[type] AS [relation_type], (SELECT CAST(ep.[value] AS nvarchar(4000)) FROM {}.sys.extended_properties ep WHERE ep.[class] = 1 AND ep.[major_id] = tr.[object_id] AND ep.[minor_id] = 0 AND ep.[name] = N'MS_Description') AS [comment] FROM {}.sys.triggers tr JOIN {}.sys.objects parent ON parent.[object_id] = tr.[parent_id] JOIN {}.sys.schemas s ON s.[schema_id] = parent.[schema_id] WHERE s.[name] = {} AND tr.[is_ms_shipped] = 0 ORDER BY tr.[name] COLLATE Latin1_General_100_BIN2, tr.[object_id]",
            quote_identifier(database),
            quote_identifier(database),
            quote_identifier(database),
            quote_identifier(database),
            quote_literal(schema)
        );
        let mut entries = Vec::new();
        for row in query_rows(&pool, &sql).await? {
            let name = required_string(&row, "name")?;
            let relation_name = required_string(&row, "relation_name")?;
            let relation_id = row
                .try_get::<i32, _>("relation_id")
                .map_err(|error| tiberius_error(error, ErrorCategory::Internal))?
                .ok_or_else(|| decode_error("SQL Server returned NULL for [relation_id]"))?;
            let trigger_id = row
                .try_get::<i32, _>("object_id")
                .map_err(|error| tiberius_error(error, ErrorCategory::Internal))?
                .ok_or_else(|| decode_error("SQL Server returned NULL for [object_id]"))?;
            let relation_kind = if required_string(&row, "relation_type")? == "V" {
                CatalogKind::View
            } else {
                CatalogKind::Table
            };
            let relation = CatalogId::new(
                self.connection_id,
                relation_kind,
                vec![
                    database.to_owned(),
                    schema.to_owned(),
                    relation_name,
                    relation_id.to_string(),
                ],
            );
            entries.push(
                CatalogEntry::relation_object(
                    CatalogId::new(
                        self.connection_id,
                        CatalogKind::Trigger,
                        vec![
                            database.to_owned(),
                            schema.to_owned(),
                            name.clone(),
                            trigger_id.to_string(),
                        ],
                    ),
                    schema_id.clone(),
                    relation,
                    qualified_object(database, schema, &name),
                    "trigger",
                    OptionalMetadata::Supported(optional_string(&row, "comment")?),
                )
                .map_err(catalog_invariant)?,
            );
        }
        let total = exact_catalog_count(entries.len())?;
        let cursor = paginate_catalog(
            &mut entries,
            request,
            |entry| entry.qualified_name.object.clone(),
            |entry| entry.id.native_path.last().cloned().unwrap_or_default(),
        )?;
        CatalogPage::new(request, entries, total, cursor).map_err(catalog_invariant)
    }

    async fn load_relation_children_page(
        &self,
        request: &CatalogRequest,
        relation: &CatalogId,
    ) -> Result<CatalogPage, DatabaseError> {
        let (database, schema, relation_name, object_id) = match relation.native_path.as_slice() {
            [database, schema, name, object_id] => (
                database.clone(),
                schema.clone(),
                name.clone(),
                object_id
                    .parse::<i32>()
                    .map_err(|_| catalog_target_not_found(&request.key.target))?,
            ),
            _ => return Err(catalog_target_not_found(&request.key.target)),
        };
        if relation.profile_id() != self.connection_id || !relation.kind.is_relation() {
            return Err(catalog_target_not_found(&request.key.target));
        }
        let pool = self.pool_for_database(&database).await?;
        let relation_sql = format!(
            "SELECT o.[type] FROM {}.sys.objects o JOIN {}.sys.schemas s ON s.[schema_id] = o.[schema_id] WHERE o.[object_id] = {object_id} AND s.[name] = {} AND o.[name] = {} AND o.[is_ms_shipped] = 0",
            quote_identifier(&database),
            quote_identifier(&database),
            quote_literal(&schema),
            quote_literal(&relation_name)
        );
        let relation_type = query_rows(&pool, &relation_sql)
            .await?
            .first()
            .map(|row| required_string(row, "type"))
            .transpose()?;
        let expected = match relation_type.as_deref() {
            Some("U") => CatalogKind::Table,
            Some("V") => CatalogKind::View,
            _ => return Err(catalog_target_not_found(&request.key.target)),
        };
        if relation.kind != expected {
            return Err(catalog_target_not_found(&request.key.target));
        }

        let q = quote_identifier(&database);
        let mut entries = Vec::new();
        let mut memberships: HashMap<String, Vec<ConstraintMembership>> = HashMap::new();

        let index_sql = format!(
            "SELECT i.[name], i.[index_id], i.[is_unique], ic.[key_ordinal], ic.[is_included_column], c.[name] AS [column_name] FROM {q}.sys.indexes i JOIN {q}.sys.index_columns ic ON ic.[object_id] = i.[object_id] AND ic.[index_id] = i.[index_id] JOIN {q}.sys.columns c ON c.[object_id] = ic.[object_id] AND c.[column_id] = ic.[column_id] WHERE i.[object_id] = {object_id} AND i.[index_id] > 0 AND i.[is_hypothetical] = 0 ORDER BY i.[index_id], ic.[is_included_column], ic.[key_ordinal], ic.[index_column_id]"
        );
        let mut indexes: HashMap<i32, (String, bool, Vec<String>)> = HashMap::new();
        for row in query_rows(&pool, &index_sql).await? {
            let id = row
                .try_get::<i32, _>("index_id")
                .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
                .ok_or_else(|| decode_error("NULL index_id"))?;
            let name = required_string(&row, "name")?;
            let unique = row
                .try_get::<bool, _>("is_unique")
                .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
                .ok_or_else(|| decode_error("NULL is_unique"))?;
            let column = required_string(&row, "column_name")?;
            indexes
                .entry(id)
                .or_insert((name, unique, Vec::new()))
                .2
                .push(column);
        }
        for (index_id, (name, unique, columns)) in indexes {
            entries.push(
                CatalogEntry::relation_child(
                    relation_child_id(relation, CatalogKind::Index, &index_id.to_string()),
                    relation.clone(),
                    qualified_object(&database, &schema, &name),
                    "index",
                    OptionalMetadata::Supported(None),
                    CatalogMetadata::Index(IndexMetadata { columns, unique }),
                )
                .map_err(catalog_invariant)?,
            );
        }

        let constraint_sql = format!(
            "SELECT kc.[name], kc.[object_id] AS [constraint_id], kc.[type], ic.[key_ordinal], c.[name] AS [column_name] FROM {q}.sys.key_constraints kc JOIN {q}.sys.index_columns ic ON ic.[object_id] = kc.[parent_object_id] AND ic.[index_id] = kc.[unique_index_id] JOIN {q}.sys.columns c ON c.[object_id] = ic.[object_id] AND c.[column_id] = ic.[column_id] WHERE kc.[parent_object_id] = {object_id} UNION ALL SELECT fk.[name], fk.[object_id], 'F', fkc.[constraint_column_id], pc.[name] FROM {q}.sys.foreign_keys fk JOIN {q}.sys.foreign_key_columns fkc ON fkc.[constraint_object_id] = fk.[object_id] JOIN {q}.sys.columns pc ON pc.[object_id] = fkc.[parent_object_id] AND pc.[column_id] = fkc.[parent_column_id] WHERE fk.[parent_object_id] = {object_id} UNION ALL SELECT cc.[name], cc.[object_id], 'C', 1, NULL FROM {q}.sys.check_constraints cc WHERE cc.[parent_object_id] = {object_id} ORDER BY [type], [constraint_id], [key_ordinal]"
        );
        let mut constraints: HashMap<i32, (String, String, Vec<String>)> = HashMap::new();
        for row in query_rows(&pool, &constraint_sql).await? {
            let id = row
                .try_get::<i32, _>("constraint_id")
                .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
                .ok_or_else(|| decode_error("NULL constraint_id"))?;
            let kind = required_string(&row, "type")?;
            let name = required_string(&row, "name")?;
            if let Some(column) = row
                .try_get::<&str, _>("column_name")
                .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
            {
                constraints
                    .entry(id)
                    .or_insert((name, kind, Vec::new()))
                    .2
                    .push(column.to_owned());
            } else {
                constraints.entry(id).or_insert((name, kind, Vec::new()));
            }
        }
        let foreign_key_sql = format!(
            "SELECT fk.[object_id] AS [constraint_id], rs.[name] AS [referenced_schema], rt.[name] AS [referenced_relation], pc.[name] AS [source_column], rc.[name] AS [referenced_column], fkc.[constraint_column_id] FROM {q}.sys.foreign_keys fk JOIN {q}.sys.foreign_key_columns fkc ON fkc.[constraint_object_id] = fk.[object_id] JOIN {q}.sys.tables rt ON rt.[object_id] = fk.[referenced_object_id] JOIN {q}.sys.schemas rs ON rs.[schema_id] = rt.[schema_id] JOIN {q}.sys.columns pc ON pc.[object_id] = fkc.[parent_object_id] AND pc.[column_id] = fkc.[parent_column_id] JOIN {q}.sys.columns rc ON rc.[object_id] = fkc.[referenced_object_id] AND rc.[column_id] = fkc.[referenced_column_id] WHERE fk.[parent_object_id] = {object_id} ORDER BY fk.[object_id], fkc.[constraint_column_id]"
        );
        let mut foreign_keys: HashMap<i32, (String, String, Vec<String>, Vec<String>)> =
            HashMap::new();
        for row in query_rows(&pool, &foreign_key_sql).await? {
            let id = row
                .try_get::<i32, _>("constraint_id")
                .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
                .ok_or_else(|| decode_error("NULL foreign-key constraint_id"))?;
            let item = foreign_keys.entry(id).or_insert((
                required_string(&row, "referenced_schema")?,
                required_string(&row, "referenced_relation")?,
                Vec::new(),
                Vec::new(),
            ));
            item.2.push(required_string(&row, "source_column")?);
            item.3.push(required_string(&row, "referenced_column")?);
        }
        for (constraint_id, (name, native_kind, columns)) in constraints {
            let kind = match native_kind.as_str() {
                "PK" => CatalogKind::PrimaryKey,
                "UQ" => CatalogKind::UniqueConstraint,
                "F" => CatalogKind::ForeignKey,
                "C" => CatalogKind::CheckConstraint,
                _ => continue,
            };
            let id = relation_child_id(relation, kind, &constraint_id.to_string());
            if matches!(
                kind,
                CatalogKind::PrimaryKey | CatalogKind::UniqueConstraint | CatalogKind::ForeignKey
            ) {
                add_memberships(&mut memberships, &columns, &id)?;
            }
            let metadata = match kind {
                CatalogKind::PrimaryKey => {
                    CatalogMetadata::Constraint(ConstraintMetadata::PrimaryKey { columns })
                }
                CatalogKind::UniqueConstraint => {
                    CatalogMetadata::Constraint(ConstraintMetadata::Unique { columns })
                }
                CatalogKind::ForeignKey => {
                    let (
                        referenced_schema,
                        referenced_relation,
                        source_columns,
                        referenced_columns,
                    ) = foreign_keys.remove(&constraint_id).ok_or_else(|| {
                        catalog_internal("foreign key has no referenced relation")
                    })?;
                    CatalogMetadata::Constraint(ConstraintMetadata::ForeignKey {
                        columns: if source_columns.is_empty() {
                            columns
                        } else {
                            source_columns
                        },
                        referenced_relation: qualified_object(
                            &database,
                            &referenced_schema,
                            &referenced_relation,
                        ),
                        referenced_columns,
                    })
                }
                CatalogKind::CheckConstraint => {
                    let sql = format!(
                        "SELECT [definition] FROM {q}.sys.check_constraints WHERE [object_id] = {constraint_id}"
                    );
                    let expression = query_rows(&pool, &sql)
                        .await?
                        .first()
                        .map(|row| required_string(row, "definition"))
                        .transpose()?
                        .ok_or_else(|| catalog_internal("check constraint has no expression"))?;
                    CatalogMetadata::Constraint(ConstraintMetadata::Check { expression })
                }
                _ => unreachable!(),
            };
            entries.push(
                CatalogEntry::relation_child(
                    id,
                    relation.clone(),
                    qualified_object(&database, &schema, &name),
                    "constraint",
                    OptionalMetadata::Supported(None),
                    metadata,
                )
                .map_err(catalog_invariant)?,
            );
        }

        let column_sql = format!(
            "SELECT c.[column_id], c.[name], t.[name] AS [type_name], c.[is_nullable], c.[is_identity], c.[is_computed], dc.[definition] AS [default_expression], cc.[definition] AS [computed_expression], CASE WHEN t.[name] IN ('rowversion','timestamp') THEN 1 ELSE 0 END AS [rowversion], (SELECT CAST(ep.[value] AS nvarchar(4000)) FROM {q}.sys.extended_properties ep WHERE ep.[class] = 1 AND ep.[major_id] = c.[object_id] AND ep.[minor_id] = c.[column_id] AND ep.[name] = N'MS_Description') AS [comment] FROM {q}.sys.columns c JOIN {q}.sys.types t ON t.[user_type_id] = c.[user_type_id] LEFT JOIN {q}.sys.default_constraints dc ON dc.[parent_object_id] = c.[object_id] AND dc.[parent_column_id] = c.[column_id] LEFT JOIN {q}.sys.computed_columns cc ON cc.[object_id] = c.[object_id] AND cc.[column_id] = c.[column_id] WHERE c.[object_id] = {object_id} ORDER BY c.[column_id]"
        );
        for row in query_rows(&pool, &column_sql).await? {
            let ordinal = row
                .try_get::<i32, _>("column_id")
                .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
                .ok_or_else(|| decode_error("NULL column_id"))?;
            let name = required_string(&row, "name")?;
            let mut metadata = ColumnMetadata::new(
                ordinal as u32,
                required_string(&row, "type_name")?,
                row.try_get("is_nullable")
                    .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
                    .unwrap_or(false),
            );
            metadata.type_family = OptionalMetadata::Supported(Some(metadata.native_type.clone()));
            let identity = row
                .try_get::<bool, _>("is_identity")
                .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
                .unwrap_or(false);
            let computed = row
                .try_get::<bool, _>("is_computed")
                .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
                .unwrap_or(false);
            metadata.identity = OptionalMetadata::Supported(Some(identity));
            metadata.auto_increment = OptionalMetadata::Supported(Some(identity));
            metadata.default_expression = OptionalMetadata::Supported(if computed {
                None
            } else {
                optional_string(&row, "default_expression")?
            });
            metadata.generated_expression = OptionalMetadata::Supported(if computed {
                optional_string(&row, "computed_expression")?
            } else {
                None
            });
            metadata.hidden = OptionalMetadata::Supported(Some(
                row.try_get::<bool, _>("rowversion")
                    .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
                    .unwrap_or(false),
            ));
            metadata.constraint_memberships = memberships.remove(&name).unwrap_or_default();
            entries.push(
                CatalogEntry::relation_child(
                    relation_child_id(relation, CatalogKind::Column, &ordinal.to_string()),
                    relation.clone(),
                    qualified_object(&database, &schema, &name),
                    "column",
                    OptionalMetadata::Supported(optional_string(&row, "comment")?),
                    CatalogMetadata::Column(metadata),
                )
                .map_err(catalog_invariant)?,
            );
        }

        let trigger_sql = format!(
            "SELECT [name], [object_id] FROM {q}.sys.triggers WHERE [parent_id] = {object_id} AND [is_ms_shipped] = 0 ORDER BY [name] COLLATE Latin1_General_100_BIN2, [object_id]"
        );
        for row in query_rows(&pool, &trigger_sql).await? {
            let name = required_string(&row, "name")?;
            let id = row
                .try_get::<i32, _>("object_id")
                .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
                .ok_or_else(|| decode_error("NULL trigger object_id"))?;
            entries.push(
                CatalogEntry::relation_child(
                    CatalogId::new(
                        self.connection_id,
                        CatalogKind::Trigger,
                        [
                            database.clone(),
                            schema.clone(),
                            relation_name.clone(),
                            object_id.to_string(),
                            id.to_string(),
                        ],
                    ),
                    relation.clone(),
                    qualified_object(&database, &schema, &name),
                    "trigger",
                    OptionalMetadata::Supported(None),
                    CatalogMetadata::None,
                )
                .map_err(catalog_invariant)?,
            );
        }
        let total = exact_catalog_count(entries.len())?;
        let cursor = paginate_catalog(&mut entries, request, child_sort_key, child_tie_breaker)?;
        CatalogPage::new(request, entries, total, cursor).map_err(catalog_invariant)
    }

    async fn load_database_page(
        &self,
        request: &CatalogRequest,
    ) -> Result<CatalogPage, DatabaseError> {
        let pool = self.pool_for_database(&self.settings.database).await?;
        let rows = query_rows(&pool, DATABASES_SQL).await?;
        let mut entries = rows
            .iter()
            .map(|row| {
                let name = required_string(row, "name")?;
                CatalogEntry::database(
                    CatalogId::new(self.connection_id, CatalogKind::Database, [name.clone()]),
                    qualified_database(&name),
                    "database",
                    OptionalMetadata::Supported(None),
                    true,
                )
                .map_err(catalog_invariant)
            })
            .filter(|entry| {
                entry
                    .as_ref()
                    .is_ok_and(|entry| request.scope.allows_database(&entry.qualified_name.object))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let total = exact_catalog_count(entries.len())?;
        let cursor = paginate_catalog(
            &mut entries,
            request,
            |entry| entry.qualified_name.object.clone(),
            |entry| entry.qualified_name.object.clone(),
        )?;
        CatalogPage::new(request, entries, total, cursor).map_err(catalog_invariant)
    }

    async fn load_schema_page(
        &self,
        request: &CatalogRequest,
        database_id: &CatalogId,
    ) -> Result<CatalogPage, DatabaseError> {
        let database = database_name(database_id, &request.key.target)?;
        let pool = self.pool_for_database(&database).await?;
        let sql = format!(
            "SELECT [name] FROM {}.sys.schemas WHERE [name] NOT IN ('guest','INFORMATION_SCHEMA','sys') AND [is_ms_shipped] = 0 ORDER BY [name] COLLATE Latin1_General_100_BIN2",
            quote_identifier(&database)
        );
        let database_parent = CatalogId::new(
            self.connection_id,
            CatalogKind::Database,
            [database.clone()],
        );
        let mut entries = query_rows(&pool, &sql)
            .await?
            .iter()
            .map(|row| {
                let name = required_string(row, "name")?;
                CatalogEntry::schema(
                    CatalogId::new(
                        self.connection_id,
                        CatalogKind::Schema,
                        [database.clone(), name.clone()],
                    ),
                    database_parent.clone(),
                    qualified_schema(&database, &name),
                    "schema",
                    OptionalMetadata::Supported(None),
                    true,
                )
                .map_err(catalog_invariant)
            })
            .filter(|entry| {
                entry.as_ref().is_ok_and(|entry| {
                    request
                        .scope
                        .allows_schema(&database, &entry.qualified_name.object)
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let total = exact_catalog_count(entries.len())?;
        let cursor = paginate_catalog(
            &mut entries,
            request,
            |entry| entry.qualified_name.object.clone(),
            |entry| entry.qualified_name.object.clone(),
        )?;
        CatalogPage::new(request, entries, total, cursor).map_err(catalog_invariant)
    }

    async fn load_group_page(
        &self,
        request: &CatalogRequest,
        schema_id: &CatalogId,
    ) -> Result<CatalogPage, DatabaseError> {
        let (database, schema) = schema_names(schema_id, &request.key.target)?;
        let pool = self.pool_for_database(&database).await?;
        let sql = format!(
            "SELECT [type] AS [group_type], COUNT_BIG(*) AS [object_count] FROM {}.sys.objects WHERE [schema_id] = SCHEMA_ID({}) AND [is_ms_shipped] = 0 AND [type] IN ('U','V','SO','FN','IF','TF','FS','FT','P') GROUP BY [type] UNION ALL SELECT 'TR' AS [group_type], COUNT_BIG(*) AS [object_count] FROM {}.sys.triggers tr JOIN {}.sys.objects parent ON parent.[object_id] = tr.[parent_id] WHERE parent.[schema_id] = SCHEMA_ID({}) AND tr.[is_ms_shipped] = 0",
            quote_identifier(&database),
            quote_literal(&schema),
            quote_identifier(&database),
            quote_identifier(&database),
            quote_literal(&schema)
        );
        let mut counts = HashMap::new();
        for row in query_rows(&pool, &sql).await? {
            let native = required_string(&row, "group_type")?;
            let count = row
                .try_get::<i64, _>("object_count")
                .map_err(|error| tiberius_error(error, ErrorCategory::Internal))?
                .ok_or_else(|| decode_error("SQL Server returned NULL for [object_count]"))?;
            counts.insert(
                native,
                u64::try_from(count)
                    .map_err(|_| decode_error("SQL Server returned a negative catalog count"))?,
            );
        }
        let groups = [
            (ObjectGroup::Tables, &["U"][..]),
            (ObjectGroup::Views, &["V"][..]),
            (ObjectGroup::Functions, &["FN", "IF", "TF", "FS", "FT"][..]),
            (ObjectGroup::Procedures, &["P"][..]),
            (ObjectGroup::Sequences, &["SO"][..]),
            (ObjectGroup::Triggers, &["TR"][..]),
        ];
        let mut summaries = groups
            .into_iter()
            .map(|(group, types)| CatalogGroupSummary {
                group,
                object_count: CatalogCount::Exact(
                    types
                        .iter()
                        .map(|kind| counts.get(*kind).copied().unwrap_or_default())
                        .sum(),
                ),
            })
            .collect::<Vec<_>>();
        let total = exact_catalog_count(summaries.len())?;
        let cursor = paginate_catalog(
            &mut summaries,
            request,
            |summary| group_key(summary.group).to_owned(),
            |summary| group_key(summary.group).to_owned(),
        )?;
        CatalogPage::groups(request, summaries, total, cursor).map_err(catalog_invariant)
    }

    async fn load_object_page(
        &self,
        request: &CatalogRequest,
        schema_id: &CatalogId,
        group: ObjectGroup,
    ) -> Result<CatalogPage, DatabaseError> {
        let (database, schema) = schema_names(schema_id, &request.key.target)?;
        let pool = self.pool_for_database(&database).await?;
        let types = match group {
            ObjectGroup::Tables => "'U'",
            ObjectGroup::Views => "'V'",
            ObjectGroup::Functions => "'FN','IF','TF','FS','FT'",
            ObjectGroup::Procedures => "'P'",
            ObjectGroup::Sequences => "'SO'",
            ObjectGroup::Triggers => {
                return self
                    .load_trigger_page(request, schema_id, &database, &schema)
                    .await;
            }
            _ => {
                return Err(DatabaseError::unsupported_catalog_target(
                    DatabaseKind::SqlServer,
                    &request.key.target,
                ));
            }
        };
        let sql = format!(
            "SELECT o.[name], o.[object_id], o.[type_desc], (SELECT CAST(ep.[value] AS nvarchar(4000)) FROM {}.sys.extended_properties ep WHERE ep.[class] = 1 AND ep.[major_id] = o.[object_id] AND ep.[minor_id] = 0 AND ep.[name] = N'MS_Description') AS [comment] FROM {}.sys.objects o JOIN {}.sys.schemas s ON s.[schema_id] = o.[schema_id] WHERE s.[name] = {} AND o.[type] IN ({types}) AND o.[is_ms_shipped] = 0 ORDER BY o.[name] COLLATE Latin1_General_100_BIN2, o.[object_id]",
            quote_identifier(&database),
            quote_identifier(&database),
            quote_identifier(&database),
            quote_literal(&schema)
        );
        let mut entries = Vec::new();
        for row in query_rows(&pool, &sql).await? {
            let name = required_string(&row, "name")?;
            let object_id = row
                .try_get::<i32, _>("object_id")
                .map_err(|error| tiberius_error(error, ErrorCategory::Internal))?
                .ok_or_else(|| decode_error("SQL Server returned NULL for [object_id]"))?;
            let native_kind = required_string(&row, "type_desc")?.to_ascii_lowercase();
            let kind = match group {
                ObjectGroup::Tables => CatalogKind::Table,
                ObjectGroup::Views => CatalogKind::View,
                ObjectGroup::Functions => CatalogKind::Function,
                ObjectGroup::Procedures => CatalogKind::Procedure,
                ObjectGroup::Sequences => CatalogKind::Sequence,
                ObjectGroup::Triggers => CatalogKind::Trigger,
                _ => unreachable!(),
            };
            let id = CatalogId::new(
                self.connection_id,
                kind,
                vec![
                    database.clone(),
                    schema.clone(),
                    name.clone(),
                    object_id.to_string(),
                ],
            );
            let entry = if kind.is_relation() {
                CatalogEntry::relation(
                    id,
                    schema_id.clone(),
                    qualified_object(&database, &schema, &name),
                    native_kind,
                    OptionalMetadata::Supported(optional_string(&row, "comment")?),
                    true,
                )
            } else {
                CatalogEntry::object(
                    id,
                    schema_id.clone(),
                    qualified_object(&database, &schema, &name),
                    native_kind,
                    OptionalMetadata::Supported(optional_string(&row, "comment")?),
                    false,
                )
            }
            .map_err(catalog_invariant)?;
            entries.push(entry);
        }
        let total = exact_catalog_count(entries.len())?;
        let cursor = paginate_catalog(
            &mut entries,
            request,
            |entry| entry.qualified_name.object.clone(),
            |entry| entry.id.native_path.last().cloned().unwrap_or_default(),
        )?;
        CatalogPage::new(request, entries, total, cursor).map_err(catalog_invariant)
    }

    pub async fn search_catalog(
        &self,
        request: &CatalogSearchRequest,
    ) -> Result<CatalogSearchPage, DatabaseError> {
        request
            .validate()
            .map_err(DatabaseError::invalid_catalog_request)?;
        if request.connection.profile_id != self.connection_id {
            return Err(DatabaseError::invalid_catalog_request(
                crate::db::catalog::CatalogValidationError::ProfileMismatch {
                    child_profile_id: request.connection.profile_id,
                    parent_profile_id: self.connection_id,
                },
            ));
        }
        validate_catalog_scope(&request.scope)?;

        let initial_pool = self.pool_for_database(&self.settings.database).await?;
        let databases = query_strings(&initial_pool, DATABASES_SQL, "name")
            .await?
            .into_iter()
            .filter(|database| request.scope.allows_database(database))
            .collect::<Vec<_>>();
        let escaped_query = search_like_pattern(&request.query);
        let mut candidates = Vec::new();
        for database in databases {
            if database
                .to_ascii_lowercase()
                .contains(&request.query.to_ascii_lowercase())
            {
                candidates.push(MsSqlSearchCandidate::database(&database, &request.query));
            }
            let pool = self.pool_for_database(&database).await?;
            let sql = format_search_candidates(&database, &escaped_query, &request.query);
            for row in query_rows(&pool, &sql).await? {
                let candidate = MsSqlSearchCandidate::decode(&row, &database, &request.query)?;
                if request
                    .scope
                    .allows_schema(&candidate.database, &candidate.schema)
                {
                    candidates.push(candidate);
                }
            }
        }
        candidates.sort_by(|left, right| {
            left.rank
                .cmp(&right.rank)
                .then_with(|| {
                    left.path
                        .to_ascii_lowercase()
                        .cmp(&right.path.to_ascii_lowercase())
                })
                .then_with(|| left.path.cmp(&right.path))
                .then_with(|| format!("{:?}", left.kind).cmp(&format!("{:?}", right.kind)))
                .then_with(|| left.object_id.cmp(&right.object_id))
        });
        let truncated = candidates.len() > request.limit;
        candidates.truncate(request.limit);

        let relation_ids = candidates
            .iter()
            .filter(|candidate| {
                candidate.kind.is_relation_child() || candidate.kind == CatalogKind::Trigger
            })
            .map(|candidate| {
                (
                    candidate.database.clone(),
                    candidate.schema.clone(),
                    candidate.relation_id,
                )
            })
            .collect::<HashSet<_>>();
        let mut children = HashMap::new();
        for database in relation_ids
            .iter()
            .map(|(database, _, _)| database)
            .collect::<HashSet<_>>()
        {
            let ids = relation_ids
                .iter()
                .filter(|(item_database, _, _)| item_database == database)
                .map(|(_, _, id)| *id)
                .collect::<Vec<_>>();
            let pool = self.pool_for_database(database).await?;
            for entry in self.load_search_children(&pool, database, &ids).await? {
                children.insert(entry.id.clone(), entry);
            }
        }

        let mut hits = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            hits.push(self.hydrate_search_candidate(candidate, &children)?);
        }
        CatalogSearchPage::new(request, hits, None, truncated)
            .map_err(DatabaseError::invalid_catalog_request)
    }

    async fn load_search_children(
        &self,
        pool: &MsSqlConnectionPool,
        database: &str,
        relation_ids: &[i32],
    ) -> Result<Vec<CatalogEntry>, DatabaseError> {
        if relation_ids.is_empty() {
            return Ok(Vec::new());
        }
        return self
            .load_search_children_batch(pool, database, relation_ids)
            .await;
        #[allow(unreachable_code)]
        let q = quote_identifier(database);
        let ids = relation_ids
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT 'column' AS [kind], c.[object_id] AS [relation_id], c.[column_id] AS [identity], c.[name], c.[column_id] AS [ordinal], t.[name] AS [type_name], c.[is_nullable], c.[is_identity], c.[is_computed], dc.[definition] AS [default_expression], cc.[definition] AS [computed_expression], NULL AS [is_unique], NULL AS [definition], NULL AS [referenced_schema], NULL AS [referenced_relation], NULL AS [source_column], NULL AS [referenced_column] FROM {q}.sys.columns c JOIN {q}.sys.types t ON t.[user_type_id] = c.[user_type_id] LEFT JOIN {q}.sys.default_constraints dc ON dc.[parent_object_id] = c.[object_id] AND dc.[parent_column_id] = c.[column_id] LEFT JOIN {q}.sys.computed_columns cc ON cc.[object_id] = c.[object_id] AND cc.[column_id] = c.[column_id] WHERE c.[object_id] IN ({ids}) UNION ALL SELECT 'index', i.[object_id], i.[index_id], i.[name], ic.[key_ordinal], NULL, NULL, NULL, NULL, NULL, NULL, i.[is_unique], NULL, NULL, NULL, c.[name], NULL FROM {q}.sys.indexes i JOIN {q}.sys.index_columns ic ON ic.[object_id] = i.[object_id] AND ic.[index_id] = i.[index_id] JOIN {q}.sys.columns c ON c.[object_id] = ic.[object_id] AND c.[column_id] = ic.[column_id] WHERE i.[object_id] IN ({ids}) AND i.[index_id] > 0 AND i.[is_hypothetical] = 0 UNION ALL SELECT CASE kc.[type] WHEN 'PK' THEN 'primary_key' ELSE 'unique_constraint' END, kc.[parent_object_id], kc.[object_id], kc.[name], ic.[key_ordinal], NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, c.[name], NULL FROM {q}.sys.key_constraints kc JOIN {q}.sys.index_columns ic ON ic.[object_id] = kc.[parent_object_id] AND ic.[index_id] = kc.[unique_index_id] JOIN {q}.sys.columns c ON c.[object_id] = ic.[object_id] AND c.[column_id] = ic.[column_id] WHERE kc.[parent_object_id] IN ({ids}) UNION ALL SELECT 'foreign_key', fk.[parent_object_id], fk.[object_id], fk.[name], fkc.[constraint_column_id], NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, rs.[name], rt.[name], pc.[name], rc.[name] FROM {q}.sys.foreign_keys fk JOIN {q}.sys.foreign_key_columns fkc ON fkc.[constraint_object_id] = fk.[object_id] JOIN {q}.sys.tables rt ON rt.[object_id] = fk.[referenced_object_id] JOIN {q}.sys.schemas rs ON rs.[schema_id] = rt.[schema_id] JOIN {q}.sys.columns pc ON pc.[object_id] = fkc.[parent_object_id] AND pc.[column_id] = fkc.[parent_column_id] JOIN {q}.sys.columns rc ON rc.[object_id] = fkc.[referenced_object_id] AND rc.[column_id] = fkc.[referenced_column_id] WHERE fk.[parent_object_id] IN ({ids}) UNION ALL SELECT 'check_constraint', cc.[parent_object_id], cc.[object_id], cc.[name], 1, NULL, NULL, NULL, NULL, NULL, NULL, NULL, cc.[definition], NULL, NULL, NULL, NULL, NULL FROM {q}.sys.check_constraints cc WHERE cc.[parent_object_id] IN ({ids}) UNION ALL SELECT 'trigger', tr.[parent_id], tr.[object_id], tr.[name], 1, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL FROM {q}.sys.triggers tr WHERE tr.[parent_id] IN ({ids}) AND tr.[is_ms_shipped] = 0 ORDER BY [relation_id], [kind], [identity], [ordinal]"
        );
        let rows = query_rows(pool, &sql).await?;
        let _ = rows;
        Ok(Vec::new())
    }

    async fn load_search_children_batch(
        &self,
        pool: &MsSqlConnectionPool,
        database: &str,
        relation_ids: &[i32],
    ) -> Result<Vec<CatalogEntry>, DatabaseError> {
        let q = quote_identifier(database);
        let ids = relation_ids
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT CASE WHEN c.[column_id] IS NOT NULL THEN 'column' WHEN i.[index_id] IS NOT NULL THEN 'index' WHEN kc.[object_id] IS NOT NULL THEN CASE kc.[type] WHEN 'PK' THEN 'primary_key' ELSE 'unique_constraint' END ELSE 'trigger' END AS [kind], o.[object_id] AS [relation_id], o.[name] AS [relation_name], s.[name] AS [schema_name], COALESCE(c.[column_id], i.[index_id], kc.[object_id], tr.[object_id]) AS [identity], COALESCE(c.[name], i.[name], kc.[name], tr.[name]) AS [name], c.[column_id] AS [ordinal], t.[name] AS [type_name], c.[is_nullable], c.[is_identity], c.[is_computed], i.[is_unique] FROM {q}.sys.objects o JOIN {q}.sys.schemas s ON s.[schema_id]=o.[schema_id] LEFT JOIN {q}.sys.columns c ON c.[object_id]=o.[object_id] LEFT JOIN {q}.sys.types t ON t.[user_type_id]=c.[user_type_id] LEFT JOIN {q}.sys.indexes i ON i.[object_id]=o.[object_id] LEFT JOIN {q}.sys.key_constraints kc ON kc.[parent_object_id]=o.[object_id] LEFT JOIN {q}.sys.triggers tr ON tr.[parent_id]=o.[object_id] WHERE o.[object_id] IN ({ids}) AND (c.[column_id] IS NOT NULL OR (i.[index_id]>0 AND i.[is_hypothetical]=0) OR kc.[object_id] IS NOT NULL OR (tr.[object_id] IS NOT NULL AND tr.[is_ms_shipped]=0)) ORDER BY o.[object_id], [kind], [identity]"
        );
        let rows = query_rows(pool, &sql).await?;
        let mut entries = Vec::new();
        for row in rows {
            let schema = required_string(&row, "schema_name")?;
            let relation_name = required_string(&row, "relation_name")?;
            let relation_id = row
                .try_get::<i32, _>("relation_id")
                .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
                .ok_or_else(|| decode_error("NULL relation_id"))?;
            let relation = CatalogId::new(
                self.connection_id,
                CatalogKind::Table,
                [database, &schema, &relation_name, &relation_id.to_string()],
            );
            let kind_name = required_string(&row, "kind")?;
            let kind = match kind_name.as_str() {
                "column" => CatalogKind::Column,
                "index" => CatalogKind::Index,
                "primary_key" => CatalogKind::PrimaryKey,
                "unique_constraint" => CatalogKind::UniqueConstraint,
                "trigger" => CatalogKind::Trigger,
                _ => continue,
            };
            let identity = row
                .try_get::<i32, _>("identity")
                .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
                .unwrap_or_default();
            let name = required_string(&row, "name")?;
            let metadata = match kind {
                CatalogKind::Column => CatalogMetadata::Column(ColumnMetadata::new(
                    row.try_get::<i32, _>("ordinal")
                        .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
                        .unwrap_or_default() as u32,
                    optional_string(&row, "type_name")?.unwrap_or_else(|| "unknown".to_owned()),
                    row.try_get::<bool, _>("is_nullable")
                        .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
                        .unwrap_or(false),
                )),
                CatalogKind::Index => CatalogMetadata::Index(IndexMetadata {
                    columns: Vec::new(),
                    unique: row
                        .try_get::<bool, _>("is_unique")
                        .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
                        .unwrap_or(false),
                }),
                CatalogKind::PrimaryKey => {
                    CatalogMetadata::Constraint(ConstraintMetadata::PrimaryKey {
                        columns: Vec::new(),
                    })
                }
                CatalogKind::UniqueConstraint => {
                    CatalogMetadata::Constraint(ConstraintMetadata::Unique {
                        columns: Vec::new(),
                    })
                }
                CatalogKind::Trigger => CatalogMetadata::None,
                _ => CatalogMetadata::None,
            };
            entries.push(
                CatalogEntry::relation_child(
                    CatalogId::new(
                        self.connection_id,
                        kind,
                        [
                            database,
                            &schema,
                            &relation_name,
                            &relation_id.to_string(),
                            &identity.to_string(),
                        ],
                    ),
                    relation,
                    qualified_object(database, &schema, &name),
                    kind_name,
                    OptionalMetadata::Supported(None),
                    metadata,
                )
                .map_err(catalog_invariant)?,
            );
        }
        Ok(entries)
    }

    fn hydrate_search_candidate(
        &self,
        candidate: MsSqlSearchCandidate,
        children: &HashMap<CatalogId, CatalogEntry>,
    ) -> Result<CatalogSearchHit, DatabaseError> {
        let database = CatalogEntry::database(
            CatalogId::new(
                self.connection_id,
                CatalogKind::Database,
                [&candidate.database],
            ),
            qualified_database(&candidate.database),
            "database",
            OptionalMetadata::Supported(None),
            true,
        )
        .map_err(catalog_invariant)?;
        if candidate.kind == CatalogKind::Database {
            return Ok(CatalogSearchHit {
                entry: database,
                ancestors: Vec::new(),
            });
        }
        let schema = CatalogEntry::schema(
            CatalogId::new(
                self.connection_id,
                CatalogKind::Schema,
                [&candidate.database, &candidate.schema],
            ),
            database.id.clone(),
            qualified_schema(&candidate.database, &candidate.schema),
            "schema",
            OptionalMetadata::Supported(None),
            true,
        )
        .map_err(catalog_invariant)?;
        if candidate.kind == CatalogKind::Schema {
            return Ok(CatalogSearchHit {
                entry: schema,
                ancestors: vec![database],
            });
        }
        let relation = CatalogEntry::relation(
            CatalogId::new(
                self.connection_id,
                candidate.relation_kind,
                [
                    &candidate.database,
                    &candidate.schema,
                    &candidate.relation_name,
                    &candidate.relation_id.to_string(),
                ],
            ),
            schema.id.clone(),
            qualified_object(
                &candidate.database,
                &candidate.schema,
                &candidate.relation_name,
            ),
            "table",
            OptionalMetadata::Supported(None),
            true,
        )
        .map_err(catalog_invariant)?;
        if candidate.kind.is_relation_child() || candidate.kind == CatalogKind::Trigger {
            let id = CatalogId::new(
                self.connection_id,
                candidate.kind,
                [
                    &candidate.database,
                    &candidate.schema,
                    &candidate.relation_name,
                    &candidate.relation_id.to_string(),
                    &candidate.object_id.to_string(),
                ],
            );
            let entry = children
                .get(&id)
                .cloned()
                .unwrap_or(self.search_child_entry(&candidate, id.clone())?);
            return Ok(CatalogSearchHit {
                entry,
                ancestors: vec![database, schema, relation],
            });
        }
        let id = CatalogId::new(
            self.connection_id,
            candidate.kind,
            [
                &candidate.database,
                &candidate.schema,
                &candidate.name,
                &candidate.object_id.to_string(),
            ],
        );
        let entry = if candidate.kind.is_relation() {
            CatalogEntry::relation(
                id,
                schema.id.clone(),
                qualified_object(&candidate.database, &candidate.schema, &candidate.name),
                candidate.native_kind,
                OptionalMetadata::Supported(None),
                true,
            )
        } else {
            CatalogEntry::object(
                id,
                schema.id.clone(),
                qualified_object(&candidate.database, &candidate.schema, &candidate.name),
                candidate.native_kind,
                OptionalMetadata::Supported(None),
                false,
            )
        }
        .map_err(catalog_invariant)?;
        Ok(CatalogSearchHit {
            entry,
            ancestors: vec![database, schema],
        })
    }

    fn search_child_entry(
        &self,
        candidate: &MsSqlSearchCandidate,
        id: CatalogId,
    ) -> Result<CatalogEntry, DatabaseError> {
        let relation = CatalogId::new(
            self.connection_id,
            candidate.relation_kind,
            [
                candidate.database.clone(),
                candidate.schema.clone(),
                candidate.relation_name.clone(),
                candidate.relation_id.to_string(),
            ],
        );
        let metadata = match candidate.kind {
            CatalogKind::Column => {
                CatalogMetadata::Column(ColumnMetadata::new(0, "unknown", false))
            }
            CatalogKind::Index => CatalogMetadata::Index(IndexMetadata {
                columns: Vec::new(),
                unique: false,
            }),
            CatalogKind::PrimaryKey => {
                CatalogMetadata::Constraint(ConstraintMetadata::PrimaryKey {
                    columns: Vec::new(),
                })
            }
            CatalogKind::UniqueConstraint => {
                CatalogMetadata::Constraint(ConstraintMetadata::Unique {
                    columns: Vec::new(),
                })
            }
            CatalogKind::ForeignKey => {
                CatalogMetadata::Constraint(ConstraintMetadata::ForeignKey {
                    columns: Vec::new(),
                    referenced_relation: qualified_object(
                        &candidate.database,
                        &candidate.schema,
                        &candidate.relation_name,
                    ),
                    referenced_columns: Vec::new(),
                })
            }
            CatalogKind::CheckConstraint => {
                CatalogMetadata::Constraint(ConstraintMetadata::Check {
                    expression: String::new(),
                })
            }
            CatalogKind::Trigger => CatalogMetadata::None,
            _ => unreachable!(),
        };
        CatalogEntry::relation_child(
            id,
            relation,
            qualified_object(&candidate.database, &candidate.schema, &candidate.name),
            candidate.native_kind.clone(),
            OptionalMetadata::Supported(None),
            metadata,
        )
        .map_err(catalog_invariant)
    }

    pub async fn execute_pool(&self, sql: &str) -> Result<QueryOutcome, DatabaseError> {
        let pool = self.pool_for_database(&self.settings.database).await?;
        execute_pool(&pool, sql).await
    }

    pub async fn preview_relation(
        &self,
        relation: &CatalogId,
        options: &crate::model::relation::RelationPreviewOptions,
        mut page: crate::model::pagination::PageRequest,
    ) -> Result<RelationPreview, DatabaseError> {
        let target = format!("SQL Server relation {:?}", relation.native_path);
        if relation.profile_id() != self.connection_id
            || !relation.kind.is_relation()
            || relation.native_path.len() != 3
        {
            return Err(DatabaseError::configuration(format!(
                "catalog target is not a SQL Server relation: {target}"
            )));
        }
        let [database, schema, name] = relation.native_path.as_slice() else {
            unreachable!("relation path length was checked")
        };
        if !self.catalog_scope.allows_database(database)
            || !self.catalog_scope.allows_schema(database, schema)
        {
            return Err(DatabaseError::configuration(
                "SQL Server relation is outside the configured catalog scope",
            ));
        }
        let options = crate::sql::validate_relation_preview_options(
            options.where_clause.as_deref().unwrap_or_default(),
            options.order_by_clause.as_deref().unwrap_or_default(),
            crate::sql::SqlDialect::SqlServer,
        )
        .map_err(|error| DatabaseError::configuration(error.to_string()))?;
        let mut base_sql = format!(
            "SELECT * FROM {}.{}.{}",
            quote_identifier(database),
            quote_identifier(schema),
            quote_identifier(name)
        );
        let order_by_clause = options.order_by_clause;
        if let Some(clause) = options.where_clause {
            base_sql.push_str(" WHERE ");
            base_sql.push_str(&clause);
        }
        let filtered_sql = base_sql.clone();
        let order_by = order_by_clause
            .clone()
            .unwrap_or_else(|| "(SELECT NULL)".to_owned());

        let pool = self.pool_for_database(database).await?;
        let total = if page.resolve_total {
            let count_sql = format!(
                "SELECT COUNT_BIG(*) AS [__lazydb_count] FROM ({filtered_sql}) AS [__lazydb_count_source]"
            );
            let rows = query_rows(&pool, &count_sql).await?;
            let count = rows
                .first()
                .and_then(|row| row.try_get::<i64, _>(0).ok().flatten())
                .ok_or_else(|| decode_error("SQL Server returned an invalid relation row count"))?;
            let total = u64::try_from(count)
                .map_err(|_| decode_error("SQL Server returned an invalid relation row count"))?;
            page.offset = crate::model::pagination::ResultPagination::last_offset(page.size, total);
            Some(total)
        } else {
            None
        };
        let sql = format!(
            "SELECT * FROM ({base_sql}) AS [__lazydb_page] ORDER BY {order_by} OFFSET {} ROWS FETCH NEXT {} ROWS ONLY",
            page.offset,
            page.size.lookahead_limit()
        );
        let started = Instant::now();
        let mut result = query_result_set(&pool, &sql).await?;
        let fetched_len = result.rows.len();
        result.rows.truncate(page.size.get());
        Ok(RelationPreview {
            sql,
            result: QueryOutcome::from_result_set(result, started.elapsed(), Duration::ZERO),
            pagination: relation_pagination(page, fetched_len, total),
        })
    }

    pub async fn relation_ddl(&self, relation: &CatalogId) -> Result<RelationDdl, DatabaseError> {
        let target = CatalogTarget::RelationChildren {
            relation: relation.clone(),
        };
        if relation.profile_id() != self.connection_id
            || !relation.kind.is_relation()
            || relation.native_path.len() != 4
        {
            return Err(catalog_target_not_found(&target));
        }
        let [database, schema, name, object_id] = relation.native_path.as_slice() else {
            unreachable!("relation path length was checked")
        };
        let object_id = object_id
            .parse::<i32>()
            .map_err(|_| catalog_target_not_found(&target))?;
        if !self.catalog_scope.allows_schema(database, schema) {
            return Err(catalog_target_not_found(&target));
        }
        let schema_id = CatalogId::new(self.connection_id, CatalogKind::Schema, [database, schema]);
        let relation_entry = CatalogEntry::relation(
            relation.clone(),
            schema_id,
            qualified_object(database, schema, name),
            if relation.kind == CatalogKind::View {
                "view"
            } else {
                "user_table"
            },
            OptionalMetadata::Unsupported,
            true,
        )
        .map_err(catalog_invariant)?;
        let request = CatalogRequest {
            key: CatalogRequestKey {
                connection: ConnectionIdentity {
                    profile_id: self.connection_id,
                    generation: 0,
                },
                catalog_epoch: 0,
                request_id: 0,
                target,
                cursor: None,
            },
            scope: self.catalog_scope.clone(),
            page_size: RELATION_PREVIEW_LIMIT,
        };
        let children = self.load_relation_children_page(&request, relation).await?;
        let pool = self.pool_for_database(database).await?;
        let main = if relation.kind == CatalogKind::View {
            match load_module_definition(&pool, object_id).await? {
                Some(definition) => definition,
                None => {
                    return Err(catalog_internal(
                        "SQL Server view definition is unavailable (encrypted or hidden by permissions)",
                    ));
                }
            }
        } else {
            reconstruct_relation_ddl(database, schema, name, &children)?
        };
        let trigger_sql = load_relation_trigger_definitions(&pool, object_id).await?;
        let sql = assemble_ddl(vec![
            DdlSection {
                label: "Object",
                statements: vec![main],
            },
            DdlSection {
                label: "Triggers",
                statements: trigger_sql,
            },
        ])
        .ok_or_else(|| {
            catalog_internal("SQL Server relation DDL assembly produced no statements")
        })?;
        Ok(RelationDdl {
            relation: relation_entry,
            children,
            sql,
            provenance: DdlProvenance::AdapterGenerated,
        })
    }

    pub async fn object_ddl(
        &self,
        kind: CatalogKind,
        schema: &str,
        name: &str,
    ) -> Result<Option<String>, DatabaseError> {
        if kind == CatalogKind::Sequence {
            return self.sequence_ddl(schema, name).await;
        }
        if !matches!(
            kind,
            CatalogKind::View
                | CatalogKind::Function
                | CatalogKind::Procedure
                | CatalogKind::Trigger
        ) {
            return Ok(None);
        }
        let pool = self.pool_for_database(&self.settings.database).await?;
        let object_id_sql = match kind {
            CatalogKind::Trigger => format!(
                "SELECT tr.[object_id] FROM {}.sys.triggers tr JOIN {}.sys.objects parent ON parent.[object_id] = tr.[parent_id] JOIN {}.sys.schemas s ON s.[schema_id] = parent.[schema_id] WHERE s.[name] = {} AND tr.[name] = {} AND tr.[is_ms_shipped] = 0",
                quote_identifier(&self.settings.database),
                quote_identifier(&self.settings.database),
                quote_identifier(&self.settings.database),
                quote_literal(schema),
                quote_literal(name)
            ),
            _ => format!(
                "SELECT o.[object_id] FROM {}.sys.objects o JOIN {}.sys.schemas s ON s.[schema_id] = o.[schema_id] WHERE s.[name] = {} AND o.[name] = {} AND o.[is_ms_shipped] = 0",
                quote_identifier(&self.settings.database),
                quote_identifier(&self.settings.database),
                quote_literal(schema),
                quote_literal(name)
            ),
        };
        let rows = query_rows(&pool, &object_id_sql).await?;
        let Some(object_id) = rows
            .first()
            .and_then(|row| row.try_get::<i32, _>(0).ok().flatten())
        else {
            return Ok(None);
        };
        load_module_definition(&pool, object_id).await
    }

    async fn sequence_ddl(
        &self,
        schema: &str,
        name: &str,
    ) -> Result<Option<String>, DatabaseError> {
        let pool = self.pool_for_database(&self.settings.database).await?;
        let sql = format!(
            "SELECT CAST(s.[start_value] AS nvarchar(128)) AS [start_value], CAST(s.[increment] AS nvarchar(128)) AS [increment], CAST(s.[minimum_value] AS nvarchar(128)) AS [minimum_value], CAST(s.[maximum_value] AS nvarchar(128)) AS [maximum_value], s.[is_cycling], s.[is_cached] FROM {}.sys.sequences s JOIN {}.sys.schemas sc ON sc.[schema_id] = s.[schema_id] WHERE sc.[name] = {} AND s.[name] = {} AND s.[is_ms_shipped] = 0",
            quote_identifier(&self.settings.database),
            quote_identifier(&self.settings.database),
            quote_literal(schema),
            quote_literal(name)
        );
        let Some(row) = query_rows(&pool, &sql).await?.into_iter().next() else {
            return Ok(None);
        };
        let start = required_string(&row, "start_value")?;
        let increment = required_string(&row, "increment")?;
        let minimum = required_string(&row, "minimum_value")?;
        let maximum = required_string(&row, "maximum_value")?;
        let cycling = row
            .try_get::<bool, _>("is_cycling")
            .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
            .unwrap_or(false);
        let cached = row
            .try_get::<bool, _>("is_cached")
            .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
            .unwrap_or(false);
        Ok(Some(format!(
            "CREATE SEQUENCE {}.{} AS bigint START WITH {start} INCREMENT BY {increment} MINVALUE {minimum} MAXVALUE {maximum} {} {}",
            quote_identifier(schema),
            quote_identifier(name),
            if cycling { "CYCLE" } else { "NO CYCLE" },
            if cached { "CACHE" } else { "NO CACHE" }
        )))
    }

    pub fn quote_identifier(&self, value: &str) -> String {
        quote_identifier(value)
    }

    pub async fn close(self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        let pools = {
            let mut pools = self.pools.lock().await;
            pools.drain().map(|(_, pool)| pool).collect::<Vec<_>>()
        };
        for pool in pools {
            pool.close().await;
        }
    }
}

async fn load_module_definition(
    pool: &MsSqlConnectionPool,
    object_id: i32,
) -> Result<Option<String>, DatabaseError> {
    let sql = format!(
        "SELECT [definition] FROM {}.sys.sql_modules WHERE [object_id] = {object_id}",
        quote_identifier(&pool.settings.database)
    );
    query_rows(pool, &sql)
        .await?
        .first()
        .map(|row| optional_string(row, "definition"))
        .transpose()
        .map(|value| value.flatten())
}

async fn load_relation_trigger_definitions(
    pool: &MsSqlConnectionPool,
    relation_id: i32,
) -> Result<Vec<String>, DatabaseError> {
    let sql = format!(
        "SELECT tr.[name], sm.[definition] FROM {}.sys.triggers tr LEFT JOIN {}.sys.sql_modules sm ON sm.[object_id] = tr.[object_id] WHERE tr.[parent_id] = {relation_id} AND tr.[is_ms_shipped] = 0 ORDER BY tr.[name] COLLATE Latin1_General_100_BIN2, tr.[object_id]",
        quote_identifier(&pool.settings.database),
        quote_identifier(&pool.settings.database)
    );
    let mut definitions = Vec::new();
    for row in query_rows(pool, &sql).await? {
        let name = required_string(&row, "name")?;
        let definition = optional_string(&row, "definition")?
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                catalog_internal(format!(
                    "SQL Server trigger {name} definition is unavailable (encrypted or hidden by permissions)"
                ))
            })?;
        definitions.push(definition);
    }
    Ok(definitions)
}

fn reconstruct_relation_ddl(
    database: &str,
    schema: &str,
    name: &str,
    children: &CatalogPage,
) -> Result<String, DatabaseError> {
    let qualified = format!("{}.{}", quote_identifier(schema), quote_identifier(name));
    let mut columns = children
        .entries
        .iter()
        .filter_map(|entry| match &entry.metadata {
            CatalogMetadata::Column(metadata) => Some((metadata.ordinal_position, entry, metadata)),
            _ => None,
        })
        .collect::<Vec<_>>();
    columns.sort_by_key(|(ordinal, _, _)| *ordinal);
    if columns.is_empty() {
        return Err(catalog_internal(format!(
            "SQL Server relation {database}.{schema}.{name} has no columns"
        )));
    }
    let column_sql = columns
        .into_iter()
        .map(|(_, entry, metadata)| {
            let mut definition = format!(
                "{} {}",
                quote_identifier(&entry.qualified_name.object),
                metadata.native_type
            );
            if let OptionalMetadata::Supported(Some(expression)) = &metadata.generated_expression {
                definition.push_str(" AS ");
                definition.push_str(expression);
            } else {
                if let OptionalMetadata::Supported(Some(expression)) = &metadata.default_expression
                {
                    definition.push_str(" DEFAULT ");
                    definition.push_str(expression);
                }
                if !metadata.nullable {
                    definition.push_str(" NOT NULL");
                }
            }
            definition
        })
        .collect::<Vec<_>>();
    let mut statements = vec![format!(
        "CREATE TABLE {qualified} (\n    {}\n)",
        column_sql.join(",\n    ")
    )];
    let mut indexes = children
        .entries
        .iter()
        .filter_map(|entry| match &entry.metadata {
            CatalogMetadata::Index(index) => {
                Some((entry.qualified_name.object.clone(), index.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let constraint_names = children
        .entries
        .iter()
        .filter(|entry| matches!(entry.metadata, CatalogMetadata::Constraint(_)))
        .map(|entry| entry.qualified_name.object.as_str())
        .collect::<HashSet<_>>();
    indexes.retain(|(index_name, _)| !constraint_names.contains(index_name.as_str()));
    indexes.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    for (index_name, index) in indexes {
        if index.columns.is_empty() {
            continue;
        }
        statements.push(reconstructed_index_sql(&qualified, &index_name, &index));
    }
    let mut constraints = children
        .entries
        .iter()
        .filter_map(|entry| match &entry.metadata {
            CatalogMetadata::Constraint(constraint) => {
                Some((entry.qualified_name.object.clone(), constraint.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    constraints.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    for (constraint_name, constraint) in constraints {
        let statement = match constraint {
            ConstraintMetadata::PrimaryKey { columns } => format!(
                "ALTER TABLE {qualified} ADD CONSTRAINT {} PRIMARY KEY ({})",
                quote_identifier(&constraint_name),
                quoted_columns(&columns)
            ),
            ConstraintMetadata::Unique { columns } => format!(
                "ALTER TABLE {qualified} ADD CONSTRAINT {} UNIQUE ({})",
                quote_identifier(&constraint_name),
                quoted_columns(&columns)
            ),
            ConstraintMetadata::Check { expression } => format!(
                "ALTER TABLE {qualified} ADD CONSTRAINT {} CHECK ({expression})",
                quote_identifier(&constraint_name)
            ),
            ConstraintMetadata::ForeignKey {
                columns,
                referenced_relation,
                referenced_columns,
            } => format!(
                "ALTER TABLE {qualified} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({})",
                quote_identifier(&constraint_name),
                quoted_columns(&columns),
                qualified_name_sql(&referenced_relation),
                quoted_columns(&referenced_columns)
            ),
        };
        statements.push(statement);
    }
    Ok(assemble_ddl(vec![DdlSection {
        label: "Table",
        statements,
    }])
    .expect("relation DDL has a table statement"))
}

fn reconstructed_index_sql(qualified: &str, name: &str, index: &IndexMetadata) -> String {
    format!(
        "CREATE {}INDEX {} ON {qualified} ({})",
        if index.unique { "UNIQUE " } else { "" },
        quote_identifier(name),
        quoted_columns(&index.columns)
    )
}

fn quoted_columns(columns: &[String]) -> String {
    columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ")
}

fn qualified_name_sql(name: &QualifiedName) -> String {
    match (&name.schema, &name.object) {
        (Some(schema), object) => {
            format!("{}.{}", quote_identifier(schema), quote_identifier(object))
        }
        (None, object) => quote_identifier(object),
    }
}

async fn execute_pool(
    pool: &MsSqlConnectionPool,
    sql: &str,
) -> Result<QueryOutcome, DatabaseError> {
    let batches = crate::sql::split_sql_server_batches(sql).map_err(|error| DatabaseError {
        category: ErrorCategory::Sql,
        code: Some("sql_server_go_count_unsupported".to_owned()),
        message: crate::security::sanitize_terminal_text(&error.to_string()),
    })?;
    let mut lease = pool.checkout().await?;
    let mut result_sets = Vec::new();
    let mut execution = std::time::Duration::ZERO;
    let mut fetch = std::time::Duration::ZERO;
    let mut row_count = 0;

    for (index, batch) in batches.into_iter().enumerate() {
        let outcome = execute_one_batch(&mut lease, batch)
            .await
            .map_err(|error| batch_error(index + 1, error))?;
        result_sets.extend(outcome.result_sets);
        execution += outcome.stats.execution;
        fetch += outcome.stats.fetch;
        row_count += outcome.stats.row_count;
    }

    lease.mark_reusable();
    Ok(QueryOutcome {
        result_sets,
        stats: QueryStats::new(execution, fetch, row_count),
    })
}

async fn execute_one_batch(
    client: &mut TdsClient,
    sql: &str,
) -> Result<QueryOutcome, DatabaseError> {
    let started = std::time::Instant::now();
    let affected_rows_column = format!("__lazydb_affected_rows_{}", Uuid::new_v4().simple());
    let batch = format!("{sql}\n;SELECT CONVERT(bigint, @@ROWCOUNT) AS [{affected_rows_column}]");
    let mut stream = client
        .simple_query(batch)
        .await
        .map_err(|error| tiberius_error(error, ErrorCategory::Sql))?;
    let execution = started.elapsed();
    let mut result_sets = Vec::new();
    let mut current = None;
    let mut affected_rows = None;

    while let Some(item) = stream
        .try_next()
        .await
        .map_err(|error| tiberius_error(error, ErrorCategory::Sql))?
    {
        match item {
            QueryItem::Metadata(metadata) => {
                if let Some(result_set) = current.take() {
                    result_sets.push(result_set);
                }
                current = Some(ResultSet {
                    columns: metadata.columns().iter().map(decode_column).collect(),
                    rows: Vec::new(),
                    affected_rows: 0,
                });
            }
            QueryItem::Row(row) if is_affected_rows_result(&row, &affected_rows_column) => {
                affected_rows = row
                    .try_get::<i64, _>(0)
                    .map_err(|error| tiberius_error(error, ErrorCategory::Internal))?
                    .and_then(|value| u64::try_from(value).ok());
                current = None;
            }
            QueryItem::Row(row) => {
                let result_set = current.as_mut().ok_or_else(|| {
                    decode_error("SQL Server returned a row before its result metadata")
                })?;
                result_set.rows.push(decode_row(row));
            }
        }
    }
    drop(stream);

    if let Some(result_set) = current {
        result_sets.push(result_set);
    }
    let affected_rows = affected_rows
        .ok_or_else(|| decode_error("SQL Server did not return the internal affected-row count"))?;
    let has_tabular_result = result_sets.iter().any(|result| !result.columns.is_empty());
    if result_sets.is_empty() {
        result_sets.push(ResultSet::default());
    }
    if affected_rows != 0 && !has_tabular_result {
        if let Some(result_set) = result_sets.last_mut() {
            result_set.affected_rows = affected_rows;
        } else {
            result_sets.push(ResultSet {
                columns: Vec::new(),
                rows: Vec::new(),
                affected_rows,
            });
        }
    }

    let total = started.elapsed();
    let row_count = result_sets.iter().map(|set| set.rows.len()).sum();
    Ok(QueryOutcome {
        result_sets,
        stats: QueryStats::new(execution, total.saturating_sub(execution), row_count),
    })
}

pub struct MsSqlTransactionBackend {
    client: Option<TdsClient>,
    settings: MsSqlConnectSettings,
    database: String,
    depth: usize,
}

#[async_trait::async_trait]
impl TransactionBackend for MsSqlTransactionBackend {
    async fn begin(&mut self) -> Result<(), TransactionError> {
        self.execute("SET XACT_ABORT ON").await?;
        self.execute("BEGIN TRANSACTION").await?;
        self.depth = 1;
        Ok(())
    }

    async fn execute(&mut self, sql: &str) -> Result<QueryOutcome, TransactionError> {
        let batches = crate::sql::split_sql_server_batches(sql)
            .map_err(|error| TransactionError(error.to_string()))?;
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| TransactionError("SQL Server transaction client is closed".into()))?;
        let mut result_sets = Vec::new();
        let mut execution = std::time::Duration::ZERO;
        let mut fetch = std::time::Duration::ZERO;
        let mut row_count = 0;
        for (index, batch) in batches.into_iter().enumerate() {
            let outcome = execute_one_batch(client, batch)
                .await
                .map_err(|error| TransactionError(batch_error(index + 1, error).to_string()))?;
            result_sets.extend(outcome.result_sets);
            execution += outcome.stats.execution;
            fetch += outcome.stats.fetch;
            row_count += outcome.stats.row_count;
        }
        Ok(QueryOutcome {
            result_sets,
            stats: QueryStats::new(execution, fetch, row_count),
        })
    }

    async fn relation_mutation(
        &mut self,
        request: RelationMutationRequest,
    ) -> Result<MutationResult, TransactionError> {
        let [database, schema, relation, _] = request.relation.native_path.as_slice() else {
            return Err(TransactionError(
                "SQL Server relation has no canonical database, schema, table, and object path"
                    .into(),
            ));
        };
        let columns = &request.metadata.columns;
        if columns.is_empty() {
            return Err(TransactionError(
                "SQL Server relation mutation has no relation columns".into(),
            ));
        }
        let quoted_table = format!(
            "{}.{}.{}",
            quote_identifier(database),
            quote_identifier(schema),
            quote_identifier(relation)
        );
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| TransactionError("SQL Server transaction client is closed".into()))?;

        match request.operation {
            RelationMutation::DeleteRows(rows) => {
                for mutation in &rows {
                    validate_delete_mutation(mutation, columns.len())?;
                }
                let savepoint = format!("lazydb_relation_{}", Uuid::new_v4().simple());
                client
                    .simple_query(format!("SAVE TRANSACTION [{savepoint}]"))
                    .await
                    .map_err(|error| mssql_mutation_error("delete", error))?;
                for mutation in &rows {
                    let mut sql = format!("DELETE FROM {quoted_table} WHERE ");
                    let mut predicates = Vec::new();
                    for index in &mutation.row.columns {
                        predicates.push(mssql_null_safe_predicate(
                            &quote_identifier(&columns[*index].0),
                            predicates.len() * 2 + 1,
                        ));
                    }
                    for column in columns {
                        predicates.push(mssql_null_safe_predicate(
                            &quote_identifier(&column.0),
                            predicates.len() * 2 + 1,
                        ));
                    }
                    sql.push_str(&predicates.join(" AND "));
                    let mut query = Query::new(sql);
                    for value in &mutation.row.values {
                        bind_mssql_cell(&mut query, value)?;
                        bind_mssql_cell(&mut query, value)?;
                    }
                    for value in &mutation.original {
                        bind_mssql_cell(&mut query, value)?;
                        bind_mssql_cell(&mut query, value)?;
                    }
                    let affected = query
                        .execute(client)
                        .await
                        .map_err(|error| mssql_mutation_error("delete", error))?
                        .total();
                    if affected != 1 {
                        let _ = client
                            .simple_query(format!("ROLLBACK TRANSACTION [{savepoint}]"))
                            .await;
                        return Err(TransactionError(
                            "SQL Server relation mutation conflict: delete matched no row or multiple rows"
                                .into(),
                        ));
                    }
                }
                Ok(MutationResult::Deleted { rows: rows.len() })
            }
            RelationMutation::InsertRow(insert) => {
                validate_insert_mutation(&insert, columns.len())?;
                let names = insert
                    .columns
                    .iter()
                    .map(|index| quote_identifier(&columns[*index].0))
                    .collect::<Vec<_>>();
                let mut parameter = 0;
                let expressions = insert
                    .values
                    .iter()
                    .map(|value| match value {
                        InputValue::Default => "DEFAULT".to_owned(),
                        InputValue::Null | InputValue::Value(_) => {
                            parameter += 1;
                            format!("@P{parameter}")
                        }
                    })
                    .collect::<Vec<_>>();
                let sql = if names.is_empty() {
                    format!("INSERT INTO {quoted_table} DEFAULT VALUES OUTPUT inserted.*")
                } else {
                    format!(
                        "INSERT INTO {quoted_table} ({}) OUTPUT inserted.* VALUES ({})",
                        names.join(", "),
                        expressions.join(", ")
                    )
                };
                let mut query = Query::new(sql);
                for value in &insert.values {
                    if !matches!(value, InputValue::Default) {
                        bind_mssql_input(&mut query, value)?;
                    }
                }
                let mut result = query_rows_from_query(query, client)
                    .await
                    .map_err(|error| mssql_mutation_error("insert", error))?;
                if result.len() != 1 {
                    return Err(TransactionError(if result.is_empty() {
                        "SQL Server insert returned no inserted row (possibly an INSTEAD OF trigger)"
                            .into()
                    } else {
                        "SQL Server insert returned multiple rows unexpectedly".into()
                    }));
                }
                let row = result.pop().expect("checked exactly one inserted row");
                /*
                 * OUTPUT is deliberately used here instead of a follow-up lookup: it observes
                 * identity, defaults, computed columns, and rowversion values atomically.
                 */
                Ok(MutationResult::Inserted {
                    row: decode_row(row),
                })
            }
            RelationMutation::UpdateCell(update) => {
                let savepoint = format!("lazydb_relation_{}", Uuid::new_v4().simple());
                client
                    .simple_query(format!("SAVE TRANSACTION [{savepoint}]"))
                    .await
                    .map_err(|error| mssql_mutation_error("update", error))?;
                let Some((column_name, _, _)) = columns.get(update.column) else {
                    return Err(TransactionError(
                        "SQL Server update column is out of range".into(),
                    ));
                };
                if update.row.columns.len() != update.row.values.len() {
                    return Err(TransactionError(
                        "SQL Server row locator is malformed".into(),
                    ));
                }
                if update.row.columns.is_empty()
                    || update
                        .row
                        .columns
                        .iter()
                        .any(|index| *index >= columns.len())
                {
                    return Err(TransactionError(
                        "SQL Server row locator must contain valid primary-key, non-null unique, or full-row columns"
                        .into(),
                    ));
                }
                if update.row.columns.windows(2).any(|pair| pair[0] == pair[1]) {
                    return Err(TransactionError(
                        "SQL Server row locator contains duplicate columns".into(),
                    ));
                }
                let primary_key = request
                    .metadata
                    .primary_key
                    .iter()
                    .map(|name| {
                        columns
                            .iter()
                            .position(|(column, _, _)| column == name)
                            .ok_or_else(|| {
                                TransactionError(
                                    "SQL Server metadata primary-key column is missing".into(),
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let full_row = update.row.columns.len() == columns.len()
                    && update
                        .row
                        .columns
                        .iter()
                        .enumerate()
                        .all(|(index, value)| *value == index);
                let is_primary_key = update.row.columns == primary_key;
                let non_null_locator = update.row.columns.iter().all(|index| !columns[*index].2);
                if !full_row && !is_primary_key && !non_null_locator {
                    return Err(TransactionError(
                        "SQL Server row locator must be the primary key, a non-null unique key, or the full row"
                            .into(),
                    ));
                }
                if matches!(update.original, CellValue::Unsupported { .. }) {
                    return Err(TransactionError(
                        "SQL Server cannot compare an unsupported original cell value".into(),
                    ));
                }
                let set = match &update.value {
                    InputValue::Default => format!("{} = DEFAULT", quote_identifier(column_name)),
                    InputValue::Null | InputValue::Value(_) => {
                        format!("{} = @P1", quote_identifier(column_name))
                    }
                };
                let mut sql = format!("UPDATE {quoted_table} SET {set} OUTPUT inserted.* WHERE ");
                let mut predicates = Vec::new();
                for index in &update.row.columns {
                    predicates.push(mssql_null_safe_predicate(
                        &quote_identifier(&columns[*index].0),
                        predicates.len() * 2
                            + if matches!(update.value, InputValue::Default) {
                                1
                            } else {
                                2
                            },
                    ));
                }
                predicates.push(mssql_null_safe_predicate(
                    &quote_identifier(column_name),
                    predicates.len() * 2
                        + if matches!(update.value, InputValue::Default) {
                            1
                        } else {
                            2
                        },
                ));
                sql.push_str(&predicates.join(" AND "));
                let mut query = Query::new(sql);
                if !matches!(update.value, InputValue::Default) {
                    bind_mssql_input(&mut query, &update.value)?;
                }
                for value in &update.row.values {
                    bind_mssql_cell(&mut query, value)?;
                    bind_mssql_cell(&mut query, value)?;
                }
                bind_mssql_cell(&mut query, &update.original)?;
                bind_mssql_cell(&mut query, &update.original)?;
                let mut result = query_rows_from_query(query, client)
                    .await
                    .map_err(|error| mssql_mutation_error("update", error))?;
                if result.len() != 1 {
                    let _ = client
                        .simple_query(format!("ROLLBACK TRANSACTION [{savepoint}]"))
                        .await;
                    return Err(TransactionError(if result.is_empty() {
                        "SQL Server relation mutation conflict: update matched no row".into()
                    } else {
                        "SQL Server relation mutation conflict: update matched multiple rows".into()
                    }));
                }
                let row = result.pop().expect("checked exactly one updated row");
                Ok(MutationResult::Updated {
                    row: decode_row(row),
                })
            }
        }
    }

    async fn commit(&mut self) -> Result<(), TransactionError> {
        if self.depth == 0 {
            return Ok(());
        }
        self.execute("COMMIT TRANSACTION").await?;
        self.depth = 0;
        Ok(())
    }

    async fn rollback(&mut self) -> Result<(), TransactionError> {
        if self.depth == 0 {
            return Ok(());
        }
        self.execute("ROLLBACK TRANSACTION").await?;
        self.depth = 0;
        Ok(())
    }

    async fn cancel(&mut self) -> Result<(), TransactionError> {
        // Tiberius exposes no TDS attention/cancel primitive. Closing the dedicated transport
        // is the only safe way to interrupt a running request; the worker will quarantine this
        // backend rather than trying to reuse a stream whose response was abandoned.
        let Some(client) = self.client.take() else {
            return Err(TransactionError(
                "SQL Server transaction client is closed".into(),
            ));
        };
        drop(client);
        self.depth = 0;
        Ok(())
    }

    fn depth(&self) -> usize {
        self.depth
    }

    fn force_close(self) -> BoxFuture<'static, Result<(), TransactionError>> {
        Box::pin(async move {
            // Never return a possibly dirty session to the SQL Server pool. Dropping the
            // transport is intentional: force-close must not claim commit or rollback.
            drop(self.client);
            drop(self.settings);
            drop(self.database);
            Ok(())
        })
    }
}

fn mssql_null_safe_predicate(column: &str, first_parameter: usize) -> String {
    format!(
        "(({column} = @P{first_parameter}) OR ({column} IS NULL AND @P{} IS NULL))",
        first_parameter + 1
    )
}

fn validate_delete_mutation(
    mutation: &crate::db::mutation::DeleteRowMutation,
    column_count: usize,
) -> Result<(), TransactionError> {
    if mutation.row.columns.is_empty()
        || mutation.row.columns.len() != mutation.row.values.len()
        || mutation.original.len() != column_count
        || mutation
            .row
            .columns
            .iter()
            .any(|index| *index >= column_count)
        || mutation
            .row
            .values
            .iter()
            .chain(mutation.original.iter())
            .any(|value| matches!(value, CellValue::Unsupported { .. }))
    {
        return Err(TransactionError(
            "SQL Server delete mutation is malformed or contains an unsupported value".into(),
        ));
    }
    Ok(())
}

fn validate_insert_mutation(
    insert: &crate::db::mutation::InsertRowMutation,
    column_count: usize,
) -> Result<(), TransactionError> {
    if insert.columns.len() != insert.values.len()
        || insert.columns.iter().any(|index| *index >= column_count)
        || insert
            .values
            .iter()
            .any(|value| matches!(value, InputValue::Value(CellValue::Unsupported { .. })))
    {
        return Err(TransactionError(
            "SQL Server insert mutation is malformed or contains an unsupported value".into(),
        ));
    }
    if insert
        .columns
        .windows(2)
        .any(|columns| columns[0] >= columns[1])
    {
        return Err(TransactionError(
            "SQL Server insert mutation columns must be unique and ordered".into(),
        ));
    }
    Ok(())
}

fn bind_mssql_input(
    query: &mut Query<'static>,
    value: &InputValue,
) -> Result<(), TransactionError> {
    match value {
        InputValue::Null => query.bind(Option::<String>::None),
        InputValue::Value(value) => bind_mssql_cell(query, value)?,
        InputValue::Default => {
            return Err(TransactionError(
                "SQL Server DEFAULT values must not be bound".into(),
            ));
        }
    }
    Ok(())
}

fn bind_mssql_cell(query: &mut Query<'static>, value: &CellValue) -> Result<(), TransactionError> {
    match value {
        CellValue::Null => query.bind(Option::<String>::None),
        CellValue::Boolean(value) => query.bind(*value),
        CellValue::Integer(value) => query.bind(*value),
        CellValue::Unsigned(value) => query.bind(i64::try_from(*value).map_err(|_| {
            TransactionError("SQL Server cannot bind an unsigned value larger than i64".into())
        })?),
        CellValue::Float(value) => query.bind(*value),
        CellValue::Text(value) => query.bind(value.clone()),
        CellValue::Bytes(value) => query.bind(value.clone()),
        CellValue::Date(value) => query.bind(*value),
        CellValue::Time(value) => query.bind(*value),
        CellValue::DateTime(value) => query.bind(*value),
        CellValue::Timestamp(value) => query.bind(*value),
        CellValue::Unsupported { .. } => {
            return Err(TransactionError(
                "SQL Server cannot bind an unsupported cell value".into(),
            ));
        }
    }
    Ok(())
}

async fn query_rows_from_query(
    query: Query<'static>,
    client: &mut TdsClient,
) -> Result<Vec<Row>, tiberius::error::Error> {
    let mut stream = query.query(client).await?;
    let mut rows = Vec::new();
    while let Some(item) = stream.try_next().await? {
        if let QueryItem::Row(row) = item {
            rows.push(row);
        }
    }
    Ok(rows)
}

fn mssql_mutation_error(operation: &str, error: tiberius::error::Error) -> TransactionError {
    let message = error.to_string();
    let detail = if message.to_ascii_lowercase().contains("identity") {
        "identity columns cannot be edited explicitly"
    } else if message.to_ascii_lowercase().contains("computed") {
        "computed columns cannot be edited explicitly"
    } else if message.to_ascii_lowercase().contains("timestamp")
        || message.to_ascii_lowercase().contains("rowversion")
    {
        "rowversion columns cannot be edited explicitly"
    } else {
        "mutation failed"
    };
    TransactionError(crate::security::sanitize_terminal_text(&format!(
        "SQL Server {operation} relation mutation {detail}: {message}"
    )))
}

fn batch_error(batch: usize, error: DatabaseError) -> DatabaseError {
    DatabaseError {
        category: error.category,
        code: error.code,
        message: crate::security::sanitize_terminal_text(&format!(
            "SQL Server batch {batch} failed: {}",
            error.message
        )),
    }
}

fn is_affected_rows_result(row: &Row, affected_rows_column: &str) -> bool {
    row.columns().len() == 1 && row.columns()[0].name() == affected_rows_column
}

fn decode_column(column: &tiberius::Column) -> ColumnMeta {
    ColumnMeta {
        name: column.name().to_owned(),
        type_name: column_type_name(column.column_type()).to_owned(),
    }
}

fn decode_row(row: Row) -> Vec<CellValue> {
    let column_types = row
        .columns()
        .iter()
        .map(|column| column.column_type())
        .collect::<Vec<_>>();
    row.into_iter()
        .zip(column_types)
        .map(|(data, column_type)| decode_cell(column_type, data))
        .collect()
}

fn decode_cell(column_type: ColumnType, data: ColumnData<'static>) -> CellValue {
    if column_data_is_null(&data) {
        return CellValue::Null;
    }

    match (column_type, data) {
        (ColumnType::Udt | ColumnType::SSVariant, data) => unsupported_cell(column_type, &data),
        (ColumnType::Money | ColumnType::Money4, _) => CellValue::Unsupported {
            type_name: column_type_name(column_type).to_owned(),
            preview: "exact value unavailable from Tiberius".to_owned(),
        },
        (_, ColumnData::Bit(Some(value))) => CellValue::Boolean(value),
        (_, ColumnData::U8(Some(value))) => CellValue::Unsigned(value.into()),
        (_, ColumnData::I16(Some(value))) => CellValue::Integer(value.into()),
        (_, ColumnData::I32(Some(value))) => CellValue::Integer(value.into()),
        (_, ColumnData::I64(Some(value))) => CellValue::Integer(value),
        (_, ColumnData::F32(Some(value))) => CellValue::Float(value.into()),
        (_, ColumnData::F64(Some(value))) => CellValue::Float(value),
        (_, ColumnData::Numeric(Some(value))) => CellValue::Text(format_numeric(value)),
        (
            ColumnType::BigVarChar
            | ColumnType::BigChar
            | ColumnType::NVarchar
            | ColumnType::NChar
            | ColumnType::Text
            | ColumnType::NText,
            ColumnData::String(Some(value)),
        ) => CellValue::Text(value.into_owned()),
        (ColumnType::Xml, ColumnData::Xml(Some(value))) => {
            CellValue::Text(value.into_owned().into_string())
        }
        (
            ColumnType::BigVarBin | ColumnType::BigBinary | ColumnType::Image,
            ColumnData::Binary(Some(value)),
        ) => CellValue::Bytes(value.into_owned()),
        (ColumnType::Guid, ColumnData::Guid(Some(value))) => CellValue::Text(value.to_string()),
        (ColumnType::Daten, data) => decode_owned::<NaiveDate>(data, CellValue::Date),
        (ColumnType::Timen, data) => decode_owned::<NaiveTime>(data, CellValue::Time),
        (
            ColumnType::Datetime
            | ColumnType::Datetime4
            | ColumnType::Datetimen
            | ColumnType::Datetime2,
            data,
        ) => decode_owned::<NaiveDateTime>(data, CellValue::DateTime),
        (ColumnType::DatetimeOffsetn, ColumnData::DateTimeOffset(Some(value))) => {
            decode_datetimeoffset(value)
        }
        (column_type, data) => unsupported_cell(column_type, &data),
    }
}

fn format_numeric(value: tiberius::numeric::Numeric) -> String {
    let coefficient = value.value();
    let scale = usize::from(value.scale());
    if scale == 0 {
        return coefficient.to_string();
    }

    let negative = coefficient.is_negative();
    let digits = coefficient.unsigned_abs().to_string();
    let split = digits.len().saturating_sub(scale);
    let mut formatted =
        String::with_capacity(digits.len().max(scale + 1) + usize::from(negative) + 1);
    if negative {
        formatted.push('-');
    }
    if split == 0 {
        formatted.push_str("0.");
        formatted.extend(std::iter::repeat_n('0', scale - digits.len()));
        formatted.push_str(&digits);
    } else {
        formatted.push_str(&digits[..split]);
        formatted.push('.');
        formatted.push_str(&digits[split..]);
    }
    formatted
}

fn decode_datetimeoffset(value: tiberius::time::DateTimeOffset) -> CellValue {
    let datetime = value.datetime2();
    let date = NaiveDate::from_ymd_opt(1, 1, 1)
        .and_then(|date| date.checked_add_days(chrono::Days::new(datetime.date().days().into())));
    let time = datetime.time();
    let nanoseconds = time
        .increments()
        .checked_mul(10_u64.pow(9 - u32::from(time.scale())));
    let time = nanoseconds.and_then(|nanoseconds| {
        u32::try_from(nanoseconds / 1_000_000_000)
            .ok()
            .and_then(|seconds| {
                NaiveTime::from_num_seconds_from_midnight_opt(
                    seconds,
                    (nanoseconds % 1_000_000_000) as u32,
                )
            })
    });
    let offset = FixedOffset::east_opt(i32::from(value.offset()) * 60);

    match (date, time, offset) {
        (Some(date), Some(time), Some(offset)) => {
            let local = NaiveDateTime::new(date, time);
            CellValue::Timestamp(DateTime::from_naive_utc_and_offset(
                local - chrono::Duration::seconds(i64::from(offset.local_minus_utc())),
                offset,
            ))
        }
        _ => CellValue::Unsupported {
            type_name: "datetimeoffset".to_owned(),
            preview: bounded_preview(&format!("{value:?}")),
        },
    }
}

fn decode_owned<T: FromSqlOwned>(
    data: ColumnData<'static>,
    convert: impl FnOnce(T) -> CellValue,
) -> CellValue {
    match T::from_sql_owned(data) {
        Ok(Some(value)) => convert(value),
        Ok(None) => CellValue::Null,
        Err(error) => CellValue::Unsupported {
            type_name: std::any::type_name::<T>().to_owned(),
            preview: bounded_preview(&error.to_string()),
        },
    }
}

fn column_data_is_null(data: &ColumnData<'_>) -> bool {
    match data {
        ColumnData::U8(value) => value.is_none(),
        ColumnData::I16(value) => value.is_none(),
        ColumnData::I32(value) => value.is_none(),
        ColumnData::I64(value) => value.is_none(),
        ColumnData::F32(value) => value.is_none(),
        ColumnData::F64(value) => value.is_none(),
        ColumnData::Bit(value) => value.is_none(),
        ColumnData::String(value) => value.is_none(),
        ColumnData::Guid(value) => value.is_none(),
        ColumnData::Binary(value) => value.is_none(),
        ColumnData::Numeric(value) => value.is_none(),
        ColumnData::Xml(value) => value.is_none(),
        ColumnData::DateTime(value) => value.is_none(),
        ColumnData::SmallDateTime(value) => value.is_none(),
        ColumnData::Time(value) => value.is_none(),
        ColumnData::Date(value) => value.is_none(),
        ColumnData::DateTime2(value) => value.is_none(),
        ColumnData::DateTimeOffset(value) => value.is_none(),
    }
}

fn unsupported_cell(column_type: ColumnType, data: &ColumnData<'_>) -> CellValue {
    let preview = match data {
        ColumnData::String(Some(value)) => bounded_preview(value),
        ColumnData::Binary(Some(value)) => {
            let shown = value
                .iter()
                .take(UNSUPPORTED_PREVIEW_LEN)
                .copied()
                .collect();
            bounded_preview(&CellValue::Bytes(shown).clipboard_text())
        }
        _ => bounded_preview(&format!("{data:?}")),
    };
    CellValue::Unsupported {
        type_name: column_type_name(column_type).to_owned(),
        preview,
    }
}

fn bounded_preview(value: &str) -> String {
    crate::security::sanitize_terminal_text(value)
        .chars()
        .take(UNSUPPORTED_PREVIEW_LEN)
        .collect()
}

fn column_type_name(column_type: ColumnType) -> &'static str {
    match column_type {
        ColumnType::Null => "null",
        ColumnType::Bit | ColumnType::Bitn => "bit",
        ColumnType::Int1 => "tinyint",
        ColumnType::Int2 => "smallint",
        ColumnType::Int4 => "int",
        ColumnType::Int8 | ColumnType::Intn => "bigint",
        ColumnType::Datetime4 => "smalldatetime",
        ColumnType::Float4 => "real",
        ColumnType::Float8 | ColumnType::Floatn => "float",
        ColumnType::Money => "money",
        ColumnType::Datetime | ColumnType::Datetimen => "datetime",
        ColumnType::Money4 => "smallmoney",
        ColumnType::Guid => "uniqueidentifier",
        ColumnType::Decimaln => "decimal",
        ColumnType::Numericn => "numeric",
        ColumnType::Daten => "date",
        ColumnType::Timen => "time",
        ColumnType::Datetime2 => "datetime2",
        ColumnType::DatetimeOffsetn => "datetimeoffset",
        ColumnType::BigVarBin => "varbinary",
        ColumnType::BigVarChar => "varchar",
        ColumnType::BigBinary => "binary",
        ColumnType::BigChar => "char",
        ColumnType::NVarchar => "nvarchar",
        ColumnType::NChar => "nchar",
        ColumnType::Xml => "xml",
        ColumnType::Udt => "udt",
        ColumnType::Text => "text",
        ColumnType::Image => "image",
        ColumnType::NText => "ntext",
        ColumnType::SSVariant => "sql_variant",
    }
}

async fn query_rows(pool: &MsSqlConnectionPool, sql: &str) -> Result<Vec<Row>, DatabaseError> {
    let mut lease = pool.checkout().await?;
    let mut results = lease
        .simple_query(sql)
        .await
        .map_err(|error| tiberius_error(error, ErrorCategory::Sql))?
        .into_results()
        .await
        .map_err(|error| tiberius_error(error, ErrorCategory::Sql))?;
    lease.mark_reusable();

    if results.len() == 1 {
        Ok(results.pop().expect("result count was checked"))
    } else {
        Err(decode_error(
            "SQL Server internal query returned an unexpected result count",
        ))
    }
}

async fn query_result_set(
    pool: &MsSqlConnectionPool,
    sql: &str,
) -> Result<ResultSet, DatabaseError> {
    let mut lease = pool.checkout().await?;
    let mut stream = lease
        .simple_query(sql)
        .await
        .map_err(|error| tiberius_error(error, ErrorCategory::Sql))?;
    let mut result = ResultSet::default();
    while let Some(item) = stream
        .try_next()
        .await
        .map_err(|error| tiberius_error(error, ErrorCategory::Sql))?
    {
        match item {
            QueryItem::Metadata(metadata) => {
                if !result.columns.is_empty() {
                    return Err(decode_error(
                        "SQL Server returned multiple preview result sets",
                    ));
                }
                result.columns = metadata.columns().iter().map(decode_column).collect();
            }
            QueryItem::Row(row) => result.rows.push(decode_row(row)),
        }
    }
    drop(stream);
    lease.mark_reusable();
    Ok(result)
}

fn relation_pagination(
    page: crate::model::pagination::PageRequest,
    fetched_len: usize,
    total: Option<u64>,
) -> crate::model::pagination::ResultPagination {
    let mut pagination = crate::model::pagination::ResultPagination::from_page(page, fetched_len);
    if let Some(total) = total {
        pagination.total = crate::model::pagination::TotalRows::Exact(total);
    }
    pagination
}

async fn query_strings(
    pool: &MsSqlConnectionPool,
    sql: &str,
    column: &str,
) -> Result<Vec<String>, DatabaseError> {
    query_rows(pool, sql)
        .await?
        .iter()
        .map(|row| required_string(row, column))
        .collect()
}

fn required_string(row: &Row, column: &str) -> Result<String, DatabaseError> {
    row.try_get::<&str, _>(column)
        .map_err(|error| tiberius_error(error, ErrorCategory::Internal))?
        .map(str::to_owned)
        .ok_or_else(|| decode_error(format!("SQL Server returned NULL for [{column}]")))
}

fn optional_string(row: &Row, column: &str) -> Result<Option<String>, DatabaseError> {
    row.try_get::<&str, _>(column)
        .map_err(|error| tiberius_error(error, ErrorCategory::Internal))
        .map(|value| value.map(str::to_owned))
}

fn qualified_database(database: &str) -> QualifiedName {
    QualifiedName {
        database: Some(database.to_owned()),
        schema: None,
        object: database.to_owned(),
    }
}

fn qualified_schema(database: &str, schema: &str) -> QualifiedName {
    QualifiedName {
        database: Some(database.to_owned()),
        schema: Some(schema.to_owned()),
        object: schema.to_owned(),
    }
}

fn qualified_object(database: &str, schema: &str, object: &str) -> QualifiedName {
    QualifiedName {
        database: Some(database.to_owned()),
        schema: Some(schema.to_owned()),
        object: object.to_owned(),
    }
}

fn database_name(id: &CatalogId, target: &CatalogTarget) -> Result<String, DatabaseError> {
    if id.kind != CatalogKind::Database {
        return Err(catalog_target_not_found(target));
    }
    match id.native_path.as_slice() {
        [database] if !database.is_empty() => Ok(database.clone()),
        _ => Err(catalog_target_not_found(target)),
    }
}

fn schema_names(id: &CatalogId, target: &CatalogTarget) -> Result<(String, String), DatabaseError> {
    if id.kind != CatalogKind::Schema {
        return Err(catalog_target_not_found(target));
    }
    match id.native_path.as_slice() {
        [database, schema] if !database.is_empty() && !schema.is_empty() => {
            Ok((database.clone(), schema.clone()))
        }
        _ => Err(catalog_target_not_found(target)),
    }
}

fn paginate_catalog<T, S, B>(
    rows: &mut Vec<T>,
    request: &CatalogRequest,
    sort: S,
    tie: B,
) -> Result<Option<crate::db::catalog::CatalogCursor>, DatabaseError>
where
    S: Fn(&T) -> String,
    B: Fn(&T) -> String,
{
    rows.sort_by(|a, b| sort(a).cmp(&sort(b)).then_with(|| tie(a).cmp(&tie(b))));
    if let Some(cursor) = &request.key.cursor {
        let (sort_key, tie_breaker) = cursor
            .keyset_parts()
            .map_err(DatabaseError::invalid_catalog_request)?;
        rows.retain(|row| (sort(row).as_str(), tie(row).as_str()) > (sort_key, tie_breaker));
    }
    rows.truncate(request.page_size.saturating_add(1));
    finalize_keyset_page(rows, request.page_size, sort, tie).map_err(catalog_invariant)
}

fn exact_catalog_count(count: usize) -> Result<CatalogCount, DatabaseError> {
    u64::try_from(count)
        .map(CatalogCount::Exact)
        .map_err(|_| catalog_internal("SQL Server catalog count exceeds u64"))
}

fn group_key(group: ObjectGroup) -> &'static str {
    match group {
        ObjectGroup::Tables => "00:tables",
        ObjectGroup::Views => "01:views",
        ObjectGroup::Functions => "02:functions",
        ObjectGroup::Procedures => "03:procedures",
        ObjectGroup::Sequences => "04:sequences",
        ObjectGroup::Triggers => "05:triggers",
        _ => "99:unsupported",
    }
}

fn decode_error(message: impl AsRef<str>) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Internal,
        code: Some("sql_server_decode".to_owned()),
        message: crate::security::sanitize_terminal_text(message.as_ref()),
    }
}

fn discovery_warning(database: &str, error: &DatabaseError) -> String {
    crate::security::sanitize_terminal_text(&format!(
        "Unable to discover schemas for database {database}: {error}"
    ))
}

fn validate_catalog_scope(scope: &CatalogScope) -> Result<(), DatabaseError> {
    scope.validate("", None).map_err(|error| DatabaseError {
        category: ErrorCategory::Configuration,
        code: Some("invalid_catalog_request".to_owned()),
        message: crate::security::sanitize_terminal_text(&error.to_string()),
    })
}

fn catalog_target_not_found(target: &CatalogTarget) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Configuration,
        code: Some("catalog_target_not_found".to_owned()),
        message: format!(
            "SQL Server catalog target was not found: {}",
            target.description()
        ),
    }
}

fn catalog_invariant(error: crate::db::catalog::CatalogValidationError) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Internal,
        code: Some("catalog_invariant".to_owned()),
        message: crate::security::sanitize_terminal_text(&error.to_string()),
    }
}

fn catalog_internal(message: impl AsRef<str>) -> DatabaseError {
    decode_error(message)
}

fn quote_literal(value: &str) -> String {
    format!("N'{}'", value.replace('\'', "''"))
}

#[derive(Debug)]
struct MsSqlSearchCandidate {
    kind: CatalogKind,
    database: String,
    schema: String,
    name: String,
    native_kind: String,
    relation_kind: CatalogKind,
    relation_name: String,
    relation_id: i32,
    object_id: i32,
    rank: u8,
    path: String,
}

fn search_like_pattern(query: &str) -> String {
    format!(
        "%{}%",
        query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

fn format_search_candidates(database: &str, pattern: &str, query: &str) -> String {
    let q = quote_identifier(database);
    let literal = quote_literal(query);
    let match_expr = format!(
        "LOWER([name]) COLLATE Latin1_General_100_CI_AI LIKE {} ESCAPE N'\\'",
        quote_literal(pattern)
    );
    let rank_expr = format!(
        "CASE WHEN LOWER([name]) = LOWER({literal}) THEN 0 WHEN LOWER([name]) LIKE LOWER({literal}) + N'%' THEN 1 ELSE 2 END"
    );
    format!(
        "SELECT [kind], [schema], [name], [native_kind], [relation_kind], [relation_name], [relation_id], [object_id], {rank_expr} AS [rank] FROM (\
         SELECT 'schema' AS [kind], s.[name] AS [schema], s.[name], 'schema' AS [native_kind], 'table' AS [relation_kind], s.[name] AS [relation_name], 0 AS [relation_id], s.[schema_id] AS [object_id] FROM {q}.sys.schemas s WHERE s.[name] NOT IN ('guest','INFORMATION_SCHEMA','sys') AND s.[is_ms_shipped]=0\
         UNION ALL SELECT CASE o.[type] WHEN 'U' THEN 'table' WHEN 'V' THEN 'view' WHEN 'P' THEN 'procedure' WHEN 'SO' THEN 'sequence' ELSE 'function' END, s.[name], o.[name], LOWER(o.[type_desc]), CASE WHEN o.[type]='V' THEN 'view' ELSE 'table' END, o.[name], o.[object_id], o.[object_id] FROM {q}.sys.objects o JOIN {q}.sys.schemas s ON s.[schema_id]=o.[schema_id] WHERE o.[type] IN ('U','V','P','SO','FN','IF','TF','FS','FT') AND o.[is_ms_shipped]=0\
         UNION ALL SELECT 'column', s.[name], c.[name], 'column', CASE WHEN o.[type]='V' THEN 'view' ELSE 'table' END, o.[name], c.[object_id], c.[column_id] FROM {q}.sys.columns c JOIN {q}.sys.objects o ON o.[object_id]=c.[object_id] JOIN {q}.sys.schemas s ON s.[schema_id]=o.[schema_id] WHERE o.[is_ms_shipped]=0\
         UNION ALL SELECT 'index', s.[name], i.[name], 'index', CASE WHEN o.[type]='V' THEN 'view' ELSE 'table' END, o.[name], i.[object_id], i.[index_id] FROM {q}.sys.indexes i JOIN {q}.sys.objects o ON o.[object_id]=i.[object_id] JOIN {q}.sys.schemas s ON s.[schema_id]=o.[schema_id] WHERE i.[index_id]>0 AND i.[is_hypothetical]=0\
         UNION ALL SELECT CASE kc.[type] WHEN 'PK' THEN 'primary_key' ELSE 'unique_constraint' END, s.[name], kc.[name], 'constraint', 'table', o.[name], kc.[parent_object_id], kc.[object_id] FROM {q}.sys.key_constraints kc JOIN {q}.sys.objects o ON o.[object_id]=kc.[parent_object_id] JOIN {q}.sys.schemas s ON s.[schema_id]=o.[schema_id]\
         UNION ALL SELECT 'foreign_key', s.[name], fk.[name], 'constraint', 'table', o.[name], fk.[parent_object_id], fk.[object_id] FROM {q}.sys.foreign_keys fk JOIN {q}.sys.objects o ON o.[object_id]=fk.[parent_object_id] JOIN {q}.sys.schemas s ON s.[schema_id]=o.[schema_id]\
         UNION ALL SELECT 'check_constraint', s.[name], cc.[name], 'constraint', 'table', o.[name], cc.[parent_object_id], cc.[object_id] FROM {q}.sys.check_constraints cc JOIN {q}.sys.objects o ON o.[object_id]=cc.[parent_object_id] JOIN {q}.sys.schemas s ON s.[schema_id]=o.[schema_id]\
         UNION ALL SELECT 'foreign_key', s.[name], fk.[name], 'constraint', 'table', o.[name], fk.[parent_object_id], fk.[object_id] FROM {q}.sys.foreign_keys fk JOIN {q}.sys.objects o ON o.[object_id]=fk.[parent_object_id] JOIN {q}.sys.schemas s ON s.[schema_id]=o.[schema_id]\
         UNION ALL SELECT 'check_constraint', s.[name], cc.[name], 'constraint', 'table', o.[name], cc.[parent_object_id], cc.[object_id] FROM {q}.sys.check_constraints cc JOIN {q}.sys.objects o ON o.[object_id]=cc.[parent_object_id] JOIN {q}.sys.schemas s ON s.[schema_id]=o.[schema_id]\
         UNION ALL SELECT 'trigger', s.[name], tr.[name], 'trigger', CASE WHEN o.[type]='V' THEN 'view' ELSE 'table' END, o.[name], tr.[parent_id], tr.[object_id] FROM {q}.sys.triggers tr JOIN {q}.sys.objects o ON o.[object_id]=tr.[parent_id] JOIN {q}.sys.schemas s ON s.[schema_id]=o.[schema_id] WHERE tr.[is_ms_shipped]=0\
         ) candidates WHERE {match_expr} ORDER BY [rank], LOWER([schema]), LOWER([name]), [kind], [object_id]"
    )
}

impl MsSqlSearchCandidate {
    fn database(database: &str, query: &str) -> Self {
        let rank = if database.eq_ignore_ascii_case(query) {
            0
        } else if database
            .to_ascii_lowercase()
            .starts_with(&query.to_ascii_lowercase())
        {
            1
        } else {
            2
        };
        Self {
            kind: CatalogKind::Database,
            database: database.to_owned(),
            schema: String::new(),
            name: database.to_owned(),
            native_kind: "database".to_owned(),
            relation_kind: CatalogKind::Table,
            relation_name: String::new(),
            relation_id: 0,
            object_id: 0,
            rank,
            path: database.to_owned(),
        }
    }

    fn decode(row: &Row, database: &str, query: &str) -> Result<Self, DatabaseError> {
        let native = required_string(row, "native_kind")?;
        let kind = match required_string(row, "kind")?.as_str() {
            "schema" => CatalogKind::Schema,
            "table" => CatalogKind::Table,
            "view" => CatalogKind::View,
            "function" => CatalogKind::Function,
            "procedure" => CatalogKind::Procedure,
            "sequence" => CatalogKind::Sequence,
            "column" => CatalogKind::Column,
            "index" => CatalogKind::Index,
            "primary_key" => CatalogKind::PrimaryKey,
            "unique_constraint" => CatalogKind::UniqueConstraint,
            "foreign_key" => CatalogKind::ForeignKey,
            "check_constraint" => CatalogKind::CheckConstraint,
            "trigger" => CatalogKind::Trigger,
            other => {
                return Err(catalog_internal(format!(
                    "unsupported SQL Server search kind {other}"
                )));
            }
        };
        let relation_kind = if required_string(row, "relation_kind")? == "view" {
            CatalogKind::View
        } else {
            CatalogKind::Table
        };
        let relation_id = row
            .try_get("relation_id")
            .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
            .unwrap_or_default();
        let object_id = row
            .try_get("object_id")
            .map_err(|e| tiberius_error(e, ErrorCategory::Internal))?
            .unwrap_or_default();
        let schema = required_string(row, "schema")?;
        let name = required_string(row, "name")?;
        let rank = if name.eq_ignore_ascii_case(query) {
            0
        } else if name
            .to_ascii_lowercase()
            .starts_with(&query.to_ascii_lowercase())
        {
            1
        } else {
            2
        };
        Ok(Self {
            kind,
            database: database.to_owned(),
            schema: schema.clone(),
            name: name.clone(),
            native_kind: native,
            relation_kind,
            relation_name: required_string(row, "relation_name")?,
            relation_id,
            object_id,
            rank,
            path: format!("{database}.{schema}.{name}"),
        })
    }
}

fn relation_child_id(relation: &CatalogId, kind: CatalogKind, identity: &str) -> CatalogId {
    let mut path = relation.native_path.clone();
    path.push(identity.to_owned());
    CatalogId::new(relation.profile_id(), kind, path)
}

fn mssql_top_level_name(
    entry: &CatalogEntry,
    kind: CatalogKind,
) -> Result<String, CatalogDropError> {
    let name = &entry.qualified_name;
    let path = &entry.id.native_path;
    if path.len() != 4
        || entry.id.kind != kind
        || entry.parent_id.as_ref()
            != Some(&CatalogId::new(
                entry.id.profile_id(),
                CatalogKind::Schema,
                [path[0].clone(), path[1].clone()],
            ))
        || path[0] != name.database.as_deref().unwrap_or_default()
        || path[1] != name.schema.as_deref().unwrap_or_default()
        || path[2] != name.object
        || name.database.as_deref().is_none_or(str::is_empty)
        || name.schema.as_deref().is_none_or(str::is_empty)
    {
        return Err(CatalogDropError::Unsupported {
            kind,
            reason: "catalog entry has an invalid SQL Server object identity".to_owned(),
        });
    }
    Ok(format!(
        "{}.{}.{}",
        quote_identifier(name.database.as_deref().unwrap()),
        quote_identifier(name.schema.as_deref().unwrap()),
        quote_identifier(&name.object)
    ))
}

fn mssql_child_name(
    entry: &CatalogEntry,
    kind: CatalogKind,
) -> Result<(String, CatalogId), CatalogDropError> {
    let relation = entry
        .relation_id
        .as_ref()
        .ok_or_else(|| CatalogDropError::Unsupported {
            kind,
            reason: "catalog entry has no owning relation identity".to_owned(),
        })?;
    let name = &entry.qualified_name;
    let path = &entry.id.native_path;
    if entry.id.kind != kind
        || entry.parent_id.as_ref() != Some(relation)
        || relation.profile_id() != entry.id.profile_id()
        || !relation.kind.is_relation()
        || relation.native_path.len() != 4
        || path.len() != 5
        || path[..4] != relation.native_path[..]
        || path[4].is_empty()
        || name.database.as_deref() != relation.native_path.first().map(String::as_str)
        || name.schema.as_deref() != relation.native_path.get(1).map(String::as_str)
        || name.object.is_empty()
    {
        return Err(CatalogDropError::Unsupported {
            kind,
            reason: "catalog entry has an invalid owning relation identity".to_owned(),
        });
    }
    let qualified_relation = format!(
        "{}.{}.{}",
        quote_identifier(name.database.as_deref().unwrap()),
        quote_identifier(name.schema.as_deref().unwrap()),
        quote_identifier(&relation.native_path[2])
    );
    Ok((qualified_relation, relation.clone()))
}

fn mssql_trigger_name(entry: &CatalogEntry) -> Result<String, CatalogDropError> {
    let name = &entry.qualified_name;
    let relation = entry
        .relation_id
        .as_ref()
        .ok_or_else(|| CatalogDropError::Unsupported {
            kind: CatalogKind::Trigger,
            reason: "catalog entry has no owning relation identity".to_owned(),
        })?;
    let path = &entry.id.native_path;
    let valid_path =
        (path.len() == 5 && path[..4] == relation.native_path[..] && !path[4].is_empty())
            || (path.len() == 4
                && path[0] == relation.native_path[0]
                && path[1] == relation.native_path[1]
                && path[2] == name.object
                && !path[3].is_empty());
    if entry.id.kind != CatalogKind::Trigger
        || entry.parent_id.as_ref() != Some(relation)
        || relation.profile_id() != entry.id.profile_id()
        || !relation.kind.is_relation()
        || relation.native_path.len() != 4
        || !valid_path
        || name.database.as_deref() != relation.native_path.first().map(String::as_str)
        || name.schema.as_deref() != relation.native_path.get(1).map(String::as_str)
        || name.object.is_empty()
    {
        return Err(CatalogDropError::Unsupported {
            kind: CatalogKind::Trigger,
            reason: "catalog entry has an invalid owning relation identity".to_owned(),
        });
    }
    Ok(format!(
        "{}.{}.{}",
        quote_identifier(name.database.as_deref().unwrap()),
        quote_identifier(name.schema.as_deref().unwrap()),
        quote_identifier(&name.object)
    ))
}

fn add_memberships(
    memberships: &mut HashMap<String, Vec<ConstraintMembership>>,
    columns: &[String],
    constraint_id: &CatalogId,
) -> Result<(), DatabaseError> {
    for (index, column) in columns.iter().enumerate() {
        memberships
            .entry(column.clone())
            .or_default()
            .push(ConstraintMembership {
                constraint_id: constraint_id.clone(),
                ordinal_position: u32::try_from(index + 1)
                    .map_err(|_| catalog_internal("SQL Server constraint has too many columns"))?,
            });
    }
    Ok(())
}

fn child_sort_key(entry: &CatalogEntry) -> String {
    let value = match &entry.metadata {
        CatalogMetadata::Column(column) => format!("{:010}", column.ordinal_position),
        _ => entry.qualified_name.object.clone(),
    };
    format!("{:02}\0{}", catalog_kind_rank(entry.kind), value)
}

fn child_tie_breaker(entry: &CatalogEntry) -> String {
    format!(
        "{:02}\0{}",
        catalog_kind_rank(entry.kind),
        entry.id.native_path.last().map_or("", String::as_str)
    )
}

const fn catalog_kind_rank(kind: CatalogKind) -> u8 {
    match kind {
        CatalogKind::Column => 0,
        CatalogKind::Index => 1,
        CatalogKind::PrimaryKey => 2,
        CatalogKind::UniqueConstraint => 3,
        CatalogKind::ForeignKey => 4,
        CatalogKind::CheckConstraint => 5,
        CatalogKind::Trigger => 6,
        _ => 7,
    }
}

pub fn quote_identifier(value: &str) -> String {
    format!("[{}]", value.replace(']', "]]"))
}

pub fn supports_server_version(version: &str) -> bool {
    version
        .split('.')
        .next()
        .and_then(|major| major.parse::<u32>().ok())
        .is_some_and(|major| major >= 11)
}

#[cfg(test)]
mod mutation_tests {
    use super::{mssql_null_safe_predicate, validate_insert_mutation};
    use crate::db::mutation::{InputValue, InsertRowMutation};
    use crate::db::value::CellValue;

    #[test]
    fn null_safe_predicates_use_numbered_tds_parameters() {
        assert_eq!(
            mssql_null_safe_predicate("[name]", 3),
            "(([name] = @P3) OR ([name] IS NULL AND @P4 IS NULL))"
        );
    }

    #[test]
    fn insert_rejects_duplicate_columns_and_unsupported_values() {
        let duplicate = InsertRowMutation {
            columns: vec![1, 1],
            values: vec![InputValue::Null, InputValue::Null],
        };
        assert!(validate_insert_mutation(&duplicate, 2).is_err());

        let unsupported = InsertRowMutation {
            columns: vec![0],
            values: vec![InputValue::Value(CellValue::Unsupported {
                type_name: "sql_variant".into(),
                preview: "value".into(),
            })],
        };
        assert!(validate_insert_mutation(&unsupported, 1).is_err());
    }
}

#[cfg(test)]
mod ddl_tests {
    use super::{IndexMetadata, quote_identifier, quote_literal, reconstructed_index_sql};

    #[test]
    fn quotes_sql_server_identifiers_and_literals() {
        assert_eq!(quote_identifier("a]b"), "[a]]b]");
        assert_eq!(quote_identifier(""), "[]");
        assert_eq!(quote_literal("O'Brien"), "N'O''Brien'");
    }

    #[test]
    fn reconstructs_safe_index_ddl_from_catalog_metadata() {
        let sql = reconstructed_index_sql(
            "[dbo].[orders]",
            "ix]orders",
            &IndexMetadata {
                columns: vec!["customer]id".to_owned(), "created_at".to_owned()],
                unique: true,
            },
        );
        assert_eq!(
            sql,
            "CREATE UNIQUE INDEX [ix]]orders] ON [dbo].[orders] ([customer]]id], [created_at])"
        );
    }
}

fn server_error_category(code: u32) -> ErrorCategory {
    match code {
        18456 => ErrorCategory::Authentication,
        229 | 230 => ErrorCategory::Permission,
        547 | 2601 | 2627 => ErrorCategory::Constraint,
        _ => ErrorCategory::Sql,
    }
}

fn tiberius_error(error: tiberius::error::Error, default: ErrorCategory) -> DatabaseError {
    let code = error.code();
    let category = match &error {
        tiberius::error::Error::Server(server) => server_error_category(server.code()),
        tiberius::error::Error::Io { .. }
        | tiberius::error::Error::Tls(_)
        | tiberius::error::Error::Routing { .. } => ErrorCategory::Network,
        _ => default,
    };
    DatabaseError {
        category,
        code: code.map(|code| code.to_string()),
        message: crate::security::sanitize_terminal_text(&error.to_string()),
    }
}

fn network_error(error: impl fmt::Display) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Network,
        code: None,
        message: crate::security::sanitize_terminal_text(&error.to_string()),
    }
}

fn pool_closed_error() -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Configuration,
        code: Some("sql_server_pool_closed".to_owned()),
        message: "SQL Server connection pool is closed".to_owned(),
    }
}

fn unsupported_operation<T>(code: &str, message: &str) -> Result<T, DatabaseError> {
    Err(DatabaseError {
        category: ErrorCategory::Unsupported,
        code: Some(code.to_owned()),
        message: message.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::profile::{CatalogScope, ConnectionUrlFormat, CredentialPolicy, Environment};

    fn profile(ssl_mode: SslMode) -> ConnectionProfile {
        ConnectionProfile {
            id: Uuid::nil(),
            name: "SQL Server".to_owned(),
            access: Default::default(),
            group_id: None,
            kind: DatabaseKind::SqlServer,
            url_format: ConnectionUrlFormat::SqlServer,
            host: Some("db.example.test".to_owned()),
            port: Some(1433),
            user: Some("sa".to_owned()),
            database: Some("app".to_owned()),
            default_schema: Some("dbo".to_owned()),
            sqlite_path: None,
            ssl_mode,
            credential_policy: CredentialPolicy::Prompt,
            read_only: false,
            environment: Environment::Development,
            catalog_scope: CatalogScope::for_profile(DatabaseKind::SqlServer, "app", Some("dbo")),
        }
    }

    #[tokio::test]
    async fn dashboard_and_process_metrics_have_stable_unsupported_errors() {
        let adapter = MsSqlAdapter::from_profile(
            &profile(SslMode::Require),
            Some(&SecretString::from("secret")),
        )
        .unwrap();

        let dashboard = adapter.load_monitor_snapshot().await.unwrap_err();
        assert_eq!(dashboard.category, ErrorCategory::Unsupported);
        assert_eq!(dashboard.code.as_deref(), Some(DASHBOARD_UNSUPPORTED_CODE));
        assert_eq!(dashboard.message, DASHBOARD_UNSUPPORTED_MESSAGE);

        let metadata = adapter.load_monitor_metadata().await.unwrap_err();
        assert_eq!(metadata.code.as_deref(), Some(DASHBOARD_UNSUPPORTED_CODE));

        let processes = adapter.load_process_snapshot().await.unwrap_err();
        assert_eq!(processes.category, ErrorCategory::Unsupported);
        assert_eq!(
            processes.code.as_deref(),
            Some(PROCESS_METRICS_UNSUPPORTED_CODE)
        );
        assert_eq!(processes.message, PROCESS_METRICS_UNSUPPORTED_MESSAGE);
    }

    #[test]
    fn settings_require_server_fields_and_redact_password() {
        let password = SecretString::from("not-for-debug".to_owned());
        let settings =
            MsSqlConnectSettings::from_profile(&profile(SslMode::VerifyFull), Some(&password))
                .unwrap();

        assert_eq!(settings.host, "db.example.test");
        assert_eq!(settings.port, 1433);
        assert_eq!(settings.user, "sa");
        assert_eq!(settings.database, "app");
        assert_eq!(settings.tls_mode, MsSqlTlsMode::RequiredVerified);
        assert!(!format!("{settings:?}").contains("not-for-debug"));

        let mut missing = profile(SslMode::Disable);
        missing.database = None;
        let error = MsSqlConnectSettings::from_profile(&missing, Some(&password)).unwrap_err();
        assert_eq!(error.category, ErrorCategory::Configuration);
        assert_eq!(error.message, "SQL Server profile has no database");
    }

    #[test]
    fn tls_modes_match_tiberius_rustls_capabilities() {
        let password = SecretString::from("secret".to_owned());
        for (mode, expected) in [
            (SslMode::Disable, MsSqlTlsMode::Plaintext),
            (SslMode::Require, MsSqlTlsMode::RequiredUnverified),
            (SslMode::Prefer, MsSqlTlsMode::RequiredVerified),
            (SslMode::VerifyCa, MsSqlTlsMode::RequiredVerified),
            (SslMode::VerifyFull, MsSqlTlsMode::RequiredVerified),
        ] {
            let settings =
                MsSqlConnectSettings::from_profile(&profile(mode), Some(&password)).unwrap();
            assert_eq!(settings.tls_mode, expected);
            assert_eq!(
                settings.tiberius_config().get_addr(),
                "db.example.test:1433"
            );
        }
    }

    #[test]
    fn server_error_numbers_have_stable_categories() {
        for code in [2601, 2627, 547] {
            assert_eq!(server_error_category(code), ErrorCategory::Constraint);
        }
        assert_eq!(server_error_category(18456), ErrorCategory::Authentication);
        for code in [229, 230] {
            assert_eq!(server_error_category(code), ErrorCategory::Permission);
        }
        assert_eq!(server_error_category(1205), ErrorCategory::Sql);
        assert_eq!(server_error_category(102), ErrorCategory::Sql);
    }

    #[test]
    fn tiberius_errors_are_sanitized_and_transport_errors_are_network_errors() {
        let error = tiberius_error(
            tiberius::error::Error::Io {
                kind: std::io::ErrorKind::TimedOut,
                message: "timed out\r\u{1b}[31m".to_owned(),
            },
            ErrorCategory::Sql,
        );
        assert_eq!(error.category, ErrorCategory::Network);
        assert_eq!(error.code, None);
        assert_eq!(
            error.message,
            "An error occured during the attempt of performing I/O: timed out<CR><ESC>[31m"
        );
    }

    #[derive(Debug)]
    struct DropCounter(Arc<AtomicUsize>);

    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn test_lease<T>(client: T, semaphore: Arc<tokio::sync::Semaphore>) -> LeaseSlot<T> {
        let permit = semaphore.try_acquire_owned().unwrap();
        LeaseSlot {
            client: Some(client),
            idle: Arc::new(Mutex::new(Vec::new())),
            permit: Some(permit),
            closed: Arc::new(AtomicBool::new(false)),
            reusable: false,
        }
    }

    #[test]
    fn lease_is_dirty_until_explicitly_marked_reusable() {
        let drops = Arc::new(AtomicUsize::new(0));
        let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
        let lease = test_lease(DropCounter(Arc::clone(&drops)), Arc::clone(&semaphore));
        let idle = Arc::clone(&lease.idle);

        assert!(semaphore.clone().try_acquire_owned().is_err());
        drop(lease);

        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(idle.lock().unwrap().is_empty());
        assert!(semaphore.try_acquire_owned().is_ok());
    }

    #[test]
    fn reusable_lease_returns_client_unless_pool_is_closed() {
        let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
        let mut lease = test_lease(7, Arc::clone(&semaphore));
        let idle = Arc::clone(&lease.idle);
        lease.mark_reusable();
        drop(lease);
        assert_eq!(idle.lock().unwrap().pop(), Some(7));

        let mut lease = test_lease(8, semaphore);
        let idle = Arc::clone(&lease.idle);
        lease.closed.store(true, Ordering::Release);
        lease.mark_reusable();
        drop(lease);
        assert!(idle.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn closing_pool_is_idempotent_and_rejects_checkout() {
        let password = SecretString::from("secret".to_owned());
        let settings = Arc::new(
            MsSqlConnectSettings::from_profile(&profile(SslMode::VerifyFull), Some(&password))
                .unwrap(),
        );
        let pool = MsSqlConnectionPool::new(settings);
        assert_eq!(pool.permits.available_permits(), DEFAULT_MAX_CONNECTIONS);
        assert!(pool.idle.lock().unwrap().is_empty());

        pool.close().await;
        pool.close().await;
        let error = match pool.checkout().await {
            Ok(_) => panic!("closed pool unexpectedly allowed checkout"),
            Err(error) => error,
        };
        assert_eq!(error.code.as_deref(), Some("sql_server_pool_closed"));
    }

    #[tokio::test]
    async fn closing_adapter_prevents_clones_from_creating_pools() {
        let password = SecretString::from("secret".to_owned());
        let adapter =
            MsSqlAdapter::from_profile(&profile(SslMode::VerifyFull), Some(&password)).unwrap();
        let clone = adapter.clone();

        adapter.close().await;
        let error = match clone.pool_for_database("app").await {
            Ok(_) => panic!("closed adapter unexpectedly created a pool"),
            Err(error) => error,
        };
        assert_eq!(error.code.as_deref(), Some("sql_server_pool_closed"));
    }

    #[test]
    fn decodes_common_owned_tiberius_values_without_decimal_rounding() {
        use std::borrow::Cow;

        use tiberius::{
            IntoSql,
            numeric::Numeric,
            time::{Date, DateTime2, DateTimeOffset, Time},
            xml::XmlData,
        };

        assert_eq!(
            decode_cell(ColumnType::Bit, ColumnData::Bit(Some(true))),
            CellValue::Boolean(true)
        );
        assert_eq!(
            decode_cell(ColumnType::Int1, ColumnData::U8(Some(255))),
            CellValue::Unsigned(255)
        );
        assert_eq!(
            decode_cell(ColumnType::Int8, ColumnData::I64(Some(i64::MIN))),
            CellValue::Integer(i64::MIN)
        );
        assert_eq!(
            decode_cell(
                ColumnType::Decimaln,
                ColumnData::Numeric(Some(Numeric::new_with_scale(
                    12345678901234567890123456789012345678,
                    18,
                ))),
            ),
            CellValue::Text("12345678901234567890.123456789012345678".to_owned())
        );
        assert_eq!(
            decode_cell(
                ColumnType::Numericn,
                ColumnData::Numeric(Some(Numeric::new_with_scale(-12, 3))),
            ),
            CellValue::Text("-0.012".to_owned())
        );
        assert_eq!(
            decode_cell(
                ColumnType::NVarchar,
                ColumnData::String(Some(Cow::Borrowed("你好"))),
            ),
            CellValue::Text("你好".to_owned())
        );
        assert_eq!(
            decode_cell(
                ColumnType::Xml,
                ColumnData::Xml(Some(Cow::Owned(XmlData::new("<root />")))),
            ),
            CellValue::Text("<root />".to_owned())
        );
        assert_eq!(
            decode_cell(
                ColumnType::Daten,
                NaiveDate::from_ymd_opt(2026, 9, 2).unwrap().into_sql()
            ),
            CellValue::Date(NaiveDate::from_ymd_opt(2026, 9, 2).unwrap())
        );
        assert_eq!(
            decode_cell(ColumnType::Float4, ColumnData::F32(Some(1.25))),
            CellValue::Float(1.25)
        );
        assert_eq!(
            decode_cell(
                ColumnType::BigBinary,
                ColumnData::Binary(Some(vec![0, 1, 255].into())),
            ),
            CellValue::Bytes(vec![0, 1, 255])
        );
        let uuid = Uuid::parse_str("c97dbc01-fb45-4384-a194-e39a4560cf4a").unwrap();
        assert_eq!(
            decode_cell(ColumnType::Guid, ColumnData::Guid(Some(uuid))),
            CellValue::Text("c97dbc01-fb45-4384-a194-e39a4560cf4a".to_owned())
        );

        let date = Date::new(739_495);
        let time = Time::new(45_296_123_456, 6);
        let datetime2 = DateTime2::new(date, time);
        assert!(matches!(
            decode_cell(ColumnType::Timen, ColumnData::Time(Some(time))),
            CellValue::Time(_)
        ));
        assert!(matches!(
            decode_cell(
                ColumnType::Datetime2,
                ColumnData::DateTime2(Some(datetime2))
            ),
            CellValue::DateTime(_)
        ));
        let timestamp = decode_cell(
            ColumnType::DatetimeOffsetn,
            ColumnData::DateTimeOffset(Some(DateTimeOffset::new(datetime2, 480))),
        );
        let CellValue::Timestamp(timestamp) = timestamp else {
            panic!("datetimeoffset should decode to a timestamp")
        };
        assert_eq!(timestamp.offset().local_minus_utc(), 8 * 60 * 60);
        assert_eq!(
            timestamp.naive_local(),
            NaiveDateTime::new(
                NaiveDate::from_ymd_opt(2025, 9, 2).unwrap(),
                NaiveTime::from_hms_micro_opt(12, 34, 56, 123_456).unwrap(),
            )
        );
        assert!(matches!(
            decode_cell(
                ColumnType::Datetime,
                ColumnData::DateTime(Some(tiberius::time::DateTime::new(0, 0)))
            ),
            CellValue::DateTime(_)
        ));
        assert!(matches!(
            decode_cell(
                ColumnType::Datetime4,
                ColumnData::SmallDateTime(Some(tiberius::time::SmallDateTime::new(0, 0)))
            ),
            CellValue::DateTime(_)
        ));
    }

    #[test]
    fn nulls_and_unsupported_values_are_safe() {
        assert_eq!(
            decode_cell(ColumnType::Int4, ColumnData::I32(None)),
            CellValue::Null
        );
        let value = decode_cell(
            ColumnType::Udt,
            ColumnData::Binary(Some(vec![0xAB; 100].into())),
        );
        let CellValue::Unsupported { type_name, preview } = value else {
            panic!("UDT should be unsupported")
        };
        assert_eq!(type_name, "udt");
        assert_eq!(preview.len(), UNSUPPORTED_PREVIEW_LEN);

        let value = decode_cell(ColumnType::SSVariant, ColumnData::I32(Some(42)));
        assert!(matches!(
            value,
            CellValue::Unsupported { type_name, preview }
                if type_name == "sql_variant" && preview == "I32(Some(42))"
        ));

        assert_eq!(
            decode_cell(ColumnType::Money, ColumnData::F64(Some(0.1))),
            CellValue::Unsupported {
                type_name: "money".to_owned(),
                preview: "exact value unavailable from Tiberius".to_owned(),
            }
        );
    }
}
