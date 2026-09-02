use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use futures_util::TryStreamExt;
use secrecy::{ExposeSecret, SecretString};
use sqlx::{
    AssertSqlSafe, Column, Connection, Either, Executor, PgPool, Row, SqlSafeStr, Statement,
    TypeInfo, ValueRef,
    pool::PoolConnection,
    postgres::{PgConnectOptions, PgConnection, PgPoolOptions, PgRow, PgSslMode, Postgres},
};
use sqlx_core::transaction::TransactionManager;
use uuid::Uuid;

use crate::model::dashboard::MetricKey;
use crate::{
    identity::ConnectionIdentity,
    profile::{CatalogScope, CatalogSelection, ConnectionProfile, DatabaseKind, SslMode},
    security::sanitize_terminal_text,
};

use super::transaction::{TransactionBackend, TransactionError};
use super::{
    DatabaseError, ErrorCategory, ServerInfo,
    catalog::{
        CatalogCapabilities, CatalogCount, CatalogCursor, CatalogDiscovery, CatalogEntry,
        CatalogGroupSummary, CatalogId, CatalogKind, CatalogMetadata, CatalogPage, CatalogRequest,
        CatalogRequestKey, CatalogSearchHit, CatalogSearchPage, CatalogSearchRequest,
        CatalogTarget, CatalogValidationError, ColumnMetadata, ColumnMetadataCapabilities,
        ConstraintMembership, ConstraintMetadata, DdlProvenance, DiscoveredDatabase, IndexMetadata,
        NamespaceModel, ObjectGroup, OptionalMetadata, QualifiedName, RelationDdl,
        finalize_keyset_page,
    },
    catalog_drop::{CatalogDropError, CatalogDropPlan, CatalogDropRequest},
    ddl::{DdlSection, assemble_ddl},
    mutation::{InputValue, MutationResult, RelationMutation, RelationMutationRequest},
    query::{ColumnMeta, QueryOutcome, QueryOutcomeAccumulator, RELATION_PREVIEW_LIMIT, ResultSet},
    value::CellValue,
};

pub const CATALOG_TABLES_SQL: &str = r#"
SELECT table_schema, table_name, table_type
FROM information_schema.tables
WHERE table_catalog = current_database()
  AND table_schema <> 'information_schema'
  AND table_schema NOT LIKE 'pg\_%' ESCAPE '\'
ORDER BY table_schema, table_name
"#;

pub const CATALOG_SCHEMAS_SQL: &str = r#"
SELECT schema_name
FROM information_schema.schemata
WHERE schema_name <> 'information_schema'
  AND schema_name NOT LIKE 'pg\_%' ESCAPE '\'
ORDER BY schema_name COLLATE "C"
"#;

pub const CATALOG_COLUMNS_SQL: &str = r#"
SELECT table_schema, table_name, column_name, data_type, is_nullable
FROM information_schema.columns
WHERE table_catalog = current_database()
  AND table_schema <> 'information_schema'
  AND table_schema NOT LIKE 'pg\_%' ESCAPE '\'
ORDER BY table_schema, table_name, ordinal_position
"#;

pub const CATALOG_ROUTINES_SQL: &str = r#"
SELECT n.nspname AS routine_schema, p.proname AS routine_name,
       p.prokind::text AS prokind,
       pg_get_function_identity_arguments(p.oid) AS arguments,
       pg_get_function_result(p.oid) AS result
FROM pg_proc p
JOIN pg_namespace n ON n.oid = p.pronamespace
WHERE n.nspname <> 'information_schema'
  AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
ORDER BY n.nspname, p.proname, p.oid
"#;

pub const CATALOG_INDEXES_SQL: &str = r#"
SELECT schemaname AS table_schema, tablename AS table_name, indexname, indexdef
FROM pg_indexes
WHERE schemaname <> 'information_schema'
  AND schemaname NOT LIKE 'pg\_%' ESCAPE '\'
ORDER BY schemaname, tablename, indexname
"#;

pub const SEARCH_CATALOG_SQL: &str = r#"
WITH candidates AS (
    SELECT 'database'::text AS kind, current_database() AS database_name,
           NULL::text AS schema_name, current_database() AS object_name,
           NULL::bigint AS object_oid, NULL::text AS relation_kind,
           NULL::text AS relation_name, NULL::bigint AS relation_oid,
           shobj_description(d.oid, 'pg_database') AS comment, NULL::text AS relation_comment,
           shobj_description(d.oid, 'pg_database') AS database_comment,
           NULL::text AS schema_comment,
           current_database() AS qualified_path
    FROM pg_database d
    WHERE d.datname = current_database()
    UNION ALL
    SELECT 'schema', current_database(), n.nspname, n.nspname, n.oid::bigint,
           NULL, NULL, NULL, obj_description(n.oid, 'pg_namespace'), NULL,
           (SELECT shobj_description(oid, 'pg_database') FROM pg_database WHERE datname = current_database()),
           obj_description(n.oid, 'pg_namespace'),
           current_database() || '.' || n.nspname
    FROM pg_namespace n
    WHERE n.nspname <> 'information_schema'
      AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
      AND ($2::text[] IS NULL OR n.nspname = ANY($2))
    UNION ALL
    SELECT CASE c.relkind WHEN 'r' THEN 'table' WHEN 'p' THEN 'table'
               WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized_view' ELSE 'sequence' END,
           current_database(), n.nspname, c.relname, c.oid::bigint,
           NULL, NULL, NULL, obj_description(c.oid, 'pg_class'), NULL,
           (SELECT shobj_description(oid, 'pg_database') FROM pg_database WHERE datname = current_database()),
           obj_description(n.oid, 'pg_namespace'),
           current_database() || '.' || n.nspname || '.' || c.relname
    FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE c.relkind IN ('r', 'p', 'v', 'm', 'S')
      AND n.nspname <> 'information_schema'
      AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
      AND ($2::text[] IS NULL OR n.nspname = ANY($2))
    UNION ALL
    SELECT CASE p.prokind WHEN 'f' THEN 'function' ELSE 'procedure' END,
           current_database(), n.nspname, p.proname, p.oid::bigint,
           NULL, NULL, NULL, obj_description(p.oid, 'pg_proc'), NULL,
           (SELECT shobj_description(oid, 'pg_database') FROM pg_database WHERE datname = current_database()),
           obj_description(n.oid, 'pg_namespace'),
           current_database() || '.' || n.nspname || '.' || p.proname
    FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
    WHERE p.prokind IN ('f', 'p')
       AND (p.prokind <> 'f' OR NOT EXISTS (SELECT 1 FROM pg_trigger tr WHERE tr.tgfoid = p.oid))
      AND n.nspname <> 'information_schema'
      AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
      AND ($2::text[] IS NULL OR n.nspname = ANY($2))
    UNION ALL
    SELECT 'type', current_database(), n.nspname, t.typname, t.oid::bigint,
           NULL, NULL, NULL, obj_description(t.oid, 'pg_type'), NULL,
           (SELECT shobj_description(oid, 'pg_database') FROM pg_database WHERE datname = current_database()),
           obj_description(n.oid, 'pg_namespace'),
           current_database() || '.' || n.nspname || '.' || t.typname
    FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace
    WHERE t.typtype IN ('e', 'd') AND t.typisdefined AND t.typelem = 0
      AND n.nspname <> 'information_schema'
      AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
      AND ($2::text[] IS NULL OR n.nspname = ANY($2))
    UNION ALL
    SELECT 'column', current_database(), n.nspname, a.attname, a.attnum::bigint,
           c.relkind::text, c.relname, c.oid::bigint, col_description(c.oid, a.attnum),
           obj_description(c.oid, 'pg_class'),
           (SELECT shobj_description(oid, 'pg_database') FROM pg_database WHERE datname = current_database()),
           obj_description(n.oid, 'pg_namespace'),
           current_database() || '.' || n.nspname || '.' || c.relname || '.' || a.attname
    FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE c.relkind IN ('r', 'p', 'v', 'm') AND a.attnum > 0 AND NOT a.attisdropped
      AND n.nspname <> 'information_schema'
      AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
      AND ($2::text[] IS NULL OR n.nspname = ANY($2))
    UNION ALL
    SELECT 'index', current_database(), n.nspname, ic.relname, idx.indexrelid::bigint,
           c.relkind::text, c.relname, c.oid::bigint, obj_description(ic.oid, 'pg_class'),
           obj_description(c.oid, 'pg_class'),
           (SELECT shobj_description(oid, 'pg_database') FROM pg_database WHERE datname = current_database()),
           obj_description(n.oid, 'pg_namespace'),
           current_database() || '.' || n.nspname || '.' || c.relname || '.' || ic.relname
    FROM pg_index idx JOIN pg_class c ON c.oid = idx.indrelid
         JOIN pg_class ic ON ic.oid = idx.indexrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE c.relkind IN ('r', 'p', 'v', 'm')
      AND n.nspname <> 'information_schema'
      AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
      AND ($2::text[] IS NULL OR n.nspname = ANY($2))
    UNION ALL
    SELECT CASE con.contype WHEN 'p' THEN 'primary_key' WHEN 'u' THEN 'unique_constraint'
               WHEN 'f' THEN 'foreign_key' ELSE 'check_constraint' END,
           current_database(), n.nspname, con.conname, con.oid::bigint,
           c.relkind::text, c.relname, c.oid::bigint, obj_description(con.oid, 'pg_constraint'),
           obj_description(c.oid, 'pg_class'),
           (SELECT shobj_description(oid, 'pg_database') FROM pg_database WHERE datname = current_database()),
           obj_description(n.oid, 'pg_namespace'),
           current_database() || '.' || n.nspname || '.' || c.relname || '.' || con.conname
    FROM pg_constraint con JOIN pg_class c ON c.oid = con.conrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE con.contype IN ('p', 'u', 'f', 'c') AND c.relkind IN ('r', 'p', 'v', 'm')
      AND n.nspname <> 'information_schema'
      AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
      AND ($2::text[] IS NULL OR n.nspname = ANY($2))
), normalized AS (
    SELECT *, CASE WHEN $5 THEN regexp_replace(lower(object_name), '[^[:alnum:]]', '', 'g')
                   ELSE lower(object_name) END AS search_name,
              CASE WHEN $5 THEN regexp_replace(lower(qualified_path), '[^[:alnum:]]', '', 'g')
                   ELSE lower(qualified_path) END AS search_path
    FROM candidates
), ranked AS (
    SELECT *, CASE
        WHEN search_name = $1 OR search_path = $1 OR right(search_path, length($1)) = $1 THEN 0
        WHEN strpos(search_name, $1) = 1 OR strpos(search_path, $1) = 1 THEN 1
        WHEN strpos(search_name, $1) > 0 OR strpos(search_path, $1) > 0 THEN 2
        ELSE 3 END AS relevance
    FROM normalized
    WHERE $3
      AND (strpos(search_name, $1) > 0 OR strpos(search_path, $1) > 0)
)
SELECT kind, database_name, schema_name, object_name, object_oid,
       relation_kind, relation_name, relation_oid, comment, relation_comment,
       database_comment, schema_comment
FROM ranked
ORDER BY relevance, lower(qualified_path) COLLATE "C", qualified_path COLLATE "C",
         kind COLLATE "C", object_oid
LIMIT $4
"#;

#[derive(Clone, Debug)]
pub struct PostgresAdapter {
    pool: PgPool,
    connection_id: Uuid,
    catalog_scope: CatalogScope,
}

struct PgIndexInfo {
    oid: i64,
    name: String,
    columns: Vec<String>,
    unique: bool,
    comment: Option<String>,
}

struct PgConstraintInfo {
    oid: i64,
    name: String,
    kind: CatalogKind,
    columns: Vec<String>,
    referenced_schema: Option<String>,
    referenced_relation: Option<String>,
    referenced_columns: Vec<String>,
    check_expression: Option<String>,
    comment: Option<String>,
}

#[derive(Debug)]
struct PgDdlColumn {
    name: String,
    native_type: String,
    default_expression: Option<String>,
    not_null: bool,
    identity_kind: String,
    generated_kind: String,
}

#[derive(Debug)]
struct PgDdlRelation {
    schema: String,
    name: String,
    relation_kind: String,
    persistence: String,
    view_definition: Option<String>,
    materialized_populated: bool,
    partition_key: Option<String>,
    partition_parent: Option<(String, String)>,
    partition_bound: Option<String>,
    columns: Vec<PgDdlColumn>,
    constraints: Vec<(String, String)>,
    relation_comment: Option<String>,
    column_comments: Vec<(String, String)>,
    indexes: Vec<(String, String)>,
    triggers: Vec<(String, String)>,
}

struct PgSearchCandidate {
    kind: CatalogKind,
    database: String,
    schema: Option<String>,
    name: String,
    oid: Option<i64>,
    relation_kind: Option<String>,
    relation_name: Option<String>,
    relation_oid: Option<i64>,
    comment: Option<String>,
    relation_comment: Option<String>,
    database_comment: Option<String>,
    schema_comment: Option<String>,
}

impl PostgresAdapter {
    pub const MONITOR_STATUS_SQL: &str = r#"
WITH db_stats AS (
    SELECT coalesce(sum(xact_commit), 0)::double precision AS commits,
           coalesce(sum(xact_rollback), 0)::double precision AS rollbacks,
           coalesce(sum(blks_hit), 0)::double precision AS block_hits,
           coalesce(sum(blks_read), 0)::double precision AS block_reads,
           coalesce(sum(tup_returned), 0)::double precision AS selects,
           coalesce(sum(tup_inserted), 0)::double precision AS inserts,
           coalesce(sum(tup_updated), 0)::double precision AS updates,
           coalesce(sum(tup_deleted), 0)::double precision AS deletes,
           coalesce(sum(deadlocks), 0)::double precision AS deadlocks,
           coalesce(sum(temp_files), 0)::double precision AS temp_files,
           coalesce(sum(temp_bytes), 0)::double precision AS temp_bytes,
           coalesce(pg_wal_lsn_diff(pg_current_wal_lsn(), '0/0'), 0)::double precision AS wal_bytes
    FROM pg_stat_database WHERE datname = current_database()
), activity_stats AS (
    SELECT count(*)::double precision AS connections,
           count(*) FILTER (WHERE state = 'active')::double precision AS active_connections,
           count(*) FILTER (WHERE state = 'idle')::double precision AS idle_connections
    FROM pg_stat_activity WHERE pid <> pg_backend_pid()
)
SELECT floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint AS server_time_millis,
       floor(extract(epoch FROM pg_postmaster_start_time()) * 1000)::bigint AS server_generation,
       extract(epoch FROM clock_timestamp() - pg_postmaster_start_time())::double precision AS server_uptime,
       db_stats.*, activity_stats.*
FROM db_stats CROSS JOIN activity_stats
"#;

    pub const MONITOR_METADATA_SQL: &str = "SELECT current_setting('server_version') AS version, current_setting('max_connections')::bigint AS max_connections";

    pub const PROCESS_LIST_SQL: &str = r#"
SELECT pid, usename AS user_name, datname AS database_name,
       coalesce(host(client_addr), client_hostname, 'local') AS client,
       application_name, state,
       coalesce(nullif(wait_event_type, '') || ':' || wait_event, wait_event_type) AS wait,
       extract(epoch FROM clock_timestamp() - coalesce(query_start, xact_start, backend_start))::double precision AS elapsed_seconds,
       query
FROM pg_stat_activity
WHERE pid <> pg_backend_pid()
ORDER BY CASE WHEN state = 'active' THEN 0 ELSE 1 END,
         elapsed_seconds DESC NULLS LAST
LIMIT 2001
"#;

    pub async fn load_monitor_snapshot(
        &self,
    ) -> Result<crate::db::monitor::MonitorSnapshot, DatabaseError> {
        let row = sqlx::query(Self::MONITOR_STATUS_SQL)
            .fetch_one(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let mut values = std::collections::BTreeMap::new();
        for (name, key) in [
            ("commits", MetricKey::Commits),
            ("rollbacks", MetricKey::Rollbacks),
            ("block_hits", MetricKey::BlockHits),
            ("block_reads", MetricKey::BlockReads),
            ("selects", MetricKey::Selects),
            ("inserts", MetricKey::Inserts),
            ("updates", MetricKey::Updates),
            ("deletes", MetricKey::Deletes),
            ("deadlocks", MetricKey::Deadlocks),
            ("temp_files", MetricKey::TempFiles),
            ("temp_bytes", MetricKey::TempBytes),
            ("wal_bytes", MetricKey::WalBytes),
            ("connections", MetricKey::Connections),
            ("active_connections", MetricKey::ActiveConnections),
            ("idle_connections", MetricKey::IdleConnections),
        ] {
            values.insert(key, row.try_get(name).map_err(decode_error)?);
        }
        let commits = values[&MetricKey::Commits];
        let rollbacks = values[&MetricKey::Rollbacks];
        values.insert(MetricKey::Transactions, commits + rollbacks);
        values.insert(
            MetricKey::ServerUptime,
            row.try_get("server_uptime").map_err(decode_error)?,
        );
        Ok(crate::db::monitor::MonitorSnapshot {
            server_time_millis: monitor_timestamp(&row, "server_time_millis")?,
            server_generation: monitor_timestamp(&row, "server_generation")?,
            values,
        })
    }

    pub async fn load_monitor_metadata(
        &self,
    ) -> Result<crate::db::monitor::MonitorMetadata, DatabaseError> {
        let row = sqlx::query(Self::MONITOR_METADATA_SQL)
            .fetch_one(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        Ok(crate::db::monitor::MonitorMetadata {
            version: row.try_get("version").map_err(decode_error)?,
            max_connections: row
                .try_get::<i64, _>("max_connections")
                .map_err(decode_error)?
                .try_into()
                .ok(),
        })
    }

    pub async fn load_process_snapshot(
        &self,
    ) -> Result<crate::db::monitor::ProcessSnapshot, DatabaseError> {
        let rows = sqlx::query(Self::PROCESS_LIST_SQL)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let truncated = rows.len() > crate::db::monitor::MAX_PROCESS_ROWS;
        let rows = rows
            .into_iter()
            .take(crate::db::monitor::MAX_PROCESS_ROWS)
            .map(|row| {
                Ok(crate::model::dashboard::ProcessRow {
                    id: row.try_get::<i32, _>("pid").map_err(decode_error)? as u64,
                    user: sanitize_terminal_text(
                        &row.try_get::<Option<String>, _>("user_name")
                            .map_err(decode_error)?
                            .unwrap_or_default(),
                    ),
                    database: row.try_get("database_name").map_err(decode_error)?,
                    client: row.try_get("client").map_err(decode_error)?,
                    application: row.try_get("application_name").map_err(decode_error)?,
                    state: row.try_get("state").map_err(decode_error)?,
                    wait: row.try_get("wait").map_err(decode_error)?,
                    elapsed: crate::db::monitor::parse_process_duration(
                        row.try_get("elapsed_seconds").map_err(decode_error)?,
                    ),
                    query: row
                        .try_get::<Option<String>, _>("query")
                        .map_err(decode_error)?
                        .map(|value| sanitize_terminal_text(&value)),
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        Ok(crate::db::monitor::ProcessSnapshot {
            rows,
            truncated,
            visibility: crate::db::monitor::MonitorVisibility::Unknown,
        })
    }
    pub fn plan_catalog_drop(
        request: CatalogDropRequest,
        entry: &CatalogEntry,
    ) -> Result<CatalogDropPlan, CatalogDropError> {
        let sql = match entry.kind {
            CatalogKind::Table => {
                format!("DROP TABLE {}", postgres_qualified_name(entry, entry.kind)?)
            }
            CatalogKind::View => {
                format!("DROP VIEW {}", postgres_qualified_name(entry, entry.kind)?)
            }
            CatalogKind::MaterializedView => format!(
                "DROP MATERIALIZED VIEW {}",
                postgres_qualified_name(entry, entry.kind)?
            ),
            CatalogKind::Sequence => format!(
                "DROP SEQUENCE {}",
                postgres_qualified_name(entry, entry.kind)?
            ),
            CatalogKind::Type => {
                format!("DROP TYPE {}", postgres_qualified_name(entry, entry.kind)?)
            }
            CatalogKind::Column => format!(
                "ALTER TABLE {} DROP COLUMN {}",
                relation_name_for_drop(entry)?,
                quote_identifier(&entry.qualified_name.object)
            ),
            CatalogKind::Index => {
                format!("DROP INDEX {}", postgres_qualified_name(entry, entry.kind)?)
            }
            CatalogKind::PrimaryKey
            | CatalogKind::UniqueConstraint
            | CatalogKind::ForeignKey
            | CatalogKind::CheckConstraint => format!(
                "ALTER TABLE {} DROP CONSTRAINT {}",
                relation_name_for_drop(entry)?,
                quote_identifier(&entry.qualified_name.object)
            ),
            CatalogKind::Trigger => format!(
                "DROP TRIGGER {} ON {}",
                quote_identifier(&entry.qualified_name.object),
                relation_name_for_drop(entry)?
            ),
            kind => {
                return Err(CatalogDropError::Unsupported {
                    kind,
                    reason:
                        "catalog metadata does not provide an unambiguous PostgreSQL drop target"
                            .to_owned(),
                });
            }
        };
        CatalogDropPlan::new(request, entry, sql)
    }

    pub fn catalog_capabilities() -> CatalogCapabilities {
        CatalogCapabilities {
            namespace_model: NamespaceModel::DatabaseAndSchema,
            top_level_groups: vec![
                ObjectGroup::Tables,
                ObjectGroup::Views,
                ObjectGroup::MaterializedViews,
                ObjectGroup::Sequences,
                ObjectGroup::Functions,
                ObjectGroup::Procedures,
                ObjectGroup::Types,
            ],
            column_metadata: ColumnMetadataCapabilities {
                type_family: true,
                default_expression: true,
                identity: true,
                generated_expression: true,
                numeric_precision_and_scale: false,
                character_length: true,
                collation: true,
                comment: true,
                ..ColumnMetadataCapabilities::default()
            },
            supports_lazy_children: true,
        }
    }

    pub(crate) async fn transaction_backend(
        &self,
    ) -> Result<PostgresTransactionBackend, DatabaseError> {
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        let pid = sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
            .fetch_one(&mut *connection)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        Ok(PostgresTransactionBackend {
            connection,
            control: self.pool.clone(),
            pid,
            adapter: self.clone(),
        })
    }

    pub async fn connect(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
    ) -> Result<Self, DatabaseError> {
        if profile.kind != DatabaseKind::Postgres {
            return Err(DatabaseError::configuration("profile is not PostgreSQL"));
        }
        let host = profile
            .host
            .as_deref()
            .ok_or_else(|| DatabaseError::configuration("PostgreSQL profile has no host"))?;
        let mut options = PgConnectOptions::new()
            .host(host)
            .port(profile.port.unwrap_or(5432))
            .ssl_mode(pg_ssl_mode(profile.ssl_mode))
            .application_name("lazydb");
        if let Some(user) = &profile.user {
            options = options.username(user);
        }
        if let Some(database) = &profile.database {
            options = options.database(database);
        }
        if let Some(password) = password {
            options = options.password(password.expose_secret());
        }
        if profile.read_only {
            options = options.options([("default_transaction_read_only", "on")]);
        }
        if let Some(schema) = &profile.default_schema {
            let search_path = format!("{}, public", quote_identifier(schema));
            options = options.options([("search_path", search_path.as_str())]);
        }

        let pool = PgPoolOptions::new()
            .max_connections(6)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        let server_version =
            match sqlx::query_scalar::<_, i32>("SELECT current_setting('server_version_num')::int")
                .fetch_one(&pool)
                .await
            {
                Ok(version) => version,
                Err(error) => {
                    pool.close().await;
                    return Err(DatabaseError::from_sqlx(error, ErrorCategory::Network));
                }
            };
        if !supports_server_version(server_version) {
            pool.close().await;
            return Err(DatabaseError {
                category: ErrorCategory::Unsupported,
                code: Some("postgres_version_unsupported".to_owned()),
                message: format!(
                    "PostgreSQL 12 or newer is required; server_version_num is {server_version}"
                ),
            });
        }
        Ok(Self {
            pool,
            connection_id: profile.id,
            catalog_scope: profile.catalog_scope.clone(),
        })
    }

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        let row = sqlx::query("SELECT version() AS version, current_database() AS database")
            .fetch_one(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        Ok(ServerInfo {
            kind: DatabaseKind::Postgres,
            version: row.try_get("version").map_err(decode_error)?,
            database: row.try_get("database").map_err(decode_error)?,
        })
    }

    pub async fn discover_catalog_scope(&self) -> Result<CatalogDiscovery, DatabaseError> {
        let database = sqlx::query_scalar::<_, String>("SELECT current_database()")
            .fetch_one(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let schemas = sqlx::query_scalar::<_, String>(CATALOG_SCHEMAS_SQL)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;

        Ok(CatalogDiscovery {
            databases: vec![DiscoveredDatabase {
                name: database,
                schemas,
            }],
            warnings: Vec::new(),
        })
    }

    pub async fn discoverable_databases(&self) -> Result<Vec<String>, DatabaseError> {
        sqlx::query_scalar::<_, String>(
            "SELECT datname FROM pg_database \
             WHERE datallowconn AND NOT datistemplate \
               AND has_database_privilege(datname, 'CONNECT') \
             ORDER BY datname COLLATE \"C\"",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))
    }

    pub async fn execute(&self, sql: &str) -> Result<QueryOutcome, DatabaseError> {
        self.execute_pool(sql).await
    }

    pub(crate) async fn execute_pool(&self, sql: &str) -> Result<QueryOutcome, DatabaseError> {
        let mut stream = sqlx::raw_sql(AssertSqlSafe(sql)).fetch_many(&self.pool);
        self.collect_stream(&mut stream).await
    }

    pub(crate) async fn execute_connection(
        &self,
        connection: &mut PgConnection,
        sql: &str,
    ) -> Result<QueryOutcome, DatabaseError> {
        let mut stream = sqlx::raw_sql(AssertSqlSafe(sql)).fetch_many(&mut *connection);
        self.collect_stream(&mut stream).await
    }

    async fn collect_stream<E>(&self, stream: &mut E) -> Result<QueryOutcome, DatabaseError>
    where
        E: futures_util::TryStream<
                Ok = Either<sqlx::postgres::PgQueryResult, PgRow>,
                Error = sqlx::Error,
            > + Unpin,
    {
        let mut accumulator = QueryOutcomeAccumulator::new();
        while let Some(event) = stream
            .try_next()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?
        {
            match event {
                Either::Right(row) => {
                    accumulator.row(columns(&row), decode_row(&row));
                }
                Either::Left(done) => {
                    accumulator.done(done.rows_affected());
                }
            }
        }
        Ok(accumulator.finish())
    }

    pub async fn load_catalog_page(
        &self,
        request: &CatalogRequest,
    ) -> Result<CatalogPage, DatabaseError> {
        request
            .validate_for_profile(self.connection_id)
            .map_err(DatabaseError::invalid_catalog_request)?;
        if matches!(
            request.key.target,
            CatalogTarget::Objects {
                group: ObjectGroup::Triggers,
                ..
            }
        ) {
            return Err(DatabaseError::unsupported_catalog_target(
                DatabaseKind::Postgres,
                &request.key.target,
            ));
        }

        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        let mut transaction = connection
            .begin()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *transaction)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let page = match &request.key.target {
            CatalogTarget::Databases => self.load_database_page(&mut transaction, request).await,
            CatalogTarget::Schemas { database } => {
                self.load_schema_page(&mut transaction, request, database)
                    .await
            }
            CatalogTarget::Groups { schema } => {
                self.load_group_page(&mut transaction, request, schema)
                    .await
            }
            CatalogTarget::Objects { schema, group } => {
                self.load_object_page(&mut transaction, request, schema, *group)
                    .await
            }
            CatalogTarget::RelationChildren { relation } => {
                self.load_relation_children_page(&mut transaction, request, relation)
                    .await
            }
        };
        let rollback = transaction
            .rollback()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql));
        if let Err(rollback_error) = rollback {
            return match page {
                Ok(_) => Err(rollback_error),
                Err(page_error) => Err(page_error),
            };
        }
        page
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
                CatalogValidationError::ProfileMismatch {
                    child_profile_id: request.connection.profile_id,
                    parent_profile_id: self.connection_id,
                },
            ));
        }

        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        let mut transaction = connection.begin().await.map_err(sql_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *transaction)
            .await
            .map_err(sql_error)?;
        let result = self
            .search_catalog_snapshot(&mut transaction, request)
            .await;
        let rollback = transaction.rollback().await.map_err(sql_error);
        if let Err(rollback_error) = rollback {
            return result.and(Err(rollback_error));
        }
        result
    }

    async fn search_catalog_snapshot(
        &self,
        connection: &mut PgConnection,
        request: &CatalogSearchRequest,
    ) -> Result<CatalogSearchPage, DatabaseError> {
        let database: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        let database_allowed = request.scope.allows_database(&database);
        let schemas = search_schemas(&request.scope, &database);
        let sql_limit = i64::try_from(request.limit.saturating_add(1))
            .map_err(|_| catalog_internal("PostgreSQL catalog search limit overflowed"))?;
        let (search_query, ignore_separators) = crate::db::catalog::search_query(&request.query);
        let rows = sqlx::query(SEARCH_CATALOG_SQL)
            .bind(search_query)
            .bind(schemas)
            .bind(database_allowed)
            .bind(sql_limit)
            .bind(ignore_separators)
            .fetch_all(&mut *connection)
            .await
            .map_err(sql_error)?;
        let mut candidates = rows
            .into_iter()
            .map(decode_search_candidate)
            .collect::<Result<Vec<_>, _>>()?;
        let truncated = candidates.len() > request.limit;
        candidates.truncate(request.limit);

        let relation_ids = candidates
            .iter()
            .filter_map(|candidate| candidate_relation_id(self.connection_id, candidate))
            .collect::<HashSet<_>>();
        let mut children = HashMap::new();
        for relation in &relation_ids {
            let [database, schema, _, oid] = relation.native_path.as_slice() else {
                return Err(catalog_internal(
                    "invalid PostgreSQL search relation identity",
                ));
            };
            let relation_oid = oid
                .parse::<i64>()
                .map_err(|_| catalog_internal("invalid PostgreSQL search relation OID"))?;
            for child in self
                .load_relation_children_entries(
                    connection,
                    database,
                    schema,
                    relation_oid,
                    relation,
                )
                .await?
            {
                children.insert(child.id.clone(), child);
            }
        }

        let mut hits = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            hits.push(self.search_hit(&database, candidate, &children)?);
        }
        CatalogSearchPage::new(request, hits, None, truncated)
            .map_err(DatabaseError::invalid_catalog_request)
    }

    fn search_hit(
        &self,
        database: &str,
        candidate: PgSearchCandidate,
        children: &HashMap<CatalogId, CatalogEntry>,
    ) -> Result<CatalogSearchHit, DatabaseError> {
        let database_entry = CatalogEntry::database(
            CatalogId::new(self.connection_id, CatalogKind::Database, [database]),
            qualified_database(database),
            "database",
            OptionalMetadata::Supported(candidate.database_comment.clone()),
            true,
        )
        .map_err(catalog_invariant)?;
        if candidate.kind == CatalogKind::Database {
            return Ok(CatalogSearchHit {
                entry: database_entry,
                ancestors: Vec::new(),
            });
        }
        let schema = candidate
            .schema
            .as_deref()
            .ok_or_else(|| catalog_internal("PostgreSQL search object has no schema"))?;
        let schema_id = CatalogId::new(self.connection_id, CatalogKind::Schema, [database, schema]);
        let schema_entry = CatalogEntry::schema(
            schema_id.clone(),
            database_entry.id.clone(),
            qualified_schema(database, schema),
            "schema",
            OptionalMetadata::Supported(candidate.schema_comment.clone()),
            true,
        )
        .map_err(catalog_invariant)?;
        if candidate.kind == CatalogKind::Schema {
            return Ok(CatalogSearchHit {
                entry: schema_entry,
                ancestors: vec![database_entry],
            });
        }

        if candidate.kind.is_relation_child() {
            let relation = candidate_relation_entry(self.connection_id, &candidate, &schema_id)?;
            let child_id = candidate_child_id(&relation.id, &candidate)?;
            let entry = children.get(&child_id).cloned().ok_or_else(|| {
                catalog_internal("PostgreSQL search child disappeared during hydration")
            })?;
            return Ok(CatalogSearchHit {
                entry,
                ancestors: vec![database_entry, schema_entry, relation],
            });
        }

        let oid = candidate
            .oid
            .ok_or_else(|| catalog_internal("PostgreSQL search object has no OID"))?;
        let id = CatalogId::new(
            self.connection_id,
            candidate.kind,
            [
                database.to_owned(),
                schema.to_owned(),
                candidate.name.clone(),
                oid.to_string(),
            ],
        );
        let qualified = qualified_object(database, schema, &candidate.name);
        let (native_kind, expandable) = search_kind_properties(candidate.kind)?;
        let entry = if candidate.kind.is_relation() {
            CatalogEntry::relation(
                id,
                schema_id,
                qualified,
                native_kind,
                OptionalMetadata::Supported(candidate.comment),
                expandable,
            )
        } else {
            CatalogEntry::object(
                id,
                schema_id,
                qualified,
                native_kind,
                OptionalMetadata::Supported(candidate.comment),
                expandable,
            )
        }
        .map_err(catalog_invariant)?;
        Ok(CatalogSearchHit {
            entry,
            ancestors: vec![database_entry, schema_entry],
        })
    }

    async fn load_database_page(
        &self,
        connection: &mut PgConnection,
        request: &CatalogRequest,
    ) -> Result<CatalogPage, DatabaseError> {
        let database: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        let mut entries = if request.scope.allows_database(&database) {
            vec![
                CatalogEntry::database(
                    CatalogId::new(
                        self.connection_id,
                        CatalogKind::Database,
                        [database.clone()],
                    ),
                    qualified_database(&database),
                    "database",
                    OptionalMetadata::Supported(None),
                    true,
                )
                .map_err(catalog_invariant)?,
            ]
        } else {
            Vec::new()
        };
        let total_count = exact_count(entries.len())?;
        let next_cursor = paginate_in_memory(
            &mut entries,
            request,
            |entry| entry.qualified_name.object.clone(),
            |entry| entry.qualified_name.object.clone(),
        )?;
        CatalogPage::new(request, entries, total_count, next_cursor).map_err(catalog_invariant)
    }

    async fn load_schema_page(
        &self,
        connection: &mut PgConnection,
        request: &CatalogRequest,
        database_id: &CatalogId,
    ) -> Result<CatalogPage, DatabaseError> {
        let database = self
            .verify_database(connection, database_id, &request.key.target)
            .await?;
        let selected = selected_schemas(request, &database);
        let rows = match selected {
            Some(schemas) => {
                sqlx::query(
                    "SELECT n.oid::bigint AS oid, n.nspname AS name, obj_description(n.oid, 'pg_namespace') AS comment \
                     FROM pg_namespace n WHERE n.nspname <> 'information_schema' \
                     AND n.nspname NOT LIKE 'pg\\_%' ESCAPE '\\' AND n.nspname = ANY($1) \
                     ORDER BY n.nspname COLLATE \"C\", n.oid",
                )
                .bind(schemas)
                .fetch_all(&mut *connection)
                .await
            }
            None => {
                sqlx::query(
                    "SELECT n.oid::bigint AS oid, n.nspname AS name, obj_description(n.oid, 'pg_namespace') AS comment \
                     FROM pg_namespace n WHERE n.nspname <> 'information_schema' \
                     AND n.nspname NOT LIKE 'pg\\_%' ESCAPE '\\' \
                     ORDER BY n.nspname COLLATE \"C\", n.oid",
                )
                .fetch_all(&mut *connection)
                .await
            }
        }
        .map_err(sql_error)?;
        let database_parent = CatalogId::new(
            self.connection_id,
            CatalogKind::Database,
            [database.clone()],
        );
        let mut entries = rows
            .into_iter()
            .map(|row| {
                let name: String = row.try_get("name").map_err(decode_error)?;
                let comment: Option<String> = row.try_get("comment").map_err(decode_error)?;
                CatalogEntry::schema(
                    CatalogId::new(
                        self.connection_id,
                        CatalogKind::Schema,
                        [database.clone(), name.clone()],
                    ),
                    database_parent.clone(),
                    qualified_schema(&database, &name),
                    "schema",
                    OptionalMetadata::Supported(comment),
                    true,
                )
                .map_err(catalog_invariant)
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        let total_count = exact_count(entries.len())?;
        let next_cursor = paginate_in_memory(
            &mut entries,
            request,
            |entry| entry.qualified_name.object.clone(),
            |entry| entry.qualified_name.object.clone(),
        )?;
        CatalogPage::new(request, entries, total_count, next_cursor).map_err(catalog_invariant)
    }

    async fn load_group_page(
        &self,
        connection: &mut PgConnection,
        request: &CatalogRequest,
        schema_id: &CatalogId,
    ) -> Result<CatalogPage, DatabaseError> {
        let (_, schema) = self
            .verify_schema(connection, schema_id, &request.key.target)
            .await?;
        let row = sqlx::query(
            "SELECT \
             (SELECT COUNT(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind IN ('r','p'))::bigint AS tables, \
             (SELECT COUNT(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind='v')::bigint AS views, \
             (SELECT COUNT(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind='m')::bigint AS materialized_views, \
             (SELECT COUNT(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind='S')::bigint AS sequences, \
              (SELECT COUNT(*) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname=$1 AND p.prokind='f' AND NOT EXISTS (SELECT 1 FROM pg_trigger tr WHERE tr.tgfoid=p.oid))::bigint AS functions, \
             (SELECT COUNT(*) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname=$1 AND p.prokind='p')::bigint AS procedures, \
             (SELECT COUNT(*) FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace WHERE n.nspname=$1 AND t.typtype IN ('e','d') AND t.typisdefined AND t.typelem=0)::bigint AS types",
        )
        .bind(&schema)
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
        let mut summaries = Vec::new();
        for (group, column) in [
            (ObjectGroup::Tables, "tables"),
            (ObjectGroup::Views, "views"),
            (ObjectGroup::MaterializedViews, "materialized_views"),
            (ObjectGroup::Sequences, "sequences"),
            (ObjectGroup::Functions, "functions"),
            (ObjectGroup::Procedures, "procedures"),
            (ObjectGroup::Types, "types"),
        ] {
            summaries.push(CatalogGroupSummary {
                group,
                object_count: CatalogCount::Exact(non_negative_count(
                    row.try_get(column).map_err(decode_error)?,
                )?),
            });
        }
        let total_count = exact_count(summaries.len())?;
        let next_cursor = paginate_in_memory(
            &mut summaries,
            request,
            |summary| group_sort_key(summary.group).to_owned(),
            |summary| group_sort_key(summary.group).to_owned(),
        )?;
        CatalogPage::groups(request, summaries, total_count, next_cursor).map_err(catalog_invariant)
    }

    async fn load_object_page(
        &self,
        connection: &mut PgConnection,
        request: &CatalogRequest,
        schema_id: &CatalogId,
        group: ObjectGroup,
    ) -> Result<CatalogPage, DatabaseError> {
        let (database, schema) = self
            .verify_schema(connection, schema_id, &request.key.target)
            .await?;
        let (source, predicate, kind, native_kind, expandable) = match group {
            ObjectGroup::Tables => (
                "pg_class c",
                "c.relkind IN ('r','p')",
                CatalogKind::Table,
                "table",
                true,
            ),
            ObjectGroup::Views => (
                "pg_class c",
                "c.relkind='v'",
                CatalogKind::View,
                "view",
                true,
            ),
            ObjectGroup::MaterializedViews => (
                "pg_class c",
                "c.relkind='m'",
                CatalogKind::MaterializedView,
                "materialized_view",
                true,
            ),
            ObjectGroup::Sequences => (
                "pg_class c",
                "c.relkind='S'",
                CatalogKind::Sequence,
                "sequence",
                false,
            ),
            ObjectGroup::Functions => (
                "pg_proc c",
                "c.prokind='f' AND NOT EXISTS (SELECT 1 FROM pg_trigger tr WHERE tr.tgfoid=c.oid)",
                CatalogKind::Function,
                "function",
                false,
            ),
            ObjectGroup::Procedures => (
                "pg_proc c",
                "c.prokind='p'",
                CatalogKind::Procedure,
                "procedure",
                false,
            ),
            ObjectGroup::Types => (
                "pg_type c",
                "c.typtype IN ('e','d') AND c.typisdefined AND c.typelem=0",
                CatalogKind::Type,
                "type",
                false,
            ),
            ObjectGroup::Triggers => {
                return Err(DatabaseError::unsupported_catalog_target(
                    DatabaseKind::Postgres,
                    &request.key.target,
                ));
            }
        };
        let namespace_column = match group {
            ObjectGroup::Functions | ObjectGroup::Procedures => "c.pronamespace",
            ObjectGroup::Types => "c.typnamespace",
            _ => "c.relnamespace",
        };
        let name_column = match group {
            ObjectGroup::Functions | ObjectGroup::Procedures => "c.proname",
            ObjectGroup::Types => "c.typname",
            _ => "c.relname",
        };
        let count_sql = format!(
            "SELECT COUNT(*)::bigint FROM {source} JOIN pg_namespace n ON n.oid={namespace_column} WHERE n.nspname=$1 AND {predicate}"
        );
        let count: i64 = sqlx::query_scalar(AssertSqlSafe(count_sql))
            .bind(&schema)
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        let total_count = CatalogCount::Exact(non_negative_count(count)?);
        let cursor = request
            .key
            .cursor
            .as_ref()
            .map(CatalogCursor::keyset_parts)
            .transpose()
            .map_err(DatabaseError::invalid_catalog_request)?;
        let limit = page_limit(request.page_size)?;
        let description = match group {
            ObjectGroup::Functions | ObjectGroup::Procedures => "obj_description(c.oid, 'pg_proc')",
            ObjectGroup::Types => "obj_description(c.oid, 'pg_type')",
            _ => "obj_description(c.oid, 'pg_class')",
        };
        let select = format!(
            "SELECT c.oid::bigint AS oid, {name_column} AS name, {description} AS comment FROM {source} JOIN pg_namespace n ON n.oid={namespace_column} WHERE n.nspname=$1 AND {predicate}"
        );
        let rows = if let Some((sort_key, tie_breaker)) = cursor {
            let oid = tie_breaker.parse::<i64>().map_err(|_| DatabaseError::invalid_catalog_request(CatalogValidationError::MalformedCursor))?;
            let sql = format!("{select} AND (({name_column} COLLATE \"C\") > ($2 COLLATE \"C\") OR ({name_column} COLLATE \"C\") = ($2 COLLATE \"C\") AND c.oid > $3::oid) ORDER BY {name_column} COLLATE \"C\", c.oid LIMIT $4");
            sqlx::query(AssertSqlSafe(sql)).bind(&schema).bind(sort_key).bind(oid).bind(limit).fetch_all(&mut *connection).await
        } else {
            let sql = format!("{select} ORDER BY {name_column} COLLATE \"C\", c.oid LIMIT $2");
            sqlx::query(AssertSqlSafe(sql)).bind(&schema).bind(limit).fetch_all(&mut *connection).await
        }.map_err(sql_error)?;
        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let oid: i64 = row.try_get("oid").map_err(decode_error)?;
            let name: String = row.try_get("name").map_err(decode_error)?;
            let comment: Option<String> = row.try_get("comment").map_err(decode_error)?;
            let id = CatalogId::new(
                self.connection_id,
                kind,
                [
                    database.clone(),
                    schema.clone(),
                    name.clone(),
                    oid.to_string(),
                ],
            );
            let qualified = qualified_object(&database, &schema, &name);
            let entry = if kind.is_relation() {
                CatalogEntry::relation(
                    id,
                    schema_id.clone(),
                    qualified,
                    native_kind,
                    OptionalMetadata::Supported(comment),
                    expandable,
                )
            } else {
                CatalogEntry::object(
                    id,
                    schema_id.clone(),
                    qualified,
                    native_kind,
                    OptionalMetadata::Supported(comment),
                    expandable,
                )
            }
            .map_err(catalog_invariant)?;
            entries.push(entry);
        }
        let next_cursor = finalize_keyset_page(
            &mut entries,
            request.page_size,
            |entry| entry.qualified_name.object.clone(),
            |entry| {
                entry
                    .id
                    .native_path
                    .last()
                    .and_then(|oid| oid.parse::<u64>().ok())
                    .map_or_else(String::new, |oid| format!("{oid:020}"))
            },
        )
        .map_err(catalog_invariant)?;
        CatalogPage::new(request, entries, total_count, next_cursor).map_err(catalog_invariant)
    }

    async fn load_relation_children_page(
        &self,
        connection: &mut PgConnection,
        request: &CatalogRequest,
        relation: &CatalogId,
    ) -> Result<CatalogPage, DatabaseError> {
        let (database, schema, _, relation_oid, _) = self
            .verify_relation(connection, relation, &request.key.target)
            .await?;
        let mut entries = self
            .load_relation_children_entries(connection, &database, &schema, relation_oid, relation)
            .await?;
        let total_count = exact_count(entries.len())?;
        let next_cursor =
            paginate_in_memory(&mut entries, request, child_sort_key, child_tie_breaker)?;
        CatalogPage::new(request, entries, total_count, next_cursor).map_err(catalog_invariant)
    }

    async fn load_relation_children_entries(
        &self,
        connection: &mut PgConnection,
        database: &str,
        schema: &str,
        relation_oid: i64,
        relation: &CatalogId,
    ) -> Result<Vec<CatalogEntry>, DatabaseError> {
        let indexes = self.load_pg_indexes(connection, relation_oid).await?;
        let constraints = self.load_pg_constraints(connection, relation_oid).await?;
        let mut memberships: HashMap<String, Vec<ConstraintMembership>> = HashMap::new();
        let mut entries = Vec::new();

        for index in indexes {
            entries.push(
                CatalogEntry::relation_child(
                    relation_child_id(relation, CatalogKind::Index, &index.oid.to_string()),
                    relation.clone(),
                    qualified_object(database, schema, &index.name),
                    "index",
                    OptionalMetadata::Supported(index.comment),
                    CatalogMetadata::Index(IndexMetadata {
                        columns: index.columns,
                        unique: index.unique,
                    }),
                )
                .map_err(catalog_invariant)?,
            );
        }
        for constraint in constraints {
            let id = relation_child_id(relation, constraint.kind, &constraint.oid.to_string());
            if matches!(
                constraint.kind,
                CatalogKind::PrimaryKey | CatalogKind::UniqueConstraint | CatalogKind::ForeignKey
            ) {
                add_memberships(&mut memberships, &constraint.columns, &id)?;
            }
            let metadata = match constraint.kind {
                CatalogKind::PrimaryKey => {
                    CatalogMetadata::Constraint(ConstraintMetadata::PrimaryKey {
                        columns: constraint.columns,
                    })
                }
                CatalogKind::UniqueConstraint => {
                    CatalogMetadata::Constraint(ConstraintMetadata::Unique {
                        columns: constraint.columns,
                    })
                }
                CatalogKind::ForeignKey => {
                    CatalogMetadata::Constraint(ConstraintMetadata::ForeignKey {
                        columns: constraint.columns,
                        referenced_relation: QualifiedName {
                            database: Some(database.to_owned()),
                            schema: constraint.referenced_schema,
                            object: constraint.referenced_relation.ok_or_else(|| {
                                catalog_internal("foreign key has no referenced relation")
                            })?,
                        },
                        referenced_columns: constraint.referenced_columns,
                    })
                }
                CatalogKind::CheckConstraint => {
                    CatalogMetadata::Constraint(ConstraintMetadata::Check {
                        expression: constraint.check_expression.ok_or_else(|| {
                            catalog_internal("check constraint has no expression")
                        })?,
                    })
                }
                _ => return Err(catalog_internal("unexpected PostgreSQL constraint kind")),
            };
            entries.push(
                CatalogEntry::relation_child(
                    id,
                    relation.clone(),
                    qualified_object(database, schema, &constraint.name),
                    "constraint",
                    OptionalMetadata::Supported(constraint.comment),
                    metadata,
                )
                .map_err(catalog_invariant)?,
            );
        }

        let rows = sqlx::query(
            "SELECT a.attnum::int AS ordinal_position, a.attname AS name, format_type(a.atttypid,a.atttypmod) AS native_type, \
             t.typcategory::text AS type_family, NOT a.attnotnull AS nullable, pg_get_expr(d.adbin,d.adrelid) AS expression, \
             a.attidentity::text AS identity_kind, a.attgenerated::text AS generated_kind, col_description(a.attrelid,a.attnum) AS comment, \
             CASE WHEN a.atttypmod >= 4 AND t.typname IN ('varchar','bpchar') THEN (a.atttypmod - 4)::bigint END AS character_length, \
             CASE WHEN a.attcollation <> 0 THEN coll_ns.nspname || '.' || coll.collname END AS collation \
             FROM pg_attribute a JOIN pg_type t ON t.oid=a.atttypid LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum \
             LEFT JOIN pg_collation coll ON coll.oid=a.attcollation LEFT JOIN pg_namespace coll_ns ON coll_ns.oid=coll.collnamespace \
             WHERE a.attrelid=$1::oid AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum"
        ).bind(relation_oid).fetch_all(&mut *connection).await.map_err(sql_error)?;
        for row in rows {
            let ordinal: i32 = row.try_get("ordinal_position").map_err(decode_error)?;
            let name: String = row.try_get("name").map_err(decode_error)?;
            let expression: Option<String> = row.try_get("expression").map_err(decode_error)?;
            let generated_kind: String = row.try_get("generated_kind").map_err(decode_error)?;
            let identity_kind: String = row.try_get("identity_kind").map_err(decode_error)?;
            let mut metadata = ColumnMetadata::new(
                u32::try_from(ordinal)
                    .map_err(|_| catalog_internal("invalid PostgreSQL column ordinal"))?,
                row.try_get::<String, _>("native_type")
                    .map_err(decode_error)?,
                row.try_get("nullable").map_err(decode_error)?,
            );
            metadata.type_family = OptionalMetadata::Supported(Some(
                row.try_get("type_family").map_err(decode_error)?,
            ));
            metadata.default_expression =
                OptionalMetadata::Supported(if generated_kind.is_empty() {
                    expression.clone()
                } else {
                    None
                });
            metadata.identity = OptionalMetadata::Supported(Some(!identity_kind.is_empty()));
            metadata.auto_increment = OptionalMetadata::Unsupported;
            metadata.generated_expression =
                OptionalMetadata::Supported(if generated_kind.is_empty() {
                    None
                } else {
                    expression
                });
            metadata.hidden = OptionalMetadata::Unsupported;
            metadata.numeric_precision = OptionalMetadata::Unsupported;
            metadata.numeric_scale = OptionalMetadata::Unsupported;
            metadata.character_maximum_length = OptionalMetadata::Supported(
                row.try_get::<Option<i64>, _>("character_length")
                    .map_err(decode_error)?
                    .map(u64::try_from)
                    .transpose()
                    .map_err(|_| catalog_internal("invalid PostgreSQL character length"))?,
            );
            metadata.collation =
                OptionalMetadata::Supported(row.try_get("collation").map_err(decode_error)?);
            metadata.character_set = OptionalMetadata::Unsupported;
            metadata.constraint_memberships = memberships.remove(&name).unwrap_or_default();
            metadata.constraint_memberships.sort_by(|left, right| {
                catalog_kind_rank(left.constraint_id.kind)
                    .cmp(&catalog_kind_rank(right.constraint_id.kind))
                    .then_with(|| left.ordinal_position.cmp(&right.ordinal_position))
                    .then_with(|| {
                        left.constraint_id
                            .native_path
                            .cmp(&right.constraint_id.native_path)
                    })
            });
            entries.push(
                CatalogEntry::relation_child(
                    relation_child_id(relation, CatalogKind::Column, &ordinal.to_string()),
                    relation.clone(),
                    qualified_object(database, schema, &name),
                    "column",
                    OptionalMetadata::Supported(row.try_get("comment").map_err(decode_error)?),
                    CatalogMetadata::Column(metadata),
                )
                .map_err(catalog_invariant)?,
            );
        }
        Ok(entries)
    }

    async fn verify_database(
        &self,
        connection: &mut PgConnection,
        database_id: &CatalogId,
        target: &CatalogTarget,
    ) -> Result<String, DatabaseError> {
        let current: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        if matches!(database_id.native_path.as_slice(), [database] if database == &current) {
            Ok(current)
        } else {
            Err(catalog_target_not_found(target))
        }
    }

    async fn verify_schema(
        &self,
        connection: &mut PgConnection,
        schema_id: &CatalogId,
        target: &CatalogTarget,
    ) -> Result<(String, String), DatabaseError> {
        let (database, schema) = match schema_id.native_path.as_slice() {
            [database, schema] => (database.as_str(), schema.as_str()),
            _ => return Err(catalog_target_not_found(target)),
        };
        let current: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        if database != current {
            return Err(catalog_target_not_found(target));
        }
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname=$1 AND nspname <> 'information_schema' AND nspname NOT LIKE 'pg\\_%' ESCAPE '\\')",
        )
        .bind(schema)
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
        if exists {
            Ok((current, schema.to_owned()))
        } else {
            Err(catalog_target_not_found(target))
        }
    }

    async fn verify_relation(
        &self,
        connection: &mut PgConnection,
        relation: &CatalogId,
        target: &CatalogTarget,
    ) -> Result<(String, String, String, i64, &'static str), DatabaseError> {
        if relation.profile_id() != self.connection_id || !relation.kind.is_relation() {
            return Err(catalog_target_not_found(target));
        }
        let (database, schema, name, oid) = match relation.native_path.as_slice() {
            [database, schema, name, oid] => (
                database.as_str(),
                schema.as_str(),
                name.as_str(),
                oid.parse::<i64>()
                    .map_err(|_| catalog_target_not_found(target))?,
            ),
            _ => return Err(catalog_target_not_found(target)),
        };
        let current: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        if current != database {
            return Err(catalog_target_not_found(target));
        }
        let native_kind = sqlx::query_scalar::<_, String>(
            "SELECT c.relkind::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.oid=$1::oid AND n.nspname=$2 AND c.relname=$3",
        )
        .bind(oid)
        .bind(schema)
        .bind(name)
        .fetch_optional(&mut *connection)
        .await
        .map_err(sql_error)?;
        let (expected, verified_native_kind) = match native_kind.as_deref() {
            Some("r" | "p") => (CatalogKind::Table, "table"),
            Some("v") => (CatalogKind::View, "view"),
            Some("m") => (CatalogKind::MaterializedView, "materialized_view"),
            _ => return Err(catalog_target_not_found(target)),
        };
        if relation.kind != expected {
            return Err(catalog_target_not_found(target));
        }
        Ok((
            current,
            schema.to_owned(),
            name.to_owned(),
            oid,
            verified_native_kind,
        ))
    }

    pub async fn preview_relation(
        &self,
        relation: &CatalogId,
        options: &crate::model::relation::RelationPreviewOptions,
        mut page: crate::model::pagination::PageRequest,
    ) -> Result<crate::db::RelationPreview, DatabaseError> {
        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        let target = CatalogTarget::RelationChildren {
            relation: relation.clone(),
        };
        let (_, schema, name, _, _) = self
            .verify_relation(&mut connection, relation, &target)
            .await?;
        let database: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        if !self.catalog_scope.allows_schema(&database, &schema) {
            return Err(catalog_target_not_found(&target));
        }
        let mut base_sql = format!(
            "SELECT * FROM {}.{}",
            quote_identifier(&schema),
            quote_identifier(&name)
        );
        append_preview_options(&mut base_sql, options);
        let total = if page.resolve_total {
            let count_sql = relation_count_sql(&base_sql);
            let count = sqlx::query_scalar::<_, i64>(AssertSqlSafe(count_sql))
                .fetch_one(&mut *connection)
                .await
                .map_err(sql_error)?;
            let total = u64::try_from(count).map_err(|_| {
                catalog_internal("PostgreSQL returned an invalid relation row count")
            })?;
            page.offset = crate::model::pagination::ResultPagination::last_offset(page.size, total);
            Some(total)
        } else {
            None
        };
        let sql = format!(
            "{base_sql} LIMIT {} OFFSET {}",
            page.size.lookahead_limit(),
            page.offset
        );
        let started = Instant::now();
        let statement = connection
            .prepare(AssertSqlSafe(sql.clone()).into_sql_str())
            .await
            .map_err(sql_error)?;
        let columns = statement
            .columns()
            .iter()
            .map(|column| ColumnMeta {
                name: column.name().to_owned(),
                type_name: column.type_info().name().to_owned(),
            })
            .collect();
        let mut rows = statement
            .query()
            .fetch_all(&mut *connection)
            .await
            .map_err(sql_error)?;
        let fetched_len = rows.len();
        rows.truncate(page.size.get());
        let result_set = ResultSet {
            columns,
            rows: rows.iter().map(decode_row).collect(),
            affected_rows: 0,
        };
        Ok(crate::db::RelationPreview {
            sql,
            result: QueryOutcome::from_result_set(result_set, started.elapsed(), Duration::ZERO),
            pagination: relation_pagination(page, fetched_len, total),
        })
    }

    pub async fn relation_ddl(&self, relation: &CatalogId) -> Result<RelationDdl, DatabaseError> {
        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        let mut transaction = connection.begin().await.map_err(sql_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *transaction)
            .await
            .map_err(sql_error)?;
        let result = self.relation_ddl_snapshot(&mut transaction, relation).await;
        let rollback = transaction.rollback().await.map_err(sql_error);
        if let Err(rollback_error) = rollback {
            return result.and(Err(rollback_error));
        }
        result
    }

    async fn relation_ddl_snapshot(
        &self,
        connection: &mut PgConnection,
        relation: &CatalogId,
    ) -> Result<RelationDdl, DatabaseError> {
        let target = CatalogTarget::RelationChildren {
            relation: relation.clone(),
        };
        let (database, schema, name, relation_oid, native_kind) =
            self.verify_relation(connection, relation, &target).await?;
        if !self.catalog_scope.allows_schema(&database, &schema) {
            return Err(catalog_target_not_found(&target));
        }
        let relation_entry = CatalogEntry::relation(
            relation.clone(),
            CatalogId::new(
                self.connection_id,
                CatalogKind::Schema,
                [database.clone(), schema.clone()],
            ),
            qualified_object(&database, &schema, &name),
            native_kind,
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
        let children = self
            .load_relation_children_page(connection, &request, relation)
            .await?;
        let ddl = self
            .load_relation_ddl_parts(connection, relation_oid, &schema, &name)
            .await?;
        let sql = assemble_relation_ddl(ddl)?;
        Ok(RelationDdl {
            relation: relation_entry,
            children,
            sql,
            provenance: DdlProvenance::AdapterGenerated,
        })
    }

    async fn load_relation_ddl_parts(
        &self,
        connection: &mut PgConnection,
        relation_oid: i64,
        schema: &str,
        name: &str,
    ) -> Result<PgDdlRelation, DatabaseError> {
        let relation_row = sqlx::query(
            "SELECT c.relkind::text AS relation_kind, c.relpersistence::text AS persistence, \
             c.relispopulated AS materialized_populated, obj_description(c.oid, 'pg_class') AS comment, \
             CASE WHEN c.relkind IN ('v','m') THEN pg_get_viewdef(c.oid, true) END AS view_definition, \
             CASE WHEN c.relkind='p' THEN pg_get_partkeydef(c.oid) END AS partition_key, \
             CASE WHEN c.relispartition THEN parent_ns.nspname END AS partition_parent_schema, \
             CASE WHEN c.relispartition THEN parent.relname END AS partition_parent_name, \
             CASE WHEN c.relispartition THEN pg_get_expr(c.relpartbound, c.oid, true) END AS partition_bound \
             FROM pg_class c \
             LEFT JOIN pg_inherits inh ON inh.inhrelid=c.oid \
             LEFT JOIN pg_class parent ON parent.oid=inh.inhparent \
             LEFT JOIN pg_namespace parent_ns ON parent_ns.oid=parent.relnamespace \
             WHERE c.oid=$1::oid",
        )
        .bind(relation_oid)
        .fetch_optional(&mut *connection)
        .await
        .map_err(sql_error)?
        .ok_or_else(|| catalog_internal(format!("PostgreSQL relation {schema}.{name} has no main definition")))?;
        let relation_kind: String = relation_row
            .try_get("relation_kind")
            .map_err(decode_error)?;

        let column_rows = sqlx::query(
            "SELECT a.attname AS name, format_type(a.atttypid,a.atttypmod) AS native_type, \
             pg_get_expr(d.adbin,d.adrelid) AS default_expression, a.attnotnull AS not_null, \
             a.attidentity::text AS identity_kind, a.attgenerated::text AS generated_kind, \
             col_description(a.attrelid,a.attnum) AS comment \
             FROM pg_attribute a LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum \
             WHERE a.attrelid=$1::oid AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum",
        )
        .bind(relation_oid)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        let mut columns = Vec::with_capacity(column_rows.len());
        let mut column_comments = Vec::new();
        for row in column_rows {
            let column_name: String = row.try_get("name").map_err(decode_error)?;
            if let Some(comment) = row.try_get("comment").map_err(decode_error)? {
                column_comments.push((column_name.clone(), comment));
            }
            columns.push(PgDdlColumn {
                name: column_name,
                native_type: row.try_get("native_type").map_err(decode_error)?,
                default_expression: row.try_get("default_expression").map_err(decode_error)?,
                not_null: row.try_get("not_null").map_err(decode_error)?,
                identity_kind: row.try_get("identity_kind").map_err(decode_error)?,
                generated_kind: row.try_get("generated_kind").map_err(decode_error)?,
            });
        }

        let constraints = sqlx::query(
            "SELECT con.conname AS name, pg_get_constraintdef(con.oid, true) AS definition \
             FROM pg_constraint con WHERE con.conrelid=$1::oid \
               AND con.contype IN ('c','f','p','u','x') \
             ORDER BY con.conname COLLATE \"C\", con.oid",
        )
        .bind(relation_oid)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?
        .into_iter()
        .map(|row| {
            Ok((
                row.try_get("name").map_err(decode_error)?,
                row.try_get("definition").map_err(decode_error)?,
            ))
        })
        .collect::<Result<Vec<_>, DatabaseError>>()?;

        let indexes = sqlx::query(
            "SELECT ic.relname AS name, pg_get_indexdef(idx.indexrelid) AS definition \
             FROM pg_index idx JOIN pg_class ic ON ic.oid=idx.indexrelid \
             WHERE idx.indrelid=$1::oid AND idx.indisvalid AND idx.indisready \
               AND NOT EXISTS (SELECT 1 FROM pg_constraint con WHERE con.conindid=idx.indexrelid) \
             ORDER BY ic.relname COLLATE \"C\", idx.indexrelid",
        )
        .bind(relation_oid)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?
        .into_iter()
        .map(|row| {
            Ok((
                row.try_get("name").map_err(decode_error)?,
                row.try_get("definition").map_err(decode_error)?,
            ))
        })
        .collect::<Result<Vec<_>, DatabaseError>>()?;

        let triggers = sqlx::query(
            "SELECT t.tgname AS name, pg_get_triggerdef(t.oid, true) AS definition \
             FROM pg_trigger t WHERE t.tgrelid=$1::oid AND NOT t.tgisinternal \
             ORDER BY t.tgname COLLATE \"C\", t.oid",
        )
        .bind(relation_oid)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?
        .into_iter()
        .map(|row| {
            Ok((
                row.try_get("name").map_err(decode_error)?,
                row.try_get("definition").map_err(decode_error)?,
            ))
        })
        .collect::<Result<Vec<_>, DatabaseError>>()?;

        Ok(PgDdlRelation {
            schema: schema.to_owned(),
            name: name.to_owned(),
            relation_kind,
            persistence: relation_row.try_get("persistence").map_err(decode_error)?,
            view_definition: relation_row
                .try_get("view_definition")
                .map_err(decode_error)?,
            materialized_populated: relation_row
                .try_get("materialized_populated")
                .map_err(decode_error)?,
            partition_key: relation_row
                .try_get("partition_key")
                .map_err(decode_error)?,
            partition_parent: match (
                relation_row
                    .try_get("partition_parent_schema")
                    .map_err(decode_error)?,
                relation_row
                    .try_get("partition_parent_name")
                    .map_err(decode_error)?,
            ) {
                (Some(schema), Some(name)) => Some((schema, name)),
                (None, None) => None,
                _ => {
                    return Err(catalog_internal(
                        "PostgreSQL partition parent is incomplete",
                    ));
                }
            },
            partition_bound: relation_row
                .try_get("partition_bound")
                .map_err(decode_error)?,
            columns,
            constraints,
            relation_comment: relation_row.try_get("comment").map_err(decode_error)?,
            column_comments,
            indexes,
            triggers,
        })
    }

    async fn load_pg_indexes(
        &self,
        connection: &mut PgConnection,
        relation_oid: i64,
    ) -> Result<Vec<PgIndexInfo>, DatabaseError> {
        let rows = sqlx::query(
            "SELECT idx.indexrelid::bigint AS oid, ic.relname AS name, idx.indisunique AS is_unique, obj_description(idx.indexrelid, 'pg_class') AS comment, \
             ARRAY(SELECT pg_get_indexdef(idx.indexrelid, key.ordinality::int, true) FROM unnest(idx.indkey) WITH ORDINALITY key(attnum, ordinality) WHERE key.ordinality <= idx.indnkeyatts ORDER BY key.ordinality) AS columns \
             FROM pg_index idx JOIN pg_class ic ON ic.oid=idx.indexrelid WHERE idx.indrelid=$1::oid ORDER BY ic.relname COLLATE \"C\", idx.indexrelid",
        )
        .bind(relation_oid)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        rows.into_iter()
            .map(|row| {
                Ok(PgIndexInfo {
                    oid: row.try_get("oid").map_err(decode_error)?,
                    name: row.try_get("name").map_err(decode_error)?,
                    columns: row.try_get("columns").map_err(decode_error)?,
                    unique: row.try_get("is_unique").map_err(decode_error)?,
                    comment: row.try_get("comment").map_err(decode_error)?,
                })
            })
            .collect()
    }

    async fn load_pg_constraints(
        &self,
        connection: &mut PgConnection,
        relation_oid: i64,
    ) -> Result<Vec<PgConstraintInfo>, DatabaseError> {
        let rows = sqlx::query(
            "SELECT con.oid::bigint AS oid, con.conname AS name, con.contype::text AS constraint_type, obj_description(con.oid, 'pg_constraint') AS comment, \
             ARRAY(SELECT a.attname FROM unnest(con.conkey) WITH ORDINALITY key(attnum, ordinality) JOIN pg_attribute a ON a.attrelid=con.conrelid AND a.attnum=key.attnum ORDER BY key.ordinality) AS columns, \
             target_ns.nspname AS referenced_schema, target.relname AS referenced_relation, \
             ARRAY(SELECT a.attname FROM unnest(con.confkey) WITH ORDINALITY key(attnum, ordinality) JOIN pg_attribute a ON a.attrelid=con.confrelid AND a.attnum=key.attnum ORDER BY key.ordinality) AS referenced_columns, \
             CASE WHEN con.contype='c' THEN pg_get_constraintdef(con.oid, true) END AS check_expression \
             FROM pg_constraint con LEFT JOIN pg_class target ON target.oid=con.confrelid LEFT JOIN pg_namespace target_ns ON target_ns.oid=target.relnamespace \
             WHERE con.conrelid=$1::oid AND con.contype IN ('p','u','f','c') ORDER BY con.conname COLLATE \"C\", con.oid",
        )
        .bind(relation_oid)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        rows.into_iter()
            .map(|row| {
                let native: String = row.try_get("constraint_type").map_err(decode_error)?;
                let kind = match native.as_str() {
                    "p" => CatalogKind::PrimaryKey,
                    "u" => CatalogKind::UniqueConstraint,
                    "f" => CatalogKind::ForeignKey,
                    "c" => CatalogKind::CheckConstraint,
                    _ => return Err(catalog_internal("unexpected PostgreSQL constraint type")),
                };
                Ok(PgConstraintInfo {
                    oid: row.try_get("oid").map_err(decode_error)?,
                    name: row.try_get("name").map_err(decode_error)?,
                    kind,
                    columns: row.try_get("columns").map_err(decode_error)?,
                    referenced_schema: row.try_get("referenced_schema").map_err(decode_error)?,
                    referenced_relation: row
                        .try_get("referenced_relation")
                        .map_err(decode_error)?,
                    referenced_columns: row.try_get("referenced_columns").map_err(decode_error)?,
                    check_expression: row.try_get("check_expression").map_err(decode_error)?,
                    comment: row.try_get("comment").map_err(decode_error)?,
                })
            })
            .collect()
    }

    pub async fn object_ddl(
        &self,
        kind: CatalogKind,
        schema: &str,
        name: &str,
    ) -> Result<Option<String>, DatabaseError> {
        if kind != CatalogKind::View {
            return Ok(None);
        }
        sqlx::query_scalar::<_, String>(
            "SELECT 'CREATE OR REPLACE VIEW ' || quote_ident($1) || '.' || quote_ident($2) || \
             ' AS\n' || pg_get_viewdef((quote_ident($1) || '.' || quote_ident($2))::regclass, true)",
        )
        .bind(schema)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))
    }

    pub async fn close(self) {
        self.pool.close().await;
    }
}

fn assemble_relation_ddl(mut relation: PgDdlRelation) -> Result<String, DatabaseError> {
    relation
        .constraints
        .sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    relation
        .column_comments
        .sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    relation
        .indexes
        .sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    relation
        .triggers
        .sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));

    let qualified = format!(
        "{}.{}",
        quote_identifier(&relation.schema),
        quote_identifier(&relation.name)
    );
    let (main_label, main_sql, comment_kind) = match relation.relation_kind.as_str() {
        "r" if relation.partition_parent.is_some() => {
            let (parent_schema, parent_name) = relation.partition_parent.as_ref().unwrap();
            let bound = relation.partition_bound.as_deref().ok_or_else(|| {
                catalog_internal(format!(
                    "PostgreSQL partition {}.{} has no partition bound",
                    relation.schema, relation.name
                ))
            })?;
            (
                "Table",
                format!(
                    "CREATE TABLE {qualified} PARTITION OF {}.{} {bound}",
                    quote_identifier(parent_schema),
                    quote_identifier(parent_name)
                ),
                "TABLE",
            )
        }
        "r" | "p" => {
            let mut definitions = relation
                .columns
                .iter()
                .map(column_definition)
                .collect::<Result<Vec<_>, _>>()?;
            definitions.extend(relation.constraints.iter().map(|(name, definition)| {
                format!("CONSTRAINT {} {definition}", quote_identifier(name))
            }));
            let persistence = match relation.persistence.as_str() {
                "p" => "",
                "u" => "UNLOGGED ",
                "t" => "TEMPORARY ",
                value => {
                    return Err(catalog_internal(format!(
                        "unexpected PostgreSQL relation persistence {value}"
                    )));
                }
            };
            let mut statement = format!(
                "CREATE {persistence}TABLE {qualified} (\n{}\n)",
                definitions
                    .iter()
                    .map(|definition| format!("  {definition}"))
                    .collect::<Vec<_>>()
                    .join(",\n")
            );
            if relation.relation_kind == "p" {
                let partition_key = relation.partition_key.as_deref().ok_or_else(|| {
                    catalog_internal(format!(
                        "PostgreSQL partitioned table {}.{} has no partition key",
                        relation.schema, relation.name
                    ))
                })?;
                statement.push_str(" PARTITION BY ");
                statement.push_str(partition_key);
            }
            ("Table", statement, "TABLE")
        }
        "v" | "m" => {
            let definition = relation
                .view_definition
                .as_deref()
                .map(str::trim)
                .filter(|definition| !definition.is_empty())
                .ok_or_else(|| {
                    catalog_internal(format!(
                        "PostgreSQL relation {}.{} has no main view definition",
                        relation.schema, relation.name
                    ))
                })?;
            if relation.relation_kind == "v" {
                (
                    "View",
                    format!("CREATE VIEW {qualified} AS\n{definition}"),
                    "VIEW",
                )
            } else {
                let no_data = if relation.materialized_populated {
                    ""
                } else {
                    "\nWITH NO DATA"
                };
                (
                    "View",
                    format!("CREATE MATERIALIZED VIEW {qualified} AS\n{definition}{no_data}"),
                    "MATERIALIZED VIEW",
                )
            }
        }
        value => {
            return Err(catalog_internal(format!(
                "unsupported PostgreSQL relation kind {value} for DDL"
            )));
        }
    };

    let mut comments = Vec::new();
    if let Some(comment) = relation.relation_comment {
        comments.push(format!(
            "COMMENT ON {comment_kind} {qualified} IS {}",
            quote_literal(&comment)
        ));
    }
    comments.extend(
        relation
            .column_comments
            .into_iter()
            .map(|(column, comment)| {
                format!(
                    "COMMENT ON COLUMN {qualified}.{} IS {}",
                    quote_identifier(&column),
                    quote_literal(&comment)
                )
            }),
    );

    assemble_ddl(vec![
        DdlSection {
            label: main_label,
            statements: vec![main_sql],
        },
        DdlSection {
            label: "Comments",
            statements: comments,
        },
        DdlSection {
            label: "Indexes",
            statements: relation.indexes.into_iter().map(|(_, sql)| sql).collect(),
        },
        DdlSection {
            label: "Triggers",
            statements: relation.triggers.into_iter().map(|(_, sql)| sql).collect(),
        },
    ])
    .ok_or_else(|| catalog_internal("PostgreSQL relation DDL assembly produced no statements"))
}

fn column_definition(column: &PgDdlColumn) -> Result<String, DatabaseError> {
    let mut definition = format!("{} {}", quote_identifier(&column.name), column.native_type);
    match (
        column.identity_kind.as_str(),
        column.generated_kind.as_str(),
    ) {
        ("a", "") => definition.push_str(" GENERATED ALWAYS AS IDENTITY"),
        ("d", "") => definition.push_str(" GENERATED BY DEFAULT AS IDENTITY"),
        ("", "s") => {
            let expression = column.default_expression.as_deref().ok_or_else(|| {
                catalog_internal(format!(
                    "PostgreSQL generated column {} has no expression",
                    column.name
                ))
            })?;
            definition.push_str(&format!(" GENERATED ALWAYS AS ({expression}) STORED"));
        }
        ("", "") => {
            if let Some(expression) = &column.default_expression {
                definition.push_str(" DEFAULT ");
                definition.push_str(expression);
            }
        }
        (identity, generated) => {
            return Err(catalog_internal(format!(
                "unsupported PostgreSQL identity/generated combination {identity:?}/{generated:?} for column {}",
                column.name
            )));
        }
    }
    if column.not_null {
        definition.push_str(" NOT NULL");
    }
    Ok(definition)
}

fn append_preview_options(
    sql: &mut String,
    options: &crate::model::relation::RelationPreviewOptions,
) {
    if let Some(clause) = &options.where_clause {
        sql.push_str(" WHERE ");
        sql.push_str(clause);
    }
    if let Some(clause) = &options.order_by_clause {
        sql.push_str(" ORDER BY ");
        sql.push_str(clause);
    }
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

fn relation_count_sql(sql: &str) -> String {
    format!("SELECT COUNT(*) FROM ({sql}) AS __lazydb_count")
}

pub(crate) struct PostgresTransactionBackend {
    connection: PoolConnection<Postgres>,
    control: PgPool,
    pid: i32,
    adapter: PostgresAdapter,
}

#[async_trait::async_trait]
impl TransactionBackend for PostgresTransactionBackend {
    async fn begin(&mut self) -> Result<(), TransactionError> {
        <Postgres as sqlx::Database>::TransactionManager::begin(&mut self.connection, None)
            .await
            .map_err(|error| TransactionError(error.to_string()))
    }
    async fn execute(&mut self, sql: &str) -> Result<QueryOutcome, TransactionError> {
        self.adapter
            .execute_connection(&mut self.connection, sql)
            .await
            .map_err(Into::into)
    }
    async fn relation_mutation(
        &mut self,
        request: RelationMutationRequest,
    ) -> Result<MutationResult, TransactionError> {
        let [_, schema, relation, _] = request.relation.native_path.as_slice() else {
            return Err(TransactionError(
                "PostgreSQL relation has no canonical database, schema, table, and oid path".into(),
            ));
        };
        let columns = &request.metadata.columns;
        let quoted_table = format!(
            "{}.{}",
            quote_identifier(schema),
            quote_identifier(relation)
        );
        match request.operation {
            RelationMutation::DeleteRows(rows) => {
                if columns.is_empty() {
                    return Err(TransactionError(
                        "PostgreSQL delete mutation has no relation columns".into(),
                    ));
                }
                for mutation in &rows {
                    if mutation.row.columns.len() != mutation.row.values.len()
                        || mutation.original.len() != columns.len()
                    {
                        return Err(TransactionError(
                            "PostgreSQL delete mutation is malformed".into(),
                        ));
                    }
                    if mutation.row.columns.is_empty() {
                        return Err(TransactionError(
                            "PostgreSQL delete mutation has no row locator".into(),
                        ));
                    }
                    let sql = postgres_delete_sql(&quoted_table, columns, &mutation.row.columns)?;
                    let mut query = sqlx::query(AssertSqlSafe(sql));
                    for value in &mutation.row.values {
                        query = bind_cell(query, value)?;
                    }
                    for value in &mutation.original {
                        query = bind_cell(query, value)?;
                    }
                    if query
                        .execute(&mut *self.connection)
                        .await
                        .map_err(|e| TransactionError(e.to_string()))?
                        .rows_affected()
                        != 1
                    {
                        return Err(TransactionError(
                            "PostgreSQL relation mutation conflict".into(),
                        ));
                    }
                }
                return Ok(MutationResult::Deleted { rows: rows.len() });
            }
            RelationMutation::InsertRow(insert) => {
                if insert.columns.len() != insert.values.len()
                    || insert.columns.iter().any(|i| *i >= columns.len())
                {
                    return Err(TransactionError(
                        "PostgreSQL insert mutation is malformed".into(),
                    ));
                }
                let mut supplied = Vec::new();
                let mut expressions = Vec::new();
                let mut bind_count = 1;
                for (index, value) in insert.columns.iter().zip(&insert.values) {
                    supplied.push(quote_identifier(&columns[*index].0));
                    if matches!(value, InputValue::Default) {
                        expressions.push("DEFAULT".into());
                    } else {
                        expressions.push(format!("${bind_count}"));
                        bind_count += 1;
                    }
                }
                let sql = if supplied.is_empty() {
                    format!("INSERT INTO {quoted_table} DEFAULT VALUES RETURNING *")
                } else {
                    format!(
                        "INSERT INTO {quoted_table} ({}) VALUES ({}) RETURNING *",
                        supplied.join(", "),
                        expressions.join(", ")
                    )
                };
                let mut query = sqlx::query(AssertSqlSafe(sql));
                for value in &insert.values {
                    match value {
                        InputValue::Default => {}
                        InputValue::Null => query = query.bind(Option::<String>::None),
                        InputValue::Value(value) => query = bind_cell(query, value)?,
                    }
                }
                let row = query
                    .fetch_one(&mut *self.connection)
                    .await
                    .map_err(|e| TransactionError(e.to_string()))?;
                return Ok(MutationResult::Inserted {
                    row: decode_row(&row),
                });
            }
            RelationMutation::UpdateCell(update) => {
                let Some((column_name, _, _)) = columns.get(update.column) else {
                    return Err(TransactionError(
                        "PostgreSQL update column is out of range".into(),
                    ));
                };
                if update.row.columns.len() != update.row.values.len() {
                    return Err(TransactionError(
                        "PostgreSQL row locator is malformed".into(),
                    ));
                }
                let primary_key_columns = request
                    .metadata
                    .primary_key
                    .iter()
                    .map(|name| {
                        columns
                            .iter()
                            .position(|(column, _, _)| column == name)
                            .ok_or_else(|| {
                                TransactionError("PostgreSQL primary key column is missing".into())
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if primary_key_columns != update.row.columns {
                    return Err(TransactionError(
                        "PostgreSQL row locator must contain the primary key columns in order"
                            .into(),
                    ));
                }
                if update
                    .row
                    .columns
                    .iter()
                    .any(|index| *index >= columns.len())
                {
                    return Err(TransactionError(
                        "PostgreSQL row locator column is out of range".into(),
                    ));
                }

                let quoted_column = quote_identifier(column_name);
                let mut sql = format!("UPDATE {quoted_table} SET {quoted_column} = ");
                let mut bind_count = 1;
                match update.value {
                    InputValue::Default => sql.push_str("DEFAULT"),
                    InputValue::Null | InputValue::Value(_) => {
                        sql.push_str(&format!("${bind_count}"));
                        bind_count += 1;
                    }
                }
                sql.push_str(" WHERE ");
                for (position, column_index) in update.row.columns.iter().enumerate() {
                    if position > 0 {
                        sql.push_str(" AND ");
                    }
                    let name = quote_identifier(&columns[*column_index].0);
                    sql.push_str(&format!(
                        "{name} IS NOT DISTINCT FROM {}",
                        postgres_placeholder(bind_count, &columns[*column_index].1)
                    ));
                    bind_count += 1;
                }
                if !update.row.columns.is_empty() {
                    sql.push_str(" AND ");
                }
                sql.push_str(&format!(
                    "{quoted_column} IS NOT DISTINCT FROM {}",
                    postgres_placeholder(bind_count, &columns[update.column].1)
                ));

                let mut query = sqlx::query(AssertSqlSafe(sql));
                match &update.value {
                    InputValue::Default => {}
                    InputValue::Null => query = query.bind(Option::<String>::None),
                    InputValue::Value(value) => query = bind_cell(query, value)?,
                }
                for value in &update.row.values {
                    query = bind_cell(query, value)?;
                }
                query = bind_cell(query, &update.original)?;
                let affected = query
                    .execute(&mut *self.connection)
                    .await
                    .map_err(|error| TransactionError(error.to_string()))?
                    .rows_affected();
                if affected != 1 {
                    return Err(TransactionError(
                        "PostgreSQL relation mutation conflict".into(),
                    ));
                }

                let mut select = format!("SELECT * FROM {quoted_table} WHERE ");
                for (position, column_index) in update.row.columns.iter().enumerate() {
                    if position > 0 {
                        select.push_str(" AND ");
                    }
                    select.push_str(&format!(
                        "{} IS NOT DISTINCT FROM {}",
                        quote_identifier(&columns[*column_index].0),
                        postgres_placeholder(position + 1, &columns[*column_index].1)
                    ));
                }
                let mut select_query = sqlx::query(AssertSqlSafe(select));
                for (column_index, value) in update.row.columns.iter().zip(&update.row.values) {
                    let value = if *column_index == update.column {
                        match &update.value {
                            InputValue::Value(value) => value,
                            InputValue::Null => &CellValue::Null,
                            InputValue::Default => {
                                return Err(TransactionError(
                                    "PostgreSQL cannot fetch an update that resets a primary key to DEFAULT"
                                        .into(),
                                ));
                            }
                        }
                    } else {
                        value
                    };
                    select_query = bind_cell(select_query, value)?;
                }
                let row = select_query
                    .fetch_optional(&mut *self.connection)
                    .await
                    .map_err(|error| TransactionError(error.to_string()))?
                    .ok_or_else(|| {
                        TransactionError("PostgreSQL relation mutation conflict".into())
                    })?;
                Ok(MutationResult::Updated {
                    row: decode_row(&row),
                })
            }
        }
    }

    async fn commit(&mut self) -> Result<(), TransactionError> {
        <Postgres as sqlx::Database>::TransactionManager::commit(&mut self.connection)
            .await
            .map_err(|error| TransactionError(error.to_string()))
    }
    async fn rollback(&mut self) -> Result<(), TransactionError> {
        <Postgres as sqlx::Database>::TransactionManager::rollback(&mut self.connection)
            .await
            .map_err(|error| TransactionError(error.to_string()))
    }
    async fn cancel(&mut self) -> Result<(), TransactionError> {
        let cancelled = sqlx::query_scalar::<_, bool>("SELECT pg_cancel_backend($1)")
            .bind(self.pid)
            .fetch_one(&self.control)
            .await
            .map_err(|error| TransactionError(error.to_string()))?;
        if cancelled {
            Ok(())
        } else {
            Err(TransactionError(
                "PostgreSQL backend refused cancellation".into(),
            ))
        }
    }
    fn depth(&self) -> usize {
        <Postgres as sqlx::Database>::TransactionManager::get_transaction_depth(&self.connection)
    }
    fn force_close(self) -> futures_util::future::BoxFuture<'static, Result<(), TransactionError>> {
        Box::pin(async move {
            // SQLx 0.9 has no close_hard on PoolConnection. Detach first, then close the raw
            // connection so a possibly dirty session can never be returned to this pool.
            let connection = self.connection.detach();
            connection
                .close()
                .await
                .map_err(|error| TransactionError(error.to_string()))
        })
    }
}

fn postgres_delete_sql(
    quoted_table: &str,
    columns: &[(String, String, bool)],
    locator_columns: &[usize],
) -> Result<String, TransactionError> {
    if columns.is_empty() {
        return Err(TransactionError(
            "PostgreSQL delete mutation has no relation columns".into(),
        ));
    }
    if locator_columns.is_empty() {
        return Err(TransactionError(
            "PostgreSQL delete mutation has no row locator".into(),
        ));
    }
    let mut predicates = Vec::with_capacity(locator_columns.len() + columns.len());
    for (position, index) in locator_columns.iter().enumerate() {
        if *index >= columns.len() {
            return Err(TransactionError(
                "PostgreSQL row locator column is out of range".into(),
            ));
        }
        let name = quote_identifier(&columns[*index].0);
        predicates.push(format!(
            "{name} IS NOT DISTINCT FROM {}",
            postgres_placeholder(position + 1, &columns[*index].1)
        ));
    }
    let original_offset = locator_columns.len();
    for (position, column) in columns.iter().enumerate() {
        predicates.push(format!(
            "{} IS NOT DISTINCT FROM {}",
            quote_identifier(&column.0),
            postgres_placeholder(original_offset + position + 1, &column.1)
        ));
    }
    Ok(format!(
        "DELETE FROM {quoted_table} WHERE {}",
        predicates.join(" AND ")
    ))
}

fn postgres_placeholder(index: usize, type_name: &str) -> String {
    let normalized = type_name.trim().to_ascii_lowercase();
    let base_type = normalized
        .split_once('(')
        .map_or(normalized.as_str(), |(base, _)| base.trim());
    let cast = match normalized.as_str() {
        "bool" | "boolean" => Some("boolean"),
        "int2" | "smallint" => Some("smallint"),
        "int4" | "integer" | "serial" => Some("integer"),
        "int8" | "bigint" | "bigserial" => Some("bigint"),
        "float4" | "real" => Some("real"),
        "float8" | "double precision" => Some("double precision"),
        "numeric" | "decimal" => Some("numeric"),
        "text" => Some("text"),
        "varchar" | "character varying" => Some("varchar"),
        "char" | "character" => Some("char"),
        "date" => Some("date"),
        "time" | "time without time zone" => Some("time"),
        "timetz" | "time with time zone" => Some("timetz"),
        "timestamp" | "timestamp without time zone" => Some("timestamp"),
        "timestamptz" | "timestamp with time zone" => Some("timestamptz"),
        "uuid" => Some("uuid"),
        "json" => Some("json"),
        "jsonb" => Some("jsonb"),
        "bytea" => Some("bytea"),
        _ => None,
    };
    let cast = cast.or(match base_type {
        "varchar" | "character varying" => Some("varchar"),
        "char" | "character" => Some("char"),
        _ => None,
    });
    cast.map_or_else(|| format!("${index}"), |cast| format!("${index}::{cast}"))
}

fn selected_schemas<'a>(request: &'a CatalogRequest, database: &str) -> Option<&'a Vec<String>> {
    match &request.scope.databases {
        CatalogSelection::All => None,
        CatalogSelection::Selected(databases) => databases
            .iter()
            .find(|selected| selected.name == database)
            .and_then(|selected| match &selected.schemas {
                CatalogSelection::All => None,
                CatalogSelection::Selected(schemas) => Some(schemas),
            }),
    }
}

fn search_schemas(scope: &CatalogScope, database: &str) -> Option<Vec<String>> {
    match &scope.databases {
        CatalogSelection::All => None,
        CatalogSelection::Selected(databases) => databases
            .iter()
            .find(|selected| selected.name == database)
            .map_or_else(
                || Some(Vec::new()),
                |selected| match &selected.schemas {
                    CatalogSelection::All => None,
                    CatalogSelection::Selected(schemas) => Some(schemas.clone()),
                },
            ),
    }
}

fn decode_search_candidate(row: PgRow) -> Result<PgSearchCandidate, DatabaseError> {
    let native_kind: String = row.try_get("kind").map_err(decode_error)?;
    let kind = match native_kind.as_str() {
        "database" => CatalogKind::Database,
        "schema" => CatalogKind::Schema,
        "table" => CatalogKind::Table,
        "view" => CatalogKind::View,
        "materialized_view" => CatalogKind::MaterializedView,
        "sequence" => CatalogKind::Sequence,
        "function" => CatalogKind::Function,
        "procedure" => CatalogKind::Procedure,
        "type" => CatalogKind::Type,
        "column" => CatalogKind::Column,
        "index" => CatalogKind::Index,
        "primary_key" => CatalogKind::PrimaryKey,
        "unique_constraint" => CatalogKind::UniqueConstraint,
        "foreign_key" => CatalogKind::ForeignKey,
        "check_constraint" => CatalogKind::CheckConstraint,
        _ => {
            return Err(catalog_internal(
                "unexpected PostgreSQL search catalog kind",
            ));
        }
    };
    Ok(PgSearchCandidate {
        kind,
        database: row.try_get("database_name").map_err(decode_error)?,
        schema: row.try_get("schema_name").map_err(decode_error)?,
        name: row.try_get("object_name").map_err(decode_error)?,
        oid: row.try_get("object_oid").map_err(decode_error)?,
        relation_kind: row.try_get("relation_kind").map_err(decode_error)?,
        relation_name: row.try_get("relation_name").map_err(decode_error)?,
        relation_oid: row.try_get("relation_oid").map_err(decode_error)?,
        comment: row.try_get("comment").map_err(decode_error)?,
        relation_comment: row.try_get("relation_comment").map_err(decode_error)?,
        database_comment: row.try_get("database_comment").map_err(decode_error)?,
        schema_comment: row.try_get("schema_comment").map_err(decode_error)?,
    })
}

fn candidate_relation_id(profile_id: Uuid, candidate: &PgSearchCandidate) -> Option<CatalogId> {
    if !candidate.kind.is_relation_child() {
        return None;
    }
    let schema = candidate.schema.as_ref()?;
    let name = candidate.relation_name.as_ref()?;
    let oid = candidate.relation_oid?;
    let (kind, _) = relation_kind(candidate.relation_kind.as_deref()?).ok()?;
    Some(CatalogId::new(
        profile_id,
        kind,
        [
            candidate.database.clone(),
            schema.clone(),
            name.clone(),
            oid.to_string(),
        ],
    ))
}

fn candidate_relation_entry(
    profile_id: Uuid,
    candidate: &PgSearchCandidate,
    schema_id: &CatalogId,
) -> Result<CatalogEntry, DatabaseError> {
    let id = candidate_relation_id(profile_id, candidate)
        .ok_or_else(|| catalog_internal("incomplete PostgreSQL search relation identity"))?;
    let schema = candidate
        .schema
        .as_deref()
        .ok_or_else(|| catalog_internal("PostgreSQL search relation has no schema"))?;
    let name = candidate
        .relation_name
        .as_deref()
        .ok_or_else(|| catalog_internal("PostgreSQL search relation has no name"))?;
    let (_, native_kind) = relation_kind(
        candidate
            .relation_kind
            .as_deref()
            .ok_or_else(|| catalog_internal("PostgreSQL search relation has no native kind"))?,
    )?;
    CatalogEntry::relation(
        id,
        schema_id.clone(),
        qualified_object(&candidate.database, schema, name),
        native_kind,
        OptionalMetadata::Supported(candidate.relation_comment.clone()),
        true,
    )
    .map_err(catalog_invariant)
}

fn candidate_child_id(
    relation: &CatalogId,
    candidate: &PgSearchCandidate,
) -> Result<CatalogId, DatabaseError> {
    let oid = candidate
        .oid
        .ok_or_else(|| catalog_internal("PostgreSQL search child has no native identity"))?;
    Ok(relation_child_id(
        relation,
        candidate.kind,
        &oid.to_string(),
    ))
}

fn relation_kind(native: &str) -> Result<(CatalogKind, &'static str), DatabaseError> {
    match native {
        "r" | "p" => Ok((CatalogKind::Table, "table")),
        "v" => Ok((CatalogKind::View, "view")),
        "m" => Ok((CatalogKind::MaterializedView, "materialized_view")),
        _ => Err(catalog_internal(
            "unexpected PostgreSQL search relation kind",
        )),
    }
}

fn search_kind_properties(kind: CatalogKind) -> Result<(&'static str, bool), DatabaseError> {
    match kind {
        CatalogKind::Table => Ok(("table", true)),
        CatalogKind::View => Ok(("view", true)),
        CatalogKind::MaterializedView => Ok(("materialized_view", true)),
        CatalogKind::Sequence => Ok(("sequence", false)),
        CatalogKind::Function => Ok(("function", false)),
        CatalogKind::Procedure => Ok(("procedure", false)),
        CatalogKind::Type => Ok(("type", false)),
        _ => Err(catalog_internal(
            "unexpected PostgreSQL top-level search kind",
        )),
    }
}

fn paginate_in_memory<T, SortKey, TieBreaker>(
    rows: &mut Vec<T>,
    request: &CatalogRequest,
    sort_key: SortKey,
    tie_breaker: TieBreaker,
) -> Result<Option<CatalogCursor>, DatabaseError>
where
    SortKey: Fn(&T) -> String,
    TieBreaker: Fn(&T) -> String,
{
    rows.sort_by(|left, right| {
        sort_key(left)
            .cmp(&sort_key(right))
            .then_with(|| tie_breaker(left).cmp(&tie_breaker(right)))
    });
    if let Some(cursor) = request.key.cursor.as_ref() {
        let (cursor_sort_key, cursor_tie_breaker) = cursor
            .keyset_parts()
            .map_err(DatabaseError::invalid_catalog_request)?;
        rows.retain(|row| {
            let row_sort_key = sort_key(row);
            let row_tie_breaker = tie_breaker(row);
            (row_sort_key.as_str(), row_tie_breaker.as_str())
                > (cursor_sort_key, cursor_tie_breaker)
        });
    }
    rows.truncate(request.page_size.saturating_add(1));
    finalize_keyset_page(rows, request.page_size, sort_key, tie_breaker).map_err(catalog_invariant)
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

fn relation_child_id(relation: &CatalogId, kind: CatalogKind, native_identity: &str) -> CatalogId {
    let mut path = relation.native_path.clone();
    path.push(native_identity.to_owned());
    CatalogId::new(relation.profile_id(), kind, path)
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
                ordinal_position: u32::try_from(index.saturating_add(1))
                    .map_err(|_| catalog_internal("PostgreSQL constraint has too many columns"))?,
            });
    }
    Ok(())
}

fn group_sort_key(group: ObjectGroup) -> &'static str {
    match group {
        ObjectGroup::Tables => "00:tables",
        ObjectGroup::Views => "01:views",
        ObjectGroup::MaterializedViews => "02:materialized_views",
        ObjectGroup::Sequences => "03:sequences",
        ObjectGroup::Functions => "04:functions",
        ObjectGroup::Procedures => "05:procedures",
        ObjectGroup::Types => "06:types",
        ObjectGroup::Triggers => "07:triggers",
    }
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

fn exact_count(count: usize) -> Result<CatalogCount, DatabaseError> {
    u64::try_from(count)
        .map(CatalogCount::Exact)
        .map_err(|_| catalog_internal("PostgreSQL catalog count exceeds u64"))
}

fn non_negative_count(count: i64) -> Result<u64, DatabaseError> {
    u64::try_from(count)
        .map_err(|_| catalog_internal("PostgreSQL returned a negative catalog count"))
}

fn page_limit(page_size: usize) -> Result<i64, DatabaseError> {
    page_size
        .checked_add(1)
        .and_then(|limit| i64::try_from(limit).ok())
        .ok_or_else(|| catalog_internal("PostgreSQL catalog page limit overflowed"))
}

fn catalog_target_not_found(target: &CatalogTarget) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Configuration,
        code: Some("catalog_target_not_found".to_owned()),
        message: sanitize_terminal_text(&format!(
            "PostgreSQL catalog target was not found: {}",
            target.description()
        )),
    }
}

fn catalog_invariant(error: CatalogValidationError) -> DatabaseError {
    catalog_internal(format!("PostgreSQL catalog invariant failed: {error}"))
}

fn catalog_internal(message: impl AsRef<str>) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Internal,
        code: Some("postgres_catalog_invariant".to_owned()),
        message: sanitize_terminal_text(message.as_ref()),
    }
}

fn sql_error(error: sqlx::Error) -> DatabaseError {
    DatabaseError::from_sqlx(error, ErrorCategory::Sql)
}

fn bind_cell<'q>(
    query: sqlx::query::Query<'q, Postgres, sqlx::postgres::PgArguments>,
    value: &CellValue,
) -> Result<sqlx::query::Query<'q, Postgres, sqlx::postgres::PgArguments>, TransactionError> {
    Ok(match value {
        CellValue::Null => query.bind(Option::<String>::None),
        CellValue::Boolean(value) => query.bind(*value),
        CellValue::Integer(value) => query.bind(*value),
        CellValue::Unsigned(value) => query.bind(i64::try_from(*value).map_err(|_| {
            TransactionError("PostgreSQL cannot bind an unsigned value larger than i64".into())
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
                "PostgreSQL cannot bind an unsupported cell value".into(),
            ));
        }
    })
}

pub const fn supports_server_version(server_version_num: i32) -> bool {
    server_version_num >= 120_000
}

pub fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn postgres_qualified_name(
    entry: &CatalogEntry,
    kind: CatalogKind,
) -> Result<String, CatalogDropError> {
    let name = &entry.qualified_name;
    if name.object.is_empty() {
        return Err(CatalogDropError::EmptyObjectName);
    }
    let schema = name
        .schema
        .as_deref()
        .ok_or_else(|| CatalogDropError::Unsupported {
            kind,
            reason: "catalog entry has no schema-qualified name".to_owned(),
        })?;
    if schema.is_empty() {
        return Err(CatalogDropError::Unsupported {
            kind,
            reason: "catalog entry has an empty schema name".to_owned(),
        });
    }
    Ok(format!(
        "{}.{}",
        quote_identifier(schema),
        quote_identifier(&name.object)
    ))
}

fn relation_name_for_drop(entry: &CatalogEntry) -> Result<String, CatalogDropError> {
    let relation = entry
        .relation_id
        .as_ref()
        .ok_or_else(|| CatalogDropError::Unsupported {
            kind: entry.kind,
            reason: "catalog entry has no owning relation identity".to_owned(),
        })?;
    if !relation.kind.is_relation() || relation.native_path.len() < 3 {
        return Err(CatalogDropError::Unsupported {
            kind: entry.kind,
            reason: "catalog entry has an invalid owning relation identity".to_owned(),
        });
    }
    let schema =
        entry
            .qualified_name
            .schema
            .as_deref()
            .ok_or_else(|| CatalogDropError::Unsupported {
                kind: entry.kind,
                reason: "catalog entry has no owning relation schema".to_owned(),
            })?;
    let relation_name = relation.native_path[2].as_str();
    if schema.is_empty() || relation_name.is_empty() {
        return Err(CatalogDropError::Unsupported {
            kind: entry.kind,
            reason: "catalog entry has an incomplete owning relation identity".to_owned(),
        });
    }
    Ok(format!(
        "{}.{}",
        quote_identifier(schema),
        quote_identifier(relation_name)
    ))
}

pub fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn pg_ssl_mode(mode: SslMode) -> PgSslMode {
    match mode {
        SslMode::Disable => PgSslMode::Disable,
        SslMode::Prefer => PgSslMode::Prefer,
        SslMode::Require => PgSslMode::Require,
        SslMode::VerifyCa => PgSslMode::VerifyCa,
        SslMode::VerifyFull => PgSslMode::VerifyFull,
    }
}

fn columns(row: &PgRow) -> Vec<ColumnMeta> {
    row.columns()
        .iter()
        .map(|column| ColumnMeta {
            name: column.name().to_owned(),
            type_name: column.type_info().name().to_owned(),
        })
        .collect()
}

fn decode_row(row: &PgRow) -> Vec<CellValue> {
    (0..row.len())
        .map(|index| decode_cell(row, index))
        .collect()
}

fn decode_cell(row: &PgRow, index: usize) -> CellValue {
    let Ok(raw) = row.try_get_raw(index) else {
        return unsupported("unknown", "decode error");
    };
    if raw.is_null() {
        return CellValue::Null;
    }
    let type_name = raw.type_info().name().to_ascii_uppercase();
    let decoded = match type_name.as_str() {
        "BOOL" => row.try_get::<bool, _>(index).map(CellValue::Boolean),
        "INT2" => row
            .try_get::<i16, _>(index)
            .map(|value| CellValue::Integer(i64::from(value))),
        "INT4" => row
            .try_get::<i32, _>(index)
            .map(|value| CellValue::Integer(i64::from(value))),
        "INT8" => row.try_get::<i64, _>(index).map(CellValue::Integer),
        "FLOAT4" => row
            .try_get::<f32, _>(index)
            .map(|value| CellValue::Float(f64::from(value))),
        "FLOAT8" => row.try_get::<f64, _>(index).map(CellValue::Float),
        "DATE" => row.try_get::<NaiveDate, _>(index).map(CellValue::Date),
        "TIME" => row.try_get::<NaiveTime, _>(index).map(CellValue::Time),
        "TIMESTAMP" => row
            .try_get::<NaiveDateTime, _>(index)
            .map(CellValue::DateTime),
        "TIMESTAMPTZ" => row
            .try_get::<DateTime<Utc>, _>(index)
            .map(|value| CellValue::Timestamp(value.fixed_offset())),
        "BYTEA" => row.try_get::<Vec<u8>, _>(index).map(CellValue::Bytes),
        "TEXT" | "VARCHAR" | "CHAR" | "NAME" | "UNKNOWN" => {
            row.try_get::<String, _>(index).map(CellValue::Text)
        }
        _ => return fallback_pg(row, index, &type_name),
    };
    decoded.unwrap_or_else(|error| unsupported(&type_name, &error.to_string()))
}

fn fallback_pg(row: &PgRow, index: usize, type_name: &str) -> CellValue {
    if let Ok(value) = row.try_get_unchecked::<String, _>(index) {
        CellValue::Text(value)
    } else if let Ok(value) = row.try_get_unchecked::<Vec<u8>, _>(index) {
        CellValue::Bytes(value)
    } else {
        unsupported(type_name, "unsupported PostgreSQL value")
    }
}

fn unsupported(type_name: &str, preview: &str) -> CellValue {
    CellValue::Unsupported {
        type_name: type_name.to_owned(),
        preview: preview.to_owned(),
    }
}

fn decode_error(error: sqlx::Error) -> DatabaseError {
    DatabaseError::from_sqlx(error, ErrorCategory::Internal)
}

fn monitor_timestamp(row: &PgRow, name: &str) -> Result<u64, DatabaseError> {
    let value = row.try_get::<i64, _>(name).map_err(decode_error)?;
    u64::try_from(value).map_err(|_| DatabaseError {
        category: ErrorCategory::Internal,
        code: Some("postgres_monitor_decode".to_owned()),
        message: format!("PostgreSQL returned a negative monitoring timestamp: {name}"),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        PgDdlColumn, PgDdlRelation, assemble_relation_ddl, column_definition, postgres_delete_sql,
        quote_identifier, quote_literal,
    };

    #[test]
    fn ddl_quoting_escapes_postgres_identifiers_and_literals() {
        assert_eq!(quote_identifier("odd\"name"), "\"odd\"\"name\"");
        assert_eq!(quote_literal("owner's note"), "'owner''s note'");
    }

    #[test]
    fn delete_sql_has_a_complete_where_clause_for_row_locators() {
        let columns = vec![
            ("id".to_owned(), "integer".to_owned(), false),
            ("name".to_owned(), "text".to_owned(), true),
        ];
        let sql = postgres_delete_sql("\"public\".\"users\"", &columns, &[0]).unwrap();

        assert_eq!(
            sql,
            "DELETE FROM \"public\".\"users\" WHERE \"id\" IS NOT DISTINCT FROM $1::integer AND \"id\" IS NOT DISTINCT FROM $2::integer AND \"name\" IS NOT DISTINCT FROM $3::text"
        );
        assert!(postgres_delete_sql("\"public\".\"users\"", &columns, &[]).is_err());
    }

    #[test]
    fn delete_sql_casts_nullable_bigint_parameters() {
        let columns = vec![
            ("id".to_owned(), "bigint".to_owned(), false),
            ("dept_id".to_owned(), "bigint".to_owned(), true),
            (
                "manager".to_owned(),
                "character varying(30)".to_owned(),
                true,
            ),
        ];
        let sql = postgres_delete_sql("\"tools\".\"sys_user\"", &columns, &[0]).unwrap();

        assert!(sql.contains("\"id\" IS NOT DISTINCT FROM $1::bigint"));
        assert!(sql.contains("\"dept_id\" IS NOT DISTINCT FROM $3::bigint"));
        assert!(sql.contains("\"manager\" IS NOT DISTINCT FROM $4"));
    }

    #[test]
    fn column_definition_uses_exactly_one_generated_identity_or_default_clause() {
        let base = PgDdlColumn {
            name: "id".to_owned(),
            native_type: "bigint".to_owned(),
            default_expression: Some("nextval('ignored')".to_owned()),
            not_null: true,
            identity_kind: "d".to_owned(),
            generated_kind: String::new(),
        };
        assert_eq!(
            column_definition(&base).unwrap(),
            "\"id\" bigint GENERATED BY DEFAULT AS IDENTITY NOT NULL"
        );

        let generated = PgDdlColumn {
            name: "slug".to_owned(),
            native_type: "text".to_owned(),
            default_expression: Some("lower(name)".to_owned()),
            not_null: false,
            identity_kind: String::new(),
            generated_kind: "s".to_owned(),
        };
        assert_eq!(
            column_definition(&generated).unwrap(),
            "\"slug\" text GENERATED ALWAYS AS (lower(name)) STORED"
        );

        let defaulted = PgDdlColumn {
            name: "active".to_owned(),
            native_type: "boolean".to_owned(),
            default_expression: Some("true".to_owned()),
            not_null: false,
            identity_kind: String::new(),
            generated_kind: String::new(),
        };
        assert_eq!(
            column_definition(&defaulted).unwrap(),
            "\"active\" boolean DEFAULT true"
        );
    }

    #[test]
    fn assembles_table_constraints_comments_indexes_and_triggers_in_sections() {
        let ddl = assemble_relation_ddl(PgDdlRelation {
            schema: "odd schema".to_owned(),
            name: "accounts".to_owned(),
            relation_kind: "r".to_owned(),
            persistence: "p".to_owned(),
            view_definition: None,
            materialized_populated: true,
            partition_key: None,
            partition_parent: None,
            partition_bound: None,
            columns: vec![PgDdlColumn {
                name: "id".to_owned(),
                native_type: "integer".to_owned(),
                default_expression: None,
                not_null: true,
                identity_kind: String::new(),
                generated_kind: String::new(),
            }],
            constraints: vec![("accounts_pk".to_owned(), "PRIMARY KEY (id)".to_owned())],
            relation_comment: Some("owner's table".to_owned()),
            column_comments: vec![("id".to_owned(), "primary key".to_owned())],
            indexes: vec![("accounts_live_idx".to_owned(), "CREATE INDEX accounts_live_idx ON \"odd schema\".accounts (id)".to_owned())],
            triggers: vec![("audit".to_owned(), "CREATE TRIGGER audit AFTER UPDATE ON \"odd schema\".accounts EXECUTE FUNCTION audit_row()".to_owned())],
        })
        .unwrap();

        assert!(ddl.contains("-- Table\n\nCREATE TABLE \"odd schema\".\"accounts\" (\n  \"id\" integer NOT NULL,\n  CONSTRAINT \"accounts_pk\" PRIMARY KEY (id)\n);"));
        assert!(ddl.contains(
            "-- Comments\n\nCOMMENT ON TABLE \"odd schema\".\"accounts\" IS 'owner''s table';"
        ));
        assert!(
            ddl.contains("COMMENT ON COLUMN \"odd schema\".\"accounts\".\"id\" IS 'primary key';")
        );
        assert!(ddl.contains("-- Indexes\n\nCREATE INDEX accounts_live_idx"));
        assert!(ddl.contains("-- Triggers\n\nCREATE TRIGGER audit"));
    }

    #[test]
    fn assembles_view_and_omits_empty_optional_sections() {
        let ddl = assemble_relation_ddl(PgDdlRelation {
            schema: "public".to_owned(),
            name: "active_accounts".to_owned(),
            relation_kind: "v".to_owned(),
            persistence: "p".to_owned(),
            view_definition: Some(" SELECT id FROM accounts ".to_owned()),
            materialized_populated: true,
            partition_key: None,
            partition_parent: None,
            partition_bound: None,
            columns: vec![],
            constraints: vec![],
            relation_comment: None,
            column_comments: vec![],
            indexes: vec![],
            triggers: vec![],
        })
        .unwrap();

        assert_eq!(
            ddl,
            "-- View\n\nCREATE VIEW \"public\".\"active_accounts\" AS\nSELECT id FROM accounts;"
        );
        assert!(!ddl.contains("-- Comments"));
        assert!(!ddl.contains("-- Indexes"));
        assert!(!ddl.contains("-- Triggers"));
    }

    #[test]
    fn rejects_a_missing_main_view_definition() {
        let error = assemble_relation_ddl(PgDdlRelation {
            schema: "public".to_owned(),
            name: "missing_definition".to_owned(),
            relation_kind: "v".to_owned(),
            persistence: "p".to_owned(),
            view_definition: Some("  ".to_owned()),
            materialized_populated: true,
            partition_key: None,
            partition_parent: None,
            partition_bound: None,
            columns: vec![],
            constraints: vec![],
            relation_comment: None,
            column_comments: vec![],
            indexes: vec![],
            triggers: vec![],
        })
        .unwrap_err();

        assert!(error.message.contains("has no main view definition"));
    }
}
