use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use futures_util::TryStreamExt;
use sqlx::{
    AssertSqlSafe, Column, Connection, Either, Executor, Row, SqlSafeStr, Sqlite, SqlitePool,
    Statement, TypeInfo, ValueRef,
    pool::PoolConnection,
    query::Query,
    sqlite::SqliteArguments,
    sqlite::{SqliteConnectOptions, SqliteConnection, SqlitePoolOptions, SqliteRow},
};
use sqlx_core::transaction::TransactionManager;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

use crate::identity::ConnectionIdentity;
use crate::profile::{CatalogScope, ConnectionProfile, DatabaseKind};
use crate::security::sanitize_terminal_text;

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
    ddl::{DdlSection, assemble_ddl},
    mutation::{InputValue, MutationResult, RelationMutation, RelationMutationRequest},
    query::{ColumnMeta, QueryOutcome, QueryOutcomeAccumulator, RELATION_PREVIEW_LIMIT, ResultSet},
    value::CellValue,
};

#[derive(Clone, Debug)]
pub struct SqliteAdapter {
    pool: SqlitePool,
    operation_gate: Arc<Semaphore>,
    connection_id: Uuid,
    database: String,
    catalog_scope: CatalogScope,
    #[cfg(test)]
    page_snapshot_hook: Arc<std::sync::Mutex<Option<PageSnapshotHook>>>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
struct PageSnapshotHook {
    count_complete: Arc<tokio::sync::Barrier>,
    continue_page: Arc<tokio::sync::Barrier>,
}

struct SqliteColumnInfo {
    name: String,
    ordinal_position: u32,
    native_type: String,
    nullable: bool,
    default_expression: Option<String>,
    primary_key_position: u32,
    hidden: bool,
}

struct SqliteIndexInfo {
    name: String,
    columns: Vec<String>,
    unique: bool,
    origin: String,
}

struct SqliteForeignKeyInfo {
    id: i64,
    referenced_relation: String,
    columns: Vec<String>,
    referenced_columns: Vec<String>,
}

struct SqliteForeignKeyBuilder {
    referenced_relation: String,
    columns: Vec<String>,
    referenced_columns: Vec<Option<String>>,
}

impl SqliteAdapter {
    pub fn catalog_capabilities() -> CatalogCapabilities {
        CatalogCapabilities {
            namespace_model: NamespaceModel::DatabaseAndSchema,
            top_level_groups: vec![
                ObjectGroup::Tables,
                ObjectGroup::Views,
                ObjectGroup::Triggers,
            ],
            column_metadata: ColumnMetadataCapabilities {
                default_expression: true,
                hidden: true,
                ..ColumnMetadataCapabilities::default()
            },
            supports_lazy_children: true,
        }
    }

    async fn acquire_operation(&self) -> Result<OwnedSemaphorePermit, DatabaseError> {
        self.operation_gate
            .clone()
            .acquire_owned()
            .await
            .map_err(|error| DatabaseError {
                category: ErrorCategory::Internal,
                code: Some("sqlite_operation_gate_closed".to_owned()),
                message: sanitize_terminal_text(&error.to_string()),
            })
    }

    pub(crate) async fn transaction_backend(
        &self,
    ) -> Result<SqliteTransactionBackend, DatabaseError> {
        let operation_permit = self.acquire_operation().await?;
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancelled);
        connection
            .lock_handle()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Internal))?
            .set_progress_handler(1000, move || !flag.load(Ordering::Relaxed));
        Ok(SqliteTransactionBackend {
            connection,
            cancelled,
            adapter: self.clone(),
            _operation_permit: operation_permit,
        })
    }

    pub async fn connect(profile: &ConnectionProfile) -> Result<Self, DatabaseError> {
        if profile.kind != DatabaseKind::Sqlite {
            return Err(DatabaseError::configuration("profile is not SQLite"));
        }

        let in_memory = profile.database.as_deref() == Some(":memory:");
        let mut options = SqliteConnectOptions::new()
            .foreign_keys(true)
            .read_only(profile.read_only)
            .create_if_missing(!profile.read_only);
        if in_memory {
            options = options.in_memory(true);
        } else if let Some(path) = &profile.sqlite_path {
            options = options.filename(path);
        } else {
            return Err(DatabaseError::configuration(
                "SQLite profile has no database path",
            ));
        }

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_with(options)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;

        Ok(Self {
            pool,
            operation_gate: Arc::new(Semaphore::new(1)),
            connection_id: profile.id,
            database: profile
                .database
                .clone()
                .unwrap_or_else(|| ":memory:".to_owned()),
            catalog_scope: profile.catalog_scope.clone(),
            #[cfg(test)]
            page_snapshot_hook: Arc::new(std::sync::Mutex::new(None)),
        })
    }

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        let _operation_permit = self.acquire_operation().await?;
        let version = sqlx::query_scalar::<_, String>("SELECT sqlite_version()")
            .fetch_one(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        Ok(ServerInfo {
            kind: DatabaseKind::Sqlite,
            version,
            database: self.database.clone(),
        })
    }

    pub async fn discover_catalog_scope(&self) -> Result<CatalogDiscovery, DatabaseError> {
        let _operation_permit = self.acquire_operation().await?;
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        let aliases = self.database_aliases(&mut connection).await?;

        Ok(CatalogDiscovery {
            databases: vec![DiscoveredDatabase {
                name: self.database.clone(),
                schemas: aliases,
            }],
            warnings: Vec::new(),
        })
    }

    pub async fn execute(&self, sql: &str) -> Result<QueryOutcome, DatabaseError> {
        self.execute_pool(sql).await
    }

    pub async fn preview_relation(
        &self,
        relation: &CatalogId,
        options: &crate::model::relation::RelationPreviewOptions,
        mut page: crate::model::pagination::PageRequest,
    ) -> Result<crate::db::RelationPreview, DatabaseError> {
        let _permit = self.acquire_operation().await?;
        let target = CatalogTarget::RelationChildren {
            relation: relation.clone(),
        };
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|e| DatabaseError::from_sqlx(e, ErrorCategory::Network))?;
        let (schema, name, _) = self
            .verified_relation_id(&mut connection, relation, &target)
            .await?;
        if !self.catalog_scope.allows_schema(&self.database, &schema) {
            return Err(catalog_target_not_found(&target));
        }
        let mut base_sql = format!(
            "SELECT * FROM {}.{}",
            self.quote_identifier(&schema),
            self.quote_identifier(&name)
        );
        append_preview_options(&mut base_sql, options);
        let total = if page.resolve_total {
            let count_sql = relation_count_sql(&base_sql);
            let count = sqlx::query_scalar::<_, i64>(AssertSqlSafe(count_sql))
                .fetch_one(&mut *connection)
                .await
                .map_err(|e| DatabaseError::from_sqlx(e, ErrorCategory::Sql))?;
            let total = u64::try_from(count).map_err(|_| {
                DatabaseError::configuration("SQLite returned an invalid relation row count")
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
            .map_err(|e| DatabaseError::from_sqlx(e, ErrorCategory::Sql))?;
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
            .map_err(|e| DatabaseError::from_sqlx(e, ErrorCategory::Sql))?;
        let fetched_len = rows.len();
        rows.truncate(page.size.get());
        let result_set = ResultSet {
            columns,
            rows: rows.iter().map(decode_row).collect(),
            affected_rows: 0,
        };
        let execution = started.elapsed();
        Ok(crate::db::RelationPreview {
            sql,
            result: QueryOutcome::from_result_set(result_set, execution, Duration::ZERO),
            pagination: relation_pagination(page, fetched_len, total),
        })
    }

    pub async fn relation_ddl(&self, relation: &CatalogId) -> Result<RelationDdl, DatabaseError> {
        let _permit = self.acquire_operation().await?;
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        <Sqlite as sqlx::Database>::TransactionManager::begin(&mut connection, None)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;

        let result = self.relation_ddl_snapshot(&mut connection, relation).await;
        let rollback = <Sqlite as sqlx::Database>::TransactionManager::rollback(&mut connection)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql));
        if let Err(error) = rollback {
            let connection = connection.detach();
            let _ = connection.close().await;
            return result.and(Err(error));
        }
        result
    }

    async fn relation_ddl_snapshot(
        &self,
        connection: &mut SqliteConnection,
        relation: &CatalogId,
    ) -> Result<RelationDdl, DatabaseError> {
        let target = CatalogTarget::RelationChildren {
            relation: relation.clone(),
        };
        let (schema, name, native_kind) = self
            .verified_relation_id(connection, relation, &target)
            .await?;
        if !self.catalog_scope.allows_schema(&self.database, &schema) {
            return Err(catalog_target_not_found(&target));
        }
        let relation_entry = CatalogEntry::relation(
            relation.clone(),
            CatalogId::new(
                self.connection_id,
                CatalogKind::Schema,
                [self.database.clone(), schema.clone()],
            ),
            child_qualified_name(&self.database, &schema, &name),
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
        let quoted_schema = self.quote_identifier(&schema);
        let main_statement = format!(
            "SELECT sql FROM {quoted_schema}.sqlite_schema \
             WHERE type = ? AND name = ? COLLATE BINARY"
        );
        let main_sql = sqlx::query_scalar::<_, Option<String>>(AssertSqlSafe(main_statement))
            .bind(native_kind)
            .bind(&name)
            .fetch_optional(&mut *connection)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?
            .flatten()
            .filter(|sql| !sql.trim().is_empty())
            .ok_or_else(|| {
                catalog_internal(format!(
                    "SQLite {native_kind} {schema}.{name} has no catalog DDL"
                ))
            })?;
        let related_statement = format!(
            "SELECT sql FROM {quoted_schema}.sqlite_schema \
             WHERE type = ? AND tbl_name = ? COLLATE BINARY AND sql IS NOT NULL \
             ORDER BY name COLLATE BINARY"
        );
        let indexes = sqlx::query_scalar::<_, String>(AssertSqlSafe(related_statement.clone()))
            .bind("index")
            .bind(&name)
            .fetch_all(&mut *connection)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let triggers = sqlx::query_scalar::<_, String>(AssertSqlSafe(related_statement))
            .bind("trigger")
            .bind(&name)
            .fetch_all(&mut *connection)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let has_related_objects = !indexes.is_empty() || !triggers.is_empty();
        let main_label = if relation.kind == CatalogKind::Table {
            "Table"
        } else {
            "View"
        };
        let sql = assemble_ddl(vec![
            DdlSection {
                label: main_label,
                statements: vec![main_sql],
            },
            DdlSection {
                label: "Indexes",
                statements: indexes,
            },
            DdlSection {
                label: "Triggers",
                statements: triggers,
            },
        ])
        .ok_or_else(|| catalog_internal("SQLite relation DDL assembly produced no statements"))?;
        let provenance = if has_related_objects {
            DdlProvenance::AdapterGenerated
        } else {
            DdlProvenance::NativeCatalog
        };
        Ok(RelationDdl {
            relation: relation_entry,
            children,
            sql,
            provenance,
        })
    }

    pub(crate) async fn execute_pool(&self, sql: &str) -> Result<QueryOutcome, DatabaseError> {
        let _operation_permit = self.acquire_operation().await?;
        let mut stream = sqlx::raw_sql(AssertSqlSafe(sql)).fetch_many(&self.pool);
        self.collect_stream(&mut stream).await
    }

    pub(crate) async fn execute_connection(
        &self,
        connection: &mut SqliteConnection,
        sql: &str,
    ) -> Result<QueryOutcome, DatabaseError> {
        let mut stream = sqlx::raw_sql(AssertSqlSafe(sql)).fetch_many(&mut *connection);
        self.collect_stream(&mut stream).await
    }

    async fn collect_stream<E>(&self, stream: &mut E) -> Result<QueryOutcome, DatabaseError>
    where
        E: futures_util::TryStream<
                Ok = Either<sqlx::sqlite::SqliteQueryResult, SqliteRow>,
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
        let _operation_permit = self.acquire_operation().await?;
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        <Sqlite as sqlx::Database>::TransactionManager::begin(&mut connection, None)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;

        let page = match &request.key.target {
            CatalogTarget::Databases => self.load_database_page(&mut connection, request),
            CatalogTarget::Schemas { database } => {
                self.load_schema_page(&mut connection, request, database)
                    .await
            }
            CatalogTarget::Groups { schema } => {
                self.load_group_page(&mut connection, request, schema).await
            }
            CatalogTarget::Objects { schema, group } => {
                self.load_object_page(&mut connection, request, schema, *group)
                    .await
            }
            CatalogTarget::RelationChildren { relation } => {
                self.load_relation_children_page(&mut connection, request, relation)
                    .await
            }
        };
        let rollback = <Sqlite as sqlx::Database>::TransactionManager::rollback(&mut connection)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql));
        if let Err(rollback_error) = rollback {
            let connection = connection.detach();
            let _ = connection.close().await;
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

        let _operation_permit = self.acquire_operation().await?;
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        <Sqlite as sqlx::Database>::TransactionManager::begin(&mut connection, None)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let result = self.search_catalog_snapshot(&mut connection, request).await;
        let rollback = <Sqlite as sqlx::Database>::TransactionManager::rollback(&mut connection)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql));
        if let Err(error) = rollback {
            let connection = connection.detach();
            let _ = connection.close().await;
            return result.and(Err(error));
        }
        result
    }

    async fn search_catalog_snapshot(
        &self,
        connection: &mut SqliteConnection,
        request: &CatalogSearchRequest,
    ) -> Result<CatalogSearchPage, DatabaseError> {
        let query = request.query.to_lowercase();
        let database = self.database_entry()?;
        let database_path = database.qualified_name.object.to_lowercase();
        let mut hits = Vec::new();
        if request.scope.allows_database(&self.database)
            && (database_path == query
                || database_path.starts_with(&query)
                || database_path.contains(&query))
        {
            hits.push(CatalogSearchHit {
                entry: database.clone(),
                ancestors: Vec::new(),
            });
        }

        if request.scope.allows_database(&self.database) {
            for schema_name in self.database_aliases(connection).await? {
                if !request.scope.allows_schema(&self.database, &schema_name) {
                    continue;
                }
                let schema = self.schema_entry(&database, &schema_name)?;
                let schema_path = format!("{}.{}", self.database, schema_name).to_lowercase();
                if schema_name.to_lowercase().contains(&query) || schema_path.contains(&query) {
                    hits.push(CatalogSearchHit {
                        entry: schema.clone(),
                        ancestors: vec![database.clone()],
                    });
                }
                self.search_schema(connection, &query, &database, &schema, &mut hits)
                    .await?;
            }
        }

        let mut ranked = hits
            .into_iter()
            .filter_map(|hit| search_rank(&hit, &query).map(|rank| (rank, hit)))
            .collect::<Vec<_>>();
        ranked.sort_by(|(left_rank, left), (right_rank, right)| {
            left_rank
                .cmp(right_rank)
                .then_with(|| {
                    left.qualified_path()
                        .to_lowercase()
                        .cmp(&right.qualified_path().to_lowercase())
                })
                .then_with(|| left.entry.id.native_path.cmp(&right.entry.id.native_path))
        });
        ranked.dedup_by(|left, right| left.1.entry.id == right.1.entry.id);
        ranked.truncate(request.limit.saturating_add(1));
        let truncated = ranked.len() > request.limit;
        let hits = ranked
            .into_iter()
            .take(request.limit)
            .map(|(_, hit)| hit)
            .collect();
        CatalogSearchPage::new(request, hits, None, truncated)
            .map_err(DatabaseError::invalid_catalog_request)
    }

    async fn search_schema(
        &self,
        connection: &mut SqliteConnection,
        query: &str,
        database: &CatalogEntry,
        schema: &CatalogEntry,
        hits: &mut Vec<CatalogSearchHit>,
    ) -> Result<(), DatabaseError> {
        let schema_name = &schema.qualified_name.object;
        let quoted_schema = self.quote_identifier(schema_name);
        let path_prefix = format!("{}.{}.", self.database, schema_name);
        let objects_sql = format!(
            "SELECT object.type, object.name, \
             (SELECT owner.type FROM {quoted_schema}.sqlite_schema AS owner \
                WHERE owner.type IN ('table', 'view') \
                  AND owner.name = object.tbl_name COLLATE NOCASE \
                ORDER BY (owner.type = 'table') DESC, owner.name COLLATE BINARY LIMIT 1 \
             ) AS owner_type, \
             (SELECT owner.name FROM {quoted_schema}.sqlite_schema AS owner \
                WHERE owner.type IN ('table', 'view') \
                  AND owner.name = object.tbl_name COLLATE NOCASE \
                ORDER BY (owner.type = 'table') DESC, owner.name COLLATE BINARY LIMIT 1 \
             ) AS owner_name \
             FROM {quoted_schema}.sqlite_schema AS object \
             WHERE object.type IN ('table', 'view', 'trigger') \
               AND object.name NOT GLOB 'sqlite_*' \
               AND (object.type <> 'trigger' OR EXISTS ( \
                   SELECT 1 FROM {quoted_schema}.sqlite_schema AS owner \
                   WHERE owner.type IN ('table', 'view') \
                     AND owner.name = object.tbl_name COLLATE NOCASE)) \
               AND (instr(lower(object.name), ?) > 0 \
                 OR instr(lower(CASE WHEN object.type = 'trigger' \
                    THEN ? || (SELECT owner.name FROM {quoted_schema}.sqlite_schema AS owner \
                         WHERE owner.type IN ('table', 'view') \
                           AND owner.name = object.tbl_name COLLATE NOCASE \
                         ORDER BY (owner.type = 'table') DESC, owner.name COLLATE BINARY LIMIT 1) \
                         || '.' || object.name \
                    ELSE ? || object.name END), ?) > 0) \
             ORDER BY object.name COLLATE BINARY"
        );
        let rows = sqlx::query(AssertSqlSafe(objects_sql))
            .bind(query)
            .bind(&path_prefix)
            .bind(&path_prefix)
            .bind(query)
            .fetch_all(&mut *connection)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        for row in rows {
            let entry = self.object_search_entry(schema, &row)?;
            let mut ancestors = vec![database.clone(), schema.clone()];
            if let Some(owner) = entry
                .relation_id
                .as_ref()
                .filter(|owner| *owner != &entry.id)
            {
                ancestors.push(self.relation_entry(schema, owner)?);
            }
            hits.push(CatalogSearchHit { entry, ancestors });
        }

        let owners_sql = format!(
            "SELECT relation.type, relation.name \
             FROM {quoted_schema}.sqlite_schema AS relation \
             WHERE relation.type IN ('table', 'view') \
               AND relation.name NOT GLOB 'sqlite_*' \
               AND (instr(lower(? || relation.name), ?) > 0 \
                 OR EXISTS (SELECT 1 FROM pragma_table_xinfo(relation.name, ?) AS column \
                    WHERE instr(lower(column.name), ?) > 0 \
                       OR instr(lower(? || relation.name || '.' || column.name), ?) > 0) \
                 OR (relation.type = 'table' AND EXISTS (\
                    SELECT 1 FROM pragma_index_list(relation.name, ?) AS idx \
                    WHERE instr(lower(idx.name), ?) > 0 \
                       OR instr(lower(? || relation.name || '.' || idx.name), ?) > 0)) \
                 OR (relation.type = 'table' AND EXISTS (\
                    SELECT 1 FROM pragma_table_xinfo(relation.name, ?) AS pk_column \
                    WHERE pk_column.pk > 0 \
                      AND NOT EXISTS (SELECT 1 FROM pragma_index_list(relation.name, ?) AS pk_index \
                                      WHERE pk_index.origin = 'pk') \
                      AND (instr('primary_key', ?) > 0 \
                        OR instr(lower(? || relation.name || '.primary_key'), ?) > 0))) \
                 OR (relation.type = 'table' AND EXISTS (\
                    SELECT 1 FROM pragma_foreign_key_list(relation.name, ?) AS fk \
                    WHERE instr(lower(printf('fk_%s_%d', relation.name, fk.id)), ?) > 0 \
                       OR instr(lower(? || relation.name || '.' || printf('fk_%s_%d', relation.name, fk.id)), ?) > 0))) \
             ORDER BY relation.name COLLATE BINARY"
        );
        let owner_rows = sqlx::query(AssertSqlSafe(owners_sql))
            .bind(&path_prefix)
            .bind(query)
            .bind(schema_name)
            .bind(query)
            .bind(&path_prefix)
            .bind(query)
            .bind(schema_name)
            .bind(query)
            .bind(&path_prefix)
            .bind(query)
            .bind(schema_name)
            .bind(schema_name)
            .bind(query)
            .bind(&path_prefix)
            .bind(query)
            .bind(schema_name)
            .bind(query)
            .bind(&path_prefix)
            .bind(query)
            .fetch_all(&mut *connection)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        for row in owner_rows {
            let native_type: String = row.try_get("type").map_err(decode_error)?;
            let relation_name: String = row.try_get("name").map_err(decode_error)?;
            let kind = relation_kind(&native_type)?;
            let relation_id = CatalogId::new(
                self.connection_id,
                kind,
                [
                    self.database.clone(),
                    schema_name.clone(),
                    relation_name.clone(),
                ],
            );
            let relation = self.relation_entry(schema, &relation_id)?;
            let children = self
                .load_relation_child_entries(connection, &relation_id, schema_name, &relation_name)
                .await?;
            for entry in children {
                hits.push(CatalogSearchHit {
                    entry,
                    ancestors: vec![database.clone(), schema.clone(), relation.clone()],
                });
            }
        }
        Ok(())
    }

    fn database_entry(&self) -> Result<CatalogEntry, DatabaseError> {
        CatalogEntry::database(
            CatalogId::new(
                self.connection_id,
                CatalogKind::Database,
                [self.database.clone()],
            ),
            QualifiedName {
                database: Some(self.database.clone()),
                schema: None,
                object: self.database.clone(),
            },
            "database",
            OptionalMetadata::Unsupported,
            true,
        )
        .map_err(catalog_invariant)
    }

    fn schema_entry(
        &self,
        database: &CatalogEntry,
        schema: &str,
    ) -> Result<CatalogEntry, DatabaseError> {
        CatalogEntry::schema(
            CatalogId::new(
                self.connection_id,
                CatalogKind::Schema,
                [self.database.clone(), schema.to_owned()],
            ),
            database.id.clone(),
            QualifiedName {
                database: Some(self.database.clone()),
                schema: Some(schema.to_owned()),
                object: schema.to_owned(),
            },
            "schema",
            OptionalMetadata::Unsupported,
            true,
        )
        .map_err(catalog_invariant)
    }

    fn relation_entry(
        &self,
        schema: &CatalogEntry,
        relation: &CatalogId,
    ) -> Result<CatalogEntry, DatabaseError> {
        let name = relation
            .native_path
            .last()
            .ok_or_else(|| catalog_internal("SQLite relation ID has no name"))?;
        CatalogEntry::relation(
            relation.clone(),
            schema.id.clone(),
            child_qualified_name(&self.database, &schema.qualified_name.object, name),
            match relation.kind {
                CatalogKind::Table => "table",
                CatalogKind::View => "view",
                _ => return Err(catalog_internal("unexpected SQLite relation kind")),
            },
            OptionalMetadata::Unsupported,
            true,
        )
        .map_err(catalog_invariant)
    }

    fn object_search_entry(
        &self,
        schema: &CatalogEntry,
        row: &SqliteRow,
    ) -> Result<CatalogEntry, DatabaseError> {
        let native_type: String = row.try_get("type").map_err(decode_error)?;
        let name: String = row.try_get("name").map_err(decode_error)?;
        let kind = match native_type.as_str() {
            "trigger" => CatalogKind::Trigger,
            _ => relation_kind(&native_type)?,
        };
        let id = CatalogId::new(
            self.connection_id,
            kind,
            [
                self.database.clone(),
                schema.qualified_name.object.clone(),
                name.clone(),
            ],
        );
        let qualified_name =
            child_qualified_name(&self.database, &schema.qualified_name.object, &name);
        if kind != CatalogKind::Trigger {
            return CatalogEntry::relation(
                id,
                schema.id.clone(),
                qualified_name,
                native_type,
                OptionalMetadata::Unsupported,
                true,
            )
            .map_err(catalog_invariant);
        }
        let owner_type: String = row.try_get("owner_type").map_err(decode_error)?;
        let owner_name: String = row.try_get("owner_name").map_err(decode_error)?;
        CatalogEntry::relation_object(
            id,
            schema.id.clone(),
            CatalogId::new(
                self.connection_id,
                relation_kind(&owner_type)?,
                [
                    self.database.clone(),
                    schema.qualified_name.object.clone(),
                    owner_name,
                ],
            ),
            qualified_name,
            native_type,
            OptionalMetadata::Unsupported,
        )
        .map_err(catalog_invariant)
    }

    fn load_database_page(
        &self,
        _connection: &mut SqliteConnection,
        request: &CatalogRequest,
    ) -> Result<CatalogPage, DatabaseError> {
        let mut entries = if request.scope.allows_database(&self.database) {
            vec![
                CatalogEntry::database(
                    CatalogId::new(
                        self.connection_id,
                        CatalogKind::Database,
                        [self.database.clone()],
                    ),
                    QualifiedName {
                        database: Some(self.database.clone()),
                        schema: None,
                        object: self.database.clone(),
                    },
                    "database",
                    OptionalMetadata::Unsupported,
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
        connection: &mut SqliteConnection,
        request: &CatalogRequest,
        database: &CatalogId,
    ) -> Result<CatalogPage, DatabaseError> {
        self.verify_database_id(database, &request.key.target)?;
        let database_id = CatalogId::new(
            self.connection_id,
            CatalogKind::Database,
            [self.database.clone()],
        );
        let mut entries = self
            .database_aliases(connection)
            .await?
            .into_iter()
            .filter(|alias| request.scope.allows_schema(&self.database, alias))
            .map(|alias| {
                CatalogEntry::schema(
                    CatalogId::new(
                        self.connection_id,
                        CatalogKind::Schema,
                        [self.database.clone(), alias.clone()],
                    ),
                    database_id.clone(),
                    QualifiedName {
                        database: Some(self.database.clone()),
                        schema: Some(alias.clone()),
                        object: alias,
                    },
                    "schema",
                    OptionalMetadata::Unsupported,
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
        connection: &mut SqliteConnection,
        request: &CatalogRequest,
        schema: &CatalogId,
    ) -> Result<CatalogPage, DatabaseError> {
        let schema = self
            .verified_schema_id(connection, schema, &request.key.target)
            .await?;
        let quoted_schema = self.quote_identifier(&schema);
        let statement = format!(
            "SELECT object.type, COUNT(*) AS object_count \
             FROM {quoted_schema}.sqlite_schema AS object \
             WHERE object.type IN ('table', 'view', 'trigger') \
               AND object.name NOT GLOB 'sqlite_*' \
               AND (object.type <> 'trigger' OR EXISTS ( \
                   SELECT 1 FROM {quoted_schema}.sqlite_schema AS owner \
                   WHERE owner.type IN ('table', 'view') \
                     AND owner.name = object.tbl_name COLLATE NOCASE \
               )) \
             GROUP BY object.type"
        );
        let rows = sqlx::query(AssertSqlSafe(statement))
            .fetch_all(&mut *connection)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let mut counts = HashMap::new();
        for row in rows {
            let native_type: String = row.try_get("type").map_err(decode_error)?;
            let count: i64 = row.try_get("object_count").map_err(decode_error)?;
            counts.insert(native_type, non_negative_count(count)?);
        }

        let mut summaries = vec![
            CatalogGroupSummary {
                group: ObjectGroup::Tables,
                object_count: CatalogCount::Exact(*counts.get("table").unwrap_or(&0)),
            },
            CatalogGroupSummary {
                group: ObjectGroup::Views,
                object_count: CatalogCount::Exact(*counts.get("view").unwrap_or(&0)),
            },
            CatalogGroupSummary {
                group: ObjectGroup::Triggers,
                object_count: CatalogCount::Exact(*counts.get("trigger").unwrap_or(&0)),
            },
        ];
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
        connection: &mut SqliteConnection,
        request: &CatalogRequest,
        schema: &CatalogId,
        group: ObjectGroup,
    ) -> Result<CatalogPage, DatabaseError> {
        let native_type = match group {
            ObjectGroup::Tables => "table",
            ObjectGroup::Views => "view",
            ObjectGroup::Triggers => "trigger",
            _ => {
                return Err(DatabaseError::unsupported_catalog_target(
                    DatabaseKind::Sqlite,
                    &request.key.target,
                ));
            }
        };
        let schema_name = self
            .verified_schema_id(connection, schema, &request.key.target)
            .await?;
        let quoted_schema = self.quote_identifier(&schema_name);
        let count_statement = format!(
            "SELECT COUNT(*) AS object_count \
             FROM {quoted_schema}.sqlite_schema AS object \
             WHERE object.type = ? AND object.name NOT GLOB 'sqlite_*' \
               AND (object.type <> 'trigger' OR EXISTS ( \
                   SELECT 1 FROM {quoted_schema}.sqlite_schema AS owner \
                   WHERE owner.type IN ('table', 'view') \
                     AND owner.name = object.tbl_name COLLATE NOCASE \
               ))"
        );
        let count_row = sqlx::query(AssertSqlSafe(count_statement))
            .bind(native_type)
            .fetch_one(&mut *connection)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let total_count = CatalogCount::Exact(non_negative_count(
            count_row.try_get("object_count").map_err(decode_error)?,
        )?);
        #[cfg(test)]
        self.pause_after_page_count().await;
        let limit = page_limit(request.page_size)?;
        let cursor = request
            .key
            .cursor
            .as_ref()
            .map(|cursor| cursor.keyset_parts())
            .transpose()
            .map_err(DatabaseError::invalid_catalog_request)?;
        let select = format!(
            "SELECT object.type, object.name, \
             (SELECT owner.type FROM {quoted_schema}.sqlite_schema AS owner \
               WHERE owner.type IN ('table', 'view') \
                 AND owner.name = object.tbl_name COLLATE NOCASE \
               ORDER BY (owner.type = 'table') DESC, owner.name COLLATE BINARY LIMIT 1 \
             ) AS owner_type, \
             (SELECT owner.name FROM {quoted_schema}.sqlite_schema AS owner \
               WHERE owner.type IN ('table', 'view') \
                 AND owner.name = object.tbl_name COLLATE NOCASE \
               ORDER BY (owner.type = 'table') DESC, owner.name COLLATE BINARY LIMIT 1 \
             ) AS owner_name \
             FROM {quoted_schema}.sqlite_schema AS object \
             WHERE object.type = ? AND object.name NOT GLOB 'sqlite_*' \
               AND (object.type <> 'trigger' OR EXISTS ( \
                   SELECT 1 FROM {quoted_schema}.sqlite_schema AS owner \
                   WHERE owner.type IN ('table', 'view') \
                     AND owner.name = object.tbl_name COLLATE NOCASE \
               ))"
        );
        let rows = if let Some((sort_key, tie_breaker)) = cursor {
            let statement = format!(
                "{select} AND (object.name COLLATE BINARY > ? \
                 OR (object.name COLLATE BINARY = ? \
                 AND object.name COLLATE BINARY > ?)) \
                 ORDER BY object.name COLLATE BINARY LIMIT ?"
            );
            sqlx::query(AssertSqlSafe(statement))
                .bind(native_type)
                .bind(sort_key)
                .bind(sort_key)
                .bind(tie_breaker)
                .bind(limit)
                .fetch_all(&mut *connection)
                .await
        } else {
            let statement = format!("{select} ORDER BY object.name COLLATE BINARY LIMIT ?");
            sqlx::query(AssertSqlSafe(statement))
                .bind(native_type)
                .bind(limit)
                .fetch_all(&mut *connection)
                .await
        }
        .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;

        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let row_type: String = row.try_get("type").map_err(decode_error)?;
            let name: String = row.try_get("name").map_err(decode_error)?;
            let id = CatalogId::new(
                self.connection_id,
                match row_type.as_str() {
                    "table" => CatalogKind::Table,
                    "view" => CatalogKind::View,
                    "trigger" => CatalogKind::Trigger,
                    _ => return Err(catalog_internal("unexpected SQLite catalog object type")),
                },
                [self.database.clone(), schema_name.clone(), name.clone()],
            );
            let qualified_name = QualifiedName {
                database: Some(self.database.clone()),
                schema: Some(schema_name.clone()),
                object: name,
            };
            let entry = if id.kind == CatalogKind::Trigger {
                let owner_type: Option<String> = row.try_get("owner_type").map_err(decode_error)?;
                let owner_name: Option<String> = row.try_get("owner_name").map_err(decode_error)?;
                let owner_kind = match owner_type.as_deref() {
                    Some("table") => CatalogKind::Table,
                    Some("view") => CatalogKind::View,
                    _ => {
                        return Err(catalog_internal(
                            "SQLite trigger has no discoverable owning relation",
                        ));
                    }
                };
                let relation = CatalogId::new(
                    self.connection_id,
                    owner_kind,
                    [
                        self.database.clone(),
                        schema_name.clone(),
                        owner_name.ok_or_else(|| {
                            catalog_internal("SQLite trigger has no canonical owning relation name")
                        })?,
                    ],
                );
                CatalogEntry::relation_object(
                    id,
                    schema.clone(),
                    relation,
                    qualified_name,
                    row_type,
                    OptionalMetadata::Unsupported,
                )
            } else {
                CatalogEntry::relation(
                    id,
                    schema.clone(),
                    qualified_name,
                    row_type,
                    OptionalMetadata::Unsupported,
                    true,
                )
            }
            .map_err(catalog_invariant)?;
            entries.push(entry);
        }
        let next_cursor = finalize_keyset_page(
            &mut entries,
            request.page_size,
            |entry| entry.qualified_name.object.clone(),
            |entry| entry.qualified_name.object.clone(),
        )
        .map_err(catalog_invariant)?;
        CatalogPage::new(request, entries, total_count, next_cursor).map_err(catalog_invariant)
    }

    #[cfg(test)]
    async fn pause_after_page_count(&self) {
        let hook = self.page_snapshot_hook.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook.count_complete.wait().await;
            hook.continue_page.wait().await;
        }
    }

    async fn load_relation_children_page(
        &self,
        connection: &mut SqliteConnection,
        request: &CatalogRequest,
        relation: &CatalogId,
    ) -> Result<CatalogPage, DatabaseError> {
        let (schema, relation_name, _) = self
            .verified_relation_id(connection, relation, &request.key.target)
            .await?;
        let mut entries = self
            .load_relation_child_entries(connection, relation, &schema, &relation_name)
            .await?;
        let total_count = exact_count(entries.len())?;
        let next_cursor =
            paginate_in_memory(&mut entries, request, child_sort_key, child_tie_breaker)?;
        CatalogPage::new(request, entries, total_count, next_cursor).map_err(catalog_invariant)
    }

    async fn load_relation_child_entries(
        &self,
        connection: &mut SqliteConnection,
        relation: &CatalogId,
        schema: &str,
        relation_name: &str,
    ) -> Result<Vec<CatalogEntry>, DatabaseError> {
        let indexes = if relation.kind == CatalogKind::Table {
            self.load_index_metadata(connection, schema, relation_name)
                .await?
        } else {
            Vec::new()
        };
        let columns = self
            .load_column_metadata(connection, schema, relation_name, &indexes)
            .await?;
        let foreign_keys = if relation.kind == CatalogKind::Table {
            self.load_foreign_key_metadata(connection, schema, relation_name)
                .await?
        } else {
            Vec::new()
        };
        let mut memberships: HashMap<String, Vec<ConstraintMembership>> = HashMap::new();
        let mut entries = Vec::new();
        let mut has_native_primary_key = false;

        for index in indexes {
            let index_id = relation_child_id(relation, CatalogKind::Index, &index.name);
            entries.push(
                CatalogEntry::relation_child(
                    index_id,
                    relation.clone(),
                    child_qualified_name(&self.database, schema, &index.name),
                    "index",
                    OptionalMetadata::Unsupported,
                    CatalogMetadata::Index(IndexMetadata {
                        columns: index.columns.clone(),
                        unique: index.unique,
                    }),
                )
                .map_err(catalog_invariant)?,
            );

            let constraint = match index.origin.as_str() {
                "pk" => {
                    has_native_primary_key = true;
                    Some((
                        CatalogKind::PrimaryKey,
                        "primary_key",
                        CatalogMetadata::Constraint(ConstraintMetadata::PrimaryKey {
                            columns: index.columns.clone(),
                        }),
                    ))
                }
                "u" => Some((
                    CatalogKind::UniqueConstraint,
                    "unique",
                    CatalogMetadata::Constraint(ConstraintMetadata::Unique {
                        columns: index.columns.clone(),
                    }),
                )),
                _ => None,
            };
            if let Some((kind, native_kind, metadata)) = constraint {
                let constraint_id = relation_child_id(relation, kind, &index.name);
                add_memberships(&mut memberships, &index.columns, &constraint_id)?;
                entries.push(
                    CatalogEntry::relation_child(
                        constraint_id,
                        relation.clone(),
                        child_qualified_name(&self.database, schema, &index.name),
                        native_kind,
                        OptionalMetadata::Unsupported,
                        metadata,
                    )
                    .map_err(catalog_invariant)?,
                );
            }
        }

        let mut primary_key_columns = columns
            .iter()
            .filter(|column| column.primary_key_position != 0)
            .collect::<Vec<_>>();
        primary_key_columns.sort_by_key(|column| column.primary_key_position);
        if !primary_key_columns.is_empty() && !has_native_primary_key {
            let primary_key_columns = primary_key_columns
                .into_iter()
                .map(|column| column.name.clone())
                .collect::<Vec<_>>();
            let native_identity = "primary_key";
            let constraint_id =
                relation_child_id(relation, CatalogKind::PrimaryKey, native_identity);
            add_memberships(&mut memberships, &primary_key_columns, &constraint_id)?;
            entries.push(
                CatalogEntry::relation_child(
                    constraint_id,
                    relation.clone(),
                    child_qualified_name(&self.database, schema, native_identity),
                    "primary_key",
                    OptionalMetadata::Unsupported,
                    CatalogMetadata::Constraint(ConstraintMetadata::PrimaryKey {
                        columns: primary_key_columns,
                    }),
                )
                .map_err(catalog_invariant)?,
            );
        }

        for foreign_key in foreign_keys {
            let native_identity = foreign_key.id.to_string();
            let name = format!("fk_{relation_name}_{}", foreign_key.id);
            let constraint_id =
                relation_child_id(relation, CatalogKind::ForeignKey, &native_identity);
            add_memberships(&mut memberships, &foreign_key.columns, &constraint_id)?;
            entries.push(
                CatalogEntry::relation_child(
                    constraint_id,
                    relation.clone(),
                    child_qualified_name(&self.database, schema, &name),
                    "foreign_key",
                    OptionalMetadata::Unsupported,
                    CatalogMetadata::Constraint(ConstraintMetadata::ForeignKey {
                        columns: foreign_key.columns,
                        referenced_relation: QualifiedName {
                            database: Some(self.database.clone()),
                            schema: Some(schema.to_owned()),
                            object: foreign_key.referenced_relation,
                        },
                        referenced_columns: foreign_key.referenced_columns,
                    }),
                )
                .map_err(catalog_invariant)?,
            );
        }

        for column in columns {
            let mut metadata = ColumnMetadata::new(
                column.ordinal_position,
                column.native_type.clone(),
                column.nullable,
            );
            metadata.default_expression = OptionalMetadata::Supported(column.default_expression);
            metadata.hidden = OptionalMetadata::Supported(Some(column.hidden));
            metadata.constraint_memberships = memberships.remove(&column.name).unwrap_or_default();
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
                    relation_child_id(relation, CatalogKind::Column, &column.name),
                    relation.clone(),
                    child_qualified_name(&self.database, schema, &column.name),
                    "column",
                    OptionalMetadata::Unsupported,
                    CatalogMetadata::Column(metadata),
                )
                .map_err(catalog_invariant)?,
            );
        }

        Ok(entries)
    }

    async fn load_column_metadata(
        &self,
        connection: &mut SqliteConnection,
        schema: &str,
        relation: &str,
        indexes: &[SqliteIndexInfo],
    ) -> Result<Vec<SqliteColumnInfo>, DatabaseError> {
        let rows = sqlx::query(
            "SELECT cid, name, type, \"notnull\" AS not_null, dflt_value, pk, hidden \
             FROM pragma_table_xinfo(?, ?) ORDER BY cid",
        )
        .bind(relation)
        .bind(schema)
        .fetch_all(&mut *connection)
        .await
        .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let table_row = sqlx::query(
            "SELECT wr FROM pragma_table_list(?) \
             WHERE schema = ? COLLATE BINARY AND name = ? COLLATE BINARY AND type = 'table'",
        )
        .bind(relation)
        .bind(schema)
        .bind(relation)
        .fetch_optional(&mut *connection)
        .await
        .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let without_rowid = table_row
            .as_ref()
            .map(|row| row.try_get::<i64, _>("wr").map(|wr| wr != 0))
            .transpose()
            .map_err(decode_error)?
            .unwrap_or(false);
        let rowid_table = table_row.is_some() && !without_rowid;

        let mut columns = rows
            .into_iter()
            .map(|row| {
                let cid: i64 = row.try_get("cid").map_err(decode_error)?;
                let ordinal_position = cid
                    .checked_add(1)
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| catalog_internal("invalid SQLite column ordinal"))?;
                let primary_key_position: i64 = row.try_get("pk").map_err(decode_error)?;
                let primary_key_position = u32::try_from(primary_key_position)
                    .map_err(|_| catalog_internal("invalid SQLite primary-key ordinal"))?;
                let not_null: i64 = row.try_get("not_null").map_err(decode_error)?;
                let hidden: i64 = row.try_get("hidden").map_err(decode_error)?;
                Ok(SqliteColumnInfo {
                    name: row.try_get("name").map_err(decode_error)?,
                    ordinal_position,
                    native_type: row.try_get("type").map_err(decode_error)?,
                    nullable: not_null == 0,
                    default_expression: row.try_get("dflt_value").map_err(decode_error)?,
                    primary_key_position,
                    hidden: hidden == 1,
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        let primary_key_columns = columns
            .iter()
            .filter(|column| column.primary_key_position != 0)
            .collect::<Vec<_>>();
        let rowid_alias = if rowid_table
            && primary_key_columns.len() == 1
            && !indexes.iter().any(|index| index.origin == "pk")
            && primary_key_columns[0]
                .native_type
                .trim()
                .eq_ignore_ascii_case("INTEGER")
        {
            Some(primary_key_columns[0].name.clone())
        } else {
            None
        };
        for column in &mut columns {
            if (without_rowid && column.primary_key_position != 0)
                || rowid_alias.as_deref() == Some(column.name.as_str())
            {
                column.nullable = false;
            }
        }
        Ok(columns)
    }

    async fn load_index_metadata(
        &self,
        connection: &mut SqliteConnection,
        schema: &str,
        relation: &str,
    ) -> Result<Vec<SqliteIndexInfo>, DatabaseError> {
        let rows = sqlx::query(
            "SELECT name, \"unique\" AS is_unique, origin \
             FROM pragma_index_list(?, ?) ORDER BY name COLLATE BINARY",
        )
        .bind(relation)
        .bind(schema)
        .fetch_all(&mut *connection)
        .await
        .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let mut indexes = Vec::with_capacity(rows.len());
        for row in rows {
            let name: String = row.try_get("name").map_err(decode_error)?;
            let parts = sqlx::query(
                "SELECT seqno, cid, name, \"key\" AS is_key \
                 FROM pragma_index_xinfo(?, ?) ORDER BY seqno",
            )
            .bind(&name)
            .bind(schema)
            .fetch_all(&mut *connection)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
            let mut columns = Vec::new();
            for part in parts {
                let is_key: i64 = part.try_get("is_key").map_err(decode_error)?;
                if is_key == 0 {
                    continue;
                }
                let seqno: i64 = part.try_get("seqno").map_err(decode_error)?;
                let cid: i64 = part.try_get("cid").map_err(decode_error)?;
                let column: Option<String> = part.try_get("name").map_err(decode_error)?;
                columns.push(column.unwrap_or_else(|| format!("<expression:{seqno}:{cid}>")));
            }
            let unique: i64 = row.try_get("is_unique").map_err(decode_error)?;
            indexes.push(SqliteIndexInfo {
                name,
                columns,
                unique: unique != 0,
                origin: row.try_get("origin").map_err(decode_error)?,
            });
        }
        Ok(indexes)
    }

    async fn load_foreign_key_metadata(
        &self,
        connection: &mut SqliteConnection,
        schema: &str,
        relation: &str,
    ) -> Result<Vec<SqliteForeignKeyInfo>, DatabaseError> {
        let rows = sqlx::query(
            "SELECT id, seq, \"table\" AS target_table, \
             \"from\" AS source_column, \"to\" AS target_column \
             FROM pragma_foreign_key_list(?, ?) ORDER BY id, seq",
        )
        .bind(relation)
        .bind(schema)
        .fetch_all(&mut *connection)
        .await
        .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let mut grouped = BTreeMap::<i64, SqliteForeignKeyBuilder>::new();
        for row in rows {
            let id: i64 = row.try_get("id").map_err(decode_error)?;
            let target_table: String = row.try_get("target_table").map_err(decode_error)?;
            let source_column: String = row.try_get("source_column").map_err(decode_error)?;
            let target_column: Option<String> =
                row.try_get("target_column").map_err(decode_error)?;
            let foreign_key = grouped
                .entry(id)
                .or_insert_with(|| SqliteForeignKeyBuilder {
                    referenced_relation: target_table.clone(),
                    columns: Vec::new(),
                    referenced_columns: Vec::new(),
                });
            if foreign_key.referenced_relation != target_table {
                return Err(catalog_internal(
                    "SQLite foreign-key rows disagree on referenced relation",
                ));
            }
            foreign_key.columns.push(source_column);
            foreign_key.referenced_columns.push(target_column);
        }

        let mut foreign_keys = Vec::with_capacity(grouped.len());
        for (id, foreign_key) in grouped {
            let inferred_columns = if foreign_key.referenced_columns.iter().any(Option::is_none) {
                self.primary_key_columns(connection, schema, &foreign_key.referenced_relation)
                    .await?
            } else {
                Vec::new()
            };
            let referenced_columns = foreign_key
                .referenced_columns
                .into_iter()
                .enumerate()
                .map(|(index, column)| {
                    column
                        .or_else(|| inferred_columns.get(index).cloned())
                        .ok_or_else(|| {
                            catalog_internal(
                                "SQLite foreign key omits an unresolved referenced column",
                            )
                        })
                })
                .collect::<Result<Vec<_>, DatabaseError>>()?;
            foreign_keys.push(SqliteForeignKeyInfo {
                id,
                referenced_relation: foreign_key.referenced_relation,
                columns: foreign_key.columns,
                referenced_columns,
            });
        }
        Ok(foreign_keys)
    }

    async fn primary_key_columns(
        &self,
        connection: &mut SqliteConnection,
        schema: &str,
        relation: &str,
    ) -> Result<Vec<String>, DatabaseError> {
        sqlx::query(
            "SELECT name FROM pragma_table_xinfo(?, ?) \
             WHERE pk > 0 ORDER BY pk",
        )
        .bind(relation)
        .bind(schema)
        .fetch_all(&mut *connection)
        .await
        .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?
        .into_iter()
        .map(|row| row.try_get("name").map_err(decode_error))
        .collect()
    }

    async fn database_aliases(
        &self,
        connection: &mut SqliteConnection,
    ) -> Result<Vec<String>, DatabaseError> {
        let mut aliases = sqlx::query("PRAGMA database_list")
            .fetch_all(&mut *connection)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?
            .into_iter()
            .map(|row| {
                Ok((
                    row.try_get::<i64, _>("seq").map_err(decode_error)?,
                    row.try_get::<String, _>("name").map_err(decode_error)?,
                ))
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        aliases.sort_by_key(|(sequence, _)| *sequence);
        Ok(aliases.into_iter().map(|(_, alias)| alias).collect())
    }

    fn verify_database_id(
        &self,
        database: &CatalogId,
        target: &CatalogTarget,
    ) -> Result<(), DatabaseError> {
        if matches!(database.native_path.as_slice(), [name] if name == &self.database) {
            Ok(())
        } else {
            Err(catalog_target_not_found(target))
        }
    }

    async fn verified_schema_id(
        &self,
        connection: &mut SqliteConnection,
        schema: &CatalogId,
        target: &CatalogTarget,
    ) -> Result<String, DatabaseError> {
        let schema_name = match schema.native_path.as_slice() {
            [database, schema] if database == &self.database => schema.as_str(),
            _ => return Err(catalog_target_not_found(target)),
        };
        self.verified_schema_name(connection, schema_name, target)
            .await
    }

    async fn verified_schema_name(
        &self,
        connection: &mut SqliteConnection,
        schema: &str,
        target: &CatalogTarget,
    ) -> Result<String, DatabaseError> {
        self.database_aliases(connection)
            .await?
            .into_iter()
            .find(|alias| alias == schema)
            .ok_or_else(|| catalog_target_not_found(target))
    }

    async fn verified_relation_id(
        &self,
        connection: &mut SqliteConnection,
        relation: &CatalogId,
        target: &CatalogTarget,
    ) -> Result<(String, String, &'static str), DatabaseError> {
        if relation.profile_id() != self.connection_id || !relation.kind.is_relation() {
            return Err(catalog_target_not_found(target));
        }
        if relation.kind == CatalogKind::MaterializedView {
            return Err(DatabaseError::unsupported_catalog_target(
                DatabaseKind::Sqlite,
                target,
            ));
        }
        let (schema, relation_name) = match relation.native_path.as_slice() {
            [database, schema, relation] if database == &self.database && !relation.is_empty() => {
                (schema.as_str(), relation.as_str())
            }
            _ => return Err(catalog_target_not_found(target)),
        };
        let schema = self
            .verified_schema_name(connection, schema, target)
            .await?;
        let quoted_schema = self.quote_identifier(&schema);
        let statement = format!(
            "SELECT type FROM {quoted_schema}.sqlite_schema \
             WHERE name = ? COLLATE BINARY AND type IN ('table', 'view')"
        );
        let native_type = sqlx::query(AssertSqlSafe(statement))
            .bind(relation_name)
            .fetch_optional(&mut *connection)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?
            .map(|row| row.try_get::<String, _>("type").map_err(decode_error))
            .transpose()?;
        let (expected_kind, native_kind) = match native_type.as_deref() {
            Some("table") => (CatalogKind::Table, "table"),
            Some("view") => (CatalogKind::View, "view"),
            _ => return Err(catalog_target_not_found(target)),
        };
        if relation.kind != expected_kind {
            return Err(catalog_target_not_found(target));
        }
        Ok((schema, relation_name.to_owned(), native_kind))
    }

    pub async fn object_ddl(
        &self,
        kind: CatalogKind,
        schema: &str,
        name: &str,
    ) -> Result<Option<String>, DatabaseError> {
        let _operation_permit = self.acquire_operation().await?;
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        let object_type = match kind {
            CatalogKind::Table => "table",
            CatalogKind::View => "view",
            CatalogKind::Index => "index",
            CatalogKind::Trigger => "trigger",
            _ => return Ok(None),
        };
        let target = CatalogTarget::Groups {
            schema: CatalogId::new(
                self.connection_id,
                CatalogKind::Schema,
                [self.database.clone(), schema.to_owned()],
            ),
        };
        let schema = self
            .verified_schema_name(&mut connection, schema, &target)
            .await?;
        let quoted_schema = self.quote_identifier(&schema);
        let statement = format!(
            "SELECT sql FROM {quoted_schema}.sqlite_schema \
             WHERE type = ? AND name = ? COLLATE BINARY"
        );
        sqlx::query_scalar::<_, Option<String>>(AssertSqlSafe(statement))
            .bind(object_type)
            .bind(name)
            .fetch_optional(&mut *connection)
            .await
            .map(|value| value.flatten())
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))
    }

    pub fn quote_identifier(&self, value: &str) -> String {
        format!("\"{}\"", value.replace('"', "\"\""))
    }

    pub async fn close(self) {
        self.pool.close().await;
    }
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

fn relation_kind(native_type: &str) -> Result<CatalogKind, DatabaseError> {
    match native_type {
        "table" => Ok(CatalogKind::Table),
        "view" => Ok(CatalogKind::View),
        _ => Err(catalog_internal("unexpected SQLite relation type")),
    }
}

fn search_rank(hit: &CatalogSearchHit, query: &str) -> Option<u8> {
    let name = hit.entry.qualified_name.object.to_lowercase();
    let path = hit.qualified_path().to_lowercase();
    if name == query {
        Some(0)
    } else if name.starts_with(query) {
        Some(1)
    } else if name.contains(query) {
        Some(2)
    } else if path.contains(query) {
        Some(3)
    } else {
        None
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

fn group_sort_key(group: ObjectGroup) -> &'static str {
    match group {
        ObjectGroup::Tables => "00:tables",
        ObjectGroup::Views => "01:views",
        ObjectGroup::Triggers => "02:triggers",
        ObjectGroup::MaterializedViews => "03:materialized_views",
        ObjectGroup::Sequences => "04:sequences",
        ObjectGroup::Functions => "05:functions",
        ObjectGroup::Procedures => "06:procedures",
        ObjectGroup::Types => "07:types",
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
        CatalogKind::Database => 7,
        CatalogKind::Schema => 8,
        CatalogKind::Table => 9,
        CatalogKind::View => 10,
        CatalogKind::MaterializedView => 11,
        CatalogKind::Function => 12,
        CatalogKind::Procedure => 13,
        CatalogKind::Sequence => 14,
        CatalogKind::Type => 15,
    }
}

fn relation_child_id(relation: &CatalogId, kind: CatalogKind, native_identity: &str) -> CatalogId {
    let mut native_path = relation.native_path.clone();
    native_path.push(native_identity.to_owned());
    CatalogId::new(relation.profile_id(), kind, native_path)
}

fn child_qualified_name(database: &str, schema: &str, object: &str) -> QualifiedName {
    QualifiedName {
        database: Some(database.to_owned()),
        schema: Some(schema.to_owned()),
        object: object.to_owned(),
    }
}

fn add_memberships(
    memberships: &mut HashMap<String, Vec<ConstraintMembership>>,
    columns: &[String],
    constraint_id: &CatalogId,
) -> Result<(), DatabaseError> {
    for (index, column) in columns.iter().enumerate() {
        let ordinal_position = u32::try_from(index.saturating_add(1))
            .map_err(|_| catalog_internal("SQLite constraint has too many columns"))?;
        memberships
            .entry(column.clone())
            .or_default()
            .push(ConstraintMembership {
                constraint_id: constraint_id.clone(),
                ordinal_position,
            });
    }
    Ok(())
}

fn catalog_target_not_found(target: &CatalogTarget) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Configuration,
        code: Some("catalog_target_not_found".to_owned()),
        message: sanitize_terminal_text(&format!(
            "SQLite catalog target was not found: {}",
            target.description()
        )),
    }
}

fn exact_count(count: usize) -> Result<CatalogCount, DatabaseError> {
    u64::try_from(count)
        .map(CatalogCount::Exact)
        .map_err(|_| catalog_internal("SQLite catalog count exceeds u64"))
}

fn non_negative_count(count: i64) -> Result<u64, DatabaseError> {
    u64::try_from(count).map_err(|_| catalog_internal("SQLite returned a negative catalog count"))
}

fn page_limit(page_size: usize) -> Result<i64, DatabaseError> {
    page_size
        .checked_add(1)
        .and_then(|limit| i64::try_from(limit).ok())
        .ok_or_else(|| catalog_internal("SQLite catalog page limit overflowed"))
}

fn catalog_invariant(error: CatalogValidationError) -> DatabaseError {
    catalog_internal(format!("SQLite catalog invariant failed: {error}"))
}

fn catalog_internal(message: impl AsRef<str>) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Internal,
        code: Some("sqlite_catalog_invariant".to_owned()),
        message: sanitize_terminal_text(message.as_ref()),
    }
}

pub(crate) struct SqliteTransactionBackend {
    connection: PoolConnection<Sqlite>,
    cancelled: Arc<AtomicBool>,
    adapter: SqliteAdapter,
    _operation_permit: OwnedSemaphorePermit,
}

#[async_trait::async_trait]
impl TransactionBackend for SqliteTransactionBackend {
    async fn begin(&mut self) -> Result<(), TransactionError> {
        <Sqlite as sqlx::Database>::TransactionManager::begin(&mut self.connection, None)
            .await
            .map_err(|error| TransactionError(error.to_string()))
    }
    async fn execute(&mut self, sql: &str) -> Result<QueryOutcome, TransactionError> {
        let result = self
            .adapter
            .execute_connection(&mut self.connection, sql)
            .await
            .map_err(Into::into);
        self.connection
            .lock_handle()
            .await
            .map_err(|error| TransactionError(error.to_string()))?
            .remove_progress_handler();
        self.cancelled.store(false, Ordering::Relaxed);
        result
    }
    async fn relation_mutation(
        &mut self,
        request: RelationMutationRequest,
    ) -> Result<MutationResult, TransactionError> {
        let [_, schema, relation] = request.relation.native_path.as_slice() else {
            return Err(TransactionError(
                "SQLite relation has no canonical database, schema, and table path".into(),
            ));
        };
        let columns = &request.metadata.columns;
        let quoted_table = format!(
            "{}.{}",
            self.adapter.quote_identifier(schema),
            self.adapter.quote_identifier(relation)
        );
        match request.operation {
            RelationMutation::DeleteRows(rows) => {
                if rows.is_empty() {
                    return Ok(MutationResult::Deleted { rows: 0 });
                }
                let mut deleted = 0;
                for mutation in rows {
                    if mutation.row.columns.len() != mutation.row.values.len()
                        || mutation.original.len() != columns.len()
                    {
                        return Err(TransactionError(
                            "SQLite delete mutation is malformed".into(),
                        ));
                    }
                    let mut sql = format!("DELETE FROM {quoted_table} WHERE ");
                    let mut predicates = Vec::new();
                    for index in &mutation.row.columns {
                        if *index >= columns.len() {
                            return Err(TransactionError(
                                "SQLite row locator column is out of range".into(),
                            ));
                        }
                        predicates.push(format!(
                            "(({} = ?) OR ({} IS NULL AND ? IS NULL))",
                            self.adapter.quote_identifier(&columns[*index].0),
                            self.adapter.quote_identifier(&columns[*index].0)
                        ));
                    }
                    for (index, _) in columns.iter().enumerate() {
                        let name = self.adapter.quote_identifier(&columns[index].0);
                        predicates
                            .push(format!("(({name} = ?) OR ({name} IS NULL AND ? IS NULL))"));
                    }
                    sql.push_str(&predicates.join(" AND "));
                    let mut query = sqlx::query(AssertSqlSafe(sql));
                    for value in &mutation.row.values {
                        query = bind_cell(query, value)?;
                        query = bind_cell(query, value)?;
                    }
                    for value in &mutation.original {
                        query = bind_cell(query, value)?;
                        query = bind_cell(query, value)?;
                    }
                    if query
                        .execute(&mut *self.connection)
                        .await
                        .map_err(|e| TransactionError(e.to_string()))?
                        .rows_affected()
                        != 1
                    {
                        return Err(TransactionError("SQLite relation mutation conflict".into()));
                    }
                    deleted += 1;
                }
                return Ok(MutationResult::Deleted { rows: deleted });
            }
            RelationMutation::InsertRow(insert) => {
                if insert.columns.len() != insert.values.len()
                    || insert.columns.iter().any(|i| *i >= columns.len())
                {
                    return Err(TransactionError(
                        "SQLite insert mutation is malformed".into(),
                    ));
                }
                // SQLite does not accept DEFAULT in a VALUES term. Omit a
                // DEFAULT request so SQLite evaluates the column default.
                let supplied = insert
                    .columns
                    .iter()
                    .zip(&insert.values)
                    .filter(|(_, value)| !matches!(value, InputValue::Default))
                    .collect::<Vec<_>>();
                let names = supplied
                    .iter()
                    .map(|(i, _)| self.adapter.quote_identifier(&columns[**i].0))
                    .collect::<Vec<_>>();
                let expressions = supplied.iter().map(|_| "?").collect::<Vec<_>>();
                let sql = if names.is_empty() {
                    format!("INSERT INTO {quoted_table} DEFAULT VALUES RETURNING *")
                } else {
                    format!(
                        "INSERT INTO {quoted_table} ({}) VALUES ({}) RETURNING *",
                        names.join(", "),
                        expressions.join(", ")
                    )
                };
                let mut query = sqlx::query(AssertSqlSafe(sql));
                for (_, value) in supplied {
                    match value {
                        InputValue::Default => unreachable!(),
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
                        "SQLite update column is out of range".into(),
                    ));
                };
                if update.row.columns.len() != update.row.values.len() {
                    return Err(TransactionError("SQLite row locator is malformed".into()));
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
                                TransactionError("SQLite primary key column is missing".into())
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if primary_key_columns != update.row.columns {
                    return Err(TransactionError(
                        "SQLite row locator must contain the primary key columns in order".into(),
                    ));
                }
                if update
                    .row
                    .columns
                    .iter()
                    .any(|index| *index >= columns.len())
                {
                    return Err(TransactionError(
                        "SQLite row locator column is out of range".into(),
                    ));
                }

                let quoted_column = self.adapter.quote_identifier(column_name);
                let set_sql = match update.value {
                    InputValue::Default => format!("{quoted_column} = DEFAULT"),
                    InputValue::Null | InputValue::Value(_) => format!("{quoted_column} = ?"),
                };
                let mut statement = format!("UPDATE {quoted_table} SET {set_sql} WHERE ");
                for (position, column_index) in update.row.columns.iter().enumerate() {
                    if position > 0 {
                        statement.push_str(" AND ");
                    }
                    let name = self.adapter.quote_identifier(&columns[*column_index].0);
                    statement
                        .push_str(&format!("(({name} = ?) OR ({name} IS NULL AND ? IS NULL))"));
                }
                if !update.row.columns.is_empty() {
                    statement.push_str(" AND ");
                }
                statement.push_str(&format!(
                    "(({quoted_column} = ?) OR ({quoted_column} IS NULL AND ? IS NULL))"
                ));

                let mut query = sqlx::query(AssertSqlSafe(statement));
                match &update.value {
                    InputValue::Default => {}
                    InputValue::Null => query = query.bind(Option::<String>::None),
                    InputValue::Value(value) => query = bind_cell(query, value)?,
                }
                for value in &update.row.values {
                    query = bind_cell(query, value)?;
                    query = bind_cell(query, value)?;
                }
                query = bind_cell(query, &update.original)?;
                query = bind_cell(query, &update.original)?;
                let affected = query
                    .execute(&mut *self.connection)
                    .await
                    .map_err(|error| TransactionError(error.to_string()))?
                    .rows_affected();
                if affected != 1 {
                    return Err(TransactionError("SQLite relation mutation conflict".into()));
                }

                let mut select = format!("SELECT * FROM {quoted_table} WHERE ");
                for (position, column_index) in update.row.columns.iter().enumerate() {
                    if position > 0 {
                        select.push_str(" AND ");
                    }
                    let name = self.adapter.quote_identifier(&columns[*column_index].0);
                    select.push_str(&format!("(({name} = ?) OR ({name} IS NULL AND ? IS NULL))"));
                }
                let mut select_query = sqlx::query(AssertSqlSafe(select));
                for value in &update.row.values {
                    select_query = bind_cell(select_query, value)?;
                    select_query = bind_cell(select_query, value)?;
                }
                let row = select_query
                    .fetch_one(&mut *self.connection)
                    .await
                    .map_err(|error| TransactionError(error.to_string()))?;
                Ok(MutationResult::Updated {
                    row: decode_row(&row),
                })
            }
        }
    }
    async fn commit(&mut self) -> Result<(), TransactionError> {
        <Sqlite as sqlx::Database>::TransactionManager::commit(&mut self.connection)
            .await
            .map_err(|error| TransactionError(error.to_string()))
    }
    async fn rollback(&mut self) -> Result<(), TransactionError> {
        <Sqlite as sqlx::Database>::TransactionManager::rollback(&mut self.connection)
            .await
            .map_err(|error| TransactionError(error.to_string()))
    }
    async fn cancel(&mut self) -> Result<(), TransactionError> {
        self.cancelled.store(true, Ordering::Relaxed);
        // The progress callback is connection-local. The worker will quarantine this
        // connection if the in-flight future cannot report its terminal interrupt.
        Err(TransactionError(
            "SQLite cancellation requires forced close".into(),
        ))
    }
    fn depth(&self) -> usize {
        <Sqlite as sqlx::Database>::TransactionManager::get_transaction_depth(&self.connection)
    }
    fn force_close(self) -> futures_util::future::BoxFuture<'static, Result<(), TransactionError>> {
        Box::pin(async move {
            let Self {
                connection,
                cancelled,
                adapter,
                _operation_permit,
            } = self;
            cancelled.store(true, Ordering::Relaxed);
            let connection = connection.detach();
            let result = connection
                .close()
                .await
                .map_err(|error| TransactionError(error.to_string()));
            adapter.pool.close().await;
            drop(_operation_permit);
            result
        })
    }
}

fn columns(row: &SqliteRow) -> Vec<ColumnMeta> {
    row.columns()
        .iter()
        .map(|column| ColumnMeta {
            name: column.name().to_owned(),
            type_name: column.type_info().name().to_owned(),
        })
        .collect()
}

fn decode_row(row: &SqliteRow) -> Vec<CellValue> {
    (0..row.len())
        .map(|index| decode_cell(row, index))
        .collect()
}

fn decode_cell(row: &SqliteRow, index: usize) -> CellValue {
    let Ok(raw) = row.try_get_raw(index) else {
        return CellValue::Unsupported {
            type_name: "unknown".to_owned(),
            preview: "decode error".to_owned(),
        };
    };
    if raw.is_null() {
        return CellValue::Null;
    }
    let type_name = raw.type_info().name().to_ascii_uppercase();
    let decoded = match type_name.as_str() {
        "BOOLEAN" | "BOOL" => row.try_get::<bool, _>(index).map(CellValue::Boolean),
        "INTEGER" | "INT" | "INT64" => row.try_get::<i64, _>(index).map(CellValue::Integer),
        "REAL" | "FLOAT" | "DOUBLE" => row.try_get::<f64, _>(index).map(CellValue::Float),
        "TEXT" => row.try_get::<String, _>(index).map(CellValue::Text),
        "BLOB" => row.try_get::<Vec<u8>, _>(index).map(CellValue::Bytes),
        _ => row.try_get::<String, _>(index).map(CellValue::Text),
    };
    decoded.unwrap_or_else(|error| CellValue::Unsupported {
        type_name,
        preview: error.to_string(),
    })
}

fn decode_error(error: sqlx::Error) -> DatabaseError {
    DatabaseError::from_sqlx(error, ErrorCategory::Internal)
}

fn bind_cell<'q>(
    query: Query<'q, Sqlite, SqliteArguments>,
    value: &CellValue,
) -> Result<Query<'q, Sqlite, SqliteArguments>, TransactionError> {
    Ok(match value {
        CellValue::Null => query.bind(Option::<String>::None),
        CellValue::Boolean(value) => query.bind(*value),
        CellValue::Integer(value) => query.bind(*value),
        CellValue::Unsigned(value) => query.bind(i64::try_from(*value).map_err(|_| {
            TransactionError("SQLite cannot bind an unsigned value larger than i64".into())
        })?),
        CellValue::Float(value) => query.bind(*value),
        CellValue::Text(value) => query.bind(value.clone()),
        CellValue::Bytes(value) => query.bind(value.clone()),
        CellValue::Date(value) => query.bind(value.format("%Y-%m-%d").to_string()),
        CellValue::Time(value) => query.bind(value.format("%H:%M:%S%.f").to_string()),
        CellValue::DateTime(value) => query.bind(value.format("%Y-%m-%d %H:%M:%S%.f").to_string()),
        CellValue::Timestamp(value) => query.bind(value.to_rfc3339()),
        CellValue::Unsupported { .. } => {
            return Err(TransactionError(
                "SQLite cannot bind an unsupported cell value".into(),
            ));
        }
    })
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use sqlx::{Connection, Executor, SqliteConnection};
    use tempfile::TempDir;
    use tokio::time::timeout;

    use super::{PageSnapshotHook, SqliteAdapter};
    use crate::{
        db::{
            catalog::{
                CatalogId, CatalogKind, CatalogRequest, CatalogRequestKey, CatalogTarget,
                ObjectGroup,
            },
            mutation::{
                DeleteRowMutation, InputValue, InsertRowMutation, MetadataFingerprint,
                RelationMutation, RelationMutationRequest, RowLocator, UpdateCellMutation,
            },
            transaction::{TransactionBackend, TransactionError},
            value::CellValue,
        },
        identity::ConnectionIdentity,
        model::{execution_target::ExecutionTarget, relation::RelationKey},
        profile::{CatalogScope, DatabaseKind, import_connection_url},
    };

    async fn memory_adapter() -> SqliteAdapter {
        let imported = import_connection_url("sqlite://:memory:", Some("sqlite-internal")).unwrap();
        SqliteAdapter::connect(&imported.profile).await.unwrap()
    }

    fn update_request(value: InputValue, original: CellValue) -> RelationMutationRequest {
        let connection_id = uuid::Uuid::nil();
        RelationMutationRequest {
            tab_id: uuid::Uuid::nil(),
            tab_generation: 1,
            edit_generation: 1,
            row_id: crate::model::relation_edit::EditableRowId(1),
            connection: ConnectionIdentity {
                profile_id: connection_id,
                generation: 1,
            },
            target: ExecutionTarget {
                profile_id: connection_id,
                database: ":memory:".into(),
                schema: Some("main".into()),
            },
            relation: CatalogId::new(
                connection_id,
                CatalogKind::Table,
                [":memory:", "main", "items"],
            ),
            relation_key: RelationKey {
                profile_id: connection_id,
                object_id: CatalogId::new(
                    connection_id,
                    CatalogKind::Table,
                    [":memory:", "main", "items"],
                ),
            },
            scope: CatalogScope::for_profile(DatabaseKind::Sqlite, ":memory:", Some("main")),
            metadata: MetadataFingerprint {
                relation: "items".into(),
                columns: vec![
                    ("id".into(), "INTEGER".into(), false),
                    ("value".into(), "TEXT".into(), true),
                ],
                primary_key: vec!["id".into()],
            },
            operation: RelationMutation::UpdateCell(UpdateCellMutation {
                row: RowLocator {
                    columns: vec![0],
                    values: vec![CellValue::Integer(1)],
                },
                column: 1,
                original,
                value,
            }),
        }
    }

    fn mutation_request(operation: RelationMutation) -> RelationMutationRequest {
        let mut request = update_request(InputValue::Null, CellValue::Null);
        request.operation = operation;
        request
    }

    #[tokio::test]
    async fn relation_mutation_updates_full_row_and_observes_transaction_outcome() {
        let adapter = memory_adapter().await;
        let mut backend = adapter.transaction_backend().await.unwrap();
        backend.begin().await.unwrap();
        backend
            .execute("CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT)")
            .await
            .unwrap();
        backend
            .execute("INSERT INTO items VALUES (1, NULL)")
            .await
            .unwrap();
        backend.commit().await.unwrap();
        backend.begin().await.unwrap();

        let result = backend
            .relation_mutation(update_request(
                InputValue::Value(CellValue::Text("updated".into())),
                CellValue::Null,
            ))
            .await
            .unwrap();
        assert_eq!(
            result,
            super::MutationResult::Updated {
                row: vec![CellValue::Integer(1), CellValue::Text("updated".into())]
            }
        );
        backend.rollback().await.unwrap();
        let count = backend.execute("SELECT count(*) FROM items").await.unwrap();
        assert_eq!(count.result_sets[0].rows, vec![vec![CellValue::Integer(1)]]);
        backend.begin().await.unwrap();
        backend
            .relation_mutation(update_request(
                InputValue::Value(CellValue::Text("committed".into())),
                CellValue::Null,
            ))
            .await
            .unwrap();
        backend.commit().await.unwrap();
        let row = backend.execute("SELECT * FROM items").await.unwrap();
        assert_eq!(
            row.result_sets[0].rows,
            vec![vec![
                CellValue::Integer(1),
                CellValue::Text("committed".into())
            ]]
        );
        drop(backend);
        adapter.close().await;
    }

    #[tokio::test]
    async fn relation_mutation_conflicts_when_original_value_changed() {
        let adapter = memory_adapter().await;
        let mut backend = adapter.transaction_backend().await.unwrap();
        backend.begin().await.unwrap();
        backend
            .execute("CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT)")
            .await
            .unwrap();
        backend
            .execute("INSERT INTO items VALUES (1, 'actual')")
            .await
            .unwrap();
        let error = backend
            .relation_mutation(update_request(
                InputValue::Value(CellValue::Text("new".into())),
                CellValue::Text("stale".into()),
            ))
            .await
            .unwrap_err();
        assert_eq!(
            error,
            TransactionError("SQLite relation mutation conflict".into())
        );
        backend.commit().await.unwrap();
        drop(backend);
        adapter.close().await;
    }

    #[tokio::test]
    async fn relation_mutation_deletes_multiple_rows_with_full_null_safe_snapshot() {
        let adapter = memory_adapter().await;
        adapter
            .execute("CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT)")
            .await
            .unwrap();
        adapter
            .execute("INSERT INTO items VALUES (1, NULL), (2, 'two')")
            .await
            .unwrap();
        let mut backend = adapter.transaction_backend().await.unwrap();
        backend.begin().await.unwrap();
        let request = mutation_request(RelationMutation::DeleteRows(vec![
            DeleteRowMutation {
                row: RowLocator {
                    columns: vec![0],
                    values: vec![CellValue::Integer(1)],
                },
                original: vec![CellValue::Integer(1), CellValue::Null],
            },
            DeleteRowMutation {
                row: RowLocator {
                    columns: vec![0],
                    values: vec![CellValue::Integer(2)],
                },
                original: vec![CellValue::Integer(2), CellValue::Text("two".into())],
            },
        ]));
        assert_eq!(
            { backend.relation_mutation(request).await.unwrap() },
            super::MutationResult::Deleted { rows: 2 }
        );
        backend.commit().await.unwrap();
        assert!(
            backend
                .execute("SELECT * FROM items")
                .await
                .unwrap()
                .result_sets[0]
                .rows
                .is_empty()
        );
        drop(backend);
        adapter.close().await;
    }

    #[tokio::test]
    async fn relation_mutation_delete_batch_conflict_is_atomic() {
        let adapter = memory_adapter().await;
        adapter
            .execute("CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT)")
            .await
            .unwrap();
        adapter
            .execute("INSERT INTO items VALUES (1, 'one'), (2, 'two')")
            .await
            .unwrap();
        let mut backend = adapter.transaction_backend().await.unwrap();
        backend.begin().await.unwrap();
        let request = mutation_request(RelationMutation::DeleteRows(vec![
            DeleteRowMutation {
                row: RowLocator {
                    columns: vec![0],
                    values: vec![CellValue::Integer(1)],
                },
                original: vec![CellValue::Integer(1), CellValue::Text("one".into())],
            },
            DeleteRowMutation {
                row: RowLocator {
                    columns: vec![0],
                    values: vec![CellValue::Integer(2)],
                },
                original: vec![CellValue::Integer(2), CellValue::Text("stale".into())],
            },
        ]));
        assert!(backend.relation_mutation(request).await.is_err());
        backend.rollback().await.unwrap();
        assert_eq!(
            backend
                .execute("SELECT count(*) FROM items")
                .await
                .unwrap()
                .result_sets[0]
                .rows,
            vec![vec![CellValue::Integer(2)]]
        );
        drop(backend);
        adapter.close().await;
    }

    #[tokio::test]
    async fn relation_mutation_inserts_bound_null_default_and_generated_values() {
        let adapter = memory_adapter().await;
        adapter
            .execute("CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT DEFAULT 'default', doubled INTEGER GENERATED ALWAYS AS (id * 2) STORED)")
            .await
            .unwrap();
        let mut backend = adapter.transaction_backend().await.unwrap();
        backend.begin().await.unwrap();
        let request = mutation_request(RelationMutation::InsertRow(InsertRowMutation {
            columns: vec![0, 1],
            values: vec![
                InputValue::Value(CellValue::Integer(7)),
                InputValue::Default,
            ],
        }));
        assert_eq!(
            backend.relation_mutation(request).await.unwrap(),
            super::MutationResult::Inserted {
                row: vec![
                    CellValue::Integer(7),
                    CellValue::Text("default".into()),
                    CellValue::Integer(14)
                ]
            }
        );
        let request = mutation_request(RelationMutation::InsertRow(InsertRowMutation {
            columns: vec![0, 1],
            values: vec![InputValue::Value(CellValue::Integer(8)), InputValue::Null],
        }));
        assert_eq!(
            backend.relation_mutation(request).await.unwrap(),
            super::MutationResult::Inserted {
                row: vec![
                    CellValue::Integer(8),
                    CellValue::Null,
                    CellValue::Integer(16)
                ]
            }
        );
        backend.rollback().await.unwrap();
        drop(backend);
        adapter.close().await;
    }

    #[tokio::test]
    async fn connection_local_pool_disables_automatic_replacement() {
        let adapter = memory_adapter().await;
        let options = adapter.pool.options();

        assert_eq!(options.get_max_connections(), 1);
        assert_eq!(options.get_idle_timeout(), None);
        assert_eq!(options.get_max_lifetime(), None);

        adapter.close().await;
    }

    #[tokio::test]
    async fn operations_wait_for_transaction_backend_lifetime() {
        let adapter = memory_adapter().await;
        let mut backend = adapter.transaction_backend().await.unwrap();
        backend.begin().await.unwrap();
        assert_eq!(adapter.operation_gate.available_permits(), 0);

        let mut probe = tokio::spawn({
            let adapter = adapter.clone();
            async move { adapter.probe().await }
        });
        let mut catalog = tokio::spawn({
            let adapter = adapter.clone();
            async move {
                let request = CatalogRequest {
                    key: CatalogRequestKey {
                        connection: ConnectionIdentity {
                            profile_id: adapter.connection_id,
                            generation: 1,
                        },
                        catalog_epoch: 1,
                        request_id: 1,
                        target: CatalogTarget::Databases,
                        cursor: None,
                    },
                    scope: CatalogScope::for_profile(
                        DatabaseKind::Sqlite,
                        &adapter.database,
                        Some("main"),
                    ),
                    page_size: 10,
                };
                adapter.load_catalog_page(&request).await
            }
        });
        assert!(
            timeout(Duration::from_millis(20), &mut probe)
                .await
                .is_err()
        );
        assert!(
            timeout(Duration::from_millis(20), &mut catalog)
                .await
                .is_err()
        );

        backend.rollback().await.unwrap();
        drop(backend);

        timeout(Duration::from_secs(1), probe)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(1), catalog)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(adapter.operation_gate.available_permits(), 1);
        adapter.close().await;
    }

    #[tokio::test]
    async fn forced_close_invalidates_the_connection_local_pool() {
        let adapter = memory_adapter().await;
        let backend = adapter.transaction_backend().await.unwrap();

        backend.force_close().await.unwrap();

        assert!(adapter.pool.is_closed());
        assert!(adapter.probe().await.is_err());
    }

    #[tokio::test]
    async fn catalog_page_count_and_rows_share_one_file_snapshot() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("snapshot.db");
        let url = format!("sqlite://{}", path.display());
        let imported = import_connection_url(&url, Some("snapshot")).unwrap();
        let adapter = SqliteAdapter::connect(&imported.profile).await.unwrap();
        adapter
            .execute("CREATE TABLE initial_table (id INTEGER PRIMARY KEY)")
            .await
            .unwrap();

        let hook = PageSnapshotHook {
            count_complete: Arc::new(tokio::sync::Barrier::new(2)),
            continue_page: Arc::new(tokio::sync::Barrier::new(2)),
        };
        *adapter.page_snapshot_hook.lock().unwrap() = Some(hook.clone());
        let request = CatalogRequest {
            key: CatalogRequestKey {
                connection: ConnectionIdentity {
                    profile_id: imported.profile.id,
                    generation: 1,
                },
                catalog_epoch: 1,
                request_id: 1,
                target: CatalogTarget::objects(
                    CatalogId::new(
                        imported.profile.id,
                        CatalogKind::Schema,
                        [
                            imported.profile.database.clone().unwrap(),
                            "main".to_owned(),
                        ],
                    ),
                    ObjectGroup::Tables,
                )
                .unwrap(),
                cursor: None,
            },
            scope: CatalogScope::for_profile(
                imported.profile.kind,
                imported.profile.database.as_deref().unwrap(),
                Some("main"),
            ),
            page_size: 10,
        };
        let page_task = tokio::spawn({
            let adapter = adapter.clone();
            let request = request.clone();
            async move { adapter.load_catalog_page(&request).await }
        });
        hook.count_complete.wait().await;

        let mut external = SqliteConnection::connect(&url).await.unwrap();
        external.execute("PRAGMA busy_timeout = 0").await.unwrap();
        let concurrent_ddl = external
            .execute("CREATE TABLE concurrent_table (id INTEGER PRIMARY KEY)")
            .await;
        hook.continue_page.wait().await;
        let page = page_task.await.unwrap().unwrap();

        assert!(
            concurrent_ddl.is_err(),
            "external DDL must be locked while the catalog read snapshot is open"
        );
        assert_eq!(page.total_count, super::CatalogCount::Exact(1));
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].qualified_name.object, "initial_table");
        external
            .execute("CREATE TABLE concurrent_table (id INTEGER PRIMARY KEY)")
            .await
            .unwrap();
        *adapter.page_snapshot_hook.lock().unwrap() = None;
        let next_page = adapter.load_catalog_page(&request).await.unwrap();
        assert_eq!(next_page.total_count, super::CatalogCount::Exact(2));
        assert_eq!(next_page.entries.len(), 2);
        assert!(
            next_page
                .entries
                .iter()
                .any(|entry| entry.qualified_name.object == "concurrent_table")
        );
        adapter.close().await;
    }
}
