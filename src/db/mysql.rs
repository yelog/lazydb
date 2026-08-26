use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

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
    profile::{CatalogScope, CatalogSelection, ConnectionProfile, DatabaseKind, SslMode},
};

use super::transaction::{TransactionBackend, TransactionError};
use super::{
    DatabaseError, ErrorCategory, ServerInfo,
    catalog::{
        CatalogCapabilities, CatalogCount, CatalogDiscovery, CatalogEntry, CatalogGroupSummary,
        CatalogId, CatalogKind, CatalogMetadata, CatalogPage, CatalogRequest, CatalogRequestKey,
        CatalogTarget, CatalogValidationError, ColumnMetadata, ColumnMetadataCapabilities,
        ConstraintMembership, ConstraintMetadata, Ddl, DdlProvenance, DiscoveredDatabase,
        IndexMetadata, NamespaceModel, ObjectGroup, OptionalMetadata, QualifiedName,
        RelationStructure, finalize_keyset_page,
    },
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

impl MySqlAdapter {
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

        let mut pool_options = MySqlPoolOptions::new().max_connections(6);
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

        Ok(CatalogDiscovery { databases })
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
            return Err(DatabaseError {
                category: ErrorCategory::Unsupported,
                code: Some("mysql_catalog_version_unsupported".to_owned()),
                message: sanitize_terminal_text(&format!(
                    "MySQL catalog pages require Oracle MySQL 8.0.13 or newer; server reported {version}"
                )),
            });
        }
        let mut transaction = connection
            .begin_with(CATALOG_PAGE_BEGIN_SQL)
            .await
            .map_err(sql_error)?;
        let lower_case_table_names: i64 = sqlx::query_scalar("SELECT @@lower_case_table_names")
            .fetch_one(&mut *transaction)
            .await
            .map_err(sql_error)?;
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

    async fn load_database_page(
        &self,
        connection: &mut MySqlConnection,
        request: &CatalogRequest,
    ) -> Result<CatalogPage, DatabaseError> {
        let selected = selected_databases(request);
        let scope_predicate = selected
            .as_ref()
            .map(|databases| {
                format!(
                    " AND BINARY schema_name IN ({})",
                    placeholders(databases.len())
                )
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
                let name: String = row.try_get("schema_name").map_err(decode_error)?;
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
             (SELECT CAST(COUNT(*) AS SIGNED) FROM information_schema.tables WHERE BINARY table_schema=BINARY ? AND table_type='BASE TABLE') AS tables, \
             (SELECT CAST(COUNT(*) AS SIGNED) FROM information_schema.tables WHERE BINARY table_schema=BINARY ? AND table_type='VIEW') AS views, \
             (SELECT CAST(COUNT(*) AS SIGNED) FROM information_schema.routines WHERE BINARY routine_schema=BINARY ? AND routine_type='FUNCTION') AS functions, \
             (SELECT CAST(COUNT(*) AS SIGNED) FROM information_schema.routines WHERE BINARY routine_schema=BINARY ? AND routine_type='PROCEDURE') AS procedures, \
             (SELECT CAST(COUNT(*) AS SIGNED) FROM information_schema.triggers WHERE BINARY trigger_schema=BINARY ?) AS triggers",
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
            let name: String = row.try_get("name").map_err(decode_error)?;
            let native_identity: String = row.try_get("native_identity").map_err(decode_error)?;
            let comment = empty_as_none(row.try_get("comment").map_err(decode_error)?);
            let mut path = vec![database.clone(), database.clone(), name.clone()];
            if matches!(kind, CatalogKind::Function | CatalogKind::Procedure) {
                path.push(native_identity);
            }
            let id = CatalogId::new(self.connection_id, kind, path);
            let entry = if kind == CatalogKind::Trigger {
                let owner: String = row.try_get("owner_name").map_err(decode_error)?;
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
        let indexes = self
            .load_index_metadata(connection, &database, &relation_name)
            .await?;
        let constraints = self
            .load_constraint_metadata(connection, &database, &relation_name)
            .await?;
        let mut memberships: HashMap<String, Vec<ConstraintMembership>> = HashMap::new();
        let mut entries = Vec::new();

        for index in indexes {
            entries.push(
                CatalogEntry::relation_child(
                    relation_child_id(relation, CatalogKind::Index, &index.name),
                    relation.clone(),
                    qualified_object(&database, &index.name),
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
                    qualified_object(&database, &constraint.name),
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
        .bind(&database)
        .bind(&relation_name)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        for row in rows {
            let ordinal = checked_u32(
                row.try_get::<u64, _>("ordinal_position")
                    .map_err(decode_error)?,
                "column ordinal",
            )?;
            let name: String = row.try_get("column_name").map_err(decode_error)?;
            let extra: String = row.try_get("extra").map_err(decode_error)?;
            let generation_expression: String =
                row.try_get("generation_expression").map_err(decode_error)?;
            let generated = !generation_expression.is_empty()
                || extra.to_ascii_uppercase().contains("VIRTUAL GENERATED")
                || extra.to_ascii_uppercase().contains("STORED GENERATED");
            let mut metadata = ColumnMetadata::new(
                ordinal,
                row.try_get::<String, _>("column_type")
                    .map_err(decode_error)?,
                row.try_get::<String, _>("is_nullable")
                    .map_err(decode_error)?
                    == "YES",
            );
            metadata.type_family =
                OptionalMetadata::Supported(Some(row.try_get("data_type").map_err(decode_error)?));
            metadata.default_expression = OptionalMetadata::Supported(if generated {
                None
            } else {
                row.try_get("column_default").map_err(decode_error)?
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
                row.try_get::<Option<u64>, _>("numeric_precision")
                    .map_err(decode_error)?
                    .map(|value| checked_u32(value, "numeric precision"))
                    .transpose()?,
            );
            metadata.numeric_scale = OptionalMetadata::Supported(
                row.try_get::<Option<u64>, _>("numeric_scale")
                    .map_err(decode_error)?
                    .map(|value| checked_u32(value, "numeric scale"))
                    .transpose()?,
            );
            metadata.character_maximum_length = OptionalMetadata::Supported(
                row.try_get("character_maximum_length")
                    .map_err(decode_error)?,
            );
            metadata.collation =
                OptionalMetadata::Supported(row.try_get("collation_name").map_err(decode_error)?);
            metadata.character_set = OptionalMetadata::Supported(
                row.try_get("character_set_name").map_err(decode_error)?,
            );
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
                    qualified_object(&database, &name),
                    "column",
                    OptionalMetadata::Supported(empty_as_none(
                        row.try_get("column_comment").map_err(decode_error)?,
                    )),
                    CatalogMetadata::Column(metadata),
                )
                .map_err(catalog_invariant)?,
            );
        }
        let total_count = exact_count(entries.len())?;
        let next_cursor =
            paginate_in_memory(&mut entries, request, child_sort_key, child_tie_breaker)?;
        CatalogPage::new(request, entries, total_count, next_cursor).map_err(catalog_invariant)
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
            .filter(|actual| actual == database)
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
        if database != schema {
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
        let name_comparison = canonical_name_comparison(lower_case_table_names, "table_name")?;
        let statement = format!(
            "SELECT table_name, table_type FROM information_schema.tables \
             WHERE BINARY table_schema=BINARY ? AND {name_comparison} \
             AND table_type IN ('BASE TABLE','VIEW')"
        );
        let row = sqlx::query(AssertSqlSafe(statement))
            .bind(database)
            .bind(name)
            .fetch_optional(&mut *connection)
            .await
            .map_err(sql_error)?
            .ok_or_else(|| catalog_internal("owning relation was not found"))?;
        let actual_name: String = row.try_get("table_name").map_err(decode_error)?;
        if actual_name != name {
            return Err(catalog_internal(
                "MySQL owning relation name was not canonical",
            ));
        }
        let table_type: String = row.try_get("table_type").map_err(decode_error)?;
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
        if database != schema {
            return Err(catalog_target_not_found(target));
        }
        let name_comparison = canonical_name_comparison(lower_case_table_names, "table_name")?;
        let statement = format!(
            "SELECT table_name, table_type FROM information_schema.tables \
             WHERE BINARY table_schema=BINARY ? AND {name_comparison} \
             AND table_type IN ('BASE TABLE','VIEW')"
        );
        let row = sqlx::query(AssertSqlSafe(statement))
            .bind(database)
            .bind(name)
            .fetch_optional(&mut *connection)
            .await
            .map_err(sql_error)?
            .ok_or_else(|| catalog_target_not_found(target))?;
        let actual_name: String = row.try_get("table_name").map_err(decode_error)?;
        if actual_name != name {
            return Err(catalog_target_not_found(target));
        }
        let native_kind: String = row.try_get("table_type").map_err(decode_error)?;
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
        let lower_case: i64 = sqlx::query_scalar("SELECT @@lower_case_table_names")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
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

    pub async fn relation_structure(
        &self,
        relation: &CatalogId,
    ) -> Result<RelationStructure, DatabaseError> {
        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        let target = CatalogTarget::RelationChildren {
            relation: relation.clone(),
        };
        let lower_case: i64 = sqlx::query_scalar("SELECT @@lower_case_table_names")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        let (database, name, native_kind) = self
            .verify_relation(&mut connection, relation, &target, lower_case)
            .await?;
        if !self.catalog_scope.allows_schema(&database, &database) {
            return Err(catalog_target_not_found(&target));
        }
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
        drop(connection);
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
        let children = self.load_catalog_page(&request).await?;
        let ddl = self.object_ddl(relation.kind, &database, &name).await?;
        let provenance = if ddl.is_some() {
            DdlProvenance::NativeCatalog
        } else {
            DdlProvenance::AdapterGenerated
        };
        Ok(RelationStructure {
            relation: relation_entry,
            children,
            ddl: Ddl {
                sql: ddl,
                provenance,
            },
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
                    name: row.try_get("index_name").map_err(decode_error)?,
                    unique: row.try_get::<i64, _>("non_unique").map_err(decode_error)? == 0,
                    ordinal: checked_u32(
                        row.try_get::<u64, _>("seq_in_index")
                            .map_err(decode_error)?,
                        "index ordinal",
                    )?,
                    column: row.try_get("column_name").map_err(decode_error)?,
                    expression: row.try_get("expression").map_err(decode_error)?,
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
                let native_kind: String = row.try_get("constraint_type").map_err(decode_error)?;
                let kind = match native_kind.as_str() {
                    "PRIMARY KEY" => CatalogKind::PrimaryKey,
                    "UNIQUE" => CatalogKind::UniqueConstraint,
                    "FOREIGN KEY" => CatalogKind::ForeignKey,
                    _ => return Err(catalog_internal("unexpected MySQL constraint type")),
                };
                Ok(MySqlConstraintPart {
                    catalog: row.try_get("constraint_catalog").map_err(decode_error)?,
                    schema: row.try_get("constraint_schema").map_err(decode_error)?,
                    table_schema: row.try_get("table_schema").map_err(decode_error)?,
                    table: row.try_get("table_name").map_err(decode_error)?,
                    name: row.try_get("constraint_name").map_err(decode_error)?,
                    kind,
                    ordinal: checked_u32(
                        row.try_get::<u64, _>("ordinal_position")
                            .map_err(decode_error)?,
                        "constraint ordinal",
                    )?,
                    column: row.try_get("column_name").map_err(decode_error)?,
                    referenced_database: row
                        .try_get("referenced_table_schema")
                        .map_err(decode_error)?,
                    referenced_relation: row
                        .try_get("referenced_table_name")
                        .map_err(decode_error)?,
                    referenced_column: row
                        .try_get("referenced_column_name")
                        .map_err(decode_error)?,
                    referenced_ordinal: row
                        .try_get::<Option<u64>, _>("position_in_unique_constraint")
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
        let statement = format!(
            "SHOW CREATE {} {}.{}",
            if kind == CatalogKind::View {
                "VIEW"
            } else {
                "TABLE"
            },
            quote_identifier(schema),
            quote_identifier(name)
        );
        let row = sqlx::query(AssertSqlSafe(statement))
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        row.map(|row| row.try_get::<String, _>(1).map_err(decode_error))
            .transpose()
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
    format!(
        "{:02}\0{}",
        catalog_kind_rank(entry.kind),
        entry.qualified_name.object
    )
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

pub fn supports_catalog_version(version: &str) -> bool {
    if version.to_ascii_lowercase().contains("mariadb") {
        return false;
    }
    parse_version_triplet(version).is_some_and(|version| version >= (8, 0, 13))
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
        MySqlConstraintPart, MySqlIndexPart, PROBE_SQL, group_constraint_parts, group_index_parts,
        relation_path,
    };
    use crate::db::catalog::{CatalogId, CatalogKind};
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
