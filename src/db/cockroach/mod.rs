//! CockroachDB adapter. Only the wire protocol is shared with PostgreSQL.
mod transaction;
pub use transaction::CockroachTransactionBackend;

use super::{
    DatabaseError, ErrorCategory, RelationPreview, ServerInfo,
    catalog::*,
    pgwire,
    query::{ColumnMeta, QueryBudget, QueryOutcome, ResultSet},
};
use crate::{
    identity::ConnectionIdentity,
    model::{
        pagination::{PageRequest, ResultPagination, TotalRows},
        relation::RelationPreviewOptions,
    },
    profile::{CatalogScope, ConnectionProfile, DatabaseKind},
};
use secrecy::SecretString;
use sqlx::{
    AssertSqlSafe, Column, Connection, Executor, PgPool, Row, SqlSafeStr, Statement, TypeInfo,
    postgres::{PgConnection, PgPoolOptions, PgRow},
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use uuid::Uuid;

const SCHEMAS_SQL: &str = "SELECT nspname FROM pg_catalog.pg_namespace WHERE nspname <> 'information_schema' AND nspname <> 'crdb_internal' AND nspname NOT LIKE 'pg\\_%' ESCAPE '\\' ORDER BY nspname";
const RELATIONS_SQL: &str = "SELECT c.oid::INT8 AS object_id, n.nspname AS schema_name, c.relname AS object_name, c.relkind::TEXT AS native_kind FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind IN ('r','v') AND n.nspname <> 'information_schema' AND n.nspname <> 'crdb_internal' AND n.nspname NOT LIKE 'pg\\_%' ESCAPE '\\'";

#[derive(Clone, Debug)]
pub struct CockroachAdapter {
    pool: PgPool,
    connection_id: Uuid,
    catalog_scope: CatalogScope,
}

impl CockroachAdapter {
    pub const PROBE_SQL: &str =
        "SELECT version() AS version, current_database() AS database, current_user AS current_user";

    pub async fn connect(
        profile: &ConnectionProfile,
        password: Option<&SecretString>,
    ) -> Result<Self, DatabaseError> {
        if profile.kind != DatabaseKind::CockroachDb {
            return Err(DatabaseError::configuration("profile is not CockroachDB"));
        }
        let options = pgwire::connect_options(profile, password, 26257).await?;
        let pool = PgPoolOptions::new()
            .max_connections(6)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options)
            .await
            .map_err(network_error)?;
        let adapter = Self {
            pool,
            connection_id: profile.id,
            catalog_scope: profile.catalog_scope.clone(),
        };
        if let Err(error) = adapter.probe().await {
            adapter.pool.close().await;
            return Err(error);
        }
        Ok(adapter)
    }

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        let row = sqlx::query(Self::PROBE_SQL)
            .fetch_one(&self.pool)
            .await
            .map_err(network_error)?;
        let version: String = row.try_get("version").map_err(decode_error)?;
        if !version.to_ascii_lowercase().contains("cockroachdb") {
            return Err(DatabaseError::unsupported(
                "CockroachDB profile requires a CockroachDB server",
            ));
        }
        Ok(ServerInfo {
            kind: DatabaseKind::CockroachDb,
            version,
            database: row.try_get("database").map_err(decode_error)?,
            current_user: Some(row.try_get("current_user").map_err(decode_error)?),
        })
    }

    pub fn catalog_capabilities() -> CatalogCapabilities {
        CatalogCapabilities {
            namespace_model: NamespaceModel::DatabaseAndSchema,
            top_level_groups: vec![ObjectGroup::Tables, ObjectGroup::Views],
            column_metadata: ColumnMetadataCapabilities {
                default_expression: true,
                ..Default::default()
            },
            supports_lazy_children: true,
        }
    }

    pub async fn discover_catalog_scope(&self) -> Result<CatalogDiscovery, DatabaseError> {
        let mut connection = self.pool.acquire().await.map_err(network_error)?;
        let database = current_database(&mut connection).await?;
        let schemas = sqlx::query_scalar(SCHEMAS_SQL)
            .fetch_all(&mut *connection)
            .await
            .map_err(sql_error)?;
        Ok(CatalogDiscovery {
            databases: vec![DiscoveredDatabase {
                name: database,
                schemas,
            }],
            warnings: vec![],
        })
    }

    pub async fn discoverable_databases(&self) -> Result<Vec<String>, DatabaseError> {
        sqlx::query_scalar("SELECT datname FROM pg_catalog.pg_database WHERE has_database_privilege(datname, 'CONNECT') ORDER BY datname")
            .fetch_all(&self.pool).await.map_err(sql_error)
    }

    pub async fn execute(&self, sql: &str) -> Result<QueryOutcome, DatabaseError> {
        self.execute_pool_with_budget(sql, QueryBudget::UNBOUNDED)
            .await
    }

    pub async fn execute_pool_with_budget(
        &self,
        sql: &str,
        budget: QueryBudget,
    ) -> Result<QueryOutcome, DatabaseError> {
        pgwire::execute_pool(&self.pool, pgwire::Server::Cockroach, sql, budget).await
    }

    pub async fn transaction_backend(&self) -> Result<CockroachTransactionBackend, DatabaseError> {
        Ok(CockroachTransactionBackend::new(
            self.pool.acquire().await.map_err(network_error)?,
        ))
    }

    pub async fn load_catalog_page(
        &self,
        request: &CatalogRequest,
    ) -> Result<CatalogPage, DatabaseError> {
        request
            .validate_for_profile(self.connection_id)
            .map_err(DatabaseError::invalid_catalog_request)?;
        let mut connection = self.pool.acquire().await.map_err(network_error)?;
        let mut transaction = connection.begin().await.map_err(sql_error)?;
        let result = self.catalog_page(&mut transaction, request).await;
        transaction.rollback().await.map_err(sql_error)?;
        result
    }

    async fn catalog_page(
        &self,
        connection: &mut PgConnection,
        request: &CatalogRequest,
    ) -> Result<CatalogPage, DatabaseError> {
        let database = current_database(connection).await?;
        let scope = &request.scope;
        match &request.key.target {
            CatalogTarget::Databases => {
                let databases: Vec<String> = sqlx::query_scalar("SELECT datname FROM pg_catalog.pg_database WHERE has_database_privilege(datname, 'CONNECT') ORDER BY datname")
                    .fetch_all(&mut *connection).await.map_err(sql_error)?;
                let entries = databases
                    .into_iter()
                    .filter(|name| scope.allows_database(name))
                    .map(|name| self.database_entry(&name))
                    .collect::<Result<Vec<_>, _>>()?;
                entry_page(request, entries)
            }
            CatalogTarget::Schemas { database: id } => {
                self.verify_database(id, &database)?;
                let schemas: Vec<String> = sqlx::query_scalar(SCHEMAS_SQL)
                    .fetch_all(&mut *connection)
                    .await
                    .map_err(sql_error)?;
                let entries = schemas
                    .into_iter()
                    .filter(|schema| scope.allows_schema(&database, schema))
                    .map(|schema| self.schema_entry(&database, &schema))
                    .collect::<Result<Vec<_>, _>>()?;
                entry_page(request, entries)
            }
            CatalogTarget::Groups { schema } => {
                let schema = self
                    .verify_schema(connection, schema, &database, scope)
                    .await?;
                let mut groups = vec![];
                for (group, kind) in [(ObjectGroup::Tables, "r"), (ObjectGroup::Views, "v")] {
                    let count: i64 = sqlx::query_scalar("SELECT count(*)::INT8 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind::TEXT=$2")
                        .bind(&schema).bind(kind).fetch_one(&mut *connection).await.map_err(sql_error)?;
                    groups.push(CatalogGroupSummary {
                        group,
                        object_count: CatalogCount::Exact(count as u64),
                    });
                }
                // Fixed group keys follow the same keyset contract as object entries.
                let count = groups.len() as u64;
                groups.retain(|group| {
                    after_cursor(
                        request,
                        &format!("{:?}", group.group),
                        &format!("{:?}", group.group),
                    )
                });
                let next = finalize_keyset_page(
                    &mut groups,
                    request.page_size,
                    |g| format!("{:?}", g.group),
                    |g| format!("{:?}", g.group),
                )
                .map_err(invariant)?;
                CatalogPage::groups(request, groups, CatalogCount::Exact(count), next)
                    .map_err(invariant)
            }
            CatalogTarget::Objects { schema, group } => {
                let schema = self
                    .verify_schema(connection, schema, &database, scope)
                    .await?;
                let kind = match group {
                    ObjectGroup::Tables => "r",
                    ObjectGroup::Views => "v",
                    _ => {
                        return Err(DatabaseError::unsupported_catalog_target(
                            DatabaseKind::CockroachDb,
                            &request.key.target,
                        ));
                    }
                };
                let (sort, tie) = cursor_parts(request)?;
                let sql = format!(
                    "{RELATIONS_SQL} AND n.nspname=$1 AND c.relkind::TEXT=$2 AND (c.relname, c.oid::TEXT) > ($3, $4) ORDER BY c.relname, c.oid::TEXT LIMIT $5"
                );
                let rows = sqlx::query(AssertSqlSafe(sql))
                    .bind(schema)
                    .bind(kind)
                    .bind(sort)
                    .bind(tie)
                    .bind((request.page_size + 1) as i64)
                    .fetch_all(&mut *connection)
                    .await
                    .map_err(sql_error)?;
                let entries = rows
                    .iter()
                    .map(|row| self.relation_entry(&database, row))
                    .collect::<Result<Vec<_>, _>>()?;
                entry_page(request, entries)
            }
            CatalogTarget::RelationChildren { relation } => {
                let entry = self.verify_relation(connection, relation, scope).await?;
                let entries = self.relation_children(connection, &entry).await?;
                entry_page(request, entries)
            }
            _ => Err(DatabaseError::unsupported_catalog_target(
                DatabaseKind::CockroachDb,
                &request.key.target,
            )),
        }
    }

    fn database_entry(&self, database: &str) -> Result<CatalogEntry, DatabaseError> {
        CatalogEntry::database(
            CatalogId::new(self.connection_id, CatalogKind::Database, [database]),
            qualified(database, None, database),
            "database",
            OptionalMetadata::Unsupported,
            true,
        )
        .map_err(invariant)
    }

    fn schema_entry(&self, database: &str, schema: &str) -> Result<CatalogEntry, DatabaseError> {
        CatalogEntry::schema(
            CatalogId::new(self.connection_id, CatalogKind::Schema, [database, schema]),
            CatalogId::new(self.connection_id, CatalogKind::Database, [database]),
            qualified(database, Some(schema), schema),
            "schema",
            OptionalMetadata::Unsupported,
            true,
        )
        .map_err(invariant)
    }

    fn relation_entry(&self, database: &str, row: &PgRow) -> Result<CatalogEntry, DatabaseError> {
        let schema: String = row.try_get("schema_name").map_err(decode_error)?;
        let name: String = row.try_get("object_name").map_err(decode_error)?;
        let native: String = row.try_get("native_kind").map_err(decode_error)?;
        let oid: i64 = row.try_get("object_id").map_err(decode_error)?;
        let (kind, native) = match native.as_str() {
            "r" => (CatalogKind::Table, "table"),
            "v" => (CatalogKind::View, "view"),
            _ => return Err(not_found()),
        };
        CatalogEntry::relation(
            CatalogId::new(
                self.connection_id,
                kind,
                [
                    database.to_owned(),
                    schema.clone(),
                    name.clone(),
                    oid.to_string(),
                ],
            ),
            CatalogId::new(self.connection_id, CatalogKind::Schema, [database, &schema]),
            qualified(database, Some(&schema), &name),
            native,
            OptionalMetadata::Unsupported,
            true,
        )
        .map_err(invariant)
    }

    fn verify_database(&self, id: &CatalogId, current: &str) -> Result<(), DatabaseError> {
        if id.profile_id() == self.connection_id
            && id.kind == CatalogKind::Database
            && id.native_path == [current]
        {
            Ok(())
        } else {
            Err(not_found())
        }
    }

    async fn verify_schema(
        &self,
        connection: &mut PgConnection,
        id: &CatalogId,
        current: &str,
        scope: &CatalogScope,
    ) -> Result<String, DatabaseError> {
        let [database, schema] = id.native_path.as_slice() else {
            return Err(not_found());
        };
        if id.profile_id() != self.connection_id
            || id.kind != CatalogKind::Schema
            || database != current
            || !scope.allows_schema(database, schema)
            || !visible_schema(schema)
        {
            return Err(not_found());
        }
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname=$1)",
        )
        .bind(schema)
        .fetch_one(&mut *connection)
        .await
        .map_err(sql_error)?;
        if exists {
            Ok(schema.clone())
        } else {
            Err(not_found())
        }
    }

    async fn resolve_on(
        &self,
        connection: &mut PgConnection,
        id: &CatalogId,
        scope: &CatalogScope,
    ) -> Result<Option<CatalogEntry>, DatabaseError> {
        let [database, schema, name, oid] = id.native_path.as_slice() else {
            return Ok(None);
        };
        if id.profile_id() != self.connection_id
            || !matches!(id.kind, CatalogKind::Table | CatalogKind::View)
            || schema.is_empty()
            || name.is_empty()
            || !scope.allows_schema(database, schema)
        {
            return Ok(None);
        }
        let Ok(oid) = oid.parse::<i64>() else {
            return Ok(None);
        };
        if database != &current_database(connection).await? {
            return Ok(None);
        }
        let sql = format!("{RELATIONS_SQL} AND c.oid=$1::OID");
        let row = sqlx::query(AssertSqlSafe(sql))
            .bind(oid)
            .fetch_optional(&mut *connection)
            .await
            .map_err(sql_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let entry = self.relation_entry(database, &row)?;
        if entry.kind != id.kind
            || !scope.allows_schema(
                database,
                entry.qualified_name.schema.as_deref().unwrap_or_default(),
            )
        {
            return Ok(None);
        }
        Ok(Some(entry))
    }

    async fn verify_relation(
        &self,
        connection: &mut PgConnection,
        relation: &CatalogId,
        scope: &CatalogScope,
    ) -> Result<CatalogEntry, DatabaseError> {
        match self.resolve_on(connection, relation, scope).await? {
            Some(entry) if entry.id == *relation => Ok(entry),
            _ => Err(not_found()),
        }
    }

    pub async fn resolve_relation_identity(
        &self,
        relation: &CatalogId,
    ) -> Result<Option<CatalogEntry>, DatabaseError> {
        self.resolve_relation_identity_with_scope(relation, &self.catalog_scope)
            .await
    }

    pub async fn resolve_relation_identity_with_scope(
        &self,
        relation: &CatalogId,
        scope: &CatalogScope,
    ) -> Result<Option<CatalogEntry>, DatabaseError> {
        scope
            .validate("", None)
            .map_err(|error| DatabaseError::configuration(error.to_string()))?;
        let mut connection = self.pool.acquire().await.map_err(network_error)?;
        self.resolve_on(&mut connection, relation, scope).await
    }

    async fn relation_children(
        &self,
        connection: &mut PgConnection,
        relation: &CatalogEntry,
    ) -> Result<Vec<CatalogEntry>, DatabaseError> {
        let database = relation
            .qualified_name
            .database
            .as_deref()
            .ok_or_else(not_found)?;
        let schema = relation
            .qualified_name
            .schema
            .as_deref()
            .ok_or_else(not_found)?;
        let name = &relation.qualified_name.object;
        let oid = relation.id.native_path[3]
            .parse::<i64>()
            .map_err(|_| not_found())?;
        let rows = sqlx::query("SELECT a.attnum::INT8 AS ordinal, a.attname, format_type(a.atttypid, a.atttypmod) AS native_type, a.attnotnull, pg_get_expr(d.adbin, d.adrelid) AS default_expression FROM pg_catalog.pg_attribute a LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum WHERE a.attrelid=$1::OID AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum")
            .bind(oid).fetch_all(&mut *connection).await.map_err(sql_error)?;
        let mut entries = vec![];
        for row in rows {
            let column: String = row.try_get("attname").map_err(decode_error)?;
            let ordinal: i64 = row.try_get("ordinal").map_err(decode_error)?;
            let mut metadata = ColumnMetadata::new(
                ordinal as u32,
                row.try_get::<String, _>("native_type")
                    .map_err(decode_error)?,
                !row.try_get::<bool, _>("attnotnull").map_err(decode_error)?,
            );
            metadata.default_expression = OptionalMetadata::Supported(
                row.try_get("default_expression").map_err(decode_error)?,
            );
            let mut path = relation.id.native_path.clone();
            path.extend(["column".into(), ordinal.to_string(), column.clone()]);
            entries.push(
                CatalogEntry::relation_child(
                    CatalogId::new(self.connection_id, CatalogKind::Column, path),
                    relation.id.clone(),
                    qualified(database, Some(schema), &column),
                    "column",
                    OptionalMetadata::Unsupported,
                    CatalogMetadata::Column(metadata),
                )
                .map_err(invariant)?,
            );
        }
        if relation.kind == CatalogKind::Table {
            let sql = format!("SHOW INDEX FROM {}.{}", quote(schema), quote(name));
            let rows = sqlx::query(AssertSqlSafe(sql))
                .fetch_all(&mut *connection)
                .await
                .map_err(sql_error)?;
            let mut indexes: BTreeMap<String, (bool, Vec<(i64, String)>)> = BTreeMap::new();
            for row in rows {
                if row.try_get::<bool, _>("storing").map_err(decode_error)?
                    || row.try_get::<bool, _>("implicit").map_err(decode_error)?
                {
                    continue;
                }
                let index: String = row.try_get("index_name").map_err(decode_error)?;
                let non_unique: bool = row.try_get("non_unique").map_err(decode_error)?;
                let seq: i64 = row.try_get("seq_in_index").map_err(decode_error)?;
                let column: String = row.try_get("column_name").map_err(decode_error)?;
                indexes
                    .entry(index)
                    .or_insert_with(|| (!non_unique, vec![]))
                    .1
                    .push((seq, column));
            }
            let primary: Option<String> = sqlx::query_scalar("SELECT conname FROM pg_catalog.pg_constraint WHERE conrelid=$1::OID AND contype='p'")
                .bind(oid).fetch_optional(&mut *connection).await.map_err(sql_error)?;
            for (index, (unique, mut columns)) in indexes {
                columns.sort_by_key(|(seq, _)| *seq);
                let columns: Vec<String> = columns.into_iter().map(|(_, column)| column).collect();
                let mut path = relation.id.native_path.clone();
                path.extend(["index".into(), index.clone()]);
                entries.push(
                    CatalogEntry::relation_child(
                        CatalogId::new(self.connection_id, CatalogKind::Index, path),
                        relation.id.clone(),
                        qualified(database, Some(schema), &index),
                        "index",
                        OptionalMetadata::Unsupported,
                        CatalogMetadata::Index(IndexMetadata {
                            columns: columns.clone(),
                            unique,
                        }),
                    )
                    .map_err(invariant)?,
                );
                if primary.as_deref() == Some(index.as_str()) {
                    let mut path = relation.id.native_path.clone();
                    path.extend(["primary_key".into(), index.clone()]);
                    let id = CatalogId::new(self.connection_id, CatalogKind::PrimaryKey, path);
                    for entry in &mut entries {
                        if let CatalogMetadata::Column(metadata) = &mut entry.metadata
                            && let Some(position) = columns
                                .iter()
                                .position(|column| column == &entry.qualified_name.object)
                        {
                            metadata.constraint_memberships.push(ConstraintMembership {
                                constraint_id: id.clone(),
                                ordinal_position: position as u32 + 1,
                            });
                        }
                    }
                    entries.push(
                        CatalogEntry::relation_child(
                            id,
                            relation.id.clone(),
                            qualified(database, Some(schema), &index),
                            "primary_key",
                            OptionalMetadata::Unsupported,
                            CatalogMetadata::Constraint(ConstraintMetadata::PrimaryKey { columns }),
                        )
                        .map_err(invariant)?,
                    );
                }
            }
        }
        Ok(entries)
    }

    pub async fn preview_relation(
        &self,
        relation: &CatalogId,
        options: &RelationPreviewOptions,
        page: PageRequest,
    ) -> Result<RelationPreview, DatabaseError> {
        self.preview_relation_with_scope(relation, &self.catalog_scope, options, page)
            .await
    }

    pub async fn preview_relation_with_scope(
        &self,
        relation: &CatalogId,
        scope: &CatalogScope,
        options: &RelationPreviewOptions,
        mut page: PageRequest,
    ) -> Result<RelationPreview, DatabaseError> {
        scope
            .validate("", None)
            .map_err(|error| DatabaseError::configuration(error.to_string()))?;
        let mut connection = self.pool.acquire().await.map_err(network_error)?;
        let mut transaction = connection.begin().await.map_err(sql_error)?;
        let entry = self
            .verify_relation(&mut transaction, relation, scope)
            .await?;
        let schema = entry
            .qualified_name
            .schema
            .as_deref()
            .ok_or_else(not_found)?;
        let mut base = format!(
            "SELECT * FROM {}.{}",
            quote(schema),
            quote(&entry.qualified_name.object)
        );
        if let Some(clause) = &options.where_clause {
            base.push_str(" WHERE ");
            base.push_str(clause);
        }
        if let Some(clause) = &options.order_by_clause {
            base.push_str(" ORDER BY ");
            base.push_str(clause);
        }
        let total = if page.resolve_total {
            let sql = format!("SELECT count(*)::INT8 FROM ({base}) AS __lazydb_count");
            let count: i64 = sqlx::query_scalar(AssertSqlSafe(sql))
                .fetch_one(&mut *transaction)
                .await
                .map_err(sql_error)?;
            let count = u64::try_from(count)
                .map_err(|_| DatabaseError::internal("negative CockroachDB row count"))?;
            page.offset = ResultPagination::last_offset(page.size, count);
            Some(count)
        } else {
            None
        };
        let sql = format!(
            "{base} LIMIT {} OFFSET {}",
            page.size.lookahead_limit(),
            page.offset
        );
        let started = Instant::now();
        let statement = transaction
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
            .fetch_all(&mut *transaction)
            .await
            .map_err(sql_error)?;
        let fetched = rows.len();
        rows.truncate(page.size.get());
        let result = QueryOutcome::from_result_set(
            ResultSet {
                columns,
                rows: rows.iter().map(pgwire::decode::decode_row).collect(),
                affected_rows: 0,
            },
            started.elapsed(),
            Duration::ZERO,
        );
        transaction.rollback().await.map_err(sql_error)?;
        let mut pagination = ResultPagination::from_page(page, fetched);
        if let Some(total) = total {
            pagination.total = TotalRows::Exact(total);
        }
        Ok(RelationPreview {
            sql,
            result,
            pagination,
            row_versions: None,
        })
    }

    pub async fn relation_ddl(&self, relation: &CatalogId) -> Result<RelationDdl, DatabaseError> {
        self.relation_ddl_with_scope(relation, &self.catalog_scope)
            .await
    }

    pub async fn relation_ddl_with_scope(
        &self,
        relation: &CatalogId,
        scope: &CatalogScope,
    ) -> Result<RelationDdl, DatabaseError> {
        scope
            .validate("", None)
            .map_err(|error| DatabaseError::configuration(error.to_string()))?;
        let mut connection = self.pool.acquire().await.map_err(network_error)?;
        let mut transaction = connection.begin().await.map_err(sql_error)?;
        let entry = self
            .verify_relation(&mut transaction, relation, scope)
            .await?;
        let schema = entry
            .qualified_name
            .schema
            .as_deref()
            .ok_or_else(not_found)?;
        let sql = native_ddl(
            &mut transaction,
            entry.kind,
            schema,
            &entry.qualified_name.object,
        )
        .await?;
        let request = CatalogRequest {
            key: CatalogRequestKey {
                connection: ConnectionIdentity {
                    profile_id: self.connection_id,
                    generation: 0,
                },
                catalog_epoch: 0,
                request_id: 0,
                target: CatalogTarget::RelationChildren {
                    relation: relation.clone(),
                },
                cursor: None,
            },
            scope: scope.clone(),
            page_size: MAX_CATALOG_PAGE_SIZE,
        };
        let entries = self.relation_children(&mut transaction, &entry).await?;
        let children = entry_page(&request, entries)?;
        transaction.rollback().await.map_err(sql_error)?;
        Ok(RelationDdl {
            relation: entry,
            children,
            sql,
            provenance: DdlProvenance::NativeCatalog,
        })
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
        let mut connection = self.pool.acquire().await.map_err(network_error)?;
        let database = current_database(&mut connection).await?;
        if !visible_schema(schema) || !self.catalog_scope.allows_schema(&database, schema) {
            return Err(not_found());
        }
        native_ddl(&mut connection, kind, schema, name)
            .await
            .map(Some)
    }

    pub async fn search_catalog(
        &self,
        request: &CatalogSearchRequest,
    ) -> Result<CatalogSearchPage, DatabaseError> {
        request
            .validate()
            .map_err(DatabaseError::invalid_catalog_request)?;
        if request.connection.profile_id != self.connection_id {
            return Err(DatabaseError::configuration(
                "catalog search profile mismatch",
            ));
        }
        let mut connection = self.pool.acquire().await.map_err(network_error)?;
        let mut transaction = connection.begin().await.map_err(sql_error)?;
        let database = current_database(&mut transaction).await?;
        if !request.scope.allows_database(&database) {
            return CatalogSearchPage::new(request, vec![], Some(0), false).map_err(invariant);
        }
        let (query, normalized) = search_query(&request.query);
        // SQL filters and limits candidates before transfer. Names use the same separator-insensitive convention as the other adapters.
        let schemas: Vec<String> = sqlx::query_scalar(SCHEMAS_SQL)
            .fetch_all(&mut *transaction)
            .await
            .map_err(sql_error)?;
        let schemas: Vec<String> = schemas
            .into_iter()
            .filter(|schema| request.scope.allows_schema(&database, schema))
            .collect();
        let name_expr = if normalized {
            "regexp_replace(lower(c.relname), '[^[:alnum:]]', '', 'g')"
        } else {
            "lower(c.relname)"
        };
        let path_expr = if normalized {
            "regexp_replace(lower($3 || '.' || n.nspname || '.' || c.relname), '[^[:alnum:]]', '', 'g')"
        } else {
            "lower($3 || '.' || n.nspname || '.' || c.relname)"
        };
        let sql = format!(
            "{RELATIONS_SQL} AND n.nspname=ANY($1::TEXT[]) AND (strpos({name_expr}, $2)>0 OR strpos({path_expr}, $2)>0) ORDER BY n.nspname,c.relname,c.oid LIMIT $4"
        );
        let rows = sqlx::query(AssertSqlSafe(sql))
            .bind(&schemas)
            .bind(&query)
            .bind(&database)
            .bind((request.limit + 1) as i64)
            .fetch_all(&mut *transaction)
            .await
            .map_err(sql_error)?;
        let mut hits = vec![];
        if request.object_scope.includes(CatalogKind::Database)
            && search_text_matches(&database, &request.query)
        {
            hits.push(CatalogSearchHit {
                entry: self.database_entry(&database)?,
                ancestors: vec![],
            });
        }
        if request.object_scope.includes(CatalogKind::Schema) {
            for schema in &schemas {
                if search_text_matches(schema, &request.query)
                    || search_text_matches(&format!("{database}.{schema}"), &request.query)
                {
                    hits.push(CatalogSearchHit {
                        entry: self.schema_entry(&database, schema)?,
                        ancestors: vec![self.database_entry(&database)?],
                    });
                }
            }
        }
        for row in rows {
            let entry = self.relation_entry(&database, &row)?;
            let schema = entry
                .qualified_name
                .schema
                .as_deref()
                .ok_or_else(not_found)?;
            hits.push(CatalogSearchHit {
                ancestors: vec![
                    self.database_entry(&database)?,
                    self.schema_entry(&database, schema)?,
                ],
                entry,
            });
        }
        if request.object_scope.includes(CatalogKind::Column) {
            let child_name = if normalized {
                "regexp_replace(lower(child_name), '[^[:alnum:]]', '', 'g')"
            } else {
                "lower(child_name)"
            };
            let child_path = if normalized {
                "regexp_replace(lower($3 || '.' || schema_name || '.' || object_name || '.' || child_name), '[^[:alnum:]]', '', 'g')"
            } else {
                "lower($3 || '.' || schema_name || '.' || object_name || '.' || child_name)"
            };
            let sql = format!(
                "WITH relations AS ({RELATIONS_SQL} AND n.nspname=ANY($1::TEXT[])), children AS (\
                 SELECT r.*, a.attname AS child_name, 'column' AS child_kind FROM relations r \
                 JOIN pg_catalog.pg_attribute a ON a.attrelid=r.object_id::OID WHERE a.attnum>0 AND NOT a.attisdropped \
                 UNION ALL SELECT r.*, ic.relname, 'index' FROM relations r \
                 JOIN pg_catalog.pg_index i ON i.indrelid=r.object_id::OID JOIN pg_catalog.pg_class ic ON ic.oid=i.indexrelid \
                 UNION ALL SELECT r.*, con.conname, 'primary_key' FROM relations r \
                 JOIN pg_catalog.pg_constraint con ON con.conrelid=r.object_id::OID WHERE con.contype='p') \
                 SELECT * FROM children WHERE strpos({child_name}, $2)>0 OR strpos({child_path}, $2)>0 \
                 ORDER BY schema_name,object_name,child_name,child_kind LIMIT $4"
            );
            let rows = sqlx::query(AssertSqlSafe(sql))
                .bind(&schemas)
                .bind(&query)
                .bind(&database)
                .bind((request.limit + 1) as i64)
                .fetch_all(&mut *transaction)
                .await
                .map_err(sql_error)?;
            let mut cached_children: BTreeMap<i64, Vec<CatalogEntry>> = BTreeMap::new();
            for row in rows {
                let relation = self.relation_entry(&database, &row)?;
                let oid: i64 = row.try_get("object_id").map_err(decode_error)?;
                let name: String = row.try_get("child_name").map_err(decode_error)?;
                let kind: String = row.try_get("child_kind").map_err(decode_error)?;
                let kind = match kind.as_str() {
                    "column" => CatalogKind::Column,
                    "index" => CatalogKind::Index,
                    _ => CatalogKind::PrimaryKey,
                };
                if let std::collections::btree_map::Entry::Vacant(slot) = cached_children.entry(oid)
                {
                    slot.insert(self.relation_children(&mut transaction, &relation).await?);
                }
                if let Some(entry) = cached_children[&oid]
                    .iter()
                    .find(|entry| entry.kind == kind && entry.qualified_name.object == name)
                {
                    let schema = relation
                        .qualified_name
                        .schema
                        .as_deref()
                        .ok_or_else(not_found)?;
                    hits.push(CatalogSearchHit {
                        entry: entry.clone(),
                        ancestors: vec![
                            self.database_entry(&database)?,
                            self.schema_entry(&database, schema)?,
                            relation,
                        ],
                    });
                }
            }
        }
        hits.sort_by_key(|hit| hit.qualified_path());
        let truncated = hits.len() > request.limit;
        hits.truncate(request.limit);
        transaction.rollback().await.map_err(sql_error)?;
        let count = if truncated { None } else { Some(hits.len()) };
        CatalogSearchPage::new(request, hits, count, truncated).map_err(invariant)
    }

    pub async fn close(self) {
        self.pool.close().await;
    }
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn visible_schema(schema: &str) -> bool {
    schema != "information_schema" && schema != "crdb_internal" && !schema.starts_with("pg_")
}
fn qualified(database: &str, schema: Option<&str>, name: &str) -> QualifiedName {
    QualifiedName {
        database: Some(database.into()),
        schema: schema.map(str::to_owned),
        object: name.into(),
    }
}
async fn current_database(connection: &mut PgConnection) -> Result<String, DatabaseError> {
    sqlx::query_scalar("SELECT current_database()")
        .fetch_one(connection)
        .await
        .map_err(sql_error)
}
async fn native_ddl(
    connection: &mut PgConnection,
    kind: CatalogKind,
    schema: &str,
    name: &str,
) -> Result<String, DatabaseError> {
    let kind = match kind {
        CatalogKind::Table => "TABLE",
        CatalogKind::View => "VIEW",
        _ => {
            return Err(DatabaseError::unsupported(
                "CockroachDB DDL kind is unsupported",
            ));
        }
    };
    let sql = format!("SHOW CREATE {kind} {}.{}", quote(schema), quote(name));
    let row = sqlx::query(AssertSqlSafe(sql))
        .fetch_one(connection)
        .await
        .map_err(sql_error)?;
    row.try_get("create_statement").map_err(decode_error)
}
fn sort_key(entry: &CatalogEntry) -> String {
    entry.qualified_name.object.clone()
}
fn tie_key(entry: &CatalogEntry) -> String {
    if entry.kind.is_relation() {
        entry.id.native_path[3].clone()
    } else {
        entry.id.native_path.join("/") + &format!("/{:?}", entry.kind)
    }
}
fn cursor_parts(request: &CatalogRequest) -> Result<(&str, &str), DatabaseError> {
    request
        .key
        .cursor
        .as_ref()
        .map(CatalogCursor::keyset_parts)
        .transpose()
        .map(|parts| parts.unwrap_or(("", "")))
        .map_err(DatabaseError::invalid_catalog_request)
}
fn after_cursor(request: &CatalogRequest, sort: &str, tie: &str) -> bool {
    request
        .key
        .cursor
        .as_ref()
        .is_none_or(|cursor| cursor.keyset_parts().is_ok_and(|parts| (sort, tie) > parts))
}
fn entry_page(
    request: &CatalogRequest,
    mut entries: Vec<CatalogEntry>,
) -> Result<CatalogPage, DatabaseError> {
    entries.sort_by_key(|entry| (sort_key(entry), tie_key(entry)));
    entries.retain(|entry| after_cursor(request, &sort_key(entry), &tie_key(entry)));
    let next = finalize_keyset_page(&mut entries, request.page_size, sort_key, tie_key)
        .map_err(invariant)?;
    let count = if request.key.cursor.is_none() && next.is_none() {
        CatalogCount::Exact(entries.len() as u64)
    } else {
        CatalogCount::Unknown
    };
    CatalogPage::new(request, entries, count, next).map_err(invariant)
}
fn not_found() -> DatabaseError {
    DatabaseError { category: ErrorCategory::Sql, code: Some("catalog_target_not_found".into()), message: "CockroachDB catalog target no longer exists, belongs to another database, or is outside the requested scope".into(), diagnostic: None }
}
fn invariant(error: CatalogValidationError) -> DatabaseError {
    DatabaseError::internal(format!("CockroachDB catalog invariant: {error}"))
}
fn sql_error(error: sqlx::Error) -> DatabaseError {
    DatabaseError::from_sqlx(error, ErrorCategory::Sql)
}
fn network_error(error: sqlx::Error) -> DatabaseError {
    DatabaseError::from_sqlx(error, ErrorCategory::Network)
}
fn decode_error(error: sqlx::Error) -> DatabaseError {
    DatabaseError::from_sqlx(error, ErrorCategory::Internal)
}
