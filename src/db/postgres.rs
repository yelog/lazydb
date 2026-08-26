use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

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
        CatalogRequestKey, CatalogTarget, CatalogValidationError, ColumnMetadata,
        ColumnMetadataCapabilities, ConstraintMembership, ConstraintMetadata, Ddl, DdlProvenance,
        DiscoveredDatabase, IndexMetadata, NamespaceModel, ObjectGroup, OptionalMetadata,
        QualifiedName, RelationStructure, finalize_keyset_page,
    },
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

impl PostgresAdapter {
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

        let pool = PgPoolOptions::new()
            .max_connections(6)
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
             (SELECT COUNT(*) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname=$1 AND p.prokind='f')::bigint AS functions, \
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
                "c.prokind='f'",
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
        let indexes = self.load_pg_indexes(connection, relation_oid).await?;
        let constraints = self.load_pg_constraints(connection, relation_oid).await?;
        let mut memberships: HashMap<String, Vec<ConstraintMembership>> = HashMap::new();
        let mut entries = Vec::new();

        for index in indexes {
            entries.push(
                CatalogEntry::relation_child(
                    relation_child_id(relation, CatalogKind::Index, &index.oid.to_string()),
                    relation.clone(),
                    qualified_object(&database, &schema, &index.name),
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
                            database: Some(database.clone()),
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
                    qualified_object(&database, &schema, &constraint.name),
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
                    qualified_object(&database, &schema, &name),
                    "column",
                    OptionalMetadata::Supported(row.try_get("comment").map_err(decode_error)?),
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
        let sql = format!(
            "SELECT * FROM {}.{} LIMIT {}",
            quote_identifier(&schema),
            quote_identifier(&name),
            RELATION_PREVIEW_LIMIT
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
        let (database, schema, name, _, native_kind) = self
            .verify_relation(&mut connection, relation, &target)
            .await?;
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
        let ddl = self.object_ddl(relation.kind, &schema, &name).await?;
        let provenance = if relation.kind == CatalogKind::MaterializedView {
            DdlProvenance::AdapterGenerated
        } else if ddl.is_some() {
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

pub const fn supports_server_version(server_version_num: i32) -> bool {
    server_version_num >= 120_000
}

pub fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
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
