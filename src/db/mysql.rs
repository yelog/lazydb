use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use futures_util::TryStreamExt;
use secrecy::{ExposeSecret, SecretString};
use sqlx::{
    AssertSqlSafe, Column, Connection, Either, Executor, MySqlPool, Row, SqlSafeStr, Statement,
    TypeInfo, ValueRef,
    mysql::{
        MySql, MySqlConnectOptions, MySqlConnection, MySqlPoolOptions, MySqlRow, MySqlSslMode,
    },
    pool::PoolConnection,
};
use sqlx_core::transaction::TransactionManager;
use uuid::Uuid;

use crate::{
    identity::ConnectionIdentity,
    profile::{
        CatalogScope, CatalogSelection, ConnectionProfile, DatabaseKind, DatabaseScope, SslMode,
    },
};

use super::transaction::{TransactionBackend, TransactionError};
use super::{
    DatabaseError, ErrorCategory, ServerInfo,
    catalog::{
        CatalogCapabilities, CatalogCount, CatalogDiscovery, CatalogEntry, CatalogGroupSummary,
        CatalogId, CatalogKind, CatalogMetadata, CatalogPage, CatalogRequest, CatalogRequestKey,
        CatalogSearchHit, CatalogSearchPage, CatalogSearchRequest, CatalogTarget,
        CatalogValidationError, ColumnMetadata, ColumnMetadataCapabilities, ConstraintMembership,
        ConstraintMetadata, DdlProvenance, DiscoveredDatabase, IndexMetadata, NamespaceModel,
        ObjectGroup, OptionalMetadata, QualifiedName, RelationDdl, finalize_keyset_page,
    },
    catalog_drop::{CatalogDropError, CatalogDropPlan, CatalogDropRequest},
    ddl::{DdlSection, assemble_ddl},
    mutation::{InputValue, MutationResult, RelationMutation, RelationMutationRequest},
    query::{ColumnMeta, QueryOutcome, QueryOutcomeAccumulator, RELATION_PREVIEW_LIMIT, ResultSet},
    sanitize_terminal_text,
    value::CellValue,
};

pub const CATALOG_TABLES_SQL: &str = r#"
SELECT table_schema, table_name, table_type
FROM information_schema.tables
WHERE table_schema = ?
ORDER BY table_name
"#;

pub const CATALOG_ROUTINES_SQL: &str = r#"
SELECT routine_schema, routine_name, routine_type, data_type, dtd_identifier
FROM information_schema.routines
WHERE routine_schema = ?
ORDER BY routine_name, routine_type
"#;

pub const CATALOG_INDEXES_SQL: &str = r#"
SELECT table_schema, table_name, index_name, non_unique, seq_in_index, column_name
FROM information_schema.statistics
WHERE table_schema = ?
ORDER BY table_name, index_name, seq_in_index
"#;

pub const CATALOG_PAGE_INDEXES_SQL: &str = r#"
SELECT index_name, non_unique, seq_in_index, column_name, expression
FROM information_schema.statistics
WHERE BINARY table_schema=BINARY ? AND BINARY table_name=BINARY ?
ORDER BY BINARY index_name, seq_in_index
"#;

const PROBE_SQL: &str = "SELECT VERSION() AS version, DATABASE() AS current_database";

pub const CATALOG_PAGE_BEGIN_SQL: &str = "START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY";

pub const CATALOG_SEARCH_CANDIDATES_SQL: &str = r#"
WITH candidates AS (
    SELECT 'database' AS kind, schema_name AS database_name, schema_name AS object_name,
           NULL AS relation_name, NULL AS relation_type, schema_name AS native_identity,
           schema_name AS qualified_path, NULL AS comment
    FROM information_schema.schemata
    WHERE schema_name NOT IN ('information_schema','mysql','performance_schema','sys')
    UNION ALL
    SELECT 'schema', schema_name, schema_name, NULL, NULL, schema_name, schema_name, NULL
    FROM information_schema.schemata
    WHERE schema_name NOT IN ('information_schema','mysql','performance_schema','sys')
    UNION ALL
    SELECT IF(table_type='VIEW','view','table'), table_schema, table_name, table_name, table_type,
           table_name, CONCAT(table_schema,'.',table_name), table_comment
    FROM information_schema.tables WHERE table_type IN ('BASE TABLE','VIEW')
    UNION ALL
    SELECT LOWER(routine_type), routine_schema, routine_name, NULL, NULL, specific_name,
           CONCAT(routine_schema,'.',routine_name), routine_comment
    FROM information_schema.routines WHERE routine_type IN ('FUNCTION','PROCEDURE')
    UNION ALL
    SELECT 'trigger', tr.trigger_schema, tr.trigger_name, tr.event_object_table, t.table_type,
           tr.trigger_name, CONCAT(tr.trigger_schema,'.',tr.event_object_table,'.',tr.trigger_name), NULL
    FROM information_schema.triggers tr JOIN information_schema.tables t
      ON BINARY t.table_schema=BINARY tr.event_object_schema
     AND BINARY t.table_name=BINARY tr.event_object_table
     AND t.table_type IN ('BASE TABLE','VIEW')
    UNION ALL
    SELECT 'column', c.table_schema, c.column_name, c.table_name, t.table_type,
           CAST(c.ordinal_position AS CHAR), CONCAT(c.table_schema,'.',c.table_name,'.',c.column_name), NULL
    FROM information_schema.columns c JOIN information_schema.tables t
      ON BINARY t.table_schema=BINARY c.table_schema AND BINARY t.table_name=BINARY c.table_name
     AND t.table_type IN ('BASE TABLE','VIEW')
    UNION ALL
    SELECT 'index', s.table_schema, s.index_name, s.table_name, t.table_type, s.index_name,
           CONCAT(s.table_schema,'.',s.table_name,'.',s.index_name), NULL
    FROM information_schema.statistics s JOIN information_schema.tables t
      ON BINARY t.table_schema=BINARY s.table_schema AND BINARY t.table_name=BINARY s.table_name
     AND t.table_type IN ('BASE TABLE','VIEW')
    GROUP BY s.table_schema, s.table_name, s.index_name, t.table_type
    UNION ALL
    SELECT CASE constraint_type WHEN 'PRIMARY KEY' THEN 'primary_key'
               WHEN 'UNIQUE' THEN 'unique_constraint' ELSE 'foreign_key' END,
           tc.table_schema, tc.constraint_name, tc.table_name, t.table_type, tc.constraint_name,
           CONCAT(tc.table_schema,'.',tc.table_name,'.',tc.constraint_name), NULL
    FROM information_schema.table_constraints tc JOIN information_schema.tables t
      ON BINARY t.table_schema=BINARY tc.table_schema AND BINARY t.table_name=BINARY tc.table_name
     AND t.table_type IN ('BASE TABLE','VIEW')
    WHERE tc.constraint_type IN ('PRIMARY KEY','UNIQUE','FOREIGN KEY')
)
SELECT kind, database_name, object_name, relation_name, relation_type, native_identity, comment
FROM candidates
WHERE {scope_predicate}
  AND database_name NOT IN ('information_schema','mysql','performance_schema','sys')
  AND (LOCATE(LOWER(?), LOWER(object_name)) > 0
       OR LOCATE(LOWER(?), LOWER(qualified_path)) > 0)
ORDER BY CASE
    WHEN LOWER(object_name)=LOWER(?) THEN 0
    WHEN LOCATE(LOWER(?), LOWER(object_name))=1 THEN 1
    WHEN LOCATE(LOWER(?), LOWER(object_name))>0 THEN 2
    ELSE 3 END,
    LOWER(qualified_path), kind, BINARY native_identity
LIMIT 101
"#;

pub const CATALOG_DATABASES_SQL: &str = r#"
SELECT schema_name
FROM information_schema.schemata
WHERE schema_name NOT IN ('information_schema', 'mysql', 'performance_schema', 'sys')
ORDER BY BINARY schema_name
"#;

#[derive(Clone, Debug)]
pub struct MySqlAdapter {
    pool: MySqlPool,
    connection_id: Uuid,
    catalog_scope: CatalogScope,
}

#[derive(Debug)]
struct MySqlSearchCandidate {
    kind: CatalogKind,
    database: String,
    name: String,
    relation_name: Option<String>,
    relation_type: Option<String>,
    native_identity: String,
    comment: Option<String>,
}

#[derive(Debug)]
struct MySqlHydratedRelation {
    entry: CatalogEntry,
    children: Option<Vec<CatalogEntry>>,
}

impl MySqlSearchCandidate {
    fn try_from_row(row: MySqlRow) -> Result<Self, DatabaseError> {
        let native_kind: String = row.try_get("kind").map_err(decode_error)?;
        Ok(Self {
            kind: search_catalog_kind(&native_kind)?,
            database: row.try_get("database_name").map_err(decode_error)?,
            name: row.try_get("object_name").map_err(decode_error)?,
            relation_name: row.try_get("relation_name").map_err(decode_error)?,
            relation_type: row.try_get("relation_type").map_err(decode_error)?,
            native_identity: row.try_get("native_identity").map_err(decode_error)?,
            comment: row.try_get("comment").map_err(decode_error)?,
        })
    }

    fn id(&self, connection_id: Uuid) -> CatalogId {
        let mut path = match self.kind {
            CatalogKind::Database => vec![self.database.clone()],
            CatalogKind::Schema => vec![self.database.clone(), self.database.clone()],
            _ => vec![
                self.database.clone(),
                self.database.clone(),
                self.name.clone(),
            ],
        };
        if matches!(self.kind, CatalogKind::Function | CatalogKind::Procedure) {
            path.push(self.native_identity.clone());
        }
        CatalogId::new(connection_id, self.kind, path)
    }
}

impl MySqlAdapter {
    pub fn plan_catalog_drop(
        request: CatalogDropRequest,
        entry: &CatalogEntry,
    ) -> Result<CatalogDropPlan, CatalogDropError> {
        let sql = match entry.kind {
            CatalogKind::Database | CatalogKind::Schema => {
                format!("DROP DATABASE {}", mysql_namespace_name(entry)?)
            }
            CatalogKind::Table => format!("DROP TABLE {}", mysql_relation_name(entry)?),
            CatalogKind::View => format!("DROP VIEW {}", mysql_relation_name(entry)?),
            CatalogKind::Index | CatalogKind::UniqueConstraint => format!(
                "ALTER TABLE {} DROP INDEX {}",
                mysql_relation_owner(entry)?,
                quote_identifier(&entry.qualified_name.object)
            ),
            CatalogKind::PrimaryKey => format!(
                "ALTER TABLE {} DROP PRIMARY KEY",
                mysql_relation_owner(entry)?
            ),
            CatalogKind::ForeignKey => format!(
                "ALTER TABLE {} DROP FOREIGN KEY {}",
                mysql_relation_owner(entry)?,
                quote_identifier(&entry.qualified_name.object)
            ),
            CatalogKind::Trigger => format!("DROP TRIGGER {}", mysql_trigger_name(entry)?),
            CatalogKind::Function => format!("DROP FUNCTION {}", mysql_routine_name(entry)?),
            CatalogKind::Procedure => format!("DROP PROCEDURE {}", mysql_routine_name(entry)?),
            kind => {
                return Err(CatalogDropError::Unsupported {
                    kind,
                    reason: "MySQL catalog metadata does not provide an unambiguous drop target"
                        .to_owned(),
                });
            }
        };
        CatalogDropPlan::new(request, entry, sql)
    }

    pub fn catalog_capabilities() -> CatalogCapabilities {
        CatalogCapabilities {
            namespace_model: NamespaceModel::DatabaseIsSchema,
            top_level_groups: vec![
                ObjectGroup::Tables,
                ObjectGroup::Views,
                ObjectGroup::Functions,
                ObjectGroup::Procedures,
                ObjectGroup::Triggers,
            ],
            column_metadata: ColumnMetadataCapabilities {
                type_family: true,
                default_expression: true,
                auto_increment: true,
                generated_expression: true,
                numeric_precision_and_scale: true,
                character_length: true,
                collation: true,
                character_set: true,
                comment: true,
                ..ColumnMetadataCapabilities::default()
            },
            supports_lazy_children: true,
        }
    }

    pub(crate) async fn transaction_backend(
        &self,
    ) -> Result<MySqlTransactionBackend, DatabaseError> {
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        let connection_id = sqlx::query_scalar::<_, u64>("SELECT CONNECTION_ID()")
            .fetch_one(&mut *connection)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        Ok(MySqlTransactionBackend {
            connection,
            control: self.pool.clone(),
            connection_id,
            adapter: self.clone(),
        })
    }

    pub async fn connect(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
    ) -> Result<Self, DatabaseError> {
        if profile.kind != DatabaseKind::MySql {
            return Err(DatabaseError::configuration("profile is not MySQL"));
        }
        let host = profile
            .host
            .as_deref()
            .ok_or_else(|| DatabaseError::configuration("MySQL profile has no host"))?;
        let mut options = MySqlConnectOptions::new()
            .host(host)
            .port(profile.port.unwrap_or(3306))
            .ssl_mode(mysql_ssl_mode(profile.ssl_mode));
        if let Some(user) = &profile.user {
            options = options.username(user);
        }
        if let Some(database) = &profile.database {
            options = options.database(database);
        }
        if let Some(password) = password {
            options = options.password(password.expose_secret());
        }

        let mut pool_options = MySqlPoolOptions::new()
            .max_connections(6)
            .acquire_timeout(Duration::from_secs(10));
        if profile.read_only {
            pool_options = pool_options.after_connect(|connection, _| {
                Box::pin(async move {
                    connection
                        .execute("SET SESSION TRANSACTION READ ONLY")
                        .await?;
                    Ok(())
                })
            });
        }
        let pool = pool_options
            .connect_with(options)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        Ok(Self {
            pool,
            connection_id: profile.id,
            catalog_scope: profile.catalog_scope.clone(),
        })
    }

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        let row = sqlx::query(PROBE_SQL)
            .fetch_one(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        Ok(ServerInfo {
            kind: DatabaseKind::MySql,
            version: row.try_get("version").map_err(decode_error)?,
            database: row
                .try_get::<Option<String>, _>("current_database")
                .map_err(decode_error)?
                .unwrap_or_default(),
        })
    }

    pub async fn discover_catalog_scope(&self) -> Result<CatalogDiscovery, DatabaseError> {
        let databases = sqlx::query_scalar::<_, String>(CATALOG_DATABASES_SQL)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?
            .into_iter()
            .map(|name| DiscoveredDatabase {
                schemas: vec![name.clone()],
                name,
            })
            .collect();

        Ok(CatalogDiscovery {
            databases,
            warnings: Vec::new(),
        })
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
        connection: &mut MySqlConnection,
        sql: &str,
    ) -> Result<QueryOutcome, DatabaseError> {
        let mut stream = sqlx::raw_sql(AssertSqlSafe(sql)).fetch_many(&mut *connection);
        self.collect_stream(&mut stream).await
    }

    async fn collect_stream<E>(&self, stream: &mut E) -> Result<QueryOutcome, DatabaseError>
    where
        E: futures_util::TryStream<
                Ok = Either<sqlx::mysql::MySqlQueryResult, MySqlRow>,
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
        validate_catalog_scope(&request.scope)?;
        if matches!(
            request.key.target,
            CatalogTarget::Objects {
                group: ObjectGroup::MaterializedViews | ObjectGroup::Sequences | ObjectGroup::Types,
                ..
            }
        ) {
            return Err(DatabaseError::unsupported_catalog_target(
                DatabaseKind::MySql,
                &request.key.target,
            ));
        }

        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        let version: String = sqlx::query_scalar("SELECT VERSION()")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        if !supports_catalog_version(&version) {
            return Err(unsupported_catalog_version(&version));
        }
        let mut transaction = connection
            .begin_with(CATALOG_PAGE_BEGIN_SQL)
            .await
            .map_err(sql_error)?;
        let lower_case_table_names: i64 =
            sqlx::query_scalar::<_, String>("SELECT CAST(@@lower_case_table_names AS CHAR)")
                .fetch_one(&mut *transaction)
                .await
                .map_err(sql_error)?
                .parse()
                .map_err(|_| {
                    catalog_internal("MySQL returned an invalid lower_case_table_names value")
                })?;
        if !(0..=2).contains(&lower_case_table_names) {
            return Err(catalog_internal(format!(
                "MySQL returned unsupported lower_case_table_names value {lower_case_table_names}"
            )));
        }
        let page = match &request.key.target {
            CatalogTarget::Databases => self.load_database_page(&mut transaction, request).await,
            CatalogTarget::Schemas { database } => {
                self.load_schema_page(&mut transaction, request, database, lower_case_table_names)
                    .await
            }
            CatalogTarget::Groups { schema } => {
                self.load_group_page(&mut transaction, request, schema, lower_case_table_names)
                    .await
            }
            CatalogTarget::Objects { schema, group } => {
                self.load_object_page(
                    &mut transaction,
                    request,
                    schema,
                    *group,
                    lower_case_table_names,
                )
                .await
            }
            CatalogTarget::RelationChildren { relation } => {
                self.load_relation_children_page(
                    &mut transaction,
                    request,
                    relation,
                    lower_case_table_names,
                )
                .await
            }
        };
        match page {
            Ok(page) => {
                transaction.commit().await.map_err(sql_error)?;
                Ok(page)
            }
            Err(page_error) => {
                let _ = transaction.rollback().await;
                Err(page_error)
            }
        }
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
        validate_catalog_scope(&request.scope)?;

        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        let version: String = sqlx::query_scalar("SELECT VERSION()")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        if !supports_catalog_version(&version) {
            return Err(unsupported_catalog_version(&version));
        }
        let mut transaction = connection
            .begin_with(CATALOG_PAGE_BEGIN_SQL)
            .await
            .map_err(sql_error)?;
        let lower_case_table_names: i64 =
            sqlx::query_scalar::<_, String>("SELECT CAST(@@lower_case_table_names AS CHAR)")
                .fetch_one(&mut *transaction)
                .await
                .map_err(sql_error)?
                .parse()
                .map_err(|_| {
                    catalog_internal("MySQL returned an invalid lower_case_table_names value")
                })?;
        if !(0..=2).contains(&lower_case_table_names) {
            return Err(catalog_internal(format!(
                "MySQL returned unsupported lower_case_table_names value {lower_case_table_names}"
            )));
        }
        let result = self
            .search_catalog_snapshot(&mut transaction, request)
            .await;
        match result {
            Ok(page) => {
                transaction.commit().await.map_err(sql_error)?;
                Ok(page)
            }
            Err(error) => {
                let _ = transaction.rollback().await;
                Err(error)
            }
        }
    }

    async fn search_catalog_snapshot(
        &self,
        connection: &mut MySqlConnection,
        request: &CatalogSearchRequest,
    ) -> Result<CatalogSearchPage, DatabaseError> {
        let selected = selected_search_databases(&request.scope);
        let scope_predicate = selected
            .as_ref()
            .map(|databases| {
                if databases.is_empty() {
                    "FALSE".to_owned()
                } else {
                    format!(
                        "BINARY database_name IN ({})",
                        placeholders(databases.len())
                    )
                }
            })
            .unwrap_or_else(|| "TRUE".to_owned());
        let sql = CATALOG_SEARCH_CANDIDATES_SQL.replace("{scope_predicate}", &scope_predicate);
        let mut query = sqlx::query(AssertSqlSafe(sql));
        if let Some(databases) = selected.as_ref() {
            for database in databases {
                query = query.bind(database);
            }
        }
        for _ in 0..5 {
            query = query.bind(&request.query);
        }
        let rows = query.fetch_all(&mut *connection).await.map_err(sql_error)?;
        let candidates = rows
            .into_iter()
            .map(MySqlSearchCandidate::try_from_row)
            .collect::<Result<Vec<_>, _>>()?;

        let mut relation_cache = HashMap::<(String, String), MySqlHydratedRelation>::new();
        for candidate in &candidates {
            if !candidate.kind.is_relation_child() && candidate.kind != CatalogKind::Trigger {
                continue;
            }
            let relation_name = candidate
                .relation_name
                .as_ref()
                .ok_or_else(|| catalog_internal("MySQL search candidate has no owner"))?;
            let key = (candidate.database.clone(), relation_name.clone());
            if let Some(relation) = relation_cache.get(&key) {
                if candidate.kind.is_relation_child() && relation.children.is_none() {
                    let loaded = self
                        .load_relation_children(
                            connection,
                            &candidate.database,
                            relation_name,
                            &relation.entry.id,
                        )
                        .await?;
                    relation_cache
                        .get_mut(&key)
                        .expect("cached relation")
                        .children = Some(loaded);
                }
                continue;
            }
            let relation = self.relation_entry(
                &candidate.database,
                relation_name,
                candidate.relation_type.as_deref(),
                None,
            )?;
            let children = if candidate.kind.is_relation_child() {
                Some(
                    self.load_relation_children(
                        connection,
                        &candidate.database,
                        relation_name,
                        &relation.id,
                    )
                    .await?,
                )
            } else {
                None
            };
            relation_cache.insert(
                key,
                MySqlHydratedRelation {
                    entry: relation,
                    children,
                },
            );
        }

        let mut hits = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            hits.push(self.hydrate_search_candidate(candidate, &relation_cache)?);
        }
        hits.dedup_by(|left, right| left.entry.id == right.entry.id);
        let truncated = hits.len() > request.limit;
        hits.truncate(request.limit);
        CatalogSearchPage::new(request, hits, None, truncated)
            .map_err(DatabaseError::invalid_catalog_request)
    }

    fn hydrate_search_candidate(
        &self,
        candidate: MySqlSearchCandidate,
        relation_cache: &HashMap<(String, String), MySqlHydratedRelation>,
    ) -> Result<CatalogSearchHit, DatabaseError> {
        let database = self.database_entry(&candidate.database)?;
        if candidate.kind == CatalogKind::Database {
            return Ok(CatalogSearchHit {
                entry: database,
                ancestors: Vec::new(),
            });
        }
        let schema = self.schema_entry(&candidate.database)?;
        let mut ancestors = vec![database, schema.clone()];
        let entry = if candidate.kind == CatalogKind::Schema {
            ancestors.pop();
            schema
        } else if candidate.kind.is_relation() {
            self.relation_entry(
                &candidate.database,
                &candidate.name,
                candidate.relation_type.as_deref(),
                candidate.comment,
            )?
        } else if let Some(relation_name) = candidate.relation_name.as_ref() {
            let relation = relation_cache
                .get(&(candidate.database.clone(), relation_name.clone()))
                .ok_or_else(|| catalog_internal("MySQL search relation was not hydrated"))?;
            if candidate.kind == CatalogKind::Trigger {
                ancestors.push(relation.entry.clone());
                CatalogEntry::relation_object(
                    candidate.id(self.connection_id),
                    schema.id,
                    relation.entry.id.clone(),
                    qualified_object(&candidate.database, &candidate.name),
                    "trigger",
                    OptionalMetadata::Unsupported,
                )
                .map_err(catalog_invariant)?
            } else if candidate.kind.is_relation_child() {
                ancestors.push(relation.entry.clone());
                relation
                    .children
                    .as_ref()
                    .ok_or_else(|| catalog_internal("MySQL search child metadata was not loaded"))?
                    .iter()
                    .find(|entry| {
                        entry.kind == candidate.kind
                            && entry.id.native_path.last() == Some(&candidate.native_identity)
                    })
                    .cloned()
                    .ok_or_else(|| catalog_internal("MySQL search child was not hydrated"))?
            } else {
                relation.entry.clone()
            }
        } else {
            CatalogEntry::object(
                candidate.id(self.connection_id),
                schema.id,
                qualified_object(&candidate.database, &candidate.name),
                search_native_kind(candidate.kind),
                OptionalMetadata::Supported(empty_as_none(candidate.comment)),
                false,
            )
            .map_err(catalog_invariant)?
        };
        Ok(CatalogSearchHit { entry, ancestors })
    }

    fn database_entry(&self, database: &str) -> Result<CatalogEntry, DatabaseError> {
        CatalogEntry::database(
            CatalogId::new(self.connection_id, CatalogKind::Database, [database]),
            qualified_database(database),
            "database",
            OptionalMetadata::Unsupported,
            true,
        )
        .map_err(catalog_invariant)
    }

    fn schema_entry(&self, database: &str) -> Result<CatalogEntry, DatabaseError> {
        CatalogEntry::schema(
            CatalogId::new(
                self.connection_id,
                CatalogKind::Schema,
                [database, database],
            ),
            CatalogId::new(self.connection_id, CatalogKind::Database, [database]),
            qualified_schema(database),
            "schema",
            OptionalMetadata::Unsupported,
            true,
        )
        .map_err(catalog_invariant)
    }

    fn relation_entry(
        &self,
        database: &str,
        name: &str,
        table_type: Option<&str>,
        comment: Option<String>,
    ) -> Result<CatalogEntry, DatabaseError> {
        let kind = relation_kind(table_type)?;
        CatalogEntry::relation(
            CatalogId::new(self.connection_id, kind, [database, database, name]),
            CatalogId::new(
                self.connection_id,
                CatalogKind::Schema,
                [database, database],
            ),
            qualified_object(database, name),
            table_type.unwrap_or("BASE TABLE"),
            OptionalMetadata::Supported(empty_as_none(comment)),
            true,
        )
        .map_err(catalog_invariant)
    }

    async fn load_database_page(
        &self,
        connection: &mut MySqlConnection,
        request: &CatalogRequest,
    ) -> Result<CatalogPage, DatabaseError> {
        let selected = selected_databases(request);
        let scope_predicate = selected
            .as_ref()
            .map(|databases| {
                if databases.is_empty() {
                    " AND FALSE".to_owned()
                } else {
                    format!(
                        " AND BINARY schema_name IN ({})",
                        placeholders(databases.len())
                    )
                }
            })
            .unwrap_or_default();
        let count_sql = format!(
            "SELECT CAST(COUNT(*) AS SIGNED) FROM information_schema.schemata \
             WHERE schema_name NOT IN ('information_schema','mysql','performance_schema','sys'){scope_predicate}"
        );
        let mut count_query = sqlx::query_scalar::<_, i64>(AssertSqlSafe(count_sql));
        if let Some(databases) = selected.as_ref() {
            for database in databases {
                count_query = count_query.bind(database);
            }
        }
        let total_count = CatalogCount::Exact(non_negative_count(
            count_query
                .fetch_one(&mut *connection)
                .await
                .map_err(sql_error)?,
        )?);
        let cursor = request_cursor(request)?;
        let cursor_predicate = if cursor.is_some() {
            " AND (BINARY schema_name > BINARY ? OR (BINARY schema_name = BINARY ? AND BINARY schema_name > BINARY ?))"
        } else {
            ""
        };
        let page_sql = format!(
            "SELECT schema_name FROM information_schema.schemata \
             WHERE schema_name NOT IN ('information_schema','mysql','performance_schema','sys'){scope_predicate}{cursor_predicate} \
             ORDER BY BINARY schema_name LIMIT ?"
        );
        let mut page_query = sqlx::query(AssertSqlSafe(page_sql));
        if let Some(databases) = selected.as_ref() {
            for database in databases {
                page_query = page_query.bind(database);
            }
        }
        if let Some((sort_key, tie_breaker)) = cursor {
            page_query = page_query.bind(sort_key).bind(sort_key).bind(tie_breaker);
        }
        let rows = page_query
            .bind(page_limit(request.page_size)?)
            .fetch_all(&mut *connection)
            .await
            .map_err(sql_error)?;
        let mut entries = rows
            .into_iter()
            .map(|row| {
                let name: String = row.try_get(0).map_err(decode_error)?;
                CatalogEntry::database(
                    CatalogId::new(self.connection_id, CatalogKind::Database, [name.clone()]),
                    qualified_database(&name),
                    "database",
                    OptionalMetadata::Unsupported,
                    true,
                )
                .map_err(catalog_invariant)
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        let next_cursor = finalize_keyset_page(
            &mut entries,
            request.page_size,
            |entry| entry.qualified_name.object.clone(),
            |entry| entry.qualified_name.object.clone(),
        )
        .map_err(catalog_invariant)?;
        CatalogPage::new(request, entries, total_count, next_cursor).map_err(catalog_invariant)
    }

    async fn load_schema_page(
        &self,
        connection: &mut MySqlConnection,
        request: &CatalogRequest,
        database_id: &CatalogId,
        lower_case_table_names: i64,
    ) -> Result<CatalogPage, DatabaseError> {
        let database = self
            .verify_database(
                connection,
                database_id,
                &request.key.target,
                lower_case_table_names,
            )
            .await?;
        let mut entries = vec![
            CatalogEntry::schema(
                CatalogId::new(
                    self.connection_id,
                    CatalogKind::Schema,
                    [database.clone(), database.clone()],
                ),
                CatalogId::new(
                    self.connection_id,
                    CatalogKind::Database,
                    [database.clone()],
                ),
                qualified_schema(&database),
                "schema",
                OptionalMetadata::Unsupported,
                true,
            )
            .map_err(catalog_invariant)?,
        ];
        let total_count = CatalogCount::Exact(1);
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
        connection: &mut MySqlConnection,
        request: &CatalogRequest,
        schema_id: &CatalogId,
        lower_case_table_names: i64,
    ) -> Result<CatalogPage, DatabaseError> {
        let database = self
            .verify_schema(
                connection,
                schema_id,
                &request.key.target,
                lower_case_table_names,
            )
            .await?;
        let row = sqlx::query(
            "SELECT \
             (SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.tables WHERE BINARY table_schema=BINARY ? AND table_type='BASE TABLE') AS tables, \
             (SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.tables WHERE BINARY table_schema=BINARY ? AND table_type='VIEW') AS views, \
             (SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.routines WHERE BINARY routine_schema=BINARY ? AND routine_type='FUNCTION') AS functions, \
             (SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.routines WHERE BINARY routine_schema=BINARY ? AND routine_type='PROCEDURE') AS procedures, \
             (SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.triggers WHERE BINARY trigger_schema=BINARY ?) AS triggers",
        )
        .bind(&database)
        .bind(&database)
        .bind(&database)
        .bind(&database)
        .bind(&database)
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
        let mut summaries = Vec::new();
        for (group, column) in [
            (ObjectGroup::Tables, "tables"),
            (ObjectGroup::Views, "views"),
            (ObjectGroup::Functions, "functions"),
            (ObjectGroup::Procedures, "procedures"),
            (ObjectGroup::Triggers, "triggers"),
        ] {
            summaries.push(CatalogGroupSummary {
                group,
                object_count: CatalogCount::Exact(
                    row.try_get::<String, _>(column)
                        .map_err(decode_error)?
                        .parse::<u64>()
                        .map_err(|_| catalog_internal("MySQL returned an invalid catalog count"))?,
                ),
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
        connection: &mut MySqlConnection,
        request: &CatalogRequest,
        schema_id: &CatalogId,
        group: ObjectGroup,
        lower_case_table_names: i64,
    ) -> Result<CatalogPage, DatabaseError> {
        let database = self
            .verify_schema(
                connection,
                schema_id,
                &request.key.target,
                lower_case_table_names,
            )
            .await?;
        let (source, schema_column, name_column, predicate, kind, native_kind, tie_column) =
            match group {
                ObjectGroup::Tables => (
                    "information_schema.tables",
                    "table_schema",
                    "table_name",
                    "table_type='BASE TABLE'",
                    CatalogKind::Table,
                    "table",
                    "table_name",
                ),
                ObjectGroup::Views => (
                    "information_schema.tables",
                    "table_schema",
                    "table_name",
                    "table_type='VIEW'",
                    CatalogKind::View,
                    "view",
                    "table_name",
                ),
                ObjectGroup::Functions => (
                    "information_schema.routines",
                    "routine_schema",
                    "routine_name",
                    "routine_type='FUNCTION'",
                    CatalogKind::Function,
                    "function",
                    "specific_name",
                ),
                ObjectGroup::Procedures => (
                    "information_schema.routines",
                    "routine_schema",
                    "routine_name",
                    "routine_type='PROCEDURE'",
                    CatalogKind::Procedure,
                    "procedure",
                    "specific_name",
                ),
                ObjectGroup::Triggers => (
                    "information_schema.triggers",
                    "trigger_schema",
                    "trigger_name",
                    "TRUE",
                    CatalogKind::Trigger,
                    "trigger",
                    "trigger_name",
                ),
                _ => {
                    return Err(DatabaseError::unsupported_catalog_target(
                        DatabaseKind::MySql,
                        &request.key.target,
                    ));
                }
            };
        let count_sql = format!(
            "SELECT CAST(COUNT(*) AS SIGNED) FROM {source} WHERE BINARY {schema_column}=BINARY ? AND {predicate}"
        );
        let count: i64 = sqlx::query_scalar(AssertSqlSafe(count_sql))
            .bind(&database)
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        let total_count = CatalogCount::Exact(non_negative_count(count)?);
        let cursor = request_cursor(request)?;
        let extra_columns = match group {
            ObjectGroup::Tables | ObjectGroup::Views => {
                "table_comment AS comment, NULL AS owner_name"
            }
            ObjectGroup::Triggers => "NULL AS comment, event_object_table AS owner_name",
            _ => "routine_comment AS comment, NULL AS owner_name",
        };
        let select = format!(
            "SELECT {name_column} AS name, {tie_column} AS native_identity, {extra_columns} \
             FROM {source} WHERE BINARY {schema_column}=BINARY ? AND {predicate}"
        );
        let rows = if let Some((sort_key, tie_breaker)) = cursor {
            let sql = format!(
                "{select} AND (BINARY {name_column} > BINARY ? OR (BINARY {name_column} = BINARY ? AND BINARY {tie_column} > BINARY ?)) \
                 ORDER BY BINARY {name_column}, BINARY {tie_column} LIMIT ?"
            );
            sqlx::query(AssertSqlSafe(sql))
                .bind(&database)
                .bind(sort_key)
                .bind(sort_key)
                .bind(tie_breaker)
                .bind(page_limit(request.page_size)?)
                .fetch_all(&mut *connection)
                .await
        } else {
            let sql = format!(
                "{select} ORDER BY BINARY {name_column}, BINARY {tie_column} LIMIT ?"
            );
            sqlx::query(AssertSqlSafe(sql))
                .bind(&database)
                .bind(page_limit(request.page_size)?)
                .fetch_all(&mut *connection)
                .await
        }
        .map_err(sql_error)?;
        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let name: String = row.try_get(0).map_err(decode_error)?;
            let native_identity: String = row.try_get(1).map_err(decode_error)?;
            let comment = empty_as_none(row.try_get(2).map_err(decode_error)?);
            let mut path = vec![database.clone(), database.clone(), name.clone()];
            if matches!(kind, CatalogKind::Function | CatalogKind::Procedure) {
                path.push(native_identity);
            }
            let id = CatalogId::new(self.connection_id, kind, path);
            let entry = if kind == CatalogKind::Trigger {
                let owner: String = row.try_get(3).map_err(decode_error)?;
                let (owner_name, owner_kind) = self
                    .verify_relation_name(connection, &database, &owner, lower_case_table_names)
                    .await?;
                CatalogEntry::relation_object(
                    id,
                    schema_id.clone(),
                    CatalogId::new(
                        self.connection_id,
                        owner_kind,
                        [database.clone(), database.clone(), owner_name],
                    ),
                    qualified_object(&database, &name),
                    native_kind,
                    OptionalMetadata::Unsupported,
                )
            } else if kind.is_relation() {
                CatalogEntry::relation(
                    id,
                    schema_id.clone(),
                    qualified_object(&database, &name),
                    native_kind,
                    OptionalMetadata::Supported(comment),
                    true,
                )
            } else {
                CatalogEntry::object(
                    id,
                    schema_id.clone(),
                    qualified_object(&database, &name),
                    native_kind,
                    OptionalMetadata::Supported(comment),
                    false,
                )
            }
            .map_err(catalog_invariant)?;
            entries.push(entry);
        }
        let next_cursor = finalize_keyset_page(
            &mut entries,
            request.page_size,
            |entry| entry.qualified_name.object.clone(),
            |entry| entry.id.native_path.last().cloned().unwrap_or_default(),
        )
        .map_err(catalog_invariant)?;
        CatalogPage::new(request, entries, total_count, next_cursor).map_err(catalog_invariant)
    }

    async fn load_relation_children_page(
        &self,
        connection: &mut MySqlConnection,
        request: &CatalogRequest,
        relation: &CatalogId,
        lower_case_table_names: i64,
    ) -> Result<CatalogPage, DatabaseError> {
        let (database, relation_name, _) = self
            .verify_relation(
                connection,
                relation,
                &request.key.target,
                lower_case_table_names,
            )
            .await?;
        let mut entries = self
            .load_relation_children(connection, &database, &relation_name, relation)
            .await?;
        let total_count = exact_count(entries.len())?;
        let next_cursor =
            paginate_in_memory(&mut entries, request, child_sort_key, child_tie_breaker)?;
        CatalogPage::new(request, entries, total_count, next_cursor).map_err(catalog_invariant)
    }

    async fn load_relation_children(
        &self,
        connection: &mut MySqlConnection,
        database: &str,
        relation_name: &str,
        relation: &CatalogId,
    ) -> Result<Vec<CatalogEntry>, DatabaseError> {
        let indexes = self
            .load_index_metadata(connection, database, relation_name)
            .await?;
        let constraints = self
            .load_constraint_metadata(connection, database, relation_name)
            .await?;
        let mut memberships: HashMap<String, Vec<ConstraintMembership>> = HashMap::new();
        let mut entries = Vec::new();

        for index in indexes {
            entries.push(
                CatalogEntry::relation_child(
                    relation_child_id(relation, CatalogKind::Index, &index.name),
                    relation.clone(),
                    qualified_object(database, &index.name),
                    "index",
                    OptionalMetadata::Unsupported,
                    CatalogMetadata::Index(IndexMetadata {
                        columns: index.columns,
                        unique: index.unique,
                    }),
                )
                .map_err(catalog_invariant)?,
            );
        }
        for constraint in constraints {
            let id = relation_child_id(relation, constraint.kind, &constraint.name);
            add_memberships(&mut memberships, &constraint.columns, &id)?;
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
                    let referenced_database = constraint.referenced_database.ok_or_else(|| {
                        catalog_internal("foreign key has no referenced database")
                    })?;
                    CatalogMetadata::Constraint(ConstraintMetadata::ForeignKey {
                        columns: constraint.columns,
                        referenced_relation: QualifiedName {
                            database: Some(referenced_database.clone()),
                            schema: Some(referenced_database),
                            object: constraint.referenced_relation.ok_or_else(|| {
                                catalog_internal("foreign key has no referenced relation")
                            })?,
                        },
                        referenced_columns: constraint.referenced_columns,
                    })
                }
                _ => return Err(catalog_internal("unexpected MySQL constraint kind")),
            };
            entries.push(
                CatalogEntry::relation_child(
                    id,
                    relation.clone(),
                    qualified_object(database, &constraint.name),
                    "constraint",
                    OptionalMetadata::Unsupported,
                    metadata,
                )
                .map_err(catalog_invariant)?,
            );
        }

        let rows = sqlx::query(
            "SELECT ordinal_position, column_name, column_type, data_type, is_nullable, \
             column_default, extra, generation_expression, numeric_precision, numeric_scale, \
             character_maximum_length, collation_name, character_set_name, column_comment \
             FROM information_schema.columns WHERE BINARY table_schema=BINARY ? AND BINARY table_name=BINARY ? \
             ORDER BY ordinal_position",
        )
        .bind(database)
        .bind(relation_name)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        for row in rows {
            let ordinal = checked_u32(
                row.try_get::<u64, _>(0).map_err(decode_error)?,
                "column ordinal",
            )?;
            let name: String = row.try_get(1).map_err(decode_error)?;
            let extra: String = row.try_get(6).map_err(decode_error)?;
            let generation_expression: String = row.try_get(7).map_err(decode_error)?;
            let generated = !generation_expression.is_empty()
                || extra.to_ascii_uppercase().contains("VIRTUAL GENERATED")
                || extra.to_ascii_uppercase().contains("STORED GENERATED");
            let mut metadata = ColumnMetadata::new(
                ordinal,
                row.try_get::<String, _>(2).map_err(decode_error)?,
                row.try_get::<String, _>(4).map_err(decode_error)? == "YES",
            );
            metadata.type_family =
                OptionalMetadata::Supported(Some(row.try_get(3).map_err(decode_error)?));
            metadata.default_expression = OptionalMetadata::Supported(if generated {
                None
            } else {
                row.try_get(5).map_err(decode_error)?
            });
            metadata.identity = OptionalMetadata::Unsupported;
            metadata.auto_increment = OptionalMetadata::Supported(Some(
                extra
                    .split_whitespace()
                    .any(|part| part.eq_ignore_ascii_case("auto_increment")),
            ));
            metadata.generated_expression = OptionalMetadata::Supported(if generated {
                empty_as_none(Some(generation_expression))
            } else {
                None
            });
            metadata.hidden = OptionalMetadata::Unsupported;
            metadata.numeric_precision = OptionalMetadata::Supported(
                row.try_get::<Option<u64>, _>(8)
                    .map_err(decode_error)?
                    .map(|value| checked_u32(value, "numeric precision"))
                    .transpose()?,
            );
            metadata.numeric_scale = OptionalMetadata::Supported(
                row.try_get::<Option<u64>, _>(9)
                    .map_err(decode_error)?
                    .map(|value| checked_u32(value, "numeric scale"))
                    .transpose()?,
            );
            metadata.character_maximum_length = OptionalMetadata::Supported(
                row.try_get::<Option<i64>, _>(10)
                    .map_err(decode_error)?
                    .map(non_negative_count)
                    .transpose()?,
            );
            metadata.collation =
                OptionalMetadata::Supported(row.try_get(11).map_err(decode_error)?);
            metadata.character_set =
                OptionalMetadata::Supported(row.try_get(12).map_err(decode_error)?);
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
                    qualified_object(database, &name),
                    "column",
                    OptionalMetadata::Supported(empty_as_none(
                        row.try_get(13).map_err(decode_error)?,
                    )),
                    CatalogMetadata::Column(metadata),
                )
                .map_err(catalog_invariant)?,
            );
        }
        Ok(entries)
    }

    async fn verify_database(
        &self,
        connection: &mut MySqlConnection,
        database_id: &CatalogId,
        target: &CatalogTarget,
        lower_case_table_names: i64,
    ) -> Result<String, DatabaseError> {
        let database = match database_id.native_path.as_slice() {
            [database] => database.as_str(),
            _ => return Err(catalog_target_not_found(target)),
        };
        let comparison = canonical_name_comparison(lower_case_table_names, "schema_name")?;
        let statement = format!(
            "SELECT schema_name FROM information_schema.schemata \
             WHERE {comparison} AND schema_name NOT IN ('information_schema','mysql','performance_schema','sys')"
        );
        let actual = sqlx::query_scalar::<_, String>(AssertSqlSafe(statement))
            .bind(database)
            .fetch_optional(&mut *connection)
            .await
            .map_err(sql_error)?;
        actual
            .filter(|actual| canonical_name_matches(lower_case_table_names, actual, database))
            .ok_or_else(|| catalog_target_not_found(target))
    }

    async fn verify_schema(
        &self,
        connection: &mut MySqlConnection,
        schema_id: &CatalogId,
        target: &CatalogTarget,
        lower_case_table_names: i64,
    ) -> Result<String, DatabaseError> {
        let (database, schema) = match schema_id.native_path.as_slice() {
            [database, schema] => (database.as_str(), schema.as_str()),
            _ => return Err(catalog_target_not_found(target)),
        };
        if !canonical_name_matches(lower_case_table_names, database, schema) {
            return Err(catalog_target_not_found(target));
        }
        self.verify_database(
            connection,
            &CatalogId::new(
                self.connection_id,
                CatalogKind::Database,
                [database.to_owned()],
            ),
            target,
            lower_case_table_names,
        )
        .await
    }

    async fn verify_relation_name(
        &self,
        connection: &mut MySqlConnection,
        database: &str,
        name: &str,
        lower_case_table_names: i64,
    ) -> Result<(String, CatalogKind), DatabaseError> {
        let schema_comparison = canonical_name_comparison(lower_case_table_names, "table_schema")?;
        let name_comparison = canonical_name_comparison(lower_case_table_names, "table_name")?;
        let statement = format!(
            "SELECT table_name, table_type FROM information_schema.tables \
             WHERE {schema_comparison} AND {name_comparison} \
             AND table_type IN ('BASE TABLE','VIEW')"
        );
        let row = sqlx::query(AssertSqlSafe(statement))
            .bind(database)
            .bind(name)
            .fetch_optional(&mut *connection)
            .await
            .map_err(sql_error)?
            .ok_or_else(|| catalog_internal("owning relation was not found"))?;
        let actual_name: String = row.try_get(0).map_err(decode_error)?;
        if !canonical_name_matches(lower_case_table_names, &actual_name, name) {
            return Err(catalog_internal(
                "MySQL owning relation name was not canonical",
            ));
        }
        let table_type: String = row.try_get(1).map_err(decode_error)?;
        Ok((
            actual_name,
            if table_type == "VIEW" {
                CatalogKind::View
            } else {
                CatalogKind::Table
            },
        ))
    }

    async fn verify_relation(
        &self,
        connection: &mut MySqlConnection,
        relation: &CatalogId,
        target: &CatalogTarget,
        lower_case_table_names: i64,
    ) -> Result<(String, String, &'static str), DatabaseError> {
        if relation.profile_id() != self.connection_id || !relation.kind.is_relation() {
            return Err(catalog_target_not_found(target));
        }
        let (database, schema, name) =
            relation_path(relation).ok_or_else(|| catalog_target_not_found(target))?;
        if !canonical_name_matches(lower_case_table_names, database, schema) {
            return Err(catalog_target_not_found(target));
        }
        let schema_comparison = canonical_name_comparison(lower_case_table_names, "table_schema")?;
        let name_comparison = canonical_name_comparison(lower_case_table_names, "table_name")?;
        let statement = format!(
            "SELECT table_name, table_type FROM information_schema.tables \
             WHERE {schema_comparison} AND {name_comparison} \
             AND table_type IN ('BASE TABLE','VIEW')"
        );
        let row = sqlx::query(AssertSqlSafe(statement))
            .bind(database)
            .bind(name)
            .fetch_optional(&mut *connection)
            .await
            .map_err(sql_error)?
            .ok_or_else(|| catalog_target_not_found(target))?;
        let actual_name: String = row.try_get(0).map_err(decode_error)?;
        if !canonical_name_matches(lower_case_table_names, &actual_name, name) {
            return Err(catalog_target_not_found(target));
        }
        let native_kind: String = row.try_get(1).map_err(decode_error)?;
        let (expected_kind, verified_native_kind) = if native_kind == "VIEW" {
            (CatalogKind::View, "VIEW")
        } else {
            (CatalogKind::Table, "BASE TABLE")
        };
        if relation.kind != expected_kind {
            return Err(catalog_target_not_found(target));
        }
        Ok((database.to_owned(), actual_name, verified_native_kind))
    }

    pub async fn preview_relation(
        &self,
        relation: &CatalogId,
        options: &crate::model::relation::RelationPreviewOptions,
    ) -> Result<crate::db::RelationPreview, DatabaseError> {
        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        let target = CatalogTarget::RelationChildren {
            relation: relation.clone(),
        };
        let lower_case: i64 =
            sqlx::query_scalar::<_, String>("SELECT CAST(@@lower_case_table_names AS CHAR)")
                .fetch_one(&mut *connection)
                .await
                .map_err(sql_error)?
                .parse()
                .map_err(|_| {
                    catalog_internal("MySQL returned an invalid lower_case_table_names value")
                })?;
        let (database, name, _) = self
            .verify_relation(&mut connection, relation, &target, lower_case)
            .await?;
        if !self.catalog_scope.allows_schema(&database, &database) {
            return Err(catalog_target_not_found(&target));
        }
        let mut sql = format!(
            "SELECT * FROM {}.{}",
            quote_identifier(&database),
            quote_identifier(&name)
        );
        append_preview_options(&mut sql, options);
        sql.push_str(&format!(" LIMIT {RELATION_PREVIEW_LIMIT}"));
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
        let rows = statement
            .query()
            .fetch_all(&mut *connection)
            .await
            .map_err(sql_error)?;
        let result_set = ResultSet {
            columns,
            rows: rows.iter().map(decode_row).collect(),
            affected_rows: 0,
        };
        Ok(crate::db::RelationPreview {
            sql,
            result: QueryOutcome::from_result_set(result_set, started.elapsed(), Duration::ZERO),
        })
    }

    pub async fn relation_ddl(&self, relation: &CatalogId) -> Result<RelationDdl, DatabaseError> {
        validate_catalog_scope(&self.catalog_scope)?;
        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        let mut transaction = connection
            .begin_with(CATALOG_PAGE_BEGIN_SQL)
            .await
            .map_err(sql_error)?;
        let result = self.relation_ddl_snapshot(&mut transaction, relation).await;
        match result {
            Ok(ddl) => {
                transaction.commit().await.map_err(sql_error)?;
                Ok(ddl)
            }
            Err(error) => {
                let _ = transaction.rollback().await;
                Err(error)
            }
        }
    }

    async fn relation_ddl_snapshot(
        &self,
        connection: &mut MySqlConnection,
        relation: &CatalogId,
    ) -> Result<RelationDdl, DatabaseError> {
        let target = CatalogTarget::RelationChildren {
            relation: relation.clone(),
        };
        let lower_case: i64 =
            sqlx::query_scalar::<_, String>("SELECT CAST(@@lower_case_table_names AS CHAR)")
                .fetch_one(&mut *connection)
                .await
                .map_err(sql_error)?
                .parse()
                .map_err(|_| {
                    catalog_internal("MySQL returned an invalid lower_case_table_names value")
                })?;
        let (database, name, native_kind) = self
            .verify_relation(connection, relation, &target, lower_case)
            .await?;
        let relation_entry = CatalogEntry::relation(
            relation.clone(),
            CatalogId::new(
                self.connection_id,
                CatalogKind::Schema,
                [database.clone(), database.clone()],
            ),
            qualified_object(&database, &name),
            native_kind,
            OptionalMetadata::Unsupported,
            true,
        )
        .map_err(catalog_invariant)?;
        let relation_database = relation
            .native_path
            .first()
            .cloned()
            .ok_or_else(|| catalog_target_not_found(&target))?;
        let relation_schema = relation
            .native_path
            .get(1)
            .cloned()
            .ok_or_else(|| catalog_target_not_found(&target))?;
        let relation_scope = CatalogScope {
            databases: CatalogSelection::Selected(vec![DatabaseScope {
                name: relation_database,
                schemas: CatalogSelection::Selected(vec![relation_schema]),
            }]),
        };
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
            scope: relation_scope,
            page_size: RELATION_PREVIEW_LIMIT,
        };
        let mut children_entries = self
            .load_relation_children(connection, &database, &name, relation)
            .await?;
        let _ = paginate_in_memory(
            &mut children_entries,
            &request,
            child_sort_key,
            child_tie_breaker,
        )?;
        let children_count = exact_count(children_entries.len())?;
        let children = CatalogPage::new(&request, children_entries, children_count, None)
            .map_err(catalog_invariant)?;
        let main_sql = show_create_relation(connection, relation.kind, &database, &name)
            .await?
            .filter(|sql| !sql.trim().is_empty())
            .ok_or_else(|| {
                catalog_internal(format!(
                    "MySQL {native_kind} {database}.{name} has no SHOW CREATE statement"
                ))
            })?;
        let trigger_names = sqlx::query_scalar::<_, String>(
            "SELECT trigger_name FROM information_schema.triggers \
             WHERE BINARY event_object_schema=BINARY ? AND BINARY event_object_table=BINARY ? \
             ORDER BY BINARY trigger_name",
        )
        .bind(&database)
        .bind(&name)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        let mut triggers = Vec::with_capacity(trigger_names.len());
        for trigger_name in trigger_names {
            let sql = show_create_trigger(connection, &database, &trigger_name)
                .await?
                .filter(|sql| !sql.trim().is_empty())
                .ok_or_else(|| {
                    catalog_internal(format!(
                        "MySQL trigger {database}.{trigger_name} has no SHOW CREATE statement"
                    ))
                })?;
            triggers.push((trigger_name, sql));
        }
        let (sql, provenance) = assemble_relation_ddl(main_sql, triggers)?;
        Ok(RelationDdl {
            relation: relation_entry,
            children,
            sql,
            provenance,
        })
    }

    async fn load_index_metadata(
        &self,
        connection: &mut MySqlConnection,
        database: &str,
        relation: &str,
    ) -> Result<Vec<MySqlIndexInfo>, DatabaseError> {
        let rows = sqlx::query(CATALOG_PAGE_INDEXES_SQL)
            .bind(database)
            .bind(relation)
            .fetch_all(&mut *connection)
            .await
            .map_err(sql_error)?;
        let parts = rows
            .into_iter()
            .map(|row| {
                Ok(MySqlIndexPart {
                    name: row.try_get(0).map_err(decode_error)?,
                    unique: row.try_get::<i64, _>(1).map_err(decode_error)? == 0,
                    ordinal: checked_u32(
                        row.try_get::<u64, _>(2).map_err(decode_error)?,
                        "index ordinal",
                    )?,
                    column: row.try_get(3).map_err(decode_error)?,
                    expression: row.try_get(4).map_err(decode_error)?,
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        group_index_parts(parts)
    }

    async fn load_constraint_metadata(
        &self,
        connection: &mut MySqlConnection,
        database: &str,
        relation: &str,
    ) -> Result<Vec<MySqlConstraintInfo>, DatabaseError> {
        let rows = sqlx::query(
            "SELECT tc.constraint_catalog, tc.constraint_schema, tc.table_schema, tc.table_name, \
             tc.constraint_name, tc.constraint_type, kcu.ordinal_position, \
             kcu.column_name, kcu.referenced_table_schema, kcu.referenced_table_name, \
             kcu.referenced_column_name, kcu.position_in_unique_constraint \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
               ON BINARY kcu.constraint_catalog=BINARY tc.constraint_catalog \
              AND BINARY kcu.constraint_schema=BINARY tc.constraint_schema \
              AND BINARY kcu.table_schema=BINARY tc.table_schema \
              AND BINARY kcu.table_name=BINARY tc.table_name \
              AND BINARY kcu.constraint_name=BINARY tc.constraint_name \
             WHERE BINARY tc.constraint_schema=BINARY ? AND BINARY tc.table_name=BINARY ? \
               AND tc.constraint_type IN ('PRIMARY KEY','UNIQUE','FOREIGN KEY') \
             ORDER BY BINARY tc.constraint_catalog, BINARY tc.constraint_schema, \
                      BINARY tc.table_schema, BINARY tc.table_name, \
                      BINARY tc.constraint_name, kcu.ordinal_position",
        )
        .bind(database)
        .bind(relation)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        let parts = rows
            .into_iter()
            .map(|row| {
                let native_kind: String = row.try_get(5).map_err(decode_error)?;
                let kind = match native_kind.as_str() {
                    "PRIMARY KEY" => CatalogKind::PrimaryKey,
                    "UNIQUE" => CatalogKind::UniqueConstraint,
                    "FOREIGN KEY" => CatalogKind::ForeignKey,
                    _ => return Err(catalog_internal("unexpected MySQL constraint type")),
                };
                Ok(MySqlConstraintPart {
                    catalog: row.try_get(0).map_err(decode_error)?,
                    schema: row.try_get(1).map_err(decode_error)?,
                    table_schema: row.try_get(2).map_err(decode_error)?,
                    table: row.try_get(3).map_err(decode_error)?,
                    name: row.try_get(4).map_err(decode_error)?,
                    kind,
                    ordinal: checked_u32(
                        row.try_get::<u64, _>(6).map_err(decode_error)?,
                        "constraint ordinal",
                    )?,
                    column: row.try_get(7).map_err(decode_error)?,
                    referenced_database: row.try_get(8).map_err(decode_error)?,
                    referenced_relation: row.try_get(9).map_err(decode_error)?,
                    referenced_column: row.try_get(10).map_err(decode_error)?,
                    referenced_ordinal: row
                        .try_get::<Option<u64>, _>(11)
                        .map_err(decode_error)?
                        .map(|value| checked_u32(value, "referenced constraint ordinal"))
                        .transpose()?,
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        group_constraint_parts(parts)
    }

    pub async fn object_ddl(
        &self,
        kind: CatalogKind,
        schema: &str,
        name: &str,
    ) -> Result<Option<String>, DatabaseError> {
        if !matches!(kind, CatalogKind::Table | CatalogKind::View) {
            return Ok(None);
        }
        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        show_create_relation(&mut connection, kind, schema, name).await
    }

    pub async fn close(self) {
        self.pool.close().await;
    }
}

async fn show_create_relation(
    connection: &mut MySqlConnection,
    kind: CatalogKind,
    schema: &str,
    name: &str,
) -> Result<Option<String>, DatabaseError> {
    let object_type = if kind == CatalogKind::View {
        "VIEW"
    } else {
        "TABLE"
    };
    let statement = format!(
        "SHOW CREATE {object_type} {}.{}",
        quote_identifier(schema),
        quote_identifier(name)
    );
    let row = sqlx::query(AssertSqlSafe(statement))
        .fetch_optional(&mut *connection)
        .await
        .map_err(sql_error)?;
    row.map(|row| show_create_statement(&row, &format!("Create {object_type}"), 1))
        .transpose()
}

async fn show_create_trigger(
    connection: &mut MySqlConnection,
    schema: &str,
    name: &str,
) -> Result<Option<String>, DatabaseError> {
    let statement = format!(
        "SHOW CREATE TRIGGER {}.{}",
        quote_identifier(schema),
        quote_identifier(name)
    );
    let row = sqlx::query(AssertSqlSafe(statement))
        .fetch_optional(&mut *connection)
        .await
        .map_err(sql_error)?;
    row.map(|row| show_create_statement(&row, "SQL Original Statement", 2))
        .transpose()
}

fn show_create_statement(
    row: &MySqlRow,
    column_name: &str,
    column_index: usize,
) -> Result<String, DatabaseError> {
    row.try_get(column_name)
        .or_else(|_| row.try_get(column_index))
        .map_err(decode_error)
}

fn assemble_relation_ddl(
    main_sql: String,
    mut triggers: Vec<(String, String)>,
) -> Result<(String, DdlProvenance), DatabaseError> {
    if main_sql.trim().is_empty() {
        return Err(catalog_internal(
            "MySQL relation has no SHOW CREATE statement",
        ));
    }
    triggers.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    let provenance = if triggers.is_empty() {
        DdlProvenance::NativeCatalog
    } else {
        DdlProvenance::AdapterGenerated
    };
    let sql = assemble_ddl(vec![
        DdlSection {
            label: "Object",
            statements: vec![main_sql],
        },
        DdlSection {
            label: "Triggers",
            statements: triggers.into_iter().map(|(_, sql)| sql).collect(),
        },
    ])
    .ok_or_else(|| catalog_internal("MySQL relation DDL assembly produced no statements"))?;
    Ok((sql, provenance))
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

pub(crate) struct MySqlTransactionBackend {
    connection: PoolConnection<MySql>,
    control: MySqlPool,
    connection_id: u64,
    adapter: MySqlAdapter,
}

#[async_trait::async_trait]
impl TransactionBackend for MySqlTransactionBackend {
    async fn begin(&mut self) -> Result<(), TransactionError> {
        <MySql as sqlx::Database>::TransactionManager::begin(&mut self.connection, None)
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
        let [database, _, relation] = request.relation.native_path.as_slice() else {
            return Err(TransactionError(
                "MySQL relation has no canonical database, schema, and table path".into(),
            ));
        };
        let columns = &request.metadata.columns;
        let quoted_table = format!(
            "{}.{}",
            quote_identifier(database),
            quote_identifier(relation)
        );
        match request.operation {
            RelationMutation::DeleteRows(rows) => {
                for mutation in &rows {
                    if mutation.row.columns.len() != mutation.row.values.len()
                        || mutation.original.len() != columns.len()
                    {
                        return Err(TransactionError(
                            "MySQL delete mutation is malformed".into(),
                        ));
                    }
                    let mut sql = format!("DELETE FROM {quoted_table} WHERE ");
                    let mut predicates = Vec::new();
                    for index in &mutation.row.columns {
                        if *index >= columns.len() {
                            return Err(TransactionError(
                                "MySQL row locator column is out of range".into(),
                            ));
                        }
                        let name = quote_identifier(&columns[*index].0);
                        predicates
                            .push(format!("(({name} = ?) OR ({name} IS NULL AND ? IS NULL))"));
                    }
                    for column in columns {
                        let name = quote_identifier(&column.0);
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
                        return Err(TransactionError("MySQL relation mutation conflict".into()));
                    }
                }
                return Ok(MutationResult::Deleted { rows: rows.len() });
            }
            RelationMutation::InsertRow(insert) => {
                if insert.columns.len() != insert.values.len()
                    || insert.columns.iter().any(|i| *i >= columns.len())
                {
                    return Err(TransactionError(
                        "MySQL insert mutation is malformed".into(),
                    ));
                }
                let supplied = insert
                    .columns
                    .iter()
                    .map(|i| quote_identifier(&columns[*i].0))
                    .collect::<Vec<_>>();
                let expressions = insert
                    .values
                    .iter()
                    .map(|v| {
                        if matches!(v, InputValue::Default) {
                            "DEFAULT".into()
                        } else {
                            "?".into()
                        }
                    })
                    .collect::<Vec<String>>();
                let sql = if supplied.is_empty() {
                    format!("INSERT INTO {quoted_table} () VALUES ()")
                } else {
                    format!(
                        "INSERT INTO {quoted_table} ({}) VALUES ({})",
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
                let result = query
                    .execute(&mut *self.connection)
                    .await
                    .map_err(|e| TransactionError(e.to_string()))?;
                let primary_key = request.metadata.primary_key.first().ok_or_else(|| {
                    TransactionError("MySQL inserted row has no primary key".into())
                })?;
                let primary_key_index = columns
                    .iter()
                    .position(|(name, _, _)| name == primary_key)
                    .ok_or_else(|| {
                        TransactionError("MySQL primary key column is missing".into())
                    })?;
                let primary_key_value = insert
                    .columns
                    .iter()
                    .position(|index| *index == primary_key_index)
                    .and_then(|position| insert.values.get(position));
                let mut sql = format!(
                    "SELECT * FROM {quoted_table} WHERE {} = ?",
                    quote_identifier(primary_key)
                );
                let mut select = sqlx::query(AssertSqlSafe(sql));
                select = match primary_key_value {
                    Some(InputValue::Value(value)) => bind_cell(select, value)?,
                    Some(InputValue::Null) => {
                        sql = format!(
                            "SELECT * FROM {quoted_table} WHERE {} IS NULL",
                            quote_identifier(primary_key)
                        );
                        sqlx::query(AssertSqlSafe(sql))
                    }
                    Some(InputValue::Default) | None => select.bind(result.last_insert_id()),
                };
                let row = select
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
                        "MySQL update column is out of range".into(),
                    ));
                };
                if update.row.columns.len() != update.row.values.len() {
                    return Err(TransactionError("MySQL row locator is malformed".into()));
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
                                TransactionError("MySQL primary key column is missing".into())
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if primary_key_columns != update.row.columns {
                    return Err(TransactionError(
                        "MySQL row locator must contain the primary key columns in order".into(),
                    ));
                }
                if update
                    .row
                    .columns
                    .iter()
                    .any(|index| *index >= columns.len())
                {
                    return Err(TransactionError(
                        "MySQL row locator column is out of range".into(),
                    ));
                }
                let quoted_column = quote_identifier(column_name);
                let set_sql = match update.value {
                    InputValue::Default => format!("{quoted_column} = DEFAULT"),
                    InputValue::Null | InputValue::Value(_) => format!("{quoted_column} = ?"),
                };
                let mut sql = format!("UPDATE {quoted_table} SET {set_sql} WHERE ");
                for (position, column_index) in update.row.columns.iter().enumerate() {
                    if position > 0 {
                        sql.push_str(" AND ");
                    }
                    let name = quote_identifier(&columns[*column_index].0);
                    sql.push_str(&format!("(({name} = ?) OR ({name} IS NULL AND ? IS NULL))"));
                }
                if !update.row.columns.is_empty() {
                    sql.push_str(" AND ");
                }
                sql.push_str(&format!(
                    "(({quoted_column} = ?) OR ({quoted_column} IS NULL AND ? IS NULL))"
                ));
                let mut query = sqlx::query(AssertSqlSafe(sql));
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
                if affected > 1 {
                    return Err(TransactionError(
                        "MySQL relation mutation matched multiple rows".into(),
                    ));
                }
                if affected == 0 {
                    let mut original_check = format!("SELECT 1 FROM {quoted_table} WHERE ");
                    for (position, column_index) in update.row.columns.iter().enumerate() {
                        if position > 0 {
                            original_check.push_str(" AND ");
                        }
                        let name = quote_identifier(&columns[*column_index].0);
                        original_check
                            .push_str(&format!("(({name} = ?) OR ({name} IS NULL AND ? IS NULL))"));
                    }
                    if !update.row.columns.is_empty() {
                        original_check.push_str(" AND ");
                    }
                    original_check.push_str(&format!(
                        "(({quoted_column} = ?) OR ({quoted_column} IS NULL AND ? IS NULL))"
                    ));
                    let mut original_query = sqlx::query(AssertSqlSafe(original_check));
                    for value in &update.row.values {
                        original_query = bind_cell(original_query, value)?;
                        original_query = bind_cell(original_query, value)?;
                    }
                    original_query = bind_cell(original_query, &update.original)?;
                    original_query = bind_cell(original_query, &update.original)?;
                    if original_query
                        .fetch_optional(&mut *self.connection)
                        .await
                        .map_err(|error| TransactionError(error.to_string()))?
                        .is_none()
                    {
                        return Err(TransactionError("MySQL relation mutation conflict".into()));
                    }
                }
                let mut select = format!("SELECT * FROM {quoted_table} WHERE ");
                for (position, column_index) in update.row.columns.iter().enumerate() {
                    if position > 0 {
                        select.push_str(" AND ");
                    }
                    select.push_str(&format!(
                        "{} = ?",
                        quote_identifier(&columns[*column_index].0)
                    ));
                }
                let mut select_query = sqlx::query(AssertSqlSafe(select));
                for (column_index, value) in update.row.columns.iter().zip(&update.row.values) {
                    if *column_index == update.column {
                        select_query = match &update.value {
                            InputValue::Value(value) => bind_cell(select_query, value)?,
                            InputValue::Null => select_query.bind(Option::<String>::None),
                            InputValue::Default => bind_cell(select_query, value)?,
                        };
                    } else {
                        select_query = bind_cell(select_query, value)?;
                    }
                }
                let row = select_query
                    .fetch_optional(&mut *self.connection)
                    .await
                    .map_err(|error| TransactionError(error.to_string()))?
                    .ok_or_else(|| TransactionError("MySQL relation mutation conflict".into()))?;
                Ok(MutationResult::Updated {
                    row: decode_row(&row),
                })
            }
        }
    }
    async fn commit(&mut self) -> Result<(), TransactionError> {
        <MySql as sqlx::Database>::TransactionManager::commit(&mut self.connection)
            .await
            .map_err(|error| TransactionError(error.to_string()))
    }
    async fn rollback(&mut self) -> Result<(), TransactionError> {
        <MySql as sqlx::Database>::TransactionManager::rollback(&mut self.connection)
            .await
            .map_err(|error| TransactionError(error.to_string()))
    }
    async fn cancel(&mut self) -> Result<(), TransactionError> {
        let sql = format!("KILL QUERY {}", self.connection_id);
        sqlx::query(AssertSqlSafe(sql))
            .execute(&self.control)
            .await
            .map_err(|error| TransactionError(error.to_string()))?;
        Ok(())
    }
    fn depth(&self) -> usize {
        <MySql as sqlx::Database>::TransactionManager::get_transaction_depth(&self.connection)
    }
    fn force_close(self) -> futures_util::future::BoxFuture<'static, Result<(), TransactionError>> {
        Box::pin(async move {
            // SQLx 0.9 has no close_hard; detaching before closing is the safe equivalent.
            let connection = self.connection.detach();
            connection
                .close()
                .await
                .map_err(|error| TransactionError(error.to_string()))
        })
    }
}

#[derive(Debug)]
struct MySqlIndexInfo {
    name: String,
    columns: Vec<String>,
    unique: bool,
}

#[derive(Debug)]
struct MySqlIndexPart {
    name: String,
    unique: bool,
    ordinal: u32,
    column: Option<String>,
    expression: Option<String>,
}

#[derive(Debug)]
struct MySqlConstraintInfo {
    name: String,
    kind: CatalogKind,
    columns: Vec<String>,
    referenced_database: Option<String>,
    referenced_relation: Option<String>,
    referenced_columns: Vec<String>,
    referenced_ordinals: Vec<u32>,
}

#[derive(Debug)]
struct MySqlConstraintPart {
    catalog: String,
    schema: String,
    table_schema: String,
    table: String,
    name: String,
    kind: CatalogKind,
    ordinal: u32,
    column: String,
    referenced_database: Option<String>,
    referenced_relation: Option<String>,
    referenced_column: Option<String>,
    referenced_ordinal: Option<u32>,
}

#[cfg(test)]
impl MySqlConstraintPart {
    fn test_primary(name: &str, ordinal: u32, column: &str) -> Self {
        Self {
            catalog: "def".to_owned(),
            schema: "app".to_owned(),
            table_schema: "app".to_owned(),
            table: "child".to_owned(),
            name: name.to_owned(),
            kind: CatalogKind::PrimaryKey,
            ordinal,
            column: column.to_owned(),
            referenced_database: None,
            referenced_relation: None,
            referenced_column: None,
            referenced_ordinal: None,
        }
    }

    fn test_foreign(
        name: &str,
        ordinal: u32,
        column: &str,
        referenced_column: Option<&str>,
        referenced_ordinal: Option<u32>,
    ) -> Self {
        Self {
            kind: CatalogKind::ForeignKey,
            referenced_database: Some("app".to_owned()),
            referenced_relation: Some("parent".to_owned()),
            referenced_column: referenced_column.map(str::to_owned),
            referenced_ordinal,
            ..Self::test_primary(name, ordinal, column)
        }
    }
}

fn group_index_parts(parts: Vec<MySqlIndexPart>) -> Result<Vec<MySqlIndexInfo>, DatabaseError> {
    let mut indexes: Vec<MySqlIndexInfo> = Vec::new();
    let mut expected_ordinal = 1;
    for part in parts {
        let same_index = indexes.last().is_some_and(|index| index.name == part.name);
        if !same_index {
            expected_ordinal = 1;
        }
        if part.ordinal != expected_ordinal {
            return Err(catalog_internal(format!(
                "MySQL index `{}` parts are not contiguous",
                part.name
            )));
        }
        expected_ordinal = expected_ordinal
            .checked_add(1)
            .ok_or_else(|| catalog_internal("MySQL index ordinal overflowed"))?;
        let component = match (
            part.column,
            part.expression.filter(|value| !value.trim().is_empty()),
        ) {
            (Some(column), _) => column,
            (None, Some(expression)) => expression,
            (None, None) => {
                return Err(catalog_internal(format!(
                    "MySQL index `{}` part {} has no column or expression",
                    part.name, part.ordinal
                )));
            }
        };
        if let Some(index) = indexes.last_mut().filter(|index| index.name == part.name) {
            if index.unique != part.unique {
                return Err(catalog_internal(
                    "MySQL index uniqueness changed between parts",
                ));
            }
            index.columns.push(component);
        } else {
            indexes.push(MySqlIndexInfo {
                name: part.name,
                columns: vec![component],
                unique: part.unique,
            });
        }
    }
    Ok(indexes)
}

fn group_constraint_parts(
    parts: Vec<MySqlConstraintPart>,
) -> Result<Vec<MySqlConstraintInfo>, DatabaseError> {
    let mut constraints: Vec<MySqlConstraintInfo> = Vec::new();
    let mut current_identity: Option<(String, String, String, String, String)> = None;
    let mut expected_ordinal = 1;
    for part in parts {
        let identity = (
            part.catalog.clone(),
            part.schema.clone(),
            part.table_schema.clone(),
            part.table.clone(),
            part.name.clone(),
        );
        let same_identity = current_identity.as_ref() == Some(&identity);
        if !same_identity {
            current_identity = Some(identity);
            expected_ordinal = 1;
        }
        if part.ordinal != expected_ordinal {
            return Err(catalog_internal(format!(
                "MySQL constraint `{}` ordinals are not contiguous",
                part.name
            )));
        }
        expected_ordinal = expected_ordinal
            .checked_add(1)
            .ok_or_else(|| catalog_internal("MySQL constraint ordinal overflowed"))?;
        if part.kind == CatalogKind::ForeignKey {
            if part.referenced_column.is_none() {
                return Err(catalog_internal(format!(
                    "MySQL foreign key `{}` part {} has no referenced column",
                    part.name, part.ordinal
                )));
            }
            if part.referenced_ordinal.is_none() {
                return Err(catalog_internal(format!(
                    "MySQL foreign key `{}` part {} has no referenced ordinal",
                    part.name, part.ordinal
                )));
            }
        } else if part.referenced_column.is_some() || part.referenced_ordinal.is_some() {
            return Err(catalog_internal(format!(
                "MySQL non-foreign constraint `{}` unexpectedly references a column",
                part.name
            )));
        }
        if let Some(constraint) = constraints.last_mut().filter(|_| same_identity) {
            if constraint.kind != part.kind
                || constraint.referenced_database != part.referenced_database
                || constraint.referenced_relation != part.referenced_relation
            {
                return Err(catalog_internal(
                    "MySQL constraint identity changed between parts",
                ));
            }
            constraint.columns.push(part.column);
            if let Some(referenced_column) = part.referenced_column {
                constraint.referenced_columns.push(referenced_column);
            }
            if let Some(referenced_ordinal) = part.referenced_ordinal {
                constraint.referenced_ordinals.push(referenced_ordinal);
            }
        } else {
            constraints.push(MySqlConstraintInfo {
                name: part.name,
                kind: part.kind,
                columns: vec![part.column],
                referenced_database: part.referenced_database,
                referenced_relation: part.referenced_relation,
                referenced_columns: part.referenced_column.into_iter().collect(),
                referenced_ordinals: part.referenced_ordinal.into_iter().collect(),
            });
        }
    }
    for constraint in &constraints {
        if constraint.kind == CatalogKind::ForeignKey {
            if constraint.columns.len() != constraint.referenced_columns.len() {
                return Err(catalog_internal(format!(
                    "MySQL foreign key `{}` source and referenced cardinality differ",
                    constraint.name
                )));
            }
            let mut referenced_ordinals = constraint.referenced_ordinals.clone();
            referenced_ordinals.sort_unstable();
            referenced_ordinals.dedup();
            let expected = (1..=constraint.columns.len())
                .map(|ordinal| {
                    u32::try_from(ordinal)
                        .map_err(|_| catalog_internal("MySQL foreign key has too many columns"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if referenced_ordinals != expected {
                return Err(catalog_internal(format!(
                    "MySQL foreign key `{}` referenced ordinals are not contiguous and unique",
                    constraint.name
                )));
            }
        }
    }
    Ok(constraints)
}

fn selected_databases(request: &CatalogRequest) -> Option<Vec<&str>> {
    match &request.scope.databases {
        crate::profile::CatalogSelection::All => None,
        crate::profile::CatalogSelection::Selected(databases) => Some(
            databases
                .iter()
                .map(|database| database.name.as_str())
                .collect(),
        ),
    }
}

fn selected_search_databases(scope: &CatalogScope) -> Option<Vec<&str>> {
    match &scope.databases {
        CatalogSelection::All => None,
        CatalogSelection::Selected(databases) => Some(
            databases
                .iter()
                .map(|database| database.name.as_str())
                .collect(),
        ),
    }
}

fn search_catalog_kind(native_kind: &str) -> Result<CatalogKind, DatabaseError> {
    match native_kind {
        "database" => Ok(CatalogKind::Database),
        "schema" => Ok(CatalogKind::Schema),
        "table" => Ok(CatalogKind::Table),
        "view" => Ok(CatalogKind::View),
        "function" => Ok(CatalogKind::Function),
        "procedure" => Ok(CatalogKind::Procedure),
        "trigger" => Ok(CatalogKind::Trigger),
        "column" => Ok(CatalogKind::Column),
        "index" => Ok(CatalogKind::Index),
        "primary_key" => Ok(CatalogKind::PrimaryKey),
        "unique_constraint" => Ok(CatalogKind::UniqueConstraint),
        "foreign_key" => Ok(CatalogKind::ForeignKey),
        _ => Err(catalog_internal(format!(
            "unexpected MySQL search catalog kind `{native_kind}`"
        ))),
    }
}

const fn search_native_kind(kind: CatalogKind) -> &'static str {
    match kind {
        CatalogKind::Function => "function",
        CatalogKind::Procedure => "procedure",
        _ => "object",
    }
}

fn relation_kind(table_type: Option<&str>) -> Result<CatalogKind, DatabaseError> {
    match table_type {
        Some("BASE TABLE") => Ok(CatalogKind::Table),
        Some("VIEW") => Ok(CatalogKind::View),
        _ => Err(catalog_internal("unexpected MySQL search relation type")),
    }
}

pub fn validate_catalog_scope(scope: &CatalogScope) -> Result<(), DatabaseError> {
    if let CatalogSelection::Selected(databases) = &scope.databases
        && let Some(database) = databases
            .iter()
            .find(|database| !matches!(database.schemas, CatalogSelection::All))
    {
        return Err(DatabaseError {
            category: ErrorCategory::Configuration,
            code: Some("invalid_catalog_request".to_owned()),
            message: sanitize_terminal_text(&format!(
                "invalid MySQL catalog scope for database `{}`: mirrored schemas must use All",
                database.name
            )),
        });
    }
    Ok(())
}

fn relation_path(relation: &CatalogId) -> Option<(&str, &str, &str)> {
    match relation.native_path.as_slice() {
        [database, schema, name] => Some((database, schema, name)),
        _ => None,
    }
}

fn canonical_name_comparison(
    lower_case_table_names: i64,
    column: &str,
) -> Result<String, DatabaseError> {
    match lower_case_table_names {
        0 => Ok(format!("BINARY {column}=BINARY ?")),
        1 | 2 => Ok(format!("LOWER({column})=LOWER(?)")),
        value => Err(catalog_internal(format!(
            "MySQL returned unsupported lower_case_table_names value {value}"
        ))),
    }
}

fn canonical_name_matches(lower_case_table_names: i64, actual: &str, expected: &str) -> bool {
    match lower_case_table_names {
        0 => actual == expected,
        1 | 2 => actual.eq_ignore_ascii_case(expected),
        _ => false,
    }
}

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(",")
}

fn request_cursor(request: &CatalogRequest) -> Result<Option<(&str, &str)>, DatabaseError> {
    request
        .key
        .cursor
        .as_ref()
        .map(|cursor| cursor.keyset_parts())
        .transpose()
        .map_err(DatabaseError::invalid_catalog_request)
}

fn paginate_in_memory<T, SortKey, TieBreaker>(
    rows: &mut Vec<T>,
    request: &CatalogRequest,
    sort_key: SortKey,
    tie_breaker: TieBreaker,
) -> Result<Option<super::catalog::CatalogCursor>, DatabaseError>
where
    SortKey: Fn(&T) -> String,
    TieBreaker: Fn(&T) -> String,
{
    rows.sort_by(|left, right| {
        sort_key(left)
            .cmp(&sort_key(right))
            .then_with(|| tie_breaker(left).cmp(&tie_breaker(right)))
    });
    if let Some((cursor_sort_key, cursor_tie_breaker)) = request_cursor(request)? {
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

fn qualified_schema(database: &str) -> QualifiedName {
    QualifiedName {
        database: Some(database.to_owned()),
        schema: Some(database.to_owned()),
        object: database.to_owned(),
    }
}

fn qualified_object(database: &str, object: &str) -> QualifiedName {
    QualifiedName {
        database: Some(database.to_owned()),
        schema: Some(database.to_owned()),
        object: object.to_owned(),
    }
}

fn mysql_namespace_name(entry: &CatalogEntry) -> Result<String, CatalogDropError> {
    let expected_path_len = match entry.kind {
        CatalogKind::Database => 1,
        CatalogKind::Schema => 2,
        _ => 0,
    };
    if expected_path_len == 0 || entry.id.native_path.len() != expected_path_len {
        return Err(CatalogDropError::Unsupported {
            kind: entry.kind,
            reason: "catalog entry has an invalid MySQL namespace identity".to_owned(),
        });
    }
    let database = entry
        .qualified_name
        .database
        .as_deref()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| CatalogDropError::Unsupported {
            kind: entry.kind,
            reason: "catalog entry has no MySQL database name".to_owned(),
        })?;
    if entry.id.native_path[0] != database
        || (entry.kind == CatalogKind::Schema
            && (entry.id.native_path[1] != database
                || entry.qualified_name.schema.as_deref() != Some(database)))
    {
        return Err(CatalogDropError::Unsupported {
            kind: entry.kind,
            reason: "catalog entry has an inconsistent MySQL namespace identity".to_owned(),
        });
    }
    Ok(quote_identifier(database))
}

fn mysql_schema_object_name(entry: &CatalogEntry) -> Result<String, CatalogDropError> {
    let database = entry
        .qualified_name
        .database
        .as_deref()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| CatalogDropError::Unsupported {
            kind: entry.kind,
            reason: "catalog entry has no MySQL database name".to_owned(),
        })?;
    let object = entry.qualified_name.object.as_str();
    if object.is_empty()
        || entry.id.native_path.len() < 3
        || entry.id.native_path[0] != database
        || entry.id.native_path[1] != database
        || entry.id.native_path[2] != object
    {
        return Err(CatalogDropError::Unsupported {
            kind: entry.kind,
            reason: "catalog entry has an inconsistent MySQL object identity".to_owned(),
        });
    }
    Ok(format!(
        "{}.{}",
        quote_identifier(database),
        quote_identifier(object)
    ))
}

fn mysql_relation_name(entry: &CatalogEntry) -> Result<String, CatalogDropError> {
    if !entry.kind.is_relation() || entry.id.native_path.len() != 3 {
        return Err(CatalogDropError::Unsupported {
            kind: entry.kind,
            reason: "catalog entry has an invalid MySQL relation identity".to_owned(),
        });
    }
    mysql_schema_object_name(entry)
}

fn mysql_relation_owner(entry: &CatalogEntry) -> Result<String, CatalogDropError> {
    let relation = entry
        .relation_id
        .as_ref()
        .filter(|relation| relation.kind.is_relation())
        .ok_or_else(|| CatalogDropError::Unsupported {
            kind: entry.kind,
            reason: "catalog entry has no owning MySQL relation identity".to_owned(),
        })?;
    if relation.native_path.len() != 3 {
        return Err(CatalogDropError::Unsupported {
            kind: entry.kind,
            reason: "catalog entry has an invalid owning MySQL relation identity".to_owned(),
        });
    }
    let database = relation.native_path[0].as_str();
    let relation_name = relation.native_path[2].as_str();
    if database.is_empty()
        || relation.native_path[1] != relation.native_path[0]
        || relation_name.is_empty()
    {
        return Err(CatalogDropError::Unsupported {
            kind: entry.kind,
            reason: "catalog entry has an incomplete owning MySQL relation identity".to_owned(),
        });
    }
    Ok(format!(
        "{}.{}",
        quote_identifier(database),
        quote_identifier(relation_name)
    ))
}

fn mysql_trigger_name(entry: &CatalogEntry) -> Result<String, CatalogDropError> {
    mysql_relation_owner(entry)?;
    let database = entry
        .qualified_name
        .database
        .as_deref()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| CatalogDropError::Unsupported {
            kind: entry.kind,
            reason: "catalog entry has no MySQL trigger database name".to_owned(),
        })?;
    if entry.id.native_path.len() != 3
        || entry.id.native_path[0] != database
        || entry.id.native_path[1] != database
        || entry.id.native_path[2] != entry.qualified_name.object
    {
        return Err(CatalogDropError::Unsupported {
            kind: entry.kind,
            reason: "catalog entry has an inconsistent MySQL trigger identity".to_owned(),
        });
    }
    Ok(format!(
        "{}.{}",
        quote_identifier(database),
        quote_identifier(&entry.qualified_name.object)
    ))
}

fn mysql_routine_name(entry: &CatalogEntry) -> Result<String, CatalogDropError> {
    if entry.id.native_path.len() != 4 || entry.id.native_path[3].is_empty() {
        return Err(CatalogDropError::Unsupported {
            kind: entry.kind,
            reason: "catalog entry has no unambiguous MySQL routine identity".to_owned(),
        });
    }
    mysql_schema_object_name(entry)
}

fn relation_child_id(relation: &CatalogId, kind: CatalogKind, identity: &str) -> CatalogId {
    let mut path = relation.native_path.clone();
    path.push(identity.to_owned());
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
                    .map_err(|_| catalog_internal("MySQL constraint has too many columns"))?,
            });
    }
    Ok(())
}

fn group_sort_key(group: ObjectGroup) -> &'static str {
    match group {
        ObjectGroup::Tables => "00:tables",
        ObjectGroup::Views => "01:views",
        ObjectGroup::Functions => "02:functions",
        ObjectGroup::Procedures => "03:procedures",
        ObjectGroup::Triggers => "04:triggers",
        ObjectGroup::MaterializedViews => "05:materialized_views",
        ObjectGroup::Sequences => "06:sequences",
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
        _ => 7,
    }
}

fn exact_count(count: usize) -> Result<CatalogCount, DatabaseError> {
    u64::try_from(count)
        .map(CatalogCount::Exact)
        .map_err(|_| catalog_internal("MySQL catalog count exceeds u64"))
}

fn non_negative_count(count: i64) -> Result<u64, DatabaseError> {
    u64::try_from(count).map_err(|_| catalog_internal("MySQL returned a negative catalog count"))
}

fn page_limit(page_size: usize) -> Result<i64, DatabaseError> {
    page_size
        .checked_add(1)
        .and_then(|limit| i64::try_from(limit).ok())
        .ok_or_else(|| catalog_internal("MySQL catalog page limit overflowed"))
}

fn checked_u32(value: u64, description: &str) -> Result<u32, DatabaseError> {
    u32::try_from(value)
        .map_err(|_| catalog_internal(format!("invalid MySQL {description}: {value}")))
}

fn empty_as_none(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

fn catalog_target_not_found(target: &CatalogTarget) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Configuration,
        code: Some("catalog_target_not_found".to_owned()),
        message: sanitize_terminal_text(&format!(
            "MySQL catalog target was not found: {}",
            target.description()
        )),
    }
}

fn catalog_invariant(error: CatalogValidationError) -> DatabaseError {
    catalog_internal(format!("MySQL catalog invariant failed: {error}"))
}

fn catalog_internal(message: impl AsRef<str>) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Internal,
        code: Some("mysql_catalog_invariant".to_owned()),
        message: sanitize_terminal_text(message.as_ref()),
    }
}

fn sql_error(error: sqlx::Error) -> DatabaseError {
    DatabaseError::from_sqlx(error, ErrorCategory::Sql)
}

fn bind_cell<'q>(
    query: sqlx::query::Query<'q, MySql, sqlx::mysql::MySqlArguments>,
    value: &CellValue,
) -> Result<sqlx::query::Query<'q, MySql, sqlx::mysql::MySqlArguments>, TransactionError> {
    Ok(match value {
        CellValue::Null => query.bind(Option::<String>::None),
        CellValue::Boolean(value) => query.bind(*value),
        CellValue::Integer(value) => query.bind(*value),
        CellValue::Unsigned(value) => query.bind(*value),
        CellValue::Float(value) => query.bind(*value),
        CellValue::Text(value) => query.bind(value.clone()),
        CellValue::Bytes(value) => query.bind(value.clone()),
        CellValue::Date(value) => query.bind(*value),
        CellValue::Time(value) => query.bind(*value),
        CellValue::DateTime(value) => query.bind(*value),
        CellValue::Timestamp(value) => query.bind(value.naive_local()),
        CellValue::Unsupported { .. } => {
            return Err(TransactionError(
                "MySQL cannot bind an unsupported cell value".into(),
            ));
        }
    })
}

pub fn supports_catalog_version(version: &str) -> bool {
    if version.to_ascii_lowercase().contains("mariadb") {
        return false;
    }
    parse_version_triplet(version).is_some_and(|version| version >= (8, 0, 13))
}

fn unsupported_catalog_version(version: &str) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Unsupported,
        code: Some("mysql_catalog_version_unsupported".to_owned()),
        message: sanitize_terminal_text(&format!(
            "MySQL catalog pages require Oracle MySQL 8.0.13 or newer; server reported {version}"
        )),
    }
}

fn parse_version_triplet(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts
        .next()?
        .split(|character: char| !character.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    Some((major, minor, patch))
}

pub fn quote_identifier(value: &str) -> String {
    format!("`{}`", value.replace('`', "``"))
}

fn mysql_ssl_mode(mode: SslMode) -> MySqlSslMode {
    match mode {
        SslMode::Disable => MySqlSslMode::Disabled,
        SslMode::Prefer => MySqlSslMode::Preferred,
        SslMode::Require => MySqlSslMode::Required,
        SslMode::VerifyCa => MySqlSslMode::VerifyCa,
        SslMode::VerifyFull => MySqlSslMode::VerifyIdentity,
    }
}

fn columns(row: &MySqlRow) -> Vec<ColumnMeta> {
    row.columns()
        .iter()
        .map(|column| ColumnMeta {
            name: column.name().to_owned(),
            type_name: column.type_info().name().to_owned(),
        })
        .collect()
}

fn decode_row(row: &MySqlRow) -> Vec<CellValue> {
    (0..row.len())
        .map(|index| decode_cell(row, index))
        .collect()
}

fn decode_cell(row: &MySqlRow, index: usize) -> CellValue {
    let Ok(raw) = row.try_get_raw(index) else {
        return unsupported("unknown", "decode error");
    };
    if raw.is_null() {
        return CellValue::Null;
    }
    let type_name = raw.type_info().name().to_ascii_uppercase();
    if type_name.ends_with(" UNSIGNED") || matches!(type_name.as_str(), "YEAR" | "BIT") {
        return row
            .try_get_unchecked::<u64, _>(index)
            .map(CellValue::Unsigned)
            .unwrap_or_else(|error| unsupported(&type_name, &error.to_string()));
    }
    let decoded = match type_name.as_str() {
        "BOOLEAN" => row
            .try_get_unchecked::<bool, _>(index)
            .map(CellValue::Boolean),
        "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" => row
            .try_get_unchecked::<i64, _>(index)
            .map(CellValue::Integer),
        "FLOAT" | "DOUBLE" => row.try_get_unchecked::<f64, _>(index).map(CellValue::Float),
        "DATE" => row
            .try_get_unchecked::<NaiveDate, _>(index)
            .map(CellValue::Date),
        "TIME" => row
            .try_get_unchecked::<NaiveTime, _>(index)
            .map(CellValue::Time),
        "DATETIME" | "TIMESTAMP" => row
            .try_get_unchecked::<NaiveDateTime, _>(index)
            .map(CellValue::DateTime),
        "BINARY" | "VARBINARY" | "TINYBLOB" | "BLOB" | "MEDIUMBLOB" | "LONGBLOB" => row
            .try_get_unchecked::<Vec<u8>, _>(index)
            .map(CellValue::Bytes),
        "CHAR" | "VARCHAR" | "TINYTEXT" | "TEXT" | "MEDIUMTEXT" | "LONGTEXT" | "ENUM" => row
            .try_get_unchecked::<String, _>(index)
            .map(CellValue::Text),
        _ => return fallback_mysql(row, index, &type_name),
    };
    decoded.unwrap_or_else(|error| unsupported(&type_name, &error.to_string()))
}

fn fallback_mysql(row: &MySqlRow, index: usize, type_name: &str) -> CellValue {
    if let Ok(value) = row.try_get_unchecked::<String, _>(index) {
        CellValue::Text(value)
    } else if let Ok(value) = row.try_get_unchecked::<Vec<u8>, _>(index) {
        CellValue::Bytes(value)
    } else {
        unsupported(type_name, "unsupported MySQL value")
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

#[cfg(test)]
mod tests {
    use super::{
        MySqlConstraintPart, MySqlIndexPart, PROBE_SQL, assemble_relation_ddl,
        group_constraint_parts, group_index_parts, relation_kind, relation_path,
        search_catalog_kind,
    };
    use crate::db::catalog::{CatalogId, CatalogKind, DdlProvenance};
    use uuid::Uuid;

    #[test]
    fn probe_query_avoids_the_reserved_database_alias() {
        assert!(PROBE_SQL.contains("AS current_database"));
        assert!(!PROBE_SQL.contains("AS database"));
    }

    #[test]
    fn relation_path_rejects_a_trailing_native_suffix() {
        let id = CatalogId::new(
            Uuid::new_v4(),
            CatalogKind::Table,
            ["app", "app", "users", "forged"],
        );
        assert!(relation_path(&id).is_none());
    }

    #[test]
    fn search_kind_mapping_is_limited_to_supported_mysql_catalog_kinds() {
        for native in [
            "database",
            "schema",
            "table",
            "view",
            "function",
            "procedure",
            "trigger",
            "column",
            "index",
            "primary_key",
            "unique_constraint",
            "foreign_key",
        ] {
            assert!(search_catalog_kind(native).is_ok(), "missing {native}");
        }
        for unsupported in ["materialized_view", "sequence", "check_constraint", "type"] {
            assert!(search_catalog_kind(unsupported).is_err());
        }
        assert_eq!(
            relation_kind(Some("BASE TABLE")).unwrap(),
            CatalogKind::Table
        );
        assert_eq!(relation_kind(Some("VIEW")).unwrap(), CatalogKind::View);
        assert!(relation_kind(Some("SYSTEM VIEW")).is_err());
    }

    #[test]
    fn relation_ddl_assembles_the_native_object_once_and_sorts_triggers() {
        let main_sql = "CREATE TABLE `users` (`id` bigint PRIMARY KEY) COMMENT='accounts'";
        let (sql, provenance) = assemble_relation_ddl(
            main_sql.to_owned(),
            vec![
                (
                    "users_zeta".to_owned(),
                    "CREATE TRIGGER `users_zeta` BEFORE UPDATE ON `users` FOR EACH ROW SET NEW.`id` = OLD.`id`".to_owned(),
                ),
                (
                    "users_alpha".to_owned(),
                    "CREATE TRIGGER `users_alpha` BEFORE INSERT ON `users` FOR EACH ROW SET NEW.`id` = COALESCE(NEW.`id`, 1)".to_owned(),
                ),
            ],
        )
        .unwrap();

        assert_eq!(sql.matches(main_sql).count(), 1);
        assert!(sql.starts_with("-- Object\n\n"));
        assert!(sql.contains("\n\n-- Triggers\n\n"));
        assert!(sql.find("users_alpha").unwrap() < sql.find("users_zeta").unwrap());
        assert_eq!(provenance, DdlProvenance::AdapterGenerated);
    }

    #[test]
    fn relation_ddl_without_triggers_preserves_native_provenance() {
        let (sql, provenance) =
            assemble_relation_ddl("CREATE VIEW `active_users` AS SELECT 1".to_owned(), vec![])
                .unwrap();

        assert_eq!(sql, "-- Object\n\nCREATE VIEW `active_users` AS SELECT 1;");
        assert_eq!(provenance, DdlProvenance::NativeCatalog);
    }

    #[test]
    fn relation_ddl_requires_a_main_show_create_statement() {
        let error = assemble_relation_ddl("  ".to_owned(), vec![]).unwrap_err();

        assert_eq!(error.category, crate::db::ErrorCategory::Internal);
        assert!(error.message.contains("no SHOW CREATE statement"));
    }

    #[test]
    fn functional_index_parts_require_a_column_or_nonempty_expression() {
        let parts = vec![MySqlIndexPart {
            name: "idx_lower".to_owned(),
            unique: false,
            ordinal: 1,
            column: None,
            expression: Some("lower(`code`)".to_owned()),
        }];
        assert_eq!(
            group_index_parts(parts).unwrap()[0].columns,
            ["lower(`code`)"].map(str::to_owned)
        );

        let invalid = vec![MySqlIndexPart {
            name: "idx_invalid".to_owned(),
            unique: false,
            ordinal: 1,
            column: None,
            expression: None,
        }];
        assert!(
            group_index_parts(invalid)
                .unwrap_err()
                .message
                .contains("no column or expression")
        );
    }

    #[test]
    fn constraint_parts_require_contiguous_ordinals_and_complete_fk_pairing() {
        let gap = vec![MySqlConstraintPart::test_primary("PRIMARY", 2, "id")];
        assert!(
            group_constraint_parts(gap)
                .unwrap_err()
                .message
                .contains("contiguous")
        );

        let missing_reference = vec![MySqlConstraintPart::test_foreign(
            "child_fk",
            1,
            "parent_id",
            None,
            Some(1),
        )];
        assert!(
            group_constraint_parts(missing_reference)
                .unwrap_err()
                .message
                .contains("referenced column")
        );
    }
}
