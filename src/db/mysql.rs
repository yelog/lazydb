use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use futures_util::TryStreamExt;
use secrecy::{ExposeSecret, SecretString};
use sqlx::{
    AssertSqlSafe, Column, Connection, Either, Executor, MySqlPool, Row, SqlSafeStr, Statement,
    Type, TypeInfo, ValueRef,
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
    catalog_mutation::{
        CatalogMutationAnchor, CatalogMutationAvailability, CatalogMutationCapabilities,
        CatalogMutationError, CatalogMutationExecutionMode, CatalogMutationOption,
        CatalogMutationPlan, CatalogMutationRequest, CatalogMutationTarget,
        CatalogObjectDefinition, CatalogObjectDefinitionRequest, CatalogObjectType,
        CatalogSelectionHint, ColumnDefinition, TableDefinition, ViewDefinition, ViewOption,
    },
    ddl::{DdlSection, assemble_ddl},
    mutation::{InputValue, MutationResult, RelationMutation, RelationMutationRequest},
    principal::{
        PrincipalDdl, PrincipalEntry, PrincipalId, PrincipalKind, PrincipalPage, PrincipalScope,
    },
    query::{
        ColumnMeta, QueryBudget, QueryOutcome, QueryOutcomeAccumulator, RELATION_PREVIEW_LIMIT,
        ResultSet,
    },
    sanitize_terminal_text,
    value::CellValue,
};
use crate::model::dashboard::MetricKey;

mod geometry;
mod tidb;

fn mysql_column_definition(
    column: &crate::model::catalog_editor::ColumnDraft,
) -> Result<String, CatalogMutationError> {
    let name = column.name.value().trim();
    let native_type = column.native_type.value().trim();
    if name.is_empty() || native_type.is_empty() {
        return Err(CatalogMutationError::InvalidDraft {
            reason: "MySQL column name and type are required".into(),
        });
    }
    let mut definition = format!("{} {}", quote_identifier(name), native_type);
    if !column.nullable {
        definition.push_str(" NOT NULL");
    }
    if column.identity {
        definition.push_str(" AUTO_INCREMENT");
    }
    if !column.collation.value().trim().is_empty() {
        definition.push_str(" COLLATE ");
        definition.push_str(column.collation.value().trim());
    }
    if !column.default_expression.value().trim().is_empty() {
        definition.push_str(" DEFAULT ");
        definition.push_str(column.default_expression.value().trim());
    }
    if !column.generated_expression.value().trim().is_empty() {
        definition.push_str(" GENERATED ALWAYS AS (");
        definition.push_str(column.generated_expression.value().trim());
        definition.push(')');
    }
    definition.push_str(" COMMENT ");
    definition.push_str(&quote_literal(column.comment.value()));
    Ok(definition)
}

fn comment_changed(before: &OptionalMetadata<String>, after: &str) -> bool {
    match before {
        OptionalMetadata::Supported(value) => value.as_deref().unwrap_or("") != after,
        OptionalMetadata::Unsupported => !after.is_empty(),
    }
}

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

const CATALOG_PAGE_INDEXES_MARIADB_SQL: &str = r#"
SELECT index_name, non_unique, CAST(seq_in_index AS UNSIGNED), column_name, NULL AS expression
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
), normalized AS (
    SELECT *, REGEXP_REPLACE(LOWER(object_name), '[^[:alnum:]]', '') AS normalized_name,
              REGEXP_REPLACE(LOWER(qualified_path), '[^[:alnum:]]', '') AS normalized_path
    FROM candidates
), searchable AS (
    SELECT *, IF(?, normalized_name, LOWER(object_name)) AS search_name,
              IF(?, normalized_path, LOWER(qualified_path)) AS search_path
    FROM normalized
)
SELECT kind, database_name, object_name, relation_name, relation_type, native_identity, comment
FROM searchable
WHERE {scope_predicate}
  AND database_name NOT IN ('information_schema','mysql','performance_schema','sys')
  AND (? OR kind IN ('table','view'))
  AND (LOCATE(?, search_name) > 0 OR LOCATE(?, search_path) > 0)
ORDER BY CASE
    WHEN search_name=? THEN 0
    WHEN LOCATE(?, search_name)=1 THEN 1
    WHEN LOCATE(?, search_name)>0 THEN 2
    ELSE 3 END,
    LOWER(qualified_path), kind, BINARY native_identity
LIMIT 101
"#;

const MARIADB_CATALOG_SEARCH_SEQUENCE_SQL: &str = " UNION ALL \
    SELECT 'sequence', table_schema, table_name, NULL, NULL, table_name, \
           CONCAT(table_schema,'.',table_name), NULL \
    FROM information_schema.tables WHERE table_type='SEQUENCE'";

pub const CATALOG_DATABASES_SQL: &str = r#"
SELECT schema_name
FROM information_schema.schemata
WHERE schema_name NOT IN ('information_schema', 'mysql', 'performance_schema', 'sys')
ORDER BY BINARY schema_name
"#;

#[derive(Clone, Debug)]
pub struct MySqlAdapter {
    pool: MySqlPool,
    kind: DatabaseKind,
    connection_id: Uuid,
    catalog_scope: CatalogScope,
    read_only: bool,
}

pub(crate) fn mysql_membership_coverage() -> crate::db::principal::PrincipalCoverage {
    crate::db::principal::PrincipalCoverage::Unavailable(
        "MySQL/MariaDB role membership is dialect-specific and not normalized in this path".into(),
    )
}

pub(crate) fn mysql_show_grants_coverage() -> crate::db::principal::PrincipalCoverage {
    crate::db::principal::PrincipalCoverage::Partial(
        "SHOW GRANTS is retained as native text until dialect-specific parsing completes".into(),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerCapabilities {
    pub catalog: bool,
    pub sequences: bool,
    pub relation_edit: bool,
}

impl ServerCapabilities {
    pub fn for_kind(kind: DatabaseKind, version: &str) -> Self {
        let catalog = supports_catalog_version_for_kind(kind, version);
        Self {
            catalog,
            sequences: matches!(kind, DatabaseKind::MariaDb) && catalog,
            // Data-grid editing is not advertised until the complete mutation
            // round trip has passed the MariaDB integration suite.
            relation_edit: false,
        }
    }
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
    pub async fn preview_relation_with_scope(
        &self,
        relation: &CatalogId,
        scope: &CatalogScope,
        options: &crate::model::relation::RelationPreviewOptions,
        page: crate::model::pagination::PageRequest,
    ) -> Result<crate::db::RelationPreview, DatabaseError> {
        let mut adapter = self.clone();
        adapter.catalog_scope = scope.clone();
        adapter.preview_relation(relation, options, page).await
    }

    pub async fn relation_ddl_with_scope(
        &self,
        relation: &CatalogId,
        scope: &CatalogScope,
    ) -> Result<RelationDdl, DatabaseError> {
        let mut adapter = self.clone();
        adapter.catalog_scope = scope.clone();
        adapter.relation_ddl(relation).await
    }
    pub const MONITOR_STATUS_SQL: &str = "SHOW GLOBAL STATUS WHERE Variable_name IN ('Queries','Com_commit','Com_rollback','Com_select','Com_insert','Com_update','Com_delete','Threads_connected','Threads_running','Innodb_buffer_pool_read_requests','Innodb_buffer_pool_reads','Created_tmp_files','Bytes_received','Bytes_sent','Connections','Aborted_clients','Aborted_connects','Uptime')";
    pub const MONITOR_METADATA_SQL: &str =
        "SHOW GLOBAL VARIABLES WHERE Variable_name IN ('version','max_connections')";
    pub const PROCESS_LIST_SQL: &str = "SHOW FULL PROCESSLIST";

    pub async fn load_monitor_snapshot(
        &self,
    ) -> Result<crate::db::monitor::MonitorSnapshot, DatabaseError> {
        self.reject_tidb_operation("monitoring")?;
        let rows = sqlx::query(Self::MONITOR_STATUS_SQL)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let mut values = std::collections::BTreeMap::new();
        for row in rows {
            let name: String = row.try_get("Variable_name").map_err(decode_error)?;
            let value: String = row.try_get("Value").map_err(decode_error)?;
            let Some(value) = crate::db::monitor::status_value(&value) else {
                continue;
            };
            let key = match name.to_ascii_lowercase().as_str() {
                "queries" => MetricKey::Queries,
                "com_commit" => MetricKey::Commits,
                "com_rollback" => MetricKey::Rollbacks,
                "com_select" => MetricKey::Selects,
                "com_insert" => MetricKey::Inserts,
                "com_update" => MetricKey::Updates,
                "com_delete" => MetricKey::Deletes,
                "threads_connected" => MetricKey::Connections,
                "threads_running" => MetricKey::ActiveConnections,
                "innodb_buffer_pool_read_requests" => MetricKey::BlockHits,
                "innodb_buffer_pool_reads" => MetricKey::BlockReads,
                "created_tmp_files" => MetricKey::TempFiles,
                "bytes_received" => MetricKey::BytesRead,
                "bytes_sent" => MetricKey::BytesWritten,
                "connections" => MetricKey::Connections,
                "aborted_clients" => MetricKey::AbortedClients,
                "aborted_connects" => MetricKey::AbortedConnections,
                "uptime" => MetricKey::ServerUptime,
                _ => continue,
            };
            values.insert(key, value);
        }
        let commits = values.get(&MetricKey::Commits).copied().unwrap_or_default();
        let rollbacks = values
            .get(&MetricKey::Rollbacks)
            .copied()
            .unwrap_or_default();
        values.insert(MetricKey::Transactions, commits + rollbacks);
        Ok(crate::db::monitor::MonitorSnapshot {
            server_time_millis: chrono::Utc::now().timestamp_millis() as u64,
            // Uptime is sampled for reset detection, not used as a generation because it changes every poll.
            server_generation: 1,
            values,
            redis_details: None,
            visibility: crate::db::monitor::MonitorVisibility::Unknown,
        })
    }

    pub async fn load_monitor_metadata(
        &self,
    ) -> Result<crate::db::monitor::MonitorMetadata, DatabaseError> {
        self.reject_tidb_operation("monitoring")?;
        let rows = sqlx::query(Self::MONITOR_METADATA_SQL)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let mut metadata = crate::db::monitor::MonitorMetadata::default();
        for row in rows {
            let name: String = row.try_get("Variable_name").map_err(decode_error)?;
            let value: String = row.try_get("Value").map_err(decode_error)?;
            match name.to_ascii_lowercase().as_str() {
                "version" => metadata.version = Some(sanitize_terminal_text(&value)),
                "max_connections" => metadata.max_connections = value.parse().ok(),
                _ => {}
            }
        }
        Ok(metadata)
    }

    pub async fn load_process_snapshot(
        &self,
    ) -> Result<crate::db::monitor::ProcessSnapshot, DatabaseError> {
        self.reject_tidb_operation("process monitoring")?;
        let rows = sqlx::query(Self::PROCESS_LIST_SQL)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Sql))?;
        let truncated = rows.len() > crate::db::monitor::MAX_PROCESS_ROWS;
        let rows = rows
            .into_iter()
            .take(crate::db::monitor::MAX_PROCESS_ROWS)
            .map(|row| {
                let query = row
                    .try_get::<Option<String>, _>("Info")
                    .map_err(decode_error)?;
                Ok(crate::model::dashboard::ProcessRow {
                    id: row.try_get("Id").map_err(decode_error)?,
                    native_id: None,
                    user: sanitize_terminal_text(
                        &row.try_get::<String, _>("User").map_err(decode_error)?,
                    ),
                    database: row.try_get("db").map_err(decode_error)?,
                    client: row.try_get("Host").map_err(decode_error)?,
                    application: row.try_get("Command").map_err(decode_error)?,
                    state: row.try_get("State").map_err(decode_error)?,
                    wait: None,
                    elapsed: Duration::from_secs(row.try_get("Time").map_err(decode_error)?),
                    query: query.map(|value| sanitize_terminal_text(&value)),
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
            CatalogKind::Sequence if entry.id.native_path.len() == 3 => format!(
                "DROP SEQUENCE {}.{}",
                quote_identifier(&entry.id.native_path[1]),
                quote_identifier(&entry.id.native_path[2])
            ),
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

    pub fn mariadb_catalog_capabilities() -> CatalogCapabilities {
        let mut capabilities = Self::catalog_capabilities();
        capabilities.top_level_groups.push(ObjectGroup::Sequences);
        capabilities
    }

    pub fn catalog_mutation_capabilities() -> CatalogMutationCapabilities {
        CatalogMutationCapabilities {
            profile_create: vec![CatalogMutationOption {
                object_type: CatalogObjectType::Catalog(CatalogKind::Database),
                availability: CatalogMutationAvailability::Available,
            }],
            create: [CatalogKind::Table, CatalogKind::View]
                .into_iter()
                .map(|kind| CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(kind),
                    availability: CatalogMutationAvailability::Available,
                })
                .collect(),
            edit: [CatalogKind::Table, CatalogKind::View]
                .into_iter()
                .map(|kind| CatalogMutationOption {
                    object_type: CatalogObjectType::Catalog(kind),
                    availability: CatalogMutationAvailability::Available,
                })
                .collect(),
            ..CatalogMutationCapabilities::default()
        }
    }

    pub fn plan_catalog_mutation(
        request: CatalogMutationRequest,
        draft: crate::model::catalog_editor::CatalogDraft,
        baseline: Option<CatalogObjectDefinition>,
    ) -> Result<CatalogMutationPlan, CatalogMutationError> {
        if request.mode == crate::db::catalog_mutation::CatalogMutationMode::Edit {
            let CatalogMutationAnchor::Catalog(object) = &request.anchor else {
                return Err(CatalogMutationError::InvalidAnchor {
                    reason: "MySQL edit requires a catalog object anchor",
                });
            };
            if object.kind == CatalogKind::View {
                let Some(CatalogObjectDefinition::View(_)) = baseline else {
                    return Err(CatalogMutationError::StaleState);
                };
                let crate::model::catalog_editor::CatalogDraft::View(draft) = draft else {
                    return Err(CatalogMutationError::InvalidDraft {
                        reason: "MySQL view edit requires a view draft".into(),
                    });
                };
                draft.validate()?;
                let [database, schema, name] = object.native_path.as_slice() else {
                    return Err(CatalogMutationError::InvalidAnchor {
                        reason: "MySQL view identity is incomplete",
                    });
                };
                let object_id = object.clone();
                let database_name = database.clone();
                let schema_name = schema.clone();
                let name_value = name.clone();
                let new_name = draft.name.value().trim();
                if new_name.is_empty() {
                    return Err(CatalogMutationError::InvalidDraft {
                        reason: "MySQL view name is required".into(),
                    });
                }
                let new_object = CatalogId::new(
                    request.connection.profile_id,
                    CatalogKind::View,
                    [database.clone(), schema.clone(), new_name.to_owned()],
                );
                let mut statements = Vec::new();
                if new_name != name {
                    statements.push(format!(
                        "RENAME TABLE {}.{} TO {}.{}",
                        quote_identifier(&schema_name),
                        quote_identifier(&name_value),
                        quote_identifier(&schema_name),
                        quote_identifier(new_name)
                    ));
                }
                statements.push(format!(
                    "CREATE OR REPLACE VIEW {}.{} AS {}",
                    quote_identifier(&schema_name),
                    quote_identifier(new_name),
                    draft.query.value().trim()
                ));
                return CatalogMutationPlan::new(
                    request,
                    CatalogObjectType::Catalog(CatalogKind::View),
                    CatalogMutationExecutionMode::Autocommit,
                    CatalogMutationTarget::database_target(
                        crate::model::execution_target::ExecutionTarget {
                            profile_id: object_id.profile_id(),
                            database: database_name.clone(),
                            schema: Some(schema_name.clone()),
                        },
                    )?,
                    vec![CatalogTarget::Objects {
                        schema: CatalogId::new(
                            object_id.profile_id(),
                            CatalogKind::Schema,
                            [database_name, schema_name.clone()],
                        ),
                        group: ObjectGroup::Views,
                    }],
                    CatalogSelectionHint::Object(new_object),
                    None,
                    Vec::new(),
                    statements,
                );
            }
            let Some(CatalogObjectDefinition::Table(table)) = baseline else {
                return Err(CatalogMutationError::StaleState);
            };
            let crate::model::catalog_editor::CatalogDraft::Table(draft) = draft else {
                return Err(CatalogMutationError::InvalidDraft {
                    reason: "MySQL table edit requires a table draft".into(),
                });
            };
            let [database, schema, old_name] = object.native_path.as_slice() else {
                return Err(CatalogMutationError::InvalidAnchor {
                    reason: "MySQL table identity is incomplete",
                });
            };
            let object_id = object.clone();
            let database_name = database.clone();
            let schema_name = schema.clone();
            let old_name_sql = old_name.clone();
            let schema_sql = schema.clone();
            let new_name = draft.name.value().trim();
            if new_name.is_empty() {
                return Err(CatalogMutationError::InvalidDraft {
                    reason: "MySQL table name is required".into(),
                });
            }
            draft.validate()?;
            let mut statements = Vec::new();
            if new_name != old_name {
                statements.push(format!(
                    "RENAME TABLE {}.{} TO {}.{}",
                    quote_identifier(&schema_sql),
                    quote_identifier(&old_name_sql),
                    quote_identifier(&schema_sql),
                    quote_identifier(new_name)
                ));
            }
            if comment_changed(&table.comment, draft.comment.value()) {
                statements.push(format!(
                    "ALTER TABLE {}.{} COMMENT = {}",
                    quote_identifier(&schema_sql),
                    quote_identifier(new_name),
                    quote_literal(draft.comment.value())
                ));
            }
            let mut current = table
                .columns
                .iter()
                .map(|column| column.name.clone())
                .collect::<Vec<_>>();
            for row in &draft.columns {
                if let crate::model::catalog_editor::DraftRowState::Removed { .. } = row.state {
                    if let Some(name) = row.existing_name.as_deref() {
                        statements.push(format!(
                            "ALTER TABLE {}.{} DROP COLUMN {}",
                            quote_identifier(&schema_sql),
                            quote_identifier(new_name),
                            quote_identifier(name)
                        ));
                        current.retain(|column| column != name);
                    }
                    continue;
                }
                let target_index = draft
                    .columns
                    .iter()
                    .position(|candidate| std::ptr::eq(candidate, row))
                    .unwrap_or(0);
                let previous = draft
                    .columns
                    .iter()
                    .take(target_index)
                    .rev()
                    .find(|candidate| {
                        !matches!(
                            candidate.state,
                            crate::model::catalog_editor::DraftRowState::Removed { .. }
                        )
                    })
                    .map(|candidate| candidate.name.value().trim().to_owned());
                let desired_index = previous
                    .as_ref()
                    .and_then(|name| current.iter().position(|current_name| current_name == name))
                    .map_or(0, |index| index + 1);
                let position_sql = previous.as_ref().map_or_else(
                    || " FIRST".to_owned(),
                    |name| format!(" AFTER {}", quote_identifier(name)),
                );
                if let Some(existing_name) = row.existing_name.as_deref() {
                    let old = table
                        .columns
                        .iter()
                        .find(|column| column.name == existing_name);
                    let needs_change = old.is_none_or(|old| {
                        old.name != row.name.value().trim()
                            || old.native_type != row.native_type.value().trim()
                            || old.nullable != row.nullable
                            || comment_changed(&old.comment, row.comment.value())
                            || comment_changed(
                                &old.default_expression,
                                row.default_expression.value().trim(),
                            )
                            || current.iter().position(|name| name == existing_name)
                                != Some(desired_index)
                    });
                    let definition = mysql_column_definition(row)?;
                    if let Some(index) = current.iter().position(|name| name == existing_name) {
                        current.remove(index);
                    }
                    if needs_change {
                        statements.push(format!(
                            "ALTER TABLE {}.{} CHANGE COLUMN {} {}{}",
                            quote_identifier(&schema_sql),
                            quote_identifier(new_name),
                            quote_identifier(existing_name),
                            definition,
                            position_sql
                        ));
                    }
                    current.insert(
                        desired_index.min(current.len()),
                        row.name.value().trim().to_owned(),
                    );
                } else {
                    let definition = mysql_column_definition(row)?;
                    statements.push(format!(
                        "ALTER TABLE {}.{} ADD COLUMN {}{}",
                        quote_identifier(&schema_sql),
                        quote_identifier(new_name),
                        definition,
                        position_sql
                    ));
                    current.insert(
                        desired_index.min(current.len()),
                        row.name.value().trim().to_owned(),
                    );
                }
            }
            if statements.is_empty() {
                return Err(CatalogMutationError::NoChanges);
            }
            let new_object = CatalogId::new(
                request.connection.profile_id,
                CatalogKind::Table,
                [database.clone(), schema.clone(), new_name.to_owned()],
            );
            return CatalogMutationPlan::new(
                request,
                CatalogObjectType::Catalog(CatalogKind::Table),
                CatalogMutationExecutionMode::Autocommit,
                CatalogMutationTarget::database_target(
                    crate::model::execution_target::ExecutionTarget {
                        profile_id: object_id.profile_id(),
                        database: database_name.clone(),
                        schema: Some(schema_name.clone()),
                    },
                )?,
                vec![CatalogTarget::Objects {
                    schema: CatalogId::new(
                        object_id.profile_id(),
                        CatalogKind::Schema,
                        [database_name.clone(), schema_name.clone()],
                    ),
                    group: ObjectGroup::Tables,
                }],
                CatalogSelectionHint::Object(new_object),
                Some(table.baseline_fingerprint),
                Vec::new(),
                statements,
            );
        }
        if request.mode != crate::db::catalog_mutation::CatalogMutationMode::Create {
            return Err(CatalogMutationError::UnsupportedOperation {
                object_type: request.object_type,
            });
        }
        if baseline.is_some() {
            return Err(CatalogMutationError::InvalidDraft {
                reason: "MySQL create plans cannot include a baseline".into(),
            });
        }
        if let CatalogMutationAnchor::Profile { .. } = &request.anchor {
            if request.object_type != CatalogObjectType::Catalog(CatalogKind::Database) {
                return Err(CatalogMutationError::UnsupportedOperation {
                    object_type: request.object_type,
                });
            }
            let crate::model::catalog_editor::CatalogDraft::Database(draft) = draft else {
                return Err(CatalogMutationError::InvalidDraft {
                    reason: "MySQL database creation requires a database draft".into(),
                });
            };
            draft.validate()?;
            let name = draft.name.value().trim().to_owned();
            let database = request.current_database.clone().ok_or({
                CatalogMutationError::InvalidAnchor {
                    reason: "MySQL database creation requires an existing execution database",
                }
            })?;
            let object = CatalogId::new(
                request.connection.profile_id,
                CatalogKind::Database,
                [name.clone()],
            );
            return CatalogMutationPlan::new(
                request,
                CatalogObjectType::Catalog(CatalogKind::Database),
                CatalogMutationExecutionMode::Autocommit,
                CatalogMutationTarget::maintenance(database)?,
                vec![CatalogTarget::Databases],
                CatalogSelectionHint::Object(object),
                None,
                Vec::new(),
                vec![format!("CREATE DATABASE {}", quote_identifier(&name))],
            );
        }
        let (schema_anchor, group_kind, database, schema_name) = match &request.anchor {
            CatalogMutationAnchor::Group { schema, group } => {
                if schema.native_path.len() != 2
                    || schema.native_path.first() != schema.native_path.get(1)
                {
                    return Err(CatalogMutationError::InvalidAnchor {
                        reason: "MySQL group anchor has an invalid namespace path",
                    });
                }
                (
                    schema.clone(),
                    *group,
                    schema.native_path.first().cloned().unwrap_or_default(),
                    schema.native_path.get(1).cloned().unwrap_or_default(),
                )
            }
            CatalogMutationAnchor::Principal(_) => {
                return Err(CatalogMutationError::InvalidAnchor {
                    reason: "principal mutations are not supported by MySQL",
                });
            }
            CatalogMutationAnchor::Catalog(id)
                if matches!(id.kind, CatalogKind::Database | CatalogKind::Schema) =>
            {
                if (id.kind == CatalogKind::Database && id.native_path.len() != 1)
                    || (id.kind == CatalogKind::Schema
                        && (id.native_path.len() != 2
                            || id.native_path.first() != id.native_path.get(1)))
                {
                    return Err(CatalogMutationError::InvalidAnchor {
                        reason: "MySQL database/schema anchor has an invalid namespace path",
                    });
                }
                let database = id.native_path.first().cloned().unwrap_or_default();
                let schema_name = if id.kind == CatalogKind::Database {
                    database.clone()
                } else {
                    id.native_path.get(1).cloned().unwrap_or_default()
                };
                let group = match request.object_type {
                    CatalogObjectType::Catalog(CatalogKind::Table) => ObjectGroup::Tables,
                    CatalogObjectType::Catalog(CatalogKind::View) => ObjectGroup::Views,
                    _ => {
                        return Err(CatalogMutationError::InvalidAnchor {
                            reason: "MySQL database/schema anchors support tables and views",
                        });
                    }
                };
                (
                    CatalogId::new(
                        id.profile_id(),
                        CatalogKind::Schema,
                        [database.clone(), schema_name.clone()],
                    ),
                    group,
                    database,
                    schema_name,
                )
            }
            CatalogMutationAnchor::Catalog(_) => {
                return Err(CatalogMutationError::InvalidAnchor {
                    reason: "MySQL table and view creation requires a database, schema, or group anchor",
                });
            }
            CatalogMutationAnchor::Profile { .. } => {
                return Err(CatalogMutationError::InvalidAnchor {
                    reason: "MySQL table and view creation requires a database, schema, or group anchor",
                });
            }
        };
        if database.is_empty() || schema_name.is_empty() {
            return Err(CatalogMutationError::InvalidAnchor {
                reason: "MySQL database/schema anchor is incomplete",
            });
        }
        let (kind, name, sql) = match (group_kind, draft) {
            (ObjectGroup::Tables, crate::model::catalog_editor::CatalogDraft::Table(draft)) => {
                draft.validate()?;
                let name = draft.name.value().trim().to_owned();
                let columns = draft
                    .columns
                    .iter()
                    .filter(|column| {
                        !matches!(
                            column.state,
                            crate::model::catalog_editor::DraftRowState::Removed { .. }
                        )
                    })
                    .map(|column| {
                        let mut sql = format!(
                            "{} {}",
                            quote_identifier(column.name.value().trim()),
                            column.native_type.value().trim()
                        );
                        if !column.nullable {
                            sql.push_str(" NOT NULL");
                        }
                        if !column.default_expression.value().trim().is_empty() {
                            sql.push_str(" DEFAULT ");
                            sql.push_str(column.default_expression.value().trim());
                        }
                        Ok(sql)
                    })
                    .collect::<Result<Vec<_>, CatalogMutationError>>()?;
                (
                    CatalogKind::Table,
                    name.clone(),
                    format!(
                        "CREATE TABLE {}.{} ({})",
                        quote_identifier(&schema_name),
                        quote_identifier(&name),
                        columns.join(", ")
                    ),
                )
            }
            (ObjectGroup::Views, crate::model::catalog_editor::CatalogDraft::View(draft)) => {
                draft.validate()?;
                let name = draft.name.value().trim().to_owned();
                (
                    CatalogKind::View,
                    name.clone(),
                    format!(
                        "CREATE VIEW {}.{} AS {}",
                        quote_identifier(&schema_name),
                        quote_identifier(&name),
                        draft.query.value().trim()
                    ),
                )
            }
            (_, draft) => {
                return Err(CatalogMutationError::InvalidDraft {
                    reason: format!("MySQL draft does not match the selected group: {draft:?}"),
                });
            }
        };
        let object = CatalogId::new(
            request.connection.profile_id,
            kind,
            [database.clone(), schema_name.clone(), name],
        );
        CatalogMutationPlan::new(
            request,
            CatalogObjectType::Catalog(kind),
            CatalogMutationExecutionMode::Autocommit,
            CatalogMutationTarget::database_target(
                crate::model::execution_target::ExecutionTarget {
                    profile_id: object.profile_id(),
                    database,
                    schema: Some(schema_name),
                },
            )?,
            vec![CatalogTarget::Objects {
                schema: schema_anchor,
                group: group_kind,
            }],
            CatalogSelectionHint::Object(object),
            None,
            Vec::new(),
            vec![sql],
        )
    }

    pub async fn execute_catalog_mutation(
        &self,
        plan: &CatalogMutationPlan,
    ) -> Result<QueryOutcome, DatabaseError> {
        self.reject_tidb_operation("structured catalog mutation")?;
        plan.validate()
            .map_err(|error| DatabaseError::configuration(error.to_string()))?;
        let mut outcome = None;
        for statement in plan.statements() {
            outcome = Some(self.execute_pool(statement).await?);
        }
        outcome.ok_or_else(|| DatabaseError::configuration("MySQL mutation plan has no statements"))
    }

    pub async fn load_catalog_object_definition(
        &self,
        request: &CatalogObjectDefinitionRequest,
    ) -> Result<CatalogObjectDefinition, DatabaseError> {
        self.reject_tidb_operation("structured catalog editing")?;
        request
            .validate()
            .map_err(|error| DatabaseError::configuration(error.to_string()))?;
        let [database, schema, name] = request.object.native_path.as_slice() else {
            return Err(DatabaseError::configuration(
                "MySQL table identity is incomplete",
            ));
        };
        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        if request.object.kind == CatalogKind::View {
            let sql = format!(
                "SHOW CREATE VIEW {}.{}",
                quote_identifier(schema),
                quote_identifier(name)
            );
            let row = sqlx::query(AssertSqlSafe(sql))
                .fetch_optional(&mut *connection)
                .await
                .map_err(sql_error)?;
            let Some(row) = row else {
                return Err(DatabaseError::configuration(
                    "MySQL view definition is unavailable",
                ));
            };
            let show_create: String = row.try_get(1).map_err(decode_error)?;
            let uppercase = show_create.to_ascii_uppercase();
            let Some(as_index) = uppercase.find(" AS ") else {
                return Err(DatabaseError::configuration(
                    "MySQL view definition has no query body",
                ));
            };
            let query = show_create[as_index + 4..].trim().to_owned();
            let output_rows = sqlx::query(
                "SELECT column_name FROM information_schema.columns WHERE BINARY table_schema=BINARY ? AND BINARY table_name=BINARY ? ORDER BY ordinal_position",
            )
            .bind(schema)
            .bind(name)
            .fetch_all(&mut *connection)
            .await
            .map_err(sql_error)?;
            let output_columns = output_rows
                .into_iter()
                .map(|row| row.try_get(0).map_err(decode_error))
                .collect::<Result<Vec<String>, DatabaseError>>()?;
            return Ok(CatalogObjectDefinition::View(ViewDefinition {
                database: database.clone(),
                schema: schema.clone(),
                name: name.clone(),
                owner: String::new(),
                comment: OptionalMetadata::Unsupported,
                query: query.clone(),
                output_columns,
                security_barrier: ViewOption::unavailable("not applicable to MySQL"),
                security_invoker: ViewOption::unavailable("not applicable to MySQL"),
                check_option: ViewOption::unavailable("not mapped for MySQL"),
                baseline_fingerprint: format!("mysql:view:{database}:{schema}:{name}:{query}"),
                clickhouse_native: None,
            }));
        }
        if request.object.kind != CatalogKind::Table {
            return Err(DatabaseError::configuration(
                "MySQL definition loading currently supports tables and views only",
            ));
        }
        let rows = sqlx::query(
            "SELECT ordinal_position, column_name, column_type, is_nullable, column_default, generation_expression, collation_name, column_comment, extra FROM information_schema.columns WHERE BINARY table_schema=BINARY ? AND BINARY table_name=BINARY ? ORDER BY ordinal_position",
        )
        .bind(schema)
        .bind(name)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        let table_comment = sqlx::query(
            "SELECT table_comment FROM information_schema.tables WHERE BINARY table_schema=BINARY ? AND BINARY table_name=BINARY ?",
        )
        .bind(schema)
        .bind(name)
        .fetch_optional(&mut *connection)
        .await
        .map_err(sql_error)?
        .map(|row| row.try_get::<String, _>(0).map_err(decode_error))
        .transpose()?;
        let mut columns = Vec::new();
        for row in rows {
            let default = row.try_get::<Option<String>, _>(4).map_err(decode_error)?;
            let extra: String = row.try_get(8).map_err(decode_error)?;
            columns.push(ColumnDefinition {
                name: row.try_get(1).map_err(decode_error)?,
                ordinal_position: row.try_get(0).map_err(decode_error)?,
                native_type: row.try_get(2).map_err(decode_error)?,
                nullable: row.try_get::<String, _>(3).map_err(decode_error)? == "YES",
                default_expression: OptionalMetadata::Supported(default),
                identity: OptionalMetadata::Supported(Some(
                    extra.to_ascii_uppercase().contains("AUTO_INCREMENT"),
                )),
                generated_expression: OptionalMetadata::Supported(
                    row.try_get(5).map_err(decode_error)?,
                ),
                collation: OptionalMetadata::Supported(row.try_get(6).map_err(decode_error)?),
                comment: OptionalMetadata::Supported(row.try_get(7).map_err(decode_error)?),
            });
        }
        if columns.is_empty() {
            return Err(DatabaseError::configuration(
                "MySQL table has no visible columns",
            ));
        }
        let baseline_fingerprint =
            format!("mysql:table:{database}:{schema}:{name}:{table_comment:?}:{columns:?}");
        Ok(CatalogObjectDefinition::Table(TableDefinition {
            database: database.clone(),
            schema: schema.clone(),
            name: name.clone(),
            owner: String::new(),
            comment: OptionalMetadata::Supported(table_comment),
            columns: columns.clone(),
            indexes: Vec::new(),
            constraints: Vec::new(),
            structured_constraints: Vec::new(),
            baseline_fingerprint,
        }))
    }

    pub(crate) async fn transaction_backend(
        &self,
    ) -> Result<MySqlTransactionBackend, DatabaseError> {
        self.reject_tidb_operation("manual transactions and structured relation mutations")?;
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
        if !matches!(
            profile.kind,
            DatabaseKind::MySql | DatabaseKind::MariaDb | DatabaseKind::TiDb
        ) {
            return Err(DatabaseError::configuration(
                "profile is not MySQL, MariaDB or TiDB",
            ));
        }
        let host = profile
            .host
            .as_deref()
            .ok_or_else(|| DatabaseError::configuration("MySQL profile has no host"))?;
        let mut options = MySqlConnectOptions::new()
            .host(host)
            .port(
                profile
                    .port
                    .unwrap_or(if profile.kind == DatabaseKind::TiDb {
                        4000
                    } else {
                        3306
                    }),
            )
            .ssl_mode(mysql_ssl_mode(profile.ssl_mode));
        if profile.kind == DatabaseKind::TiDb {
            // Each DQL statement is prepared for empty-result metadata, then
            // executed as text. DDL and USE can change identical SQL's columns.
            options = options.statement_cache_capacity(0);
        }
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
        if profile.read_only && profile.kind != DatabaseKind::TiDb {
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
        let adapter = Self {
            pool,
            kind: profile.kind,
            connection_id: profile.id,
            catalog_scope: profile.catalog_scope.clone(),
            read_only: profile.read_only,
        };
        // Driver options use the MySQL wire protocol while the native product
        // identity remains TiDB. Reject mislabelled TiDB profiles immediately.
        if let Err(error) = adapter.probe().await {
            adapter.pool.close().await;
            return Err(error);
        }
        Ok(adapter)
    }

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        let row = sqlx::query(PROBE_SQL)
            .fetch_one(&self.pool)
            .await
            .map_err(|error| DatabaseError::from_sqlx(error, ErrorCategory::Network))?;
        let version: String = row.try_get("version").map_err(decode_error)?;
        let is_tidb = version.to_ascii_lowercase().contains("tidb");
        if (self.kind == DatabaseKind::TiDb) != is_tidb {
            return Err(DatabaseError::configuration(format!(
                "server product does not match {:?} profile (server reported {})",
                self.kind,
                sanitize_terminal_text(&version)
            )));
        }
        if self.kind == DatabaseKind::TiDb
            && !supports_catalog_version_for_kind(self.kind, &version)
        {
            return Err(unsupported_catalog_version(self.kind, &version));
        }
        Ok(ServerInfo {
            kind: self.kind,
            version,
            database: row
                .try_get::<Option<String>, _>("current_database")
                .map_err(decode_error)?
                .unwrap_or_default(),
            current_user: None,
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
        self.execute_pool_with_budget(sql, QueryBudget::UNBOUNDED)
            .await
    }

    pub(crate) async fn execute_pool_with_budget(
        &self,
        sql: &str,
        budget: QueryBudget,
    ) -> Result<QueryOutcome, DatabaseError> {
        if self.kind == DatabaseKind::TiDb {
            return self.execute_tidb_with_budget(sql, budget).await;
        }
        let mut stream = sqlx::raw_sql(AssertSqlSafe(sql)).fetch_many(&self.pool);
        self.collect_stream(&mut stream, budget).await
    }

    pub(crate) async fn execute_connection(
        &self,
        connection: &mut MySqlConnection,
        sql: &str,
    ) -> Result<QueryOutcome, DatabaseError> {
        let mut stream = sqlx::raw_sql(AssertSqlSafe(sql)).fetch_many(&mut *connection);
        self.collect_stream(&mut stream, QueryBudget::UNBOUNDED)
            .await
    }

    async fn collect_stream<E>(
        &self,
        stream: &mut E,
        budget: QueryBudget,
    ) -> Result<QueryOutcome, DatabaseError>
    where
        E: futures_util::TryStream<
                Ok = Either<sqlx::mysql::MySqlQueryResult, MySqlRow>,
                Error = sqlx::Error,
            > + Unpin,
    {
        let mut accumulator = QueryOutcomeAccumulator::with_budget(budget);
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
        if self.kind == DatabaseKind::TiDb
            && matches!(&request.key.target, CatalogTarget::Objects { group, .. }
                if !matches!(group, ObjectGroup::Tables | ObjectGroup::Views))
        {
            return Err(DatabaseError::unsupported_catalog_target(
                self.kind,
                &request.key.target,
            ));
        }
        if matches!(
            request.key.target,
            CatalogTarget::Objects {
                group: ObjectGroup::MaterializedViews | ObjectGroup::Types,
                ..
            }
        ) || (self.kind != DatabaseKind::MariaDb
            && matches!(
                request.key.target,
                CatalogTarget::Objects {
                    group: ObjectGroup::Sequences,
                    ..
                }
            ))
        {
            return Err(DatabaseError::unsupported_catalog_target(
                self.kind,
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
        if !supports_catalog_version_for_kind(self.kind, &version) {
            return Err(unsupported_catalog_version(self.kind, &version));
        }
        let mut transaction = connection
            .begin_with(self.catalog_begin_sql())
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
            CatalogTarget::ProjectionChildren { .. } => Err(DatabaseError::unsupported(
                "projection catalog target is not supported by MySQL",
            )),
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
        if !supports_catalog_version_for_kind(self.kind, &version) {
            return Err(unsupported_catalog_version(self.kind, &version));
        }
        let mut transaction = connection
            .begin_with(self.catalog_begin_sql())
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
        let candidates = if self.kind == DatabaseKind::TiDb {
            tidb::search_candidates_sql()
        } else if self.kind == DatabaseKind::MariaDb {
            CATALOG_SEARCH_CANDIDATES_SQL.replace(
                "), normalized AS (",
                &format!("{}), normalized AS (", MARIADB_CATALOG_SEARCH_SEQUENCE_SQL),
            )
        } else {
            CATALOG_SEARCH_CANDIDATES_SQL.to_owned()
        };
        let sql = candidates.replace("{scope_predicate}", &scope_predicate);
        let mut query = sqlx::query(AssertSqlSafe(sql));
        let (search_query, ignore_separators) = crate::db::catalog::search_query(&request.query);
        query = query.bind(ignore_separators).bind(ignore_separators);
        if let Some(databases) = selected.as_ref() {
            for database in databases {
                query = query.bind(database);
            }
        }
        query = query
            .bind(request.object_scope == crate::db::catalog::CatalogSearchObjectScope::AllObjects);
        for _ in 0..5 {
            query = query.bind(&search_query);
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
        let count_sql = if self.kind == DatabaseKind::TiDb {
            tidb::GROUP_COUNTS_SQL
        } else if self.kind == DatabaseKind::MariaDb {
            "SELECT \
             (SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.tables WHERE BINARY table_schema=BINARY ? AND table_type='BASE TABLE') AS tables, \
             (SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.tables WHERE BINARY table_schema=BINARY ? AND table_type='VIEW') AS views, \
             (SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.routines WHERE BINARY routine_schema=BINARY ? AND routine_type='FUNCTION') AS functions, \
             (SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.routines WHERE BINARY routine_schema=BINARY ? AND routine_type='PROCEDURE') AS procedures, \
             (SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.triggers WHERE BINARY trigger_schema=BINARY ?) AS triggers, \
             (SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.tables WHERE BINARY table_schema=BINARY ? AND table_type='SEQUENCE') AS sequences"
        } else {
            "SELECT \
             (SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.tables WHERE BINARY table_schema=BINARY ? AND table_type='BASE TABLE') AS tables, \
             (SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.tables WHERE BINARY table_schema=BINARY ? AND table_type='VIEW') AS views, \
             (SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.routines WHERE BINARY routine_schema=BINARY ? AND routine_type='FUNCTION') AS functions, \
             (SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.routines WHERE BINARY routine_schema=BINARY ? AND routine_type='PROCEDURE') AS procedures, \
             (SELECT CAST(COUNT(*) AS CHAR) FROM information_schema.triggers WHERE BINARY trigger_schema=BINARY ?) AS triggers"
        };
        let mut query = sqlx::query(AssertSqlSafe(count_sql))
            .bind(&database)
            .bind(&database);
        if self.kind != DatabaseKind::TiDb {
            query = query.bind(&database).bind(&database).bind(&database);
        }
        if self.kind == DatabaseKind::MariaDb {
            query = query.bind(&database);
        }
        let row = query.fetch_one(&mut *connection).await.map_err(sql_error)?;
        let mut summaries = Vec::new();
        for (group, column) in [
            (ObjectGroup::Tables, "tables"),
            (ObjectGroup::Views, "views"),
            (ObjectGroup::Functions, "functions"),
            (ObjectGroup::Procedures, "procedures"),
            (ObjectGroup::Triggers, "triggers"),
        ] {
            if self.kind == DatabaseKind::TiDb
                && !matches!(group, ObjectGroup::Tables | ObjectGroup::Views)
            {
                continue;
            }
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
        if self.kind == DatabaseKind::MariaDb {
            summaries.push(CatalogGroupSummary {
                group: ObjectGroup::Sequences,
                object_count: CatalogCount::Exact(
                    row.try_get::<String, _>("sequences")
                        .map_err(decode_error)?
                        .parse::<u64>()
                        .map_err(|_| {
                            catalog_internal("MariaDB returned an invalid sequence count")
                        })?,
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
                ObjectGroup::Sequences if self.kind == DatabaseKind::MariaDb => (
                    "information_schema.tables",
                    "table_schema",
                    "table_name",
                    "table_type='SEQUENCE'",
                    CatalogKind::Sequence,
                    "sequence",
                    "table_name",
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
            ObjectGroup::Sequences => "NULL AS comment, NULL AS owner_name",
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
        let checks = self
            .load_check_metadata(connection, database, relation_name)
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
        for check in checks {
            entries.push(
                CatalogEntry::relation_child(
                    relation_child_id(relation, CatalogKind::CheckConstraint, &check.name),
                    relation.clone(),
                    qualified_object(database, &check.name),
                    "check_constraint",
                    OptionalMetadata::Unsupported,
                    CatalogMetadata::Constraint(ConstraintMetadata::Check {
                        expression: check.expression,
                    }),
                )
                .map_err(catalog_invariant)?,
            );
        }

        let rows = sqlx::query(
            "SELECT CAST(ordinal_position AS UNSIGNED), column_name, column_type, data_type, is_nullable, \
             column_default, extra, \
             generation_expression, \
             CAST(numeric_precision AS UNSIGNED), CAST(numeric_scale AS UNSIGNED), \
              CAST(character_maximum_length AS SIGNED), collation_name, character_set_name, column_comment \
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
            let extra = row
                .try_get::<Option<String>, _>(6)
                .map_err(decode_error)?
                .unwrap_or_default();
            let generation_expression = row
                .try_get::<Option<String>, _>(7)
                .map_err(decode_error)?
                .unwrap_or_default();
            let generated = !generation_expression.is_empty()
                || extra.to_ascii_uppercase().contains("VIRTUAL GENERATED")
                || extra.to_ascii_uppercase().contains("STORED GENERATED");
            let default_expression =
                normalize_default_expression(row.try_get(5).map_err(decode_error)?);
            let generation_expression = if generation_expression.is_empty() && generated {
                default_expression.clone().unwrap_or_default()
            } else {
                generation_expression
            };
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
                default_expression.clone()
            });
            metadata.identity = OptionalMetadata::Unsupported;
            metadata.auto_increment = OptionalMetadata::Supported(Some(
                extra
                    .split_whitespace()
                    .any(|part| part.eq_ignore_ascii_case("auto_increment")),
            ));
            metadata.generated_expression = OptionalMetadata::Supported(if generated {
                empty_as_none(Some(generation_expression)).or(default_expression)
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
        if relation.profile_id() != self.connection_id
            || !relation.kind.is_relation()
            || (self.kind == DatabaseKind::TiDb && !tidb::supported_relation(relation.kind))
        {
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
        mut page: crate::model::pagination::PageRequest,
    ) -> Result<crate::db::RelationPreview, DatabaseError> {
        validate_catalog_scope(&self.catalog_scope)?;
        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        self.check_tidb_catalog_version(&mut connection).await?;
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
        let mut base_sql = format!(
            "SELECT * FROM {}.{}",
            quote_identifier(&database),
            quote_identifier(&name)
        );
        append_preview_options(&mut base_sql, options);
        if self.kind == DatabaseKind::TiDb && self.read_only {
            tidb::validate_read_only_sql(&base_sql)?;
        }
        let total = if page.resolve_total {
            let count_sql = relation_count_sql(&base_sql);
            let count = sqlx::query_scalar::<_, i64>(AssertSqlSafe(count_sql))
                .fetch_one(&mut *connection)
                .await
                .map_err(sql_error)?;
            let total = u64::try_from(count)
                .map_err(|_| catalog_internal("MySQL returned an invalid relation row count"))?;
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
            row_versions: None,
        })
    }

    pub async fn relation_ddl(&self, relation: &CatalogId) -> Result<RelationDdl, DatabaseError> {
        validate_catalog_scope(&self.catalog_scope)?;
        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        self.check_tidb_catalog_version(&mut connection).await?;
        let mut transaction = connection
            .begin_with(self.catalog_begin_sql())
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
        if self.kind == DatabaseKind::TiDb
            && !self.catalog_scope.allows_schema(&database, &database)
        {
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
        let trigger_names = if self.kind == DatabaseKind::TiDb {
            Vec::new()
        } else {
            sqlx::query_scalar::<_, String>(
                "SELECT trigger_name FROM information_schema.triggers \
             WHERE BINARY event_object_schema=BINARY ? AND BINARY event_object_table=BINARY ? \
             ORDER BY BINARY trigger_name",
            )
            .bind(&database)
            .bind(&name)
            .fetch_all(&mut *connection)
            .await
            .map_err(sql_error)?
        };
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
        // The MariaDB integration test intentionally uses a MySQL URL so the
        // adapter remains MySQL-compatible. Detect the server product rather
        // than relying on the profile kind when selecting metadata columns.
        let server_version: String = sqlx::query_scalar("SELECT VERSION()")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
        let index_sql = if self.kind == DatabaseKind::TiDb {
            tidb::INDEXES_SQL
        } else if self.kind == DatabaseKind::MariaDb
            || server_version.to_ascii_lowercase().contains("mariadb")
        {
            CATALOG_PAGE_INDEXES_MARIADB_SQL
        } else {
            CATALOG_PAGE_INDEXES_SQL
        };
        let rows = sqlx::query(index_sql)
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
              tc.constraint_name, tc.constraint_type, CAST(kcu.ordinal_position AS UNSIGNED), \
             kcu.column_name, \
             CASE WHEN tc.constraint_type='FOREIGN KEY' THEN kcu.referenced_table_schema END, \
             CASE WHEN tc.constraint_type='FOREIGN KEY' THEN kcu.referenced_table_name END, \
             CASE WHEN tc.constraint_type='FOREIGN KEY' THEN kcu.referenced_column_name END, \
             CAST(CASE WHEN tc.constraint_type='FOREIGN KEY' \
                       THEN kcu.position_in_unique_constraint END AS UNSIGNED) \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
               ON BINARY kcu.constraint_catalog=BINARY tc.constraint_catalog \
              AND BINARY kcu.constraint_schema=BINARY tc.constraint_schema \
              AND BINARY kcu.table_schema=BINARY tc.table_schema \
              AND BINARY kcu.table_name=BINARY tc.table_name \
              AND BINARY kcu.constraint_name=BINARY tc.constraint_name \
             WHERE BINARY tc.constraint_schema=BINARY ? AND BINARY tc.table_name=BINARY ? \
               AND tc.constraint_type IN ('PRIMARY KEY','UNIQUE','FOREIGN KEY') \
               AND kcu.column_name IS NOT NULL \
             ORDER BY BINARY tc.constraint_catalog, BINARY tc.constraint_schema, \
                      BINARY tc.table_schema, BINARY tc.table_name, \
                      BINARY tc.constraint_name, kcu.ordinal_position",
        )
        .bind(database)
        .bind(relation)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        let mut parts = Vec::with_capacity(rows.len());
        for row in rows {
            let native_kind: String = row.try_get(5).map_err(decode_error)?;
            let kind = match native_kind.as_str() {
                "PRIMARY KEY" => CatalogKind::PrimaryKey,
                "UNIQUE" => CatalogKind::UniqueConstraint,
                "FOREIGN KEY" => CatalogKind::ForeignKey,
                _ => return Err(catalog_internal("unexpected MySQL constraint type")),
            };
            let column: Option<String> = row.try_get(7).map_err(decode_error)?;
            let Some(column) = column else {
                if kind == CatalogKind::ForeignKey {
                    return Err(catalog_internal(format!(
                        "MySQL foreign key `{}` has no source column",
                        row.try_get::<String, _>(4).map_err(decode_error)?
                    )));
                }
                // MariaDB may expose expression-only key parts in
                // KEY_COLUMN_USAGE without a source column. They are not
                // column constraints, so leave them to index metadata.
                continue;
            };
            parts.push(MySqlConstraintPart {
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
                column,
                referenced_database: (kind == CatalogKind::ForeignKey)
                    .then(|| row.try_get(8).map_err(decode_error))
                    .transpose()?,
                referenced_relation: (kind == CatalogKind::ForeignKey)
                    .then(|| row.try_get(9).map_err(decode_error))
                    .transpose()?,
                referenced_column: if kind == CatalogKind::ForeignKey {
                    row.try_get(10).map_err(decode_error)?
                } else {
                    None
                },
                referenced_ordinal: if kind == CatalogKind::ForeignKey {
                    row.try_get::<Option<u64>, _>(11)
                        .map_err(decode_error)?
                        .map(|value| checked_u32(value, "referenced constraint ordinal"))
                        .transpose()?
                } else {
                    None
                },
            });
        }
        group_constraint_parts(parts)
    }

    async fn load_check_metadata(
        &self,
        connection: &mut MySqlConnection,
        database: &str,
        relation: &str,
    ) -> Result<Vec<MySqlCheckInfo>, DatabaseError> {
        let rows = sqlx::query(
            "SELECT tc.constraint_name, cc.check_clause \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.check_constraints cc \
               ON BINARY cc.constraint_schema=BINARY tc.constraint_schema \
              AND BINARY cc.constraint_name=BINARY tc.constraint_name \
             WHERE BINARY tc.table_schema=BINARY ? \
               AND BINARY tc.table_name=BINARY ? \
               AND tc.constraint_type='CHECK' \
             ORDER BY BINARY tc.constraint_name",
        )
        .bind(database)
        .bind(relation)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
        rows.into_iter()
            .map(|row| {
                Ok(MySqlCheckInfo {
                    name: row.try_get(0).map_err(decode_error)?,
                    expression: row.try_get(1).map_err(decode_error)?,
                })
            })
            .collect()
    }

    /// List server-level accounts for MySQL/MariaDB.
    ///
    /// MySQL and MariaDB expose account metadata differently, so the two are
    /// handled separately:
    ///  * MariaDB has an explicit `is_role` flag on `mysql.user`.
    ///  * MySQL 8 records role membership in `mysql.role_edges`; an account is
    ///    only reported as a role when it appears there, because MySQL has no
    ///    independent role flag. Roles that have never been granted therefore
    ///    cannot be distinguished from users and are reported as users with a
    ///    native-kind note.
    pub async fn list_principals(&self) -> Result<PrincipalPage, DatabaseError> {
        self.reject_tidb_operation("principal metadata")?;
        let rows = if self.kind == DatabaseKind::MariaDb {
            match sqlx::query(
                "SELECT User AS user_name, Host AS host_name, \
                 CASE WHEN is_role = 'Y' THEN 1 ELSE 0 END AS is_role \
                 FROM mysql.user \
                 ORDER BY CASE WHEN is_role = 'Y' THEN 1 ELSE 0 END, User, Host",
            )
            .fetch_all(&self.pool)
            .await
            {
                Ok(rows) => rows,
                Err(error) if is_mysql_privilege_error(&error) => {
                    return self.list_visible_mariadb_principals().await;
                }
                Err(error) => return Err(sql_error(error)),
            }
        } else {
            sqlx::query(
                "SELECT User AS user_name, Host AS host_name, 0 AS is_role FROM mysql.user \
                 ORDER BY User, Host",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(sql_error)?
        };
        let mysql_roles = if self.kind == DatabaseKind::MariaDb {
            Vec::new()
        } else {
            sqlx::query_as::<_, (String, String)>(
                "SELECT DISTINCT FROM_USER, FROM_HOST FROM mysql.role_edges",
            )
            .fetch_all(&self.pool)
            .await
            // MySQL < 8 has no role support at all; treat that as "no roles"
            // rather than failing the whole browse.
            .unwrap_or_default()
        };
        let mut entries = rows
            .into_iter()
            .map(|row| {
                let user: String = row.try_get("user_name").map_err(decode_error)?;
                let host: String = row.try_get("host_name").map_err(decode_error)?;
                let flag: i64 = row.try_get("is_role").map_err(decode_error)?;
                let is_role = flag != 0
                    || mysql_roles
                        .iter()
                        .any(|(role_user, role_host)| role_user == &user && role_host == &host);
                // Credentials and other secrets are never part of the list.
                Ok(PrincipalEntry {
                    id: PrincipalId {
                        profile_id: self.connection_id,
                        scope: PrincipalScope::Server,
                        native_id: user.clone(),
                        host: Some(host.clone()),
                    },
                    kind: if is_role {
                        PrincipalKind::Role
                    } else {
                        PrincipalKind::User
                    },
                    name: format!("'{user}'@'{host}'"),
                    native_kind: if is_role { "role" } else { "account" }.to_owned(),
                    system: user == "root"
                        || user.starts_with("mysql.")
                        || user == "mariadb.sys"
                        || user == "PUBLIC",
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;
        if self.kind != DatabaseKind::MariaDb && mysql_roles.is_empty() {
            for entry in &mut entries {
                entry.native_kind = "account (role metadata unavailable)".to_owned();
            }
        }
        Ok(PrincipalPage {
            connection: ConnectionIdentity {
                profile_id: self.connection_id,
                generation: 0,
            },
            entries,
            complete: true,
        })
    }

    pub async fn principal_ddl(
        &self,
        principal: &PrincipalEntry,
    ) -> Result<PrincipalDdl, DatabaseError> {
        self.reject_tidb_operation("principal DDL")?;
        if principal.id.profile_id != self.connection_id {
            return Err(DatabaseError::configuration(
                "principal does not belong to this connection",
            ));
        }
        let user = principal.id.native_id.clone();
        let host = principal.id.host.clone().unwrap_or_else(|| "%".to_owned());
        // MariaDB's `mysql.user` view does not expose every MySQL column, so
        // the account-lock flag is only read where it exists.
        let detail_sql = if self.kind == DatabaseKind::MariaDb {
            "SELECT plugin, \
             (authentication_string IS NOT NULL AND authentication_string <> '') AS has_password \
             FROM mysql.user WHERE User = ? AND Host = ?"
        } else {
            "SELECT plugin, account_locked, \
             (authentication_string IS NOT NULL AND authentication_string <> '') AS has_password \
             FROM mysql.user WHERE User = ? AND Host = ?"
        };
        let row = sqlx::query(detail_sql)
            .bind(&user)
            .bind(&host)
            .fetch_optional(&self.pool)
            .await
            .map_err(sql_error)?
            .ok_or_else(|| DatabaseError::configuration("principal no longer exists"))?;
        let plugin: Option<String> = row.try_get("plugin").map_err(decode_error)?;
        let has_password: i64 = row
            .try_get::<Option<i64>, _>("has_password")
            .map_err(decode_error)?
            .unwrap_or(0);
        let locked = if self.kind == DatabaseKind::MariaDb {
            false
        } else {
            row.try_get::<Option<String>, _>("account_locked")
                .map_err(decode_error)?
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case("Y"))
        };

        let grants = self.principal_grants(&user, &host).await?;
        let sql = assemble_principal_ddl(
            principal.kind,
            &user,
            &host,
            plugin.as_deref(),
            has_password != 0,
            locked,
            &grants,
        );
        Ok(PrincipalDdl {
            principal: principal.clone(),
            sql,
        })
    }

    pub async fn principal_details(
        &self,
        principal: &PrincipalEntry,
        target: &crate::db::principal::PrincipalReadTarget,
    ) -> Result<crate::db::principal::PrincipalDetails, DatabaseError> {
        self.reject_tidb_operation("principal metadata")?;
        if principal.id.profile_id != self.connection_id || target.principal != principal.id {
            return Err(DatabaseError::configuration(
                "principal details target does not match this connection",
            ));
        }
        let grants = self
            .principal_grants(
                &principal.id.native_id,
                principal.id.host.as_deref().unwrap_or("%"),
            )
            .await?;
        let permissions = grants
            .into_iter()
            .map(|grant| crate::db::principal::PrincipalPermission {
                target: "native SHOW GRANTS".to_owned(),
                mutation_target: None,
                privilege: grant,
                source: "native grant".to_owned(),
                grantable: false,
                source_kind: crate::db::principal::PrincipalPermissionSource::Direct,
            })
            .collect();
        Ok(crate::db::principal::PrincipalDetails {
            principal: principal.clone(),
            database: target.database.clone(),
            permissions,
            member_of: Vec::new(),
            members: Vec::new(),
            permissions_coverage: mysql_show_grants_coverage(),
            membership_coverage: mysql_membership_coverage(),
        })
    }

    pub fn plan_principal_mutation(
        &self,
        principal: &PrincipalEntry,
        connection: crate::identity::ConnectionIdentity,
        database: Option<&str>,
        mutation: crate::db::principal::PrincipalMutation,
    ) -> Result<crate::db::principal::PrincipalMutationPlan, DatabaseError> {
        self.reject_tidb_operation("principal mutations")?;
        if principal.id.profile_id != self.connection_id {
            return Err(DatabaseError::configuration(
                "principal does not belong to this connection",
            ));
        }
        let account = format!(
            "{}@{}",
            quote_literal(&principal.id.native_id),
            quote_literal(principal.id.host.as_deref().unwrap_or("%"))
        );
        let sql = match mutation {
            crate::db::principal::PrincipalMutation::Grant {
                target, privilege, ..
            } => {
                let crate::db::principal::PrincipalMutationTarget::Relation { schema, relation } =
                    target
                else {
                    return Err(DatabaseError::unsupported(
                        "MySQL grants currently require a relation target",
                    ));
                };
                format!(
                    "GRANT {privilege} ON {}.{} TO {account};",
                    quote_identifier(&schema),
                    quote_identifier(&relation)
                )
            }
            crate::db::principal::PrincipalMutation::Revoke {
                target, privilege, ..
            } => {
                let crate::db::principal::PrincipalMutationTarget::Relation { schema, relation } =
                    target
                else {
                    return Err(DatabaseError::unsupported(
                        "MySQL revokes currently require a relation target",
                    ));
                };
                format!(
                    "REVOKE {privilege} ON {}.{} FROM {account};",
                    quote_identifier(&schema),
                    quote_identifier(&relation)
                )
            }
            _ => {
                return Err(DatabaseError::unsupported(
                    "MySQL role membership mutations are not enabled yet",
                ));
            }
        };
        Ok(crate::db::principal::PrincipalMutationPlan {
            connection,
            principal: principal.clone(),
            database: database.map(str::to_owned),
            sql,
        })
    }

    async fn list_visible_mariadb_principals(&self) -> Result<PrincipalPage, DatabaseError> {
        let current_user = sqlx::query_scalar::<_, String>("SELECT CURRENT_USER()")
            .fetch_one(&self.pool)
            .await
            .map_err(sql_error)?;
        let (user, host) = current_user
            .rsplit_once('@')
            .filter(|(user, host)| !user.is_empty() && !host.is_empty())
            .ok_or_else(|| {
                DatabaseError::configuration(
                    "MariaDB CURRENT_USER() has an invalid account identity",
                )
            })?;

        let mut entries = vec![PrincipalEntry {
            id: PrincipalId {
                profile_id: self.connection_id,
                scope: PrincipalScope::Server,
                native_id: user.to_owned(),
                host: Some(host.to_owned()),
            },
            kind: PrincipalKind::User,
            name: format!("'{user}'@'{host}'"),
            native_kind: "account (visible only)".to_owned(),
            system: false,
        }];
        let role_names = sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT ROLE_NAME FROM information_schema.APPLICABLE_ROLES ORDER BY ROLE_NAME",
        )
        .fetch_all(&self.pool)
        .await;
        let role_names = match role_names {
            Ok(role_names) => role_names,
            Err(error) if is_mysql_privilege_error(&error) => Vec::new(),
            Err(error) => return Err(sql_error(error)),
        };
        for role in role_names {
            if entries
                .iter()
                .any(|entry| entry.kind == PrincipalKind::Role && entry.id.native_id == role)
            {
                continue;
            }
            entries.push(PrincipalEntry {
                id: PrincipalId {
                    profile_id: self.connection_id,
                    scope: PrincipalScope::Server,
                    native_id: role.clone(),
                    host: None,
                },
                kind: PrincipalKind::Role,
                name: role,
                native_kind: "role (visible only)".to_owned(),
                system: false,
            });
        }
        Ok(PrincipalPage {
            connection: ConnectionIdentity {
                profile_id: self.connection_id,
                generation: 0,
            },
            entries,
            complete: false,
        })
    }

    async fn principal_grants(&self, user: &str, host: &str) -> Result<Vec<String>, DatabaseError> {
        // `SHOW GRANTS` cannot use placeholders, so the account identifier is
        // escaped into the statement instead.
        let statement = format!(
            "SHOW GRANTS FOR {}@{}",
            quote_literal(user),
            quote_literal(host)
        );
        let rows = sqlx::query(AssertSqlSafe(statement))
            .fetch_all(&self.pool)
            .await
            .map_err(sql_error)?;
        let mut grants = Vec::new();
        for row in rows {
            // The single column is named after the account, so read positionally.
            if let Ok(value) = row.try_get::<String, _>(0) {
                grants.push(value);
            }
        }
        Ok(grants)
    }

    pub async fn object_ddl(
        &self,
        kind: CatalogKind,
        schema: &str,
        name: &str,
    ) -> Result<Option<String>, DatabaseError> {
        if self.kind == DatabaseKind::TiDb && !tidb::supported_relation(kind) {
            return Err(DatabaseError::unsupported(
                "TiDB native DDL supports tables and views only",
            ));
        }
        if !matches!(kind, CatalogKind::Table | CatalogKind::View) {
            return Ok(None);
        }
        if self.kind == DatabaseKind::TiDb && !self.catalog_scope.allows_schema(schema, schema) {
            return Err(DatabaseError::unsupported(
                "TiDB native DDL target is outside the catalog scope",
            ));
        }
        let mut connection = self.pool.acquire().await.map_err(sql_error)?;
        self.check_tidb_catalog_version(&mut connection).await?;
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
                if self.adapter.kind == DatabaseKind::MySql {
                    if request.metadata.primary_key.is_empty() {
                        return Err(TransactionError(
                            "MySQL inserted row has no primary key for reliable lookup".into(),
                        ));
                    }
                    for name in &request.metadata.primary_key {
                        let index = columns
                            .iter()
                            .position(|(column, _, _)| column == name)
                            .ok_or_else(|| {
                                TransactionError("MySQL primary key column is missing".into())
                            })?;
                        let value = insert
                            .columns
                            .iter()
                            .position(|column| *column == index)
                            .and_then(|position| insert.values.get(position));
                        if request.metadata.primary_key.len() > 1
                            && !matches!(value, Some(InputValue::Value(_)))
                        {
                            return Err(TransactionError(
                                "MySQL inserted row has an unknown primary key value for reliable lookup".into(),
                            ));
                        }
                    }
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
                let mut sql = if supplied.is_empty() {
                    format!("INSERT INTO {quoted_table} () VALUES ()")
                } else {
                    format!(
                        "INSERT INTO {quoted_table} ({}) VALUES ({})",
                        supplied.join(", "),
                        expressions.join(", ")
                    )
                };
                let returning = self.adapter.kind == DatabaseKind::MariaDb;
                if returning {
                    if columns.is_empty() {
                        return Err(TransactionError(
                            "MariaDB insert mutation has no relation columns".into(),
                        ));
                    }
                    sql.push_str(" RETURNING ");
                    sql.push_str(
                        &columns
                            .iter()
                            .map(|(name, _, _)| quote_identifier(name))
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                }
                let mut query = sqlx::query(AssertSqlSafe(sql));
                for value in &insert.values {
                    match value {
                        InputValue::Default => {}
                        InputValue::Null => query = query.bind(Option::<String>::None),
                        InputValue::Value(value) => query = bind_cell(query, value)?,
                    }
                }
                if returning {
                    let row = query
                        .fetch_one(&mut *self.connection)
                        .await
                        .map_err(|error| TransactionError(error.to_string()))?;
                    return Ok(MutationResult::Inserted {
                        row: decode_row(&row),
                        version: None,
                    });
                }
                let result = query
                    .execute(&mut *self.connection)
                    .await
                    .map_err(|e| TransactionError(e.to_string()))?;
                let primary_key_columns = request
                    .metadata
                    .primary_key
                    .iter()
                    .map(|name| {
                        let index = columns
                            .iter()
                            .position(|(column, _, _)| column == name)
                            .ok_or_else(|| {
                                TransactionError("MySQL primary key column is missing".into())
                            })?;
                        let value = insert
                            .columns
                            .iter()
                            .position(|column| *column == index)
                            .and_then(|position| insert.values.get(position));
                        Ok((name, value))
                    })
                    .collect::<Result<Vec<_>, TransactionError>>()?;
                if primary_key_columns.is_empty() {
                    return Err(TransactionError(
                        "MySQL inserted row has no primary key".into(),
                    ));
                }
                let predicates = primary_key_columns
                    .iter()
                    .map(|(name, _)| format!("{} = ?", quote_identifier(name)))
                    .collect::<Vec<_>>();
                let sql = format!(
                    "SELECT * FROM {quoted_table} WHERE {}",
                    predicates.join(" AND ")
                );
                let mut select = sqlx::query(AssertSqlSafe(sql));
                for (_, value) in &primary_key_columns {
                    match value {
                        Some(InputValue::Value(value)) => select = bind_cell(select, value)?,
                        // A NULL supplied for an AUTO_INCREMENT primary key is
                        // replaced by the server. MySQL exposes that generated
                        // value through LAST_INSERT_ID(). A non-auto-increment
                        // primary key rejects the INSERT before this lookup.
                        Some(InputValue::Null) | Some(InputValue::Default) | None => {
                            select = select.bind(result.last_insert_id())
                        }
                    }
                }
                let row = select
                    .fetch_one(&mut *self.connection)
                    .await
                    .map_err(|e| TransactionError(e.to_string()))?;
                return Ok(MutationResult::Inserted {
                    row: decode_row(&row),
                    version: None,
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
                    version: None,
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
struct MySqlCheckInfo {
    name: String,
    expression: String,
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
        "sequence" => Ok(CatalogKind::Sequence),
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
            diagnostic: None,
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
        ObjectGroup::Dictionaries => "08:dictionaries",
        ObjectGroup::UserDefinedFunctions => "09:user_defined_functions",
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
    value
        .map(|value| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
                .unwrap_or(&value)
                .to_owned()
        })
        .filter(|value| !value.is_empty())
}

fn normalize_default_expression(value: Option<String>) -> Option<String> {
    value.map(|value| {
        value
            .strip_prefix('\'')
            .and_then(|value| value.strip_suffix('\''))
            .unwrap_or(&value)
            .to_owned()
    })
}

fn catalog_target_not_found(target: &CatalogTarget) -> DatabaseError {
    DatabaseError {
        category: ErrorCategory::Configuration,
        code: Some("catalog_target_not_found".to_owned()),
        message: sanitize_terminal_text(&format!(
            "MySQL catalog target was not found: {}",
            target.description()
        )),
        diagnostic: None,
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
        diagnostic: None,
    }
}

fn sql_error(error: sqlx::Error) -> DatabaseError {
    DatabaseError::from_sqlx(error, ErrorCategory::Sql)
}

fn is_mysql_privilege_error(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>())
        .is_some_and(|error| matches!(error.number(), 1142 | 1143))
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
        CellValue::MySqlGeometry { bytes, .. } => query.bind(bytes.clone()),
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
    supports_catalog_version_for_kind(DatabaseKind::MySql, version)
}

pub fn supports_catalog_version_for_kind(kind: DatabaseKind, version: &str) -> bool {
    match kind {
        DatabaseKind::MySql => {
            !version.to_ascii_lowercase().contains("mariadb")
                && !version.to_ascii_lowercase().contains("tidb")
                && parse_version_triplet(version).is_some_and(|version| version >= (8, 0, 13))
        }
        DatabaseKind::MariaDb => {
            version.to_ascii_lowercase().contains("mariadb")
                && !version.to_ascii_lowercase().contains("tidb")
                && parse_mariadb_version_triplet(version)
                    .is_some_and(|version| version >= (10, 5, 0))
        }
        DatabaseKind::TiDb => {
            tidb::product_version(version).is_some_and(|version| version >= (8, 5, 0))
        }
        _ => false,
    }
}

fn unsupported_catalog_version(kind: DatabaseKind, version: &str) -> DatabaseError {
    let product = match kind {
        DatabaseKind::MariaDb => "MariaDB 10.5 or newer",
        DatabaseKind::TiDb => "stable TiDB 8.5.0 or newer (8.5 LTS baseline)",
        _ => "Oracle MySQL 8.0.13 or newer",
    };
    DatabaseError {
        category: ErrorCategory::Unsupported,
        code: Some("mysql_compatible_catalog_version_unsupported".to_owned()),
        message: sanitize_terminal_text(&format!(
            "catalog pages require {product}; server reported {version}"
        )),
        diagnostic: None,
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

fn parse_mariadb_version_triplet(version: &str) -> Option<(u32, u32, u32)> {
    let version = version.to_ascii_lowercase();
    if !version.contains("mariadb") {
        return None;
    }
    version
        .split('-')
        .rev()
        .filter_map(parse_version_triplet)
        .find(|version| *version >= (10, 0, 0))
}

pub fn quote_identifier(value: &str) -> String {
    format!("`{}`", value.replace('`', "``"))
}

/// Quote a MySQL/MariaDB string literal for statements that cannot take
/// placeholders (such as `SHOW GRANTS FOR`).
pub fn quote_literal(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for character in value.chars() {
        match character {
            '\'' => quoted.push_str("''"),
            '\\' => quoted.push_str("\\\\"),
            _ => quoted.push(character),
        }
    }
    quoted.push('\'');
    quoted
}

/// Build a readable, credential-free definition for a MySQL/MariaDB account.
///
/// Password material can never be recovered from the catalog, so it is
/// deliberately omitted and documented. `SHOW GRANTS` may append account
/// authentication clauses (including password hashes) to a grant, so those
/// clauses must be removed before displaying the statements.
fn assemble_principal_ddl(
    kind: PrincipalKind,
    user: &str,
    host: &str,
    plugin: Option<&str>,
    has_password: bool,
    locked: bool,
    grants: &[String],
) -> String {
    let account = format!("{}@{}", quote_identifier(user), quote_identifier(host));
    let mut lines = Vec::new();
    match kind {
        PrincipalKind::Role => {
            lines.push(format!("CREATE ROLE {account};"));
        }
        PrincipalKind::User => {
            match plugin.filter(|plugin| !plugin.is_empty()) {
                Some(plugin) => lines.push(format!(
                    "CREATE USER {account} IDENTIFIED WITH {};",
                    quote_literal(plugin)
                )),
                None => lines.push(format!("CREATE USER {account};")),
            }
            if locked {
                lines.push(format!("ALTER USER {account} ACCOUNT LOCK;"));
            }
            if has_password {
                lines.push(
                    "-- Password is intentionally omitted from read-only catalog DDL.".to_owned(),
                );
            }
        }
    }
    for grant in grants {
        let grant = credential_free_mysql_grant(grant);
        if let Some(grant) = grant.map(str::trim).filter(|grant| !grant.is_empty()) {
            lines.push(if grant.ends_with(';') {
                grant.to_owned()
            } else {
                format!("{grant};")
            });
        }
    }
    lines.join("\n")
}

fn credential_free_mysql_grant(grant: &str) -> Option<&str> {
    let grant = grant.trim();
    // MariaDB may append `IDENTIFIED BY PASSWORD`, `IDENTIFIED VIA ... USING`,
    // or MySQL's `IDENTIFIED WITH ... AS` to SHOW GRANTS output. The suffix is
    // account authentication metadata, not a privilege grant, and can contain
    // a reusable password hash. Keep the privilege statement before it only.
    let bytes = grant.as_bytes();
    let mut index = 0;
    let mut quote = None;
    let mut safe_end = grant.len();
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                index = (index + 2).min(bytes.len());
                continue;
            }
            if byte == delimiter {
                if bytes.get(index + 1) == Some(&delimiter) {
                    index += 2;
                    continue;
                }
                quote = None;
            }
            index += 1;
            continue;
        }
        if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
            index += 1;
            continue;
        }
        const KEYWORD: &[u8] = b"IDENTIFIED";
        let keyword_start = index + 1;
        let keyword_end = keyword_start + KEYWORD.len();
        if byte.is_ascii_whitespace()
            && bytes
                .get(keyword_start..keyword_end)
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(KEYWORD))
            && bytes.get(keyword_end).is_none_or(u8::is_ascii_whitespace)
        {
            safe_end = index;
            break;
        }
        index += 1;
    }
    let safe = grant[..safe_end].trim_end();
    (!safe.is_empty()).then_some(safe)
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
    if type_name == "GEOMETRY" {
        return row
            .try_get_unchecked::<Vec<u8>, _>(index)
            .map(
                |bytes| match crate::db::mysql::geometry::mysql_geometry_wkt(&bytes) {
                    Some(wkt) => CellValue::MySqlGeometry { bytes, wkt },
                    None => CellValue::Bytes(bytes),
                },
            )
            .unwrap_or_else(|error| unsupported(&type_name, &error.to_string()));
    }
    // MariaDB reports character JSON columns as binary BLOB values because
    // their protocol flag is binary even when their collation is textual
    // (for example, utf8mb4_bin). Use SQLx's compatibility check, which
    // includes the collation, instead of relying on the lossy type name.
    if <String as Type<MySql>>::compatible(&raw.type_info()) {
        return row
            .try_get_unchecked::<String, _>(index)
            .map(CellValue::Text)
            .unwrap_or_else(|error| unsupported(&type_name, &error.to_string()));
    }
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
            .map(CellValue::Time)
            .or_else(|_| {
                // MariaDB TIME is a duration, not only a wall-clock time:
                // it may be negative or exceed 24 hours. chrono::NaiveTime
                // cannot represent those values, so preserve the server's
                // textual form instead of failing or wrapping it.
                row.try_get_unchecked::<String, _>(index)
                    .map(CellValue::Text)
            }),
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
    use super::MySqlAdapter;
    use super::{
        MySqlConstraintPart, MySqlIndexPart, PROBE_SQL, assemble_relation_ddl,
        group_constraint_parts, group_index_parts, relation_kind, relation_path,
        search_catalog_kind,
    };
    use crate::db::catalog::{CatalogId, CatalogKind, DdlProvenance};
    use crate::db::mutation::{
        InputValue, InsertRowMutation, MetadataFingerprint, MutationResult, RelationMutation,
        RelationMutationRequest,
    };
    use crate::db::transaction::TransactionBackend;
    use crate::db::value::CellValue;
    use crate::identity::ConnectionIdentity;
    use crate::model::execution_target::ExecutionTarget;
    use crate::model::relation::RelationKey;
    use crate::model::relation_edit::EditableRowId;
    use crate::profile::import_connection_url;
    use uuid::Uuid;

    #[test]
    fn principal_ddl_omits_credentials_and_keeps_account_identity() {
        let sql = super::assemble_principal_ddl(
            crate::db::principal::PrincipalKind::User,
            "app'user",
            "10.0.0.%",
            Some("caching_sha2_password"),
            true,
            true,
            &[
                "GRANT USAGE ON *.* TO `app'user`@`10.0.0.%`".to_owned(),
                "GRANT USAGE ON *.* TO `app'user`@`10.0.0.%` IDENTIFIED BY PASSWORD '*1BAA2A7EA3EBF668FFD2BC9141643BEE34B5E076'".to_owned(),
                "GRANT SELECT ON `app`.* TO `app'user`@`10.0.0.%` IDENTIFIED VIA mysql_native_password USING '*HASH'".to_owned(),
                "".to_owned(),
            ],
        );
        assert!(sql.contains(
            "CREATE USER `app'user`@`10.0.0.%` IDENTIFIED WITH 'caching_sha2_password';"
        ));
        assert!(sql.contains("ALTER USER `app'user`@`10.0.0.%` ACCOUNT LOCK;"));
        assert!(sql.contains("-- Password is intentionally omitted from read-only catalog DDL."));
        assert!(sql.contains("GRANT USAGE ON *.* TO `app'user`@`10.0.0.%`;"));
        assert!(sql.contains("GRANT SELECT ON `app`.* TO `app'user`@`10.0.0.%`;"));
        // No password material or hash may ever appear.
        assert!(!sql.contains("IDENTIFIED BY"), "{sql}");
        assert!(!sql.contains("IDENTIFIED VIA"), "{sql}");
        assert!(
            !sql.contains("1BAA2A7EA3EBF668FFD2BC9141643BEE34B5E076"),
            "{sql}"
        );
        assert!(!sql.contains("*HASH"), "{sql}");
        assert!(!sql.contains("AS '$"), "{sql}");
    }

    #[test]
    fn principal_grants_preserve_identified_inside_quoted_names() {
        let grant = "GRANT SELECT ON `IDENTIFIED schema`.* TO `app`@`%`";
        assert_eq!(super::credential_free_mysql_grant(grant), Some(grant));
    }

    #[test]
    fn principal_ddl_creates_roles_without_user_clauses() {
        let sql = super::assemble_principal_ddl(
            crate::db::principal::PrincipalKind::Role,
            "auditors",
            "%",
            None,
            false,
            true,
            &[],
        );
        assert_eq!(sql, "CREATE ROLE `auditors`@`%`;");
    }

    #[test]
    fn mysql_literal_quoting_escapes_quotes_and_backslashes() {
        assert_eq!(super::quote_literal("a'b"), "'a''b'");
        assert_eq!(super::quote_literal("a\\b"), "'a\\\\b'");
    }

    fn mariadb_test_url() -> Option<String> {
        match std::env::var("LAZYDB_TEST_MARIADB_URL") {
            Ok(url) if !url.trim().is_empty() => Some(url),
            Ok(_) | Err(std::env::VarError::NotPresent) => {
                if std::env::var_os("LAZYDB_REQUIRE_DATABASE_TESTS").is_some() {
                    panic!("LAZYDB_TEST_MARIADB_URL is required for MariaDB integration tests");
                }
                eprintln!("SKIP: LAZYDB_TEST_MARIADB_URL is not set");
                None
            }
            Err(std::env::VarError::NotUnicode(_)) => {
                panic!("LAZYDB_TEST_MARIADB_URL is not valid Unicode")
            }
        }
    }

    #[tokio::test]
    async fn mariadb_insert_returning_returns_generated_row_for_explicit_null_key() {
        let Some(url) = mariadb_test_url() else {
            return;
        };
        let imported = import_connection_url(&url, Some("MariaDB insert returning")).unwrap();
        let adapter =
            MySqlAdapter::connect(&imported.profile, imported.transient_password.as_ref())
                .await
                .unwrap();
        let table = format!("lazydb_insert_returning_{}", Uuid::new_v4().simple());
        adapter
            .execute(&format!(
                "CREATE TABLE `{table}` (id INT PRIMARY KEY AUTO_INCREMENT, value VARCHAR(32) DEFAULT 'server-default') ENGINE=InnoDB"
            ))
            .await
            .unwrap();

        let database = imported.profile.database.clone().unwrap();
        let relation = CatalogId::new(
            imported.profile.id,
            CatalogKind::Table,
            [database.clone(), database.clone(), table.clone()],
        );
        let metadata = MetadataFingerprint {
            relation: table.clone(),
            columns: vec![
                ("id".into(), "int".into(), false),
                ("value".into(), "varchar".into(), true),
            ],
            primary_key: vec!["id".into()],
        };
        let request = RelationMutationRequest {
            tab_id: Uuid::nil(),
            tab_generation: 1,
            edit_generation: 1,
            row_id: EditableRowId(1),
            connection: ConnectionIdentity {
                profile_id: imported.profile.id,
                generation: 1,
            },
            target: ExecutionTarget::from_profile(&imported.profile),
            relation: relation.clone(),
            relation_key: RelationKey {
                profile_id: imported.profile.id,
                object_id: relation,
            },
            scope: imported.profile.catalog_scope.clone(),
            metadata,
            operation: RelationMutation::InsertRow(InsertRowMutation {
                columns: vec![0],
                values: vec![InputValue::Null],
            }),
        };
        let mut backend = adapter.transaction_backend().await.unwrap();
        backend.begin().await.unwrap();
        let result = backend.relation_mutation(request).await.unwrap();
        let MutationResult::Inserted { row, .. } = result else {
            panic!("expected inserted row");
        };
        assert!(matches!(row[0], CellValue::Integer(value) if value > 0));
        assert_eq!(row[1], CellValue::Text("server-default".into()));
        backend.commit().await.unwrap();
        drop(backend);
        adapter
            .execute(&format!("DROP TABLE `{table}`"))
            .await
            .unwrap();
        adapter.close().await;
    }

    #[tokio::test]
    async fn mariadb_keyless_text_insert_round_trip_commits_and_rolls_back() {
        let Some(url) = mariadb_test_url() else {
            return;
        };
        let imported =
            import_connection_url(&url, Some("MariaDB keyless relation insert")).unwrap();
        let adapter =
            MySqlAdapter::connect(&imported.profile, imported.transient_password.as_ref())
                .await
                .unwrap();
        let table = format!("lazydb_keyless_insert_{}", Uuid::new_v4().simple());
        adapter
            .execute(&format!(
                "CREATE TABLE `{table}` (`name` TEXT DEFAULT NULL, `id` TEXT DEFAULT NULL) ENGINE=InnoDB"
            ))
            .await
            .unwrap();

        let database = imported.profile.database.clone().unwrap();
        let relation = CatalogId::new(
            imported.profile.id,
            CatalogKind::Table,
            [database.clone(), database.clone(), table.clone()],
        );
        let metadata = MetadataFingerprint {
            relation: table.clone(),
            columns: vec![
                ("name".into(), "text".into(), true),
                ("id".into(), "text".into(), true),
            ],
            primary_key: Vec::new(),
        };
        let request = |row_id, value: &str| RelationMutationRequest {
            tab_id: Uuid::nil(),
            tab_generation: 1,
            edit_generation: 1,
            row_id: EditableRowId(row_id),
            connection: ConnectionIdentity {
                profile_id: imported.profile.id,
                generation: 1,
            },
            target: ExecutionTarget::from_profile(&imported.profile),
            relation: relation.clone(),
            relation_key: RelationKey {
                profile_id: imported.profile.id,
                object_id: relation.clone(),
            },
            scope: imported.profile.catalog_scope.clone(),
            metadata: metadata.clone(),
            operation: RelationMutation::InsertRow(InsertRowMutation {
                columns: vec![0, 1],
                values: vec![
                    InputValue::Value(CellValue::Text(value.into())),
                    InputValue::Value(CellValue::Text(value.into())),
                ],
            }),
        };

        let mut backend = adapter.transaction_backend().await.unwrap();
        backend.begin().await.unwrap();
        for row_id in 1..=2 {
            let result = backend
                .relation_mutation(request(row_id, "same"))
                .await
                .unwrap();
            assert_eq!(
                result,
                MutationResult::Inserted {
                    row: vec![
                        CellValue::Text("same".into()),
                        CellValue::Text("same".into())
                    ],
                    version: None,
                }
            );
        }
        backend.commit().await.unwrap();

        let result = adapter
            .execute(&format!("SELECT COUNT(*) FROM `{table}`"))
            .await
            .unwrap();
        assert_eq!(result.result_sets[0].rows[0][0], CellValue::Integer(2));

        backend.begin().await.unwrap();
        backend
            .relation_mutation(request(3, "rolled-back"))
            .await
            .unwrap();
        backend.rollback().await.unwrap();
        drop(backend);

        let result = adapter
            .execute(&format!("SELECT COUNT(*) FROM `{table}`"))
            .await
            .unwrap();
        assert_eq!(result.result_sets[0].rows[0][0], CellValue::Integer(2));

        adapter
            .execute(&format!("DROP TABLE `{table}`"))
            .await
            .unwrap();
        adapter.close().await;
    }

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
            "sequence",
        ] {
            assert!(search_catalog_kind(native).is_ok(), "missing {native}");
        }
        for unsupported in ["materialized_view", "check_constraint", "type"] {
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
