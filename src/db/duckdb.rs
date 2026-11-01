use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

use chrono::{Datelike, Timelike};
use duckdb::{AccessMode, Config, Connection};
use futures_util::future::BoxFuture;
use tokio::task;

use crate::{
    db::{
        DatabaseError, ErrorCategory, ServerInfo,
        mutation::{InputValue, MutationResult, RelationMutation, RelationMutationRequest},
        query::{ColumnMeta, QueryBudget, QueryOutcome, QueryOutcomeAccumulator},
    },
    profile::{ConnectionProfile, DatabaseKind},
    sql::{SqlDialect, scan_statements},
};

#[derive(Clone, Debug)]
pub struct DuckDbAdapter {
    connection: Arc<Mutex<Connection>>,
    operation_gate: Arc<tokio::sync::Semaphore>,
    path: Option<PathBuf>,
    read_only: bool,
    database: String,
    profile_id: uuid::Uuid,
    catalog_scope: crate::profile::CatalogScope,
}

#[derive(Clone)]
pub(crate) struct DuckDbOperationHandle {
    interrupt: Arc<duckdb::InterruptHandle>,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
    active: Arc<Mutex<bool>>,
}

impl DuckDbOperationHandle {
    pub(crate) fn cancel(&self) {
        let active = self
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *active {
            self.cancelled
                .store(true, std::sync::atomic::Ordering::SeqCst);
            self.interrupt.interrupt();
        }
    }
}

struct DuckDbOperationGuard {
    handle: Option<DuckDbOperationHandle>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl Drop for DuckDbOperationGuard {
    fn drop(&mut self) {
        if let Some(handle) = &self.handle {
            // Finish any in-flight interrupt before releasing this connection to the next query.
            *handle
                .active
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = false;
        }
    }
}

pub(crate) struct DuckDbTransactionBackend {
    connection: Arc<Mutex<Connection>>,
    adapter: DuckDbAdapter,
    interrupt: Arc<duckdb::InterruptHandle>,
    depth: usize,
    _operation_permit: tokio::sync::OwnedSemaphorePermit,
}

#[async_trait::async_trait]
impl crate::db::transaction::TransactionBackend for DuckDbTransactionBackend {
    async fn begin(&mut self) -> Result<(), crate::db::transaction::TransactionError> {
        let connection = self.connection.clone();
        task::spawn_blocking(move || {
            connection
                .lock()
                .map_err(|_| DatabaseError::internal("DuckDB transaction lock poisoned"))?
                .execute_batch("BEGIN TRANSACTION")
                .map_err(duck_error)
        })
        .await
        .map_err(|error| DatabaseError::internal(format!("DuckDB worker failed: {error}")))??;
        self.depth = 1;
        Ok(())
    }

    async fn execute(
        &mut self,
        sql: &str,
    ) -> Result<QueryOutcome, crate::db::transaction::TransactionError> {
        self.adapter.execute(sql).await.map_err(Into::into)
    }

    async fn commit(&mut self) -> Result<(), crate::db::transaction::TransactionError> {
        let connection = self.connection.clone();
        task::spawn_blocking(move || {
            connection
                .lock()
                .map_err(|_| DatabaseError::internal("DuckDB transaction lock poisoned"))?
                .execute_batch("COMMIT")
                .map_err(duck_error)
        })
        .await
        .map_err(|error| DatabaseError::internal(format!("DuckDB worker failed: {error}")))??;
        self.depth = 0;
        Ok(())
    }

    async fn rollback(&mut self) -> Result<(), crate::db::transaction::TransactionError> {
        let connection = self.connection.clone();
        task::spawn_blocking(move || {
            connection
                .lock()
                .map_err(|_| DatabaseError::internal("DuckDB transaction lock poisoned"))?
                .execute_batch("ROLLBACK")
                .map_err(duck_error)
        })
        .await
        .map_err(|error| DatabaseError::internal(format!("DuckDB worker failed: {error}")))??;
        self.depth = 0;
        Ok(())
    }

    async fn cancel(&mut self) -> Result<(), crate::db::transaction::TransactionError> {
        self.interrupt.interrupt();
        Ok(())
    }

    fn depth(&self) -> usize {
        self.depth
    }

    async fn relation_mutation(
        &mut self,
        request: RelationMutationRequest,
    ) -> Result<MutationResult, crate::db::transaction::TransactionError> {
        use crate::db::transaction::TransactionError;
        if self.adapter.read_only {
            return Err(TransactionError(
                "DuckDB relation mutations require a writable connection".into(),
            ));
        }
        let [_, schema, table] = request.relation.native_path.as_slice() else {
            return Err(TransactionError(
                "DuckDB relation has no canonical database, schema, and table path".into(),
            ));
        };
        if request.connection.profile_id != self.adapter.profile_id
            || request.relation.profile_id() != self.adapter.profile_id
            || request.relation_key.profile_id != self.adapter.profile_id
            || request.relation_key.object_id != request.relation
            || request.target.profile_id != self.adapter.profile_id
            || !request
                .scope
                .allows_schema(&request.relation.native_path[0], schema)
        {
            return Err(TransactionError(
                "DuckDB relation mutation identity or scope does not match its session".into(),
            ));
        }
        if request.metadata.columns.is_empty()
            || request.metadata.relation != *table
            || request
                .metadata
                .columns
                .iter()
                .any(|(name, _, _)| name.is_empty())
        {
            return Err(TransactionError(
                "DuckDB relation mutation metadata is invalid".into(),
            ));
        }
        let columns = request.metadata.columns.clone();
        let quoted_table = format!(
            "{}.{}",
            quote_duck_identifier(schema),
            quote_duck_identifier(table)
        );
        let connection = self.connection.clone();
        let metadata = request.metadata.clone();
        let operation = request.operation;
        let schema = schema.clone();
        let table = table.clone();
        task::spawn_blocking(move || {
            let connection = connection
                .lock()
                .map_err(|_| TransactionError("DuckDB connection lock poisoned".into()))?;
            let mut statement = connection
                .prepare("SELECT column_name, data_type, is_nullable FROM duckdb_columns() WHERE schema_name = ? AND table_name = ? ORDER BY column_index")
                .map_err(|error| TransactionError(error.to_string()))?;
            let native_columns = statement
                .query_map([schema.as_str(), table.as_str()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, bool>(2)?))
                })
                .map_err(|error| TransactionError(error.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| TransactionError(error.to_string()))?;
            let native_primary_key = connection
                .prepare("SELECT unnest(constraint_column_names) FROM duckdb_constraints() WHERE schema_name = ? AND table_name = ? AND constraint_type = 'PRIMARY KEY'")
                .and_then(|mut statement| {
                    statement
                        .query_map([schema.as_str(), table.as_str()], |row| row.get::<_, String>(0))
                        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
                })
                .unwrap_or_default();
            if native_columns != metadata.columns || native_primary_key != metadata.primary_key {
                return Err(TransactionError(
                    "DuckDB relation mutation metadata fingerprint is stale".into(),
                ));
            }
            match operation {
                RelationMutation::UpdateCell(update) => {
                    let Some((column, _, _)) = columns.get(update.column) else {
                        return Err(TransactionError(
                            "DuckDB update column is out of range".into(),
                        ));
                    };
                    let key_indexes = metadata
                        .primary_key
                        .iter()
                        .map(|name| {
                            columns
                                .iter()
                                .position(|(column, _, _)| column == name)
                                .ok_or_else(|| {
                                    TransactionError("DuckDB primary key column is missing".into())
                                })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    if key_indexes.is_empty()
                        || update.row.columns != key_indexes
                        || update.row.columns.len() != update.row.values.len()
                    {
                        return Err(TransactionError(
                            "DuckDB row locator must contain the primary key columns in order"
                                .into(),
                        ));
                    }
                    let mut parameters = Vec::new();
                    let assignment = match update.value {
                        InputValue::Default => {
                            format!("{} = DEFAULT", quote_duck_identifier(column))
                        }
                        InputValue::Null => {
                            parameters.push(duck_parameter(&crate::db::value::CellValue::Null)?);
                            format!("{} = ?", quote_duck_identifier(column))
                        }
                        InputValue::Value(ref value) => {
                            parameters.push(duck_parameter(value)?);
                            format!("{} = ?", quote_duck_identifier(column))
                        }
                    };
                    let mut predicates = Vec::new();
                    for (index, value) in update.row.columns.iter().zip(&update.row.values) {
                        if *index >= columns.len() {
                            return Err(TransactionError(
                                "DuckDB row locator column is out of range".into(),
                            ));
                        }
                        predicates.push(format!(
                            "{} IS NOT DISTINCT FROM ?",
                            quote_duck_identifier(&columns[*index].0)
                        ));
                        parameters.push(duck_parameter(value)?);
                    }
                    predicates.push(format!(
                        "{} IS NOT DISTINCT FROM ?",
                        quote_duck_identifier(column)
                    ));
                    parameters.push(duck_parameter(&update.original)?);
                    let sql = format!(
                        "UPDATE {quoted_table} SET {assignment} WHERE {} RETURNING *",
                        predicates.join(" AND ")
                    );
                    let mut statement = connection
                        .prepare(&sql)
                        .map_err(|error| TransactionError(error.to_string()))?;
                    let row = statement
                        .query_row(duckdb::params_from_iter(parameters.iter()), decode_duck_row)
                        .map_err(|error| TransactionError(error.to_string()))?;
                    Ok(MutationResult::Updated { row, version: None })
                }
                RelationMutation::DeleteRows(rows) => {
                    let key_indexes = metadata
                        .primary_key
                        .iter()
                        .map(|name| {
                            columns
                                .iter()
                                .position(|(column, _, _)| column == name)
                                .ok_or_else(|| {
                                    TransactionError("DuckDB primary key column is missing".into())
                                })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    if key_indexes.is_empty() {
                        return Err(TransactionError(
                            "DuckDB relation mutation requires a primary key".into(),
                        ));
                    }
                    let mut deleted = 0;
                    for mutation in rows {
                        if mutation.row.columns != key_indexes
                            || mutation.row.values.len() != key_indexes.len()
                            || mutation.original.len() != columns.len()
                        {
                            return Err(TransactionError(
                                "DuckDB delete mutation is malformed".into(),
                            ));
                        }
                        let mut predicates = Vec::new();
                        let mut parameters = Vec::new();
                        for (index, value) in mutation.row.columns.iter().zip(&mutation.row.values)
                        {
                            predicates.push(format!(
                                "{} IS NOT DISTINCT FROM ?",
                                quote_duck_identifier(&columns[*index].0)
                            ));
                            parameters.push(duck_parameter(value)?);
                        }
                        for (index, value) in mutation.original.iter().enumerate() {
                            predicates.push(format!(
                                "{} IS NOT DISTINCT FROM ?",
                                quote_duck_identifier(&columns[index].0)
                            ));
                            parameters.push(duck_parameter(value)?);
                        }
                        let sql = format!(
                            "DELETE FROM {quoted_table} WHERE {} RETURNING 1",
                            predicates.join(" AND ")
                        );
                        let mut statement = connection
                            .prepare(&sql)
                            .map_err(|error| TransactionError(error.to_string()))?;
                        statement
                            .query_row(duckdb::params_from_iter(parameters.iter()), |_| Ok(()))
                            .map_err(|error| {
                                TransactionError(format!(
                                    "DuckDB relation mutation conflict: {error}"
                                ))
                            })?;
                        deleted += 1;
                    }
                    Ok(MutationResult::Deleted { rows: deleted })
                }
                RelationMutation::InsertRow(insert) => {
                    if insert.columns.len() != insert.values.len()
                        || insert.columns.iter().any(|index| *index >= columns.len())
                    {
                        return Err(TransactionError(
                            "DuckDB insert mutation is malformed".into(),
                        ));
                    }
                    let supplied = insert
                        .columns
                        .iter()
                        .zip(&insert.values)
                        .filter(|(_, value)| !matches!(value, InputValue::Default))
                        .collect::<Vec<_>>();
                    let names = supplied
                        .iter()
                        .map(|(index, _)| quote_duck_identifier(&columns[**index].0))
                        .collect::<Vec<_>>();
                    let mut parameters = Vec::new();
                    for (_, value) in &supplied {
                        parameters.push(duck_parameter_input(value)?);
                    }
                    let sql = if names.is_empty() {
                        format!("INSERT INTO {quoted_table} DEFAULT VALUES RETURNING *")
                    } else {
                        format!(
                            "INSERT INTO {quoted_table} ({}) VALUES ({}) RETURNING *",
                            names.join(", "),
                            vec!["?"; names.len()].join(", ")
                        )
                    };
                    let mut statement = connection
                        .prepare(&sql)
                        .map_err(|error| TransactionError(error.to_string()))?;
                    let row = statement
                        .query_row(duckdb::params_from_iter(parameters.iter()), decode_duck_row)
                        .map_err(|error| TransactionError(error.to_string()))?;
                    Ok(MutationResult::Inserted { row, version: None })
                }
            }
        })
        .await
        .map_err(|error| TransactionError(format!("DuckDB mutation worker failed: {error}")))?
    }

    fn force_close(
        self,
    ) -> BoxFuture<'static, Result<(), crate::db::transaction::TransactionError>> {
        Box::pin(async move {
            self.interrupt.interrupt();
            let connection = self.connection;
            task::spawn_blocking(move || drop(connection))
                .await
                .map_err(|error| {
                    DatabaseError::internal(format!("DuckDB worker failed: {error}"))
                })?;
            Ok(())
        })
    }
}

impl DuckDbAdapter {
    pub fn catalog_mutation_capabilities()
    -> crate::db::catalog_mutation::CatalogMutationCapabilities {
        use crate::db::{catalog::CatalogKind, catalog_mutation::*};
        let create = [
            (CatalogKind::Schema, CatalogMutationAvailability::Available),
            (CatalogKind::Table, CatalogMutationAvailability::Available),
            (CatalogKind::View, CatalogMutationAvailability::Available),
            (
                CatalogKind::Sequence,
                CatalogMutationAvailability::Available,
            ),
            (CatalogKind::Index, CatalogMutationAvailability::Available),
        ]
        .into_iter()
        .map(|(kind, availability)| CatalogMutationOption {
            object_type: CatalogObjectType::Catalog(kind),
            availability,
        })
        .collect();
        let edit = [CatalogKind::Table, CatalogKind::View]
            .into_iter()
            .map(|kind| CatalogMutationOption {
                object_type: CatalogObjectType::Catalog(kind),
                availability: CatalogMutationAvailability::Available,
            })
            .collect();
        let profile_create = Vec::new();
        CatalogMutationCapabilities {
            create,
            edit,
            profile_create,
            ..CatalogMutationCapabilities::default()
        }
    }

    pub fn plan_catalog_drop(
        request: crate::db::catalog_drop::CatalogDropRequest,
        entry: &crate::db::catalog::CatalogEntry,
    ) -> Result<crate::db::catalog_drop::CatalogDropPlan, crate::db::catalog_drop::CatalogDropError>
    {
        use crate::db::{catalog::CatalogKind, catalog_drop::*};
        let kind = entry.kind;
        let name = &entry.qualified_name;
        if request.object != entry.id {
            return Err(CatalogDropError::ObjectMismatch);
        }
        let object = if let Some(schema) = name.schema.as_deref() {
            format!(
                "{}.{}",
                quote_duck_identifier(schema),
                quote_duck_identifier(&name.object)
            )
        } else {
            quote_duck_identifier(&name.object)
        };
        let sql = match kind {
            CatalogKind::Schema => format!("DROP SCHEMA {}", quote_duck_identifier(&name.object)),
            CatalogKind::Type => format!("DROP TYPE {object}"),
            CatalogKind::Table => format!("DROP TABLE {object}"),
            CatalogKind::View => format!("DROP VIEW {object}"),
            CatalogKind::Sequence => format!("DROP SEQUENCE {object}"),
            CatalogKind::Index => {
                let Some(schema) = name.schema.as_deref() else {
                    return Err(CatalogDropError::Unsupported {
                        kind,
                        reason: "DuckDB index drop requires its owning table schema".into(),
                    });
                };
                let [_, _, relation, _, ..] = entry.id.native_path.as_slice() else {
                    return Err(CatalogDropError::Unsupported {
                        kind,
                        reason: "DuckDB index identity is incomplete".into(),
                    });
                };
                let _ = relation;
                format!(
                    "DROP INDEX {}.{}",
                    quote_duck_identifier(schema),
                    quote_duck_identifier(&name.object)
                )
            }
            unsupported => {
                return Err(CatalogDropError::Unsupported {
                    kind: unsupported,
                    reason: "DuckDB does not expose a safe DROP operation for this catalog kind"
                        .into(),
                });
            }
        };
        CatalogDropPlan::new(request, entry, sql)
    }

    pub fn plan_catalog_mutation(
        request: crate::db::catalog_mutation::CatalogMutationRequest,
        draft: crate::model::catalog_editor::CatalogDraft,
        baseline: Option<crate::db::catalog_mutation::CatalogObjectDefinition>,
    ) -> Result<
        crate::db::catalog_mutation::CatalogMutationPlan,
        crate::db::catalog_mutation::CatalogMutationError,
    > {
        use crate::db::{
            catalog::{CatalogId, CatalogKind, CatalogTarget, ObjectGroup},
            catalog_mutation::*,
        };
        use crate::model::catalog_editor::CatalogDraft;
        let table_comments = match &draft {
            CatalogDraft::Table(draft) if request.mode == CatalogMutationMode::Create => {
                let mut statements = Vec::new();
                let target = format!(
                    "{}.{}",
                    quote_duck_identifier(draft.schema.value().trim()),
                    quote_duck_identifier(draft.name.value().trim())
                );
                if !draft.comment.value().trim().is_empty() {
                    statements.push(format!(
                        "COMMENT ON TABLE {target} IS {}",
                        duck_sql_literal_or_null(draft.comment.value())
                    ));
                }
                for column in &draft.columns {
                    if !column.comment.value().trim().is_empty() {
                        statements.push(format!(
                            "COMMENT ON COLUMN {target}.{} IS {}",
                            quote_duck_identifier(column.name.value().trim()),
                            duck_sql_literal_or_null(column.comment.value())
                        ));
                    }
                }
                statements
            }
            _ => Vec::new(),
        };
        if request.mode == CatalogMutationMode::Edit {
            let CatalogMutationAnchor::Catalog(object) = request.anchor.clone() else {
                return Err(CatalogMutationError::InvalidAnchor {
                    reason: "DuckDB edit requires a catalog object anchor",
                });
            };
            if request.object_type != CatalogObjectType::Catalog(object.kind) {
                return Err(CatalogMutationError::InvalidPlan {
                    reason: "DuckDB edit object kind does not match its anchor".into(),
                });
            }
            let (database, schema, old_name) = if object.kind == CatalogKind::Index {
                let [database, schema, _relation, index, ..] = object.native_path.as_slice() else {
                    return Err(CatalogMutationError::InvalidAnchor {
                        reason: "DuckDB index identity is incomplete",
                    });
                };
                (database, schema, index)
            } else if let [database, schema, name, ..] = object.native_path.as_slice() {
                (database, schema, name)
            } else {
                return Err(CatalogMutationError::InvalidAnchor {
                    reason: "DuckDB relation identity is incomplete",
                });
            };
            if request.current_database.as_deref() != Some(database.as_str()) {
                return Err(CatalogMutationError::InvalidAnchor {
                    reason: "DuckDB edit targets another database",
                });
            }
            let (kind, new_name, baseline_fingerprint, view_query_edit, table_changes) = match (
                object.kind,
                baseline,
                draft,
            ) {
                (
                    CatalogKind::Table,
                    Some(CatalogObjectDefinition::Table(baseline)),
                    CatalogDraft::Table(draft),
                ) => {
                    if draft.table_constraints.len() != baseline.structured_constraints.len()
                        || draft
                            .table_constraints
                            .iter()
                            .zip(&baseline.structured_constraints)
                            .any(|(draft, definition)| {
                                !crate::model::catalog_editor::table_constraint_matches(
                                    draft, definition,
                                )
                            })
                    {
                        return Err(CatalogMutationError::UnsupportedOperation {
                            object_type: request.object_type,
                        });
                    }
                    draft.validate()?;
                    let mut existing_columns = std::collections::HashMap::new();
                    for column in &baseline.columns {
                        existing_columns.insert(column.name.as_str(), column);
                    }
                    let mut renamed_columns = Vec::new();
                    let mut altered_columns = Vec::new();
                    let mut removed_columns = Vec::new();
                    let mut column_comments: Vec<(String, String)> = Vec::new();
                    let mut added_columns = Vec::new();
                    for column in &draft.columns {
                        match &column.state {
                            crate::model::catalog_editor::DraftRowState::Existing { .. } => {
                                let original_name = column.existing_name.as_deref().ok_or(
                                    CatalogMutationError::InvalidPlan {
                                        reason: "existing DuckDB column has no baseline identity"
                                            .into(),
                                    },
                                )?;
                                let original = existing_columns.remove(original_name).ok_or(
                                    CatalogMutationError::InvalidPlan {
                                        reason: "DuckDB column does not belong to the baseline"
                                            .into(),
                                    },
                                )?;
                                let original_collation = match &original.collation {
                                    crate::db::catalog::OptionalMetadata::Supported(Some(
                                        value,
                                    )) => value.as_str(),
                                    _ => "",
                                };
                                let original_identity = matches!(
                                    original.identity,
                                    crate::db::catalog::OptionalMetadata::Supported(Some(true))
                                );
                                let original_generated = match &original.generated_expression {
                                    crate::db::catalog::OptionalMetadata::Supported(Some(
                                        value,
                                    )) => value.as_str(),
                                    _ => "",
                                };
                                let original_comment = match &original.comment {
                                    crate::db::catalog::OptionalMetadata::Supported(Some(
                                        value,
                                    )) => value.as_str(),
                                    _ => "",
                                };
                                if column.name.value().trim() != original.name {
                                    renamed_columns.push((
                                        original.name.clone(),
                                        column.name.value().trim().to_owned(),
                                    ));
                                }
                                let original_default = match &original.default_expression {
                                    crate::db::catalog::OptionalMetadata::Supported(Some(
                                        value,
                                    )) => value.as_str(),
                                    _ => "",
                                };
                                if column.collation.value().trim() != original_collation
                                    || column.identity != original_identity
                                    || column.generated_expression.value().trim()
                                        != original_generated
                                {
                                    return Err(CatalogMutationError::UnsupportedOperation {
                                        object_type: request.object_type,
                                    });
                                }
                                if column.comment.value().trim() != original_comment {
                                    column_comments.push((
                                        column.name.value().trim().to_owned(),
                                        column.comment.value().to_owned(),
                                    ));
                                }
                                let default_changed =
                                    column.default_expression.value().trim() != original_default;
                                let nullable_changed = column.nullable != original.nullable;
                                let type_changed =
                                    column.native_type.value().trim() != original.native_type;
                                if default_changed || nullable_changed || type_changed {
                                    altered_columns.push((
                                        column.name.value().trim().to_owned(),
                                        default_changed.then(|| {
                                            column.default_expression.value().trim().to_owned()
                                        }),
                                        nullable_changed.then_some(column.nullable),
                                        type_changed
                                            .then(|| column.native_type.value().trim().to_owned()),
                                    ));
                                }
                            }
                            crate::model::catalog_editor::DraftRowState::Removed { .. } => {
                                let original_name = column.existing_name.as_deref().ok_or(
                                    CatalogMutationError::InvalidPlan {
                                        reason: "removed DuckDB column has no baseline identity"
                                            .into(),
                                    },
                                )?;
                                let original =
                                    existing_columns
                                        .remove(original_name)
                                        .ok_or(CatalogMutationError::InvalidPlan {
                                        reason:
                                            "removed DuckDB column does not belong to the baseline"
                                                .into(),
                                    })?;
                                removed_columns.push(original.name.clone());
                            }
                            crate::model::catalog_editor::DraftRowState::Added => {
                                added_columns.push(column);
                            }
                        }
                    }
                    if !existing_columns.is_empty() {
                        removed_columns
                            .extend(existing_columns.keys().map(|name| (*name).to_owned()));
                    }
                    let unchanged_owner = draft.owner.value() == baseline.owner;
                    if draft.indexes != baseline.indexes {
                        return Err(CatalogMutationError::UnsupportedOperation {
                            object_type: request.object_type,
                        });
                    }
                    if draft.constraints != baseline.constraints {
                        return Err(CatalogMutationError::InvalidDraft {
                                reason: "DuckDB does not support editing table constraints in place; use SQL to change constraints".into(),
                            });
                    }
                    let table_comment = (draft.comment.value().trim()
                        != match &baseline.comment {
                            crate::db::catalog::OptionalMetadata::Supported(Some(value)) => {
                                value.as_str()
                            }
                            _ => "",
                        })
                    .then(|| draft.comment.value().to_owned());
                    let safe_additions = added_columns.iter().all(|column| {
                        matches!(
                            column.state,
                            crate::model::catalog_editor::DraftRowState::Added
                        ) && column.nullable
                            && !column.identity
                            && column.default_expression.value().trim().is_empty()
                            && column.generated_expression.value().trim().is_empty()
                            && column.collation.value().trim().is_empty()
                            && column.comment.value().trim().is_empty()
                    });
                    if draft.schema.value().trim() != schema
                        || !safe_additions
                        || !unchanged_owner
                        || (!renamed_columns.is_empty() && draft.name.value().trim() != old_name)
                        || ((renamed_columns.is_empty()
                            && altered_columns.is_empty()
                            && removed_columns.is_empty()
                            && table_comment.is_none()
                            && column_comments.is_empty()
                            && added_columns.is_empty())
                            == (draft.name.value().trim() == old_name))
                    {
                        return Err(CatalogMutationError::UnsupportedOperation {
                            object_type: request.object_type,
                        });
                    }
                    (
                        CatalogKind::Table,
                        draft.name.value().trim().to_owned(),
                        baseline.baseline_fingerprint,
                        None,
                        Some((
                            renamed_columns,
                            altered_columns,
                            removed_columns,
                            table_comment,
                            column_comments,
                            added_columns
                                .iter()
                                .map(|column| {
                                    (
                                        column.name.value().trim().to_owned(),
                                        column.native_type.value().trim().to_owned(),
                                    )
                                })
                                .collect::<Vec<_>>(),
                        )),
                    )
                }
                (
                    CatalogKind::View,
                    Some(CatalogObjectDefinition::View(baseline)),
                    CatalogDraft::View(draft),
                ) => {
                    draft.validate()?;
                    if draft.schema.value().trim() != schema
                        || draft.owner.value() != baseline.owner
                    {
                        return Err(CatalogMutationError::UnsupportedOperation {
                            object_type: request.object_type,
                        });
                    }
                    let query_changed = draft.query.value() != baseline.query;
                    let view_comment = (draft.comment.value().trim()
                        != match &baseline.comment {
                            crate::db::catalog::OptionalMetadata::Supported(Some(value)) => {
                                value.as_str()
                            }
                            _ => "",
                        })
                    .then(|| draft.comment.value().to_owned());
                    let renamed = draft.name.value().trim() != old_name;
                    if query_changed && renamed {
                        return Err(CatalogMutationError::UnsupportedOperation {
                            object_type: request.object_type,
                        });
                    }
                    if !renamed && !query_changed && view_comment.is_none() {
                        return Err(CatalogMutationError::NoChanges);
                    }
                    (
                        CatalogKind::View,
                        draft.name.value().trim().to_owned(),
                        baseline.baseline_fingerprint,
                        Some((
                            query_changed.then(|| draft.query.value().to_owned()),
                            view_comment,
                        )),
                        None,
                    )
                }
                (_, None, _) => return Err(CatalogMutationError::StaleState),
                _ => {
                    return Err(CatalogMutationError::UnsupportedOperation {
                        object_type: request.object_type,
                    });
                }
            };
            let target_name = format!(
                "{}.{}",
                quote_duck_identifier(schema),
                quote_duck_identifier(old_name)
            );
            let statements = if let Some((query, comment)) = view_query_edit {
                let mut statements = Vec::new();
                if let Some(query) = query {
                    statements.push(format!(
                        "CREATE OR REPLACE VIEW {}.{} AS {}",
                        quote_duck_identifier(schema),
                        quote_duck_identifier(&new_name),
                        query
                    ));
                } else if new_name != *old_name {
                    statements.push(format!(
                        "ALTER VIEW {target_name} RENAME TO {}",
                        quote_duck_identifier(&new_name)
                    ));
                }
                if let Some(comment) = comment {
                    statements.push(format!(
                        "COMMENT ON VIEW {}.{} IS {}",
                        quote_duck_identifier(schema),
                        quote_duck_identifier(&new_name),
                        duck_sql_literal_or_null(&comment)
                    ));
                } else if new_name != *old_name {
                    // Preserve the existing view comment after DuckDB rename.
                    // The rename itself carries the metadata, so no extra statement is needed.
                }
                statements
            } else if let Some((renamed, altered, removed, table_comment, column_comments, added)) =
                table_changes
            {
                let mut statements = renamed
                    .into_iter()
                    .map(|(old_name, new_name)| {
                        format!(
                            "ALTER TABLE {target_name} RENAME COLUMN {} TO {}",
                            quote_duck_identifier(&old_name),
                            quote_duck_identifier(&new_name)
                        )
                    })
                    .collect::<Vec<_>>();
                for (column, default, nullable, native_type) in altered {
                    let column = quote_duck_identifier(&column);
                    if let Some(native_type) = native_type {
                        statements.push(format!(
                            "ALTER TABLE {target_name} ALTER COLUMN {column} SET DATA TYPE {native_type}"
                        ));
                    }
                    if let Some(default) = default {
                        statements.push(if default.is_empty() {
                            format!("ALTER TABLE {target_name} ALTER COLUMN {column} DROP DEFAULT")
                        } else {
                            format!("ALTER TABLE {target_name} ALTER COLUMN {column} SET DEFAULT {default}")
                        });
                    }
                    if let Some(nullable) = nullable {
                        statements.push(if nullable {
                            format!("ALTER TABLE {target_name} ALTER COLUMN {column} DROP NOT NULL")
                        } else {
                            format!("ALTER TABLE {target_name} ALTER COLUMN {column} SET NOT NULL")
                        });
                    }
                }
                statements.extend(removed.into_iter().map(|column| {
                    format!(
                        "ALTER TABLE {target_name} DROP COLUMN {}",
                        quote_duck_identifier(&column)
                    )
                }));
                statements.extend(added.into_iter().map(|(column, native_type)| {
                    format!(
                        "ALTER TABLE {target_name} ADD COLUMN {} {}",
                        quote_duck_identifier(&column),
                        native_type
                    )
                }));
                if let Some(comment) = table_comment {
                    statements.push(format!(
                        "COMMENT ON TABLE {target_name} IS {}",
                        duck_sql_literal_or_null(&comment)
                    ));
                }
                for (column, comment) in column_comments {
                    statements.push(format!(
                        "COMMENT ON COLUMN {target_name}.{} IS {}",
                        quote_duck_identifier(&column),
                        duck_sql_literal_or_null(&comment)
                    ));
                }
                if statements.is_empty() && new_name != *old_name {
                    vec![format!(
                        "ALTER TABLE {target_name} RENAME TO {}",
                        quote_duck_identifier(&new_name)
                    )]
                } else {
                    statements
                }
            } else {
                vec![format!(
                    "ALTER {} {target_name} RENAME TO {}",
                    if kind == CatalogKind::Table {
                        "TABLE"
                    } else {
                        "VIEW"
                    },
                    quote_duck_identifier(&new_name)
                )]
            };
            let new_id = CatalogId::new(
                request.connection.profile_id,
                kind,
                [database.as_str(), schema.as_str(), new_name.as_str()],
            );
            let schema_id = CatalogId::new(
                request.connection.profile_id,
                CatalogKind::Schema,
                [database.as_str(), schema.as_str()],
            );
            let impact = CatalogMutationImpact {
                old_object_id: object.clone(),
                owning_relation_id: None,
                namespace: CatalogMutationNamespace {
                    database: Some(CatalogId::new(
                        request.connection.profile_id,
                        CatalogKind::Database,
                        [database.as_str()],
                    )),
                    schema: Some(schema_id.clone()),
                },
                native_identity_changed: new_name != *old_name,
            };
            return CatalogMutationPlan::new(
                request.clone(),
                request.object_type,
                CatalogMutationExecutionMode::Autocommit,
                CatalogMutationTarget::database_target(
                    crate::model::execution_target::ExecutionTarget {
                        profile_id: request.connection.profile_id,
                        database: database.clone(),
                        schema: None,
                    },
                )?,
                vec![CatalogTarget::Groups { schema: schema_id }],
                CatalogSelectionHint::Object(new_id),
                Some(baseline_fingerprint),
                Vec::new(),
                statements,
            )
            .map(|plan| plan.with_impact(impact));
        }
        if request.mode != CatalogMutationMode::Create {
            return Err(CatalogMutationError::UnsupportedOperation {
                object_type: request.object_type,
            });
        }
        if !matches!(
            request.object_type,
            CatalogObjectType::Catalog(
                CatalogKind::Schema
                    | CatalogKind::Table
                    | CatalogKind::View
                    | CatalogKind::Sequence
                    | CatalogKind::Index
            )
        ) {
            return Err(CatalogMutationError::UnsupportedOperation {
                object_type: request.object_type,
            });
        }
        let anchor_schema = match &request.anchor {
            CatalogMutationAnchor::Group { schema, .. } => schema.native_path.get(1).cloned(),
            CatalogMutationAnchor::Catalog(id) if id.kind == CatalogKind::Schema => {
                id.native_path.get(1).cloned()
            }
            _ => None,
        };
        let mut index_relation = None;
        let (schema, name, sql) = match (request.object_type, draft) {
            (CatalogObjectType::Catalog(CatalogKind::Schema), CatalogDraft::Schema(draft)) => {
                if draft.name.value().trim().is_empty() {
                    return Err(CatalogMutationError::InvalidDraft {
                        reason: "schema name is required".into(),
                    });
                }
                let name = draft.name.value().trim().to_owned();
                (
                    None,
                    name.clone(),
                    format!("CREATE SCHEMA {}", quote_duck_identifier(&name)),
                )
            }
            (CatalogObjectType::Catalog(CatalogKind::Table), CatalogDraft::Table(draft)) => {
                if request.mode == CatalogMutationMode::Create {
                    draft.validate()?;
                } else if draft.name.value().trim().is_empty()
                    || draft.schema.value().trim().is_empty()
                {
                    return Err(CatalogMutationError::InvalidDraft {
                        reason: "table name and schema are required".into(),
                    });
                }
                if draft.columns.iter().any(|column| {
                    !column.default_expression.value().trim().is_empty()
                        || !column.collation.value().trim().is_empty()
                        || column.identity
                        || !column.generated_expression.value().trim().is_empty()
                }) || !draft.indexes.is_empty()
                {
                    return Err(CatalogMutationError::UnsupportedOperation {
                        object_type: request.object_type,
                    });
                }
                let schema = if draft.schema.value().trim().is_empty() {
                    anchor_schema
                        .clone()
                        .ok_or(CatalogMutationError::InvalidAnchor {
                            reason: "table create requires a schema anchor",
                        })?
                } else {
                    draft.schema.value().trim().to_owned()
                };
                let name = draft.name.value().trim().to_owned();
                let columns = draft
                    .columns
                    .iter()
                    .map(|column| {
                        format!(
                            "{} {}{}",
                            quote_duck_identifier(column.name.value().trim()),
                            column.native_type.value().trim(),
                            if column.nullable { "" } else { " NOT NULL" }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                if !draft.constraints.is_empty() {
                    return Err(CatalogMutationError::InvalidDraft {
                        reason: "DuckDB structured table creation does not accept opaque constraint strings; use SQL for table constraints".into(),
                    });
                }
                let mut constraint_names = std::collections::HashSet::new();
                if draft.table_constraints.iter().any(|constraint| {
                    let name = constraint.name.value().trim();
                    !name.is_empty() && !constraint_names.insert(name.to_owned())
                }) {
                    return Err(CatalogMutationError::InvalidDraft {
                        reason: "table constraint names must be unique".into(),
                    });
                }
                let active_columns = draft
                    .columns
                    .iter()
                    .filter(|column| {
                        !matches!(
                            column.state,
                            crate::model::catalog_editor::DraftRowState::Removed { .. }
                        )
                    })
                    .map(|column| column.name.value().trim().to_owned())
                    .collect::<std::collections::HashSet<_>>();
                let mut constraint_sql = Vec::new();
                for constraint in &draft.table_constraints {
                    constraint.validate()?;
                    if constraint.deferrable
                        || constraint.initially_deferred
                        || constraint.not_valid
                    {
                        return Err(CatalogMutationError::InvalidDraft { reason: "DuckDB table constraints do not support DEFERRABLE, INITIALLY DEFERRED, or NOT VALID".into() });
                    }
                    let name = constraint.name.value().trim();
                    if !constraint.match_type.value().eq_ignore_ascii_case("SIMPLE")
                        || !constraint
                            .on_update
                            .value()
                            .eq_ignore_ascii_case("NO ACTION")
                        || !constraint
                            .on_delete
                            .value()
                            .eq_ignore_ascii_case("NO ACTION")
                    {
                        return Err(CatalogMutationError::InvalidDraft { reason: "DuckDB constraint metadata cannot round-trip non-default MATCH or referential actions".into() });
                    }
                    let named = if name.is_empty() {
                        String::new()
                    } else {
                        format!("CONSTRAINT {} ", quote_duck_identifier(name))
                    };
                    let clause = match &constraint.kind {
                        crate::db::catalog_mutation::ConstraintDefinitionKind::PrimaryKey {
                            ..
                        }
                        | crate::db::catalog_mutation::ConstraintDefinitionKind::Unique {
                            ..
                        } => {
                            let columns = constraint
                                .columns
                                .value()
                                .split(',')
                                .map(str::trim)
                                .filter(|column| !column.is_empty())
                                .collect::<Vec<_>>();
                            if columns.is_empty() {
                                return Err(CatalogMutationError::InvalidDraft {
                                    reason: "primary key and unique constraints require columns"
                                        .into(),
                                });
                            }
                            if columns
                                .iter()
                                .any(|column| !active_columns.contains(*column))
                            {
                                return Err(CatalogMutationError::InvalidDraft {
                                    reason: "constraint references an unknown table column".into(),
                                });
                            }
                            let keyword = if matches!(
                                constraint.kind,
                                crate::db::catalog_mutation::ConstraintDefinitionKind::PrimaryKey { .. }
                            ) {
                                "PRIMARY KEY"
                            } else {
                                "UNIQUE"
                            };
                            format!(
                                "{named}{keyword} ({})",
                                columns
                                    .iter()
                                    .map(|column| quote_duck_identifier(column))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        }
                        crate::db::catalog_mutation::ConstraintDefinitionKind::Check { .. } => {
                            let expression = constraint.expression.value().trim();
                            if expression.contains(';') || expression.is_empty() {
                                return Err(CatalogMutationError::InvalidDraft {
                                    reason:
                                        "check expression must be a non-empty single expression"
                                            .into(),
                                });
                            }
                            format!("{named}CHECK ({expression})")
                        }
                        crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey {
                            ..
                        } => {
                            let columns = constraint
                                .columns
                                .value()
                                .split(',')
                                .map(str::trim)
                                .filter(|column| !column.is_empty())
                                .collect::<Vec<_>>();
                            let referenced_columns = constraint
                                .referenced_columns
                                .value()
                                .split(',')
                                .map(str::trim)
                                .filter(|column| !column.is_empty())
                                .collect::<Vec<_>>();
                            if columns.is_empty() || columns.len() != referenced_columns.len() {
                                return Err(CatalogMutationError::InvalidDraft { reason: "foreign key source and target column counts must be equal and non-empty".into() });
                            }
                            if referenced_columns
                                .iter()
                                .any(|column| column.contains('"') || column.contains(';'))
                            {
                                return Err(CatalogMutationError::InvalidDraft {
                                    reason: "invalid referenced column identifier".into(),
                                });
                            }
                            if columns
                                .iter()
                                .any(|column| !active_columns.contains(*column))
                            {
                                return Err(CatalogMutationError::InvalidDraft {
                                    reason: "foreign key references an unknown local table column"
                                        .into(),
                                });
                            }
                            let target_schema = constraint.referenced_schema.value().trim();
                            let target_relation = constraint.referenced_relation.value().trim();
                            let target = format!(
                                "{}.{}",
                                quote_duck_identifier(target_schema),
                                quote_duck_identifier(target_relation)
                            );
                            let clause = format!(
                                "{named}FOREIGN KEY ({}) REFERENCES {target} ({})",
                                columns
                                    .iter()
                                    .map(|column| quote_duck_identifier(column))
                                    .collect::<Vec<_>>()
                                    .join(", "),
                                referenced_columns
                                    .iter()
                                    .map(|column| quote_duck_identifier(column))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            );
                            for (action, value) in [
                                ("ON UPDATE", constraint.on_update.value()),
                                ("ON DELETE", constraint.on_delete.value()),
                            ] {
                                let normalized = value.trim().to_ascii_uppercase();
                                if normalized != "NO ACTION" {
                                    return Err(CatalogMutationError::InvalidDraft {
                                        reason: format!(
                                            "DuckDB cannot round-trip non-default foreign key {action} metadata"
                                        ),
                                    });
                                }
                            }
                            clause
                        }
                    };
                    constraint_sql.push(clause);
                }
                let definitions = if constraint_sql.is_empty() {
                    columns
                } else {
                    format!("{columns}, {}", constraint_sql.join(", "))
                };
                (
                    Some(schema.clone()),
                    name.clone(),
                    format!(
                        "CREATE TABLE {}.{} ({definitions})",
                        quote_duck_identifier(&schema),
                        quote_duck_identifier(&name)
                    ),
                )
            }
            (CatalogObjectType::Catalog(CatalogKind::View), CatalogDraft::View(draft)) => {
                draft
                    .validate()
                    .map_err(|error| CatalogMutationError::InvalidDraft {
                        reason: error.to_string(),
                    })?;
                let schema = if draft.schema.value().trim().is_empty() {
                    anchor_schema.ok_or(CatalogMutationError::InvalidAnchor {
                        reason: "view create requires a schema anchor",
                    })?
                } else {
                    draft.schema.value().trim().to_owned()
                };
                let name = draft.name.value().trim().to_owned();
                let sql = format!(
                    "CREATE VIEW {}.{} AS {}",
                    quote_duck_identifier(&schema),
                    quote_duck_identifier(&name),
                    draft.query.value().trim()
                );
                (Some(schema), name, sql)
            }
            (CatalogObjectType::Catalog(CatalogKind::Sequence), CatalogDraft::Sequence(draft)) => {
                if request.mode == CatalogMutationMode::Create {
                    draft.validate()?;
                } else if draft.name.value().trim().is_empty()
                    || draft.schema.value().trim().is_empty()
                {
                    return Err(CatalogMutationError::InvalidDraft {
                        reason: "table name and schema are required".into(),
                    });
                }
                let schema = if draft.schema.value().trim().is_empty() {
                    anchor_schema
                        .clone()
                        .ok_or(CatalogMutationError::InvalidAnchor {
                            reason: "sequence create requires a schema anchor",
                        })?
                } else {
                    draft.schema.value().trim().to_owned()
                };
                let name = draft.name.value().trim().to_owned();
                let mut options = Vec::new();
                if !draft.data_type.value().trim().is_empty()
                    && !draft.data_type.value().eq_ignore_ascii_case("BIGINT")
                {
                    return Err(CatalogMutationError::UnsupportedOperation {
                        object_type: request.object_type,
                    });
                }
                if !draft.start_value.value().trim().is_empty() {
                    options.push(format!("START {}", draft.start_value.value().trim()));
                }
                if !draft.increment.value().trim().is_empty() {
                    options.push(format!("INCREMENT {}", draft.increment.value().trim()));
                }
                if let crate::db::catalog_mutation::SequenceBound::Value(value) =
                    draft.min_value.to_bound()
                {
                    options.push(format!("MINVALUE {value}"));
                } else if matches!(
                    draft.min_value.to_bound(),
                    crate::db::catalog_mutation::SequenceBound::NoLimit
                ) {
                    options.push("NO MINVALUE".into());
                }
                if let crate::db::catalog_mutation::SequenceBound::Value(value) =
                    draft.max_value.to_bound()
                {
                    options.push(format!("MAXVALUE {value}"));
                } else if matches!(
                    draft.max_value.to_bound(),
                    crate::db::catalog_mutation::SequenceBound::NoLimit
                ) {
                    options.push("NO MAXVALUE".into());
                }
                if !draft.cache.value().trim().is_empty() {
                    return Err(CatalogMutationError::UnsupportedOperation {
                        object_type: request.object_type,
                    });
                }
                if draft.cycle {
                    options.push("CYCLE".into());
                }
                if !draft.owned_by.value().trim().is_empty() {
                    return Err(CatalogMutationError::UnsupportedOperation {
                        object_type: request.object_type,
                    });
                }
                (
                    Some(schema.clone()),
                    name.clone(),
                    format!(
                        "CREATE SEQUENCE {}.{} {}",
                        quote_duck_identifier(&schema),
                        quote_duck_identifier(&name),
                        options.join(" ")
                    ),
                )
            }
            (CatalogObjectType::Catalog(CatalogKind::Index), CatalogDraft::Index(draft)) => {
                draft.validate()?;
                let schema = draft.schema.value().trim().to_owned();
                let relation = draft.relation.value().trim().to_owned();
                index_relation = Some(relation.clone());
                let name = draft.name.value().trim().to_owned();
                if !draft.access_method.value().eq_ignore_ascii_case("ART")
                    || !draft.include_columns.value().trim().is_empty()
                    || !draft.predicate.value().trim().is_empty()
                    || !draft.tablespace.value().trim().is_empty()
                    || draft.columns.iter().any(|column| {
                        column.descending || column.nulls_first || column.is_expression
                    })
                {
                    return Err(CatalogMutationError::UnsupportedOperation {
                        object_type: request.object_type,
                    });
                }
                let columns = draft
                    .columns
                    .iter()
                    .map(|column| quote_duck_identifier(column.expression.value().trim()))
                    .collect::<Vec<_>>()
                    .join(", ");
                (
                    Some(schema.clone()),
                    name.clone(),
                    format!(
                        "CREATE {}INDEX {} ON {}.{} ({columns})",
                        if draft.unique { "UNIQUE " } else { "" },
                        quote_duck_identifier(&name),
                        quote_duck_identifier(&schema),
                        quote_duck_identifier(&relation)
                    ),
                )
            }
            _ => {
                return Err(CatalogMutationError::UnsupportedOperation {
                    object_type: request.object_type,
                });
            }
        };
        if name.is_empty() {
            return Err(CatalogMutationError::InvalidDraft {
                reason: "object name is required".into(),
            });
        }
        let mut sql = vec![sql];
        sql.extend(table_comments);
        let database = request
            .current_database
            .clone()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "memory".into());
        let object_kind = match request.object_type {
            CatalogObjectType::Catalog(kind) => kind,
            _ => unreachable!(),
        };
        let id = CatalogId::new(
            request.connection.profile_id,
            object_kind,
            if object_kind == CatalogKind::Schema {
                vec![database.as_str(), name.as_str()]
            } else if object_kind == CatalogKind::Index {
                vec![
                    database.as_str(),
                    schema.as_deref().unwrap_or_default(),
                    index_relation.as_deref().unwrap_or_default(),
                    name.as_str(),
                ]
            } else {
                vec![
                    database.as_str(),
                    schema.as_deref().unwrap_or_default(),
                    name.as_str(),
                ]
            },
        );
        let selection = CatalogSelectionHint::Object(id);
        let anchor = match &request.anchor {
            CatalogMutationAnchor::Catalog(id) => Some(id.clone()),
            CatalogMutationAnchor::Group { schema, .. } => Some(schema.clone()),
            _ => None,
        };
        let refresh_schema = anchor.and_then(|id| (id.kind == CatalogKind::Schema).then_some(id));
        let selection = if object_kind == CatalogKind::Schema {
            CatalogSelectionHint::Parent(CatalogTarget::Schemas {
                database: CatalogId::new(
                    request.connection.profile_id,
                    CatalogKind::Database,
                    [database.as_str()],
                ),
            })
        } else {
            selection
        };
        let refresh = match schema {
            Some(schema) => {
                let schema_id = refresh_schema.unwrap_or_else(|| {
                    CatalogId::new(
                        request.connection.profile_id,
                        CatalogKind::Schema,
                        [database.as_str(), schema.as_str()],
                    )
                });
                vec![CatalogTarget::Groups { schema: schema_id }]
            }
            None => {
                let database_id = CatalogId::new(
                    request.connection.profile_id,
                    CatalogKind::Database,
                    [database.as_str()],
                );
                vec![CatalogTarget::Schemas {
                    database: database_id,
                }]
            }
        };
        if let Some(baseline) = baseline {
            return Err(CatalogMutationError::InvalidPlan {
                reason: format!(
                    "DuckDB create must not carry an existing baseline fingerprint ({baseline:?})"
                ),
            });
        }
        let _ = ObjectGroup::Tables;
        CatalogMutationPlan::new(
            request.clone(),
            request.object_type,
            CatalogMutationExecutionMode::Autocommit,
            CatalogMutationTarget::database_target(
                crate::model::execution_target::ExecutionTarget {
                    profile_id: request.connection.profile_id,
                    database,
                    schema: None,
                },
            )
            .map_err(|error| CatalogMutationError::InvalidPlan {
                reason: error.to_string(),
            })?,
            refresh,
            selection,
            None,
            Vec::new(),
            sql,
        )
    }

    pub async fn connect(profile: &ConnectionProfile) -> Result<Self, DatabaseError> {
        if profile.kind != DatabaseKind::DuckDb {
            return Err(DatabaseError::configuration("profile is not DuckDB"));
        }
        crate::db::duckdb_runtime::ensure_loaded()?;
        let path = profile.duckdb_path.clone();
        let read_only = profile.read_only;
        let connection = task::spawn_blocking(move || {
            let mut config = Config::default();
            if read_only {
                config = config
                    .access_mode(AccessMode::ReadOnly)
                    .map_err(duck_error)?;
            }
            match path {
                Some(path) => {
                    if read_only && !path.is_file() {
                        return Err(DatabaseError::configuration(
                            "read-only DuckDB file does not exist",
                        ));
                    }
                    if !read_only
                        && let Some(parent) = path
                            .parent()
                            .filter(|parent| !parent.as_os_str().is_empty())
                    {
                        std::fs::create_dir_all(parent).map_err(|error| {
                            DatabaseError::configuration(format!(
                                "unable to create DuckDB parent directory: {error}"
                            ))
                        })?;
                    }
                    Connection::open_with_flags(&path, config)
                }
                None => Connection::open_in_memory_with_flags(config),
            }
            .map_err(duck_error)
        })
        .await
        .map_err(|error| DatabaseError::internal(format!("DuckDB worker failed: {error}")))??;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            operation_gate: Arc::new(tokio::sync::Semaphore::new(1)),
            path: profile.duckdb_path.clone(),
            read_only,
            database: profile.database.clone().unwrap_or_else(|| "memory".into()),
            profile_id: profile.id,
            catalog_scope: profile.catalog_scope.clone(),
        })
    }

    pub fn catalog_capabilities() -> crate::db::catalog::CatalogCapabilities {
        crate::db::catalog::CatalogCapabilities {
            namespace_model: crate::db::catalog::NamespaceModel::DatabaseAndSchema,
            top_level_groups: vec![
                crate::db::catalog::ObjectGroup::Tables,
                crate::db::catalog::ObjectGroup::Views,
                crate::db::catalog::ObjectGroup::Sequences,
                crate::db::catalog::ObjectGroup::Types,
                crate::db::catalog::ObjectGroup::Functions,
            ],
            column_metadata: crate::db::catalog::ColumnMetadataCapabilities {
                type_family: true,
                ..Default::default()
            },
            supports_lazy_children: true,
        }
    }

    pub async fn probe(&self) -> Result<ServerInfo, DatabaseError> {
        let connection = self.connection.clone();
        let (version, database) = task::spawn_blocking(move || {
            let connection = connection
                .lock()
                .map_err(|_| DatabaseError::internal("DuckDB connection lock poisoned"))?;
            let version: String = connection
                .query_row("SELECT version()", [], |row| row.get(0))
                .map_err(duck_error)?;
            let database: String = connection
                .query_row("SELECT current_database()", [], |row| row.get(0))
                .map_err(duck_error)?;
            Ok::<_, DatabaseError>((version, database))
        })
        .await
        .map_err(|error| DatabaseError::internal(format!("DuckDB worker failed: {error}")))??;
        Ok(ServerInfo {
            kind: DatabaseKind::DuckDb,
            version,
            database,
            current_user: None,
        })
    }

    pub async fn execute(&self, sql: &str) -> Result<QueryOutcome, DatabaseError> {
        self.execute_with_budget(sql, QueryBudget::UNBOUNDED).await
    }

    pub(crate) async fn execute_with_budget_cancellable(
        &self,
        sql: &str,
        budget: QueryBudget,
        id: uuid::Uuid,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
        on_start: impl FnOnce(DuckDbOperationHandle) + Send + 'static,
    ) -> Result<QueryOutcome, DatabaseError> {
        self.execute_internal(sql, budget, Some((id, cancelled)), Some(Box::new(on_start)))
            .await
    }

    pub(crate) async fn execute_with_budget(
        &self,
        sql: &str,
        budget: QueryBudget,
    ) -> Result<QueryOutcome, DatabaseError> {
        self.execute_internal(sql, budget, None, None).await
    }

    async fn execute_internal(
        &self,
        sql: &str,
        budget: QueryBudget,
        operation: Option<(uuid::Uuid, Arc<std::sync::atomic::AtomicBool>)>,
        on_start: Option<Box<dyn FnOnce(DuckDbOperationHandle) + Send>>,
    ) -> Result<QueryOutcome, DatabaseError> {
        let permit = self
            .operation_gate
            .clone()
            .acquire_owned()
            .await
            .map_err(|error| {
                DatabaseError::internal(format!("DuckDB operation gate closed: {error}"))
            })?;
        let connection = self.connection.clone();
        let interrupt = {
            let connection = connection
                .lock()
                .map_err(|_| DatabaseError::internal("DuckDB connection lock poisoned"))?;
            connection.interrupt_handle()
        };
        let interrupt = operation
            .as_ref()
            .map(|(_, cancelled)| DuckDbOperationHandle {
                interrupt: interrupt.clone(),
                cancelled: Arc::clone(cancelled),
                active: Arc::new(Mutex::new(true)),
            });
        let operation_guard = DuckDbOperationGuard {
            handle: interrupt,
            _permit: permit,
        };
        if let (Some(handle), Some(on_start)) = (operation_guard.handle.as_ref(), on_start) {
            on_start(handle.clone());
        }
        if operation
            .as_ref()
            .is_some_and(|(_, cancelled)| cancelled.load(std::sync::atomic::Ordering::SeqCst))
        {
            return Err(DatabaseError {
                category: crate::db::ErrorCategory::Cancelled,
                code: Some("cancelled".into()),
                message: "DuckDB query cancelled before execution".into(),
                diagnostic: None,
            });
        }
        let cancelled = operation
            .as_ref()
            .map(|(_, cancelled)| Arc::clone(cancelled));
        let sql = sql.to_owned();
        let worker = task::spawn_blocking(move || {
            // Keep cancellation active until native work finishes, even if its async waiter is dropped.
            let _operation_guard = operation_guard;
            let started = Instant::now();
            let connection = connection
                .lock()
                .map_err(|_| DatabaseError::internal("DuckDB connection lock poisoned"))?;
            let mut outcome = QueryOutcomeAccumulator::with_budget(budget);
            for range in scan_statements(&sql, SqlDialect::DuckDb) {
                if cancelled
                    .as_ref()
                    .is_some_and(|cancelled| cancelled.load(std::sync::atomic::Ordering::SeqCst))
                {
                    return Err(DatabaseError {
                        category: crate::db::ErrorCategory::Cancelled,
                        code: Some("cancelled".into()),
                        message: "DuckDB query cancelled before execution".into(),
                        diagnostic: None,
                    });
                }
                let statement = range.get(&sql).ok_or_else(|| {
                    DatabaseError::internal("DuckDB scanner returned an invalid SQL range")
                })?;
                if statement.trim().is_empty() {
                    continue;
                }
                let parsed = sqlparser::parser::Parser::parse_sql(
                    &sqlparser::dialect::DuckDbDialect {},
                    statement,
                )
                .ok()
                .and_then(|statements| statements.into_iter().next());
                let mut output_columns = Vec::new();
                if duckdb_select_query(statement) {
                    let mut describe = connection
                        .prepare(&format!("DESCRIBE {statement}"))
                        .map_err(duck_error)?;
                    let rows = describe
                        .query_map([], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                        })
                        .map_err(duck_error)?;
                    let described = rows
                        .map(|row| row.map_err(duck_error))
                        .collect::<Result<Vec<_>, _>>()?;
                    output_columns = described;
                }
                let time_ns_columns = output_columns
                    .iter()
                    .enumerate()
                    .filter_map(|(index, (_, type_name))| {
                        (type_name.eq_ignore_ascii_case("TIME_NS")
                            || type_name.eq_ignore_ascii_case("Time64(ns)"))
                        .then_some(index)
                    })
                    .collect::<Vec<_>>();
                let execution_sql = if time_ns_columns.is_empty() {
                    statement.to_owned()
                } else {
                    duckdb_time_ns_text_query(statement, &output_columns)?
                };
                let is_dml_returning = parsed.as_ref().is_some_and(|statement| {
                    matches!(
                        statement,
                        sqlparser::ast::Statement::Insert(_)
                            | sqlparser::ast::Statement::Update(_)
                            | sqlparser::ast::Statement::Delete(_)
                    )
                });
                let mut prepared = connection.prepare(&execution_sql).map_err(duck_error)?;
                if cancelled
                    .as_ref()
                    .is_some_and(|cancelled| cancelled.load(std::sync::atomic::Ordering::SeqCst))
                {
                    return Err(DatabaseError {
                        category: crate::db::ErrorCategory::Cancelled,
                        code: Some("cancelled".into()),
                        message: "DuckDB query cancelled before native execution".into(),
                        diagnostic: None,
                    });
                }
                let mut rows = prepared.query([]).map_err(duck_error)?;
                let executed = rows.as_ref().expect("query exposes its executed statement");
                let names = executed.column_names();
                let has_columns = !names.is_empty();
                let mut types = Vec::with_capacity(names.len());
                for index in 0..names.len() {
                    types.push(executed.column_type(index).to_string());
                }
                for index in &time_ns_columns {
                    if let Some(type_name) = types.get_mut(*index) {
                        *type_name = "Time64(ns)".into();
                    }
                }
                let columns = names
                    .into_iter()
                    .zip(types)
                    .map(|(name, type_name)| ColumnMeta { name, type_name })
                    .collect::<Vec<_>>();
                if !has_columns && is_dml_returning {
                    while rows.next().map_err(duck_error)?.is_some() {}
                    outcome.start_result(Vec::new());
                    outcome.done(0);
                    continue;
                }
                outcome.start_result(columns.clone());
                let mut returned_rows = 0_u64;
                while let Some(row) = rows.next().map_err(duck_error)? {
                    let remaining = outcome.budget_remaining();
                    if remaining == (0, 0) {
                        outcome.exhaust_budget();
                        while rows.next().map_err(duck_error)?.is_some() {}
                        break;
                    }
                    let values = (0..columns.len())
                        .map(|index| {
                            if time_ns_columns.contains(&index) {
                                let value =
                                    row.get::<_, Option<String>>(index).map_err(duck_error)?;
                                match value {
                                    Some(value) => Ok(crate::db::value::CellValue::Time(
                                        chrono::NaiveTime::parse_from_str(&value, "%H:%M:%S%.f")
                                            .map_err(|error| {
                                                DatabaseError::internal(format!(
                                                    "invalid DuckDB TIME_NS text: {error}"
                                                ))
                                            })?,
                                    )),
                                    None => Ok(crate::db::value::CellValue::Null),
                                }
                            } else {
                                duck_value(row, index)
                            }
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    outcome.row(columns.clone(), values);
                    returned_rows = returned_rows.saturating_add(1);
                }
                outcome.done(if is_dml_returning { returned_rows } else { 0 });
            }
            let mut outcome = outcome.finish();
            outcome.stats.execution = started.elapsed();
            Ok(outcome)
        });
        worker
            .await
            .map_err(|error| DatabaseError::internal(format!("DuckDB worker failed: {error}")))?
            .map_err(|error| {
                duck_operation_error(
                    error,
                    operation.as_ref().is_some_and(|(_, cancelled)| {
                        cancelled.load(std::sync::atomic::Ordering::SeqCst)
                    }),
                )
            })
    }

    pub async fn close(self) {
        let connection = self.connection;
        let _ = task::spawn_blocking(move || drop(connection)).await;
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }
    pub fn profile_scope(&self) -> &crate::profile::CatalogScope {
        &self.catalog_scope
    }
    pub fn current_database(&self) -> &str {
        &self.database
    }

    pub(crate) async fn transaction_backend(
        &self,
    ) -> Result<DuckDbTransactionBackend, DatabaseError> {
        if self.read_only {
            return Err(DatabaseError::unsupported(
                "DuckDB MANUAL transactions require a writable connection",
            ));
        }
        let operation_permit =
            self.operation_gate
                .clone()
                .acquire_owned()
                .await
                .map_err(|error| {
                    DatabaseError::internal(format!("DuckDB operation gate closed: {error}"))
                })?;
        let connection = self.connection.clone();
        let interrupt = {
            let connection = connection
                .lock()
                .map_err(|_| DatabaseError::internal("DuckDB connection lock poisoned"))?;
            connection.interrupt_handle()
        };
        let mut adapter = self.clone();
        // The transaction backend already owns this session's exclusive permit.
        // Its internal executor must not reacquire the same semaphore.
        adapter.operation_gate = Arc::new(tokio::sync::Semaphore::new(1));
        Ok(DuckDbTransactionBackend {
            connection,
            adapter,
            interrupt,
            depth: 0,
            _operation_permit: operation_permit,
        })
    }

    pub fn database_clone(&self) -> Result<Self, DatabaseError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| DatabaseError::internal("DuckDB connection lock poisoned"))?
            .try_clone()
            .map_err(duck_error)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            operation_gate: Arc::new(tokio::sync::Semaphore::new(1)),
            ..self.clone()
        })
    }

    pub async fn discover_catalog_scope(
        &self,
    ) -> Result<crate::db::catalog::CatalogDiscovery, DatabaseError> {
        let connection = self.connection.clone();
        task::spawn_blocking(move || {
            let connection = connection.lock().map_err(|_| DatabaseError::internal("DuckDB connection lock poisoned"))?;
            let mut databases_statement = connection.prepare("SELECT database_name FROM duckdb_databases() ORDER BY database_name").map_err(duck_error)?;
            let mut databases = databases_statement.query_map([], |row| row.get::<_, String>(0)).map_err(duck_error)?.collect::<Result<Vec<_>, _>>().map_err(duck_error)?;
            // DuckDB exposes the session-local temporary catalog through table
            // functions, but may omit it from duckdb_databases() until a temp
            // object has been created. Advertise the catalog identity in the
            // discovery payload even when this session currently has no TEMP
            // objects.
            if !databases.iter().any(|database| database == "temp") {
                databases.push("temp".to_owned());
                databases.sort();
            }
            let mut discovered = Vec::new();
            for database in databases {
                let mut schemas_statement = connection.prepare("SELECT schema_name FROM duckdb_schemas() WHERE database_name = ? ORDER BY schema_name").map_err(duck_error)?;
                let schemas = schemas_statement.query_map([database.as_str()], |row| row.get::<_, String>(0)).map_err(duck_error)?.collect::<Result<Vec<_>, _>>().map_err(duck_error)?;
                discovered.push(crate::db::catalog::DiscoveredDatabase { name: database, schemas });
            }
            Ok(crate::db::catalog::CatalogDiscovery {
                databases: discovered,
                warnings: Vec::new(),
            })
        }).await.map_err(|error| DatabaseError::internal(format!("DuckDB worker failed: {error}")))?
    }

    pub async fn load_catalog_page(
        &self,
        request: &crate::db::catalog::CatalogRequest,
    ) -> Result<crate::db::catalog::CatalogPage, DatabaseError> {
        use crate::db::catalog::{
            CatalogCount, CatalogEntry, CatalogId, CatalogKind, CatalogMetadata, CatalogTarget,
            ColumnMetadata, ObjectGroup, OptionalMetadata, QualifiedName,
        };
        request
            .validate_for_profile(self.profile_id)
            .map_err(|error| DatabaseError::configuration(error.to_string()))?;
        let request = request.clone();
        let profile_id = self.profile_id;
        let current_database = self.database.clone();
        let profile_scope = self.catalog_scope.clone();
        let connection = self.connection.clone();
        task::spawn_blocking(move || {
            let connection = connection.lock().map_err(|_| DatabaseError::internal("DuckDB connection lock poisoned"))?;
            let attached_databases = {
                let mut statement = connection.prepare("SELECT database_name FROM duckdb_databases() ORDER BY database_name").map_err(duck_error)?;
                statement.query_map([], |row| row.get::<_, String>(0)).map_err(duck_error)?.collect::<Result<Vec<_>, _>>().map_err(duck_error)?
            };
            let scope = intersect_catalog_scopes(&profile_scope, &request.scope)
                .with_attached_databases(&current_database, attached_databases.clone());
            let target_database = match &request.key.target {
                CatalogTarget::Databases => None,
                CatalogTarget::Schemas { database } => database.native_path.first().cloned(),
                CatalogTarget::Groups { schema } | CatalogTarget::Objects { schema, .. } => schema.native_path.first().cloned(),
                CatalogTarget::RelationChildren { relation } => relation.native_path.first().cloned(),
                CatalogTarget::ProjectionChildren { table } => table.native_path.first().cloned(),
            };
            if target_database.as_deref().is_some_and(|database| !scope.allows_database(database)) {
                return Err(DatabaseError::configuration("DuckDB catalog target is outside profile scope"));
            }
            let database = target_database.unwrap_or_else(|| current_database.clone());
            let database_exists = attached_databases.contains(&database) || database == "temp";
            if !database_exists {
                return Err(DatabaseError::configuration("DuckDB catalog target database is not attached"));
            }
            let (entries, count) = match &request.key.target {
                CatalogTarget::Databases => {
                    let mut entries = Vec::new();
                    for name in attached_databases.iter().cloned() {
                        if !scope.allows_database(&name) { continue; }
                        let id = CatalogId::new(profile_id, CatalogKind::Database, [name.as_str()]);
                        entries.push(CatalogEntry::database(id, QualifiedName { database: Some(name.clone()), schema: None, object: name }, "database", OptionalMetadata::Unsupported, true).map_err(|error| DatabaseError::internal(error.to_string()))?);
                    }
                    let count = entries.len() as u64;
                    (entries, count)
                }
                CatalogTarget::Schemas { database: database_id } => {
                    let mut statement = connection.prepare("SELECT schema_name FROM duckdb_schemas() WHERE database_name = ? ORDER BY schema_name").map_err(duck_error)?;
                    let names = statement.query_map([database.as_str()], |row| row.get::<_, String>(0)).map_err(duck_error)?.collect::<Result<Vec<_>, _>>().map_err(duck_error)?;
                    let entries = names.into_iter().map(|name| {
                        let id = CatalogId::new(profile_id, CatalogKind::Schema, [database.as_str(), name.as_str()]);
                        CatalogEntry::schema(id, database_id.clone(), QualifiedName { database: Some(database.clone()), schema: Some(name.clone()), object: name }, "schema", OptionalMetadata::Unsupported, true).map_err(|error| DatabaseError::internal(error.to_string()))
                    }).collect::<Result<Vec<_>, _>>()?;
                    let count = entries.len() as u64;
                    (entries, count)
                }
                CatalogTarget::Objects { schema, group } if matches!(group, ObjectGroup::Tables | ObjectGroup::Views) => {
                    let schema_name = schema.native_path.get(1).ok_or_else(|| DatabaseError::configuration("invalid DuckDB schema path"))?;
                    let (sql, kind, native_kind) = if *group == ObjectGroup::Views { ("SELECT view_name FROM duckdb_views() WHERE database_name = ? AND schema_name = ? AND NOT internal ORDER BY view_name", CatalogKind::View, "VIEW") } else { ("SELECT table_name FROM duckdb_tables() WHERE database_name = ? AND schema_name = ? AND NOT internal ORDER BY table_name", CatalogKind::Table, "BASE TABLE") };
                    let mut statement = connection.prepare(sql).map_err(duck_error)?;
                    let names = statement.query_map([database.as_str(), schema_name.as_str()], |row| row.get::<_, String>(0)).map_err(duck_error)?.collect::<Result<Vec<_>, _>>().map_err(duck_error)?;
                    let entries = names.into_iter().map(|name| {
                        let id = CatalogId::new(profile_id, kind, [database.as_str(), schema_name.as_str(), name.as_str()]);
                        if kind.is_relation() {
                            CatalogEntry::relation(id, schema.clone(), QualifiedName { database: Some(database.clone()), schema: Some(schema_name.clone()), object: name }, native_kind, OptionalMetadata::Unsupported, true).map_err(|error| DatabaseError::internal(error.to_string()))
                        } else {
                            CatalogEntry::object(id, schema.clone(), QualifiedName { database: Some(database.clone()), schema: Some(schema_name.clone()), object: name }, native_kind, OptionalMetadata::Unsupported, false).map_err(|error| DatabaseError::internal(error.to_string()))
                        }
                    }).collect::<Result<Vec<_>, _>>()?;
                    let count = entries.len() as u64;
                    (entries, count)
                }
                CatalogTarget::Objects { schema, group } => {
                    let schema_name = schema.native_path.get(1).ok_or_else(|| DatabaseError::configuration("invalid DuckDB schema path"))?;
                    let (sql, kind, native_kind) = match group {
                        ObjectGroup::Sequences => ("SELECT sequence_name FROM duckdb_sequences() WHERE database_name = ? AND schema_name = ? ORDER BY sequence_name", CatalogKind::Sequence, "SEQUENCE"),
                        ObjectGroup::Types => ("SELECT type_name FROM duckdb_types() WHERE database_name = ? AND schema_name = ? ORDER BY type_name", CatalogKind::Type, "TYPE"),
                        ObjectGroup::Functions => ("SELECT function_name FROM duckdb_functions() WHERE database_name = ? AND schema_name = ? AND function_type = 'macro' ORDER BY function_name", CatalogKind::Function, "MACRO"),
                        _ => return Err(DatabaseError::unsupported(format!("DuckDB catalog group {group:?} is not available"))),
                    };
                    let mut statement = connection.prepare(sql).map_err(duck_error)?;
                    let names = statement.query_map([database.as_str(), schema_name.as_str()], |row| row.get::<_, String>(0)).map_err(duck_error)?.collect::<Result<Vec<_>, _>>().map_err(duck_error)?;
                    let entries = names.into_iter().map(|name| {
                        let id = CatalogId::new(profile_id, kind, [database.as_str(), schema_name.as_str(), name.as_str()]);
                        CatalogEntry::object(id, schema.clone(), QualifiedName { database: Some(database.clone()), schema: Some(schema_name.clone()), object: name }, native_kind, OptionalMetadata::Unsupported, false).map_err(|error| DatabaseError::internal(error.to_string()))
                    }).collect::<Result<Vec<_>, _>>()?;
                    let count = entries.len() as u64;
                    (entries, count)
                }
                CatalogTarget::ProjectionChildren { .. } => {
                    return Err(DatabaseError::unsupported("DuckDB projection children are not available"));
                }
                CatalogTarget::RelationChildren { relation } => {
                    let db_name = relation.native_path[0].clone();
                    let schema_name = relation.native_path[1].clone();
                    let table_name = relation.native_path[2].clone();
                    let mut primary_keys = connection.prepare("SELECT unnest(constraint_column_names) FROM duckdb_constraints() WHERE database_name = ? AND schema_name = ? AND table_name = ? AND constraint_type = 'PRIMARY KEY'").map_err(duck_error)?;
                    let primary_key_columns = primary_keys.query_map([db_name.as_str(), schema_name.as_str(), table_name.as_str()], |row| row.get::<_, String>(0)).map_err(duck_error)?.collect::<Result<Vec<_>, _>>().map_err(duck_error)?;
                    drop(primary_keys);
                    let mut indexes = connection.prepare("SELECT index_name, is_unique, expressions FROM duckdb_indexes() WHERE database_name = ? AND schema_name = ? AND table_name = ? ORDER BY index_name").map_err(duck_error)?;
                    let index_rows = indexes.query_map([db_name.as_str(), schema_name.as_str(), table_name.as_str()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?, row.get::<_, String>(2)?))).map_err(duck_error)?.collect::<Result<Vec<_>, _>>().map_err(duck_error)?;
                    let mut statement = connection.prepare("SELECT column_name, data_type, is_nullable FROM duckdb_columns() WHERE database_name = ? AND schema_name = ? AND table_name = ? ORDER BY column_index").map_err(duck_error)?;
                    let columns = statement.query_map([db_name.as_str(), schema_name.as_str(), table_name.as_str()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, bool>(2)?))).map_err(duck_error)?.collect::<Result<Vec<_>, _>>().map_err(duck_error)?;
                    let entries = columns.into_iter().enumerate().map(|(index, (name, native_type, nullable))| {
                        let ordinal = index as u32 + 1;
                        let id = CatalogId::new(profile_id, CatalogKind::Column, [db_name.as_str(), schema_name.as_str(), table_name.as_str(), name.as_str()]);
                        let mut metadata = ColumnMetadata::new(ordinal, native_type.clone(), nullable);
                        metadata.type_family = OptionalMetadata::Supported(Some(native_type));
                        let qualified = QualifiedName { database: Some(db_name.clone()), schema: Some(schema_name.clone()), object: name };
                        CatalogEntry::relation_child(id, relation.clone(), qualified, "COLUMN", OptionalMetadata::Unsupported, CatalogMetadata::Column(metadata)).map_err(|error| DatabaseError::internal(error.to_string()))
                    }).collect::<Result<Vec<_>, _>>()?;
                    let entries = entries.into_iter().map(|mut entry| {
                        if primary_key_columns.contains(&entry.qualified_name.object)
                            && let CatalogMetadata::Column(ref mut metadata) = entry.metadata
                        {
                            metadata.constraint_memberships.push(crate::db::catalog::ConstraintMembership {
                                constraint_id: CatalogId::new(profile_id, CatalogKind::PrimaryKey, [db_name.as_str(), schema_name.as_str(), table_name.as_str(), "PRIMARY KEY"]),
                                ordinal_position: primary_key_columns.iter().position(|name| name == &entry.qualified_name.object).unwrap_or_default() as u32 + 1,
                            });
                        }
                        entry
                    }).collect::<Vec<_>>();
                    let mut entries = entries;
                    for (index_name, unique, expressions) in index_rows {
                        let expression = expressions.trim();
                        let column = expression
                            .strip_prefix('[')
                            .and_then(|value| value.strip_suffix(']'))
                            .filter(|value| !value.contains(','))
                            .map(|value| value.trim().trim_matches('"').to_owned());
                        let columns = column.into_iter().collect();
                        let id = CatalogId::new(profile_id, CatalogKind::Index, [db_name.as_str(), schema_name.as_str(), table_name.as_str(), index_name.as_str()]);
                        let index = CatalogEntry::relation_child(id, relation.clone(), QualifiedName { database: Some(db_name.clone()), schema: Some(schema_name.clone()), object: index_name }, "INDEX", OptionalMetadata::Unsupported, CatalogMetadata::Index(crate::db::catalog::IndexMetadata { columns, unique })).map_err(|error| DatabaseError::internal(error.to_string()))?;
                        entries.push(index);
                    }
                    let count = entries.len() as u64;
                    (entries, count)
                }
                CatalogTarget::Groups { schema } => {
                    let mut summaries = Vec::new();
                    for group in [ObjectGroup::Tables, ObjectGroup::Views, ObjectGroup::Sequences, ObjectGroup::Types, ObjectGroup::Functions] {
                        let sql = match group {
                            ObjectGroup::Tables => "SELECT count(*) FROM duckdb_tables() WHERE database_name = ? AND schema_name = ? AND NOT internal",
                            ObjectGroup::Views => "SELECT count(*) FROM duckdb_views() WHERE database_name = ? AND schema_name = ? AND NOT internal",
                            ObjectGroup::Sequences => "SELECT count(*) FROM duckdb_sequences() WHERE database_name = ? AND schema_name = ?",
                            ObjectGroup::Types => "SELECT count(*) FROM duckdb_types() WHERE database_name = ? AND schema_name = ?",
                            ObjectGroup::Functions => "SELECT count(*) FROM duckdb_functions() WHERE database_name = ? AND schema_name = ? AND function_type = 'macro'",
                            _ => unreachable!(),
                        };
                        let schema_name = schema.native_path.get(1).ok_or_else(|| DatabaseError::configuration("invalid DuckDB schema path"))?;
                        let value: i64 = connection.query_row(sql, [database.as_str(), schema_name.as_str()], |row| row.get(0)).map_err(duck_error)?;
                        summaries.push(crate::db::catalog::CatalogGroupSummary { group, object_count: CatalogCount::Exact(value as u64) });
                    }
                    return crate::db::catalog::CatalogPage::groups(&request, summaries, CatalogCount::Exact(5), None).map_err(|error| DatabaseError::internal(error.to_string()));
                }
            };
            let mut entries = entries.into_iter().filter(|entry| match &request.key.target {
                CatalogTarget::Databases => scope.allows_database(&entry.qualified_name.object),
                CatalogTarget::Schemas { .. } | CatalogTarget::Groups { .. } | CatalogTarget::Objects { .. } | CatalogTarget::RelationChildren { .. } | CatalogTarget::ProjectionChildren { .. } => entry.qualified_name.database.as_deref().is_some_and(|db| {
                    entry.qualified_name.schema.as_deref().is_none_or(|schema| scope.allows_schema(db, schema))
                }),
            }).collect::<Vec<_>>();
            let total = entries.len() as u64;
            entries.sort_by(|left, right| left.qualified_name.object.to_lowercase().cmp(&right.qualified_name.object.to_lowercase()).then_with(|| left.qualified_name.object.cmp(&right.qualified_name.object)));
            if let Some(cursor) = request.key.cursor.as_ref() {
                let (sort_key, tie_breaker) = cursor.keyset_parts().map_err(|error| DatabaseError::configuration(error.to_string()))?;
                entries.retain(|entry| (entry.qualified_name.object.to_lowercase().as_str(), entry.qualified_name.object.as_str()) > (sort_key, tie_breaker));
            }
            let next_cursor = crate::db::catalog::finalize_keyset_page(&mut entries, request.page_size, |entry| entry.qualified_name.object.to_lowercase(), |entry| entry.qualified_name.object.clone()).map_err(|error| DatabaseError::configuration(error.to_string()))?;
            let count = if total == count { CatalogCount::Exact(count) } else { CatalogCount::Exact(total) };
            let page = crate::db::catalog::CatalogPage::new(&request, entries, count, next_cursor).map_err(|error| DatabaseError::internal(error.to_string()))?;
            let _ = CatalogMetadata::None;
            Ok(page)
        }).await.map_err(|error| DatabaseError::internal(format!("DuckDB worker failed: {error}")))?
    }

    pub async fn search_catalog(
        &self,
        request: &crate::db::catalog::CatalogSearchRequest,
    ) -> Result<crate::db::catalog::CatalogSearchPage, DatabaseError> {
        use crate::db::catalog::{
            CatalogEntry, CatalogId, CatalogKind, CatalogSearchHit, CatalogSearchPage,
            OptionalMetadata, QualifiedName,
        };
        request
            .validate()
            .map_err(|error| DatabaseError::configuration(error.to_string()))?;
        if request.connection.profile_id != self.profile_id {
            return Err(DatabaseError::configuration(
                "catalog search profile does not match DuckDB connection",
            ));
        }
        let request = request.clone();
        let current_database = self.database.clone();
        let profile_id = self.profile_id;
        let profile_scope = self.catalog_scope.clone();
        let connection = self.connection.clone();
        task::spawn_blocking(move || {
            let connection = connection.lock().map_err(|_| DatabaseError::internal("DuckDB connection lock poisoned"))?;
            let mut hits = Vec::new();
            let needle = crate::db::catalog::normalize_search_text(&request.query);
            let mut databases_statement = connection.prepare("SELECT database_name FROM duckdb_databases() ORDER BY database_name").map_err(duck_error)?;
            let mut databases = databases_statement.query_map([], |row| row.get::<_, String>(0)).map_err(duck_error)?.collect::<Result<Vec<_>, _>>().map_err(duck_error)?;
            if !databases.iter().any(|database| database == "temp") {
                databases.push("temp".to_owned());
                databases.sort();
            }
            let scope = intersect_catalog_scopes(&profile_scope, &request.scope)
                .with_attached_databases(&current_database, databases.iter().cloned());
            for database in databases.into_iter().filter(|database| scope.allows_database(database)) {
                let database_id = CatalogId::new(profile_id, CatalogKind::Database, [database.as_str()]);
                let database_entry = CatalogEntry::database(database_id.clone(), QualifiedName { database: Some(database.clone()), schema: None, object: database.clone() }, "database", OptionalMetadata::Unsupported, true).map_err(|error| DatabaseError::internal(error.to_string()))?;
                let mut schemas_statement = connection.prepare("SELECT schema_name FROM duckdb_schemas() WHERE database_name = ? ORDER BY schema_name").map_err(duck_error)?;
                let schemas = schemas_statement.query_map([database.as_str()], |row| row.get::<_, String>(0)).map_err(duck_error)?.collect::<Result<Vec<_>, _>>().map_err(duck_error)?;
                for schema_name in schemas.into_iter().filter(|name| scope.allows_schema(&database, name)) {
                let schema_id = CatalogId::new(profile_id, CatalogKind::Schema, [database.as_str(), schema_name.as_str()]);
                let schema_entry = CatalogEntry::schema(schema_id.clone(), database_id.clone(), QualifiedName { database: Some(database.clone()), schema: Some(schema_name.clone()), object: schema_name.clone() }, "schema", OptionalMetadata::Unsupported, true).map_err(|error| DatabaseError::internal(error.to_string()))?;
                if crate::db::catalog::normalize_search_text(&schema_name).contains(&needle) {
                    hits.push(CatalogSearchHit { entry: schema_entry.clone(), ancestors: vec![database_entry.clone()] });
                }
                for (sql, kind, native_kind) in [
                    ("SELECT table_name FROM duckdb_tables() WHERE database_name = ? AND schema_name = ? AND NOT internal ORDER BY table_name", CatalogKind::Table, "BASE TABLE"),
                    ("SELECT view_name FROM duckdb_views() WHERE database_name = ? AND schema_name = ? AND NOT internal ORDER BY view_name", CatalogKind::View, "VIEW"),
                    ("SELECT sequence_name FROM duckdb_sequences() WHERE database_name = ? AND schema_name = ? ORDER BY sequence_name", CatalogKind::Sequence, "SEQUENCE"),
                    ("SELECT type_name FROM duckdb_types() WHERE database_name = ? AND schema_name = ? ORDER BY type_name", CatalogKind::Type, "TYPE"),
                    ("SELECT function_name FROM duckdb_functions() WHERE database_name = ? AND schema_name = ? AND function_type = 'macro' ORDER BY function_name", CatalogKind::Function, "MACRO"),
                ] {
                    let mut statement = connection.prepare(sql).map_err(duck_error)?;
                    let names = statement.query_map([database.as_str(), schema_name.as_str()], |row| row.get::<_, String>(0)).map_err(duck_error)?.collect::<Result<Vec<_>, _>>().map_err(duck_error)?;
                    for name in names {
                        if !request.object_scope.includes(kind) || !crate::db::catalog::normalize_search_text(&name).contains(&needle) { continue; }
                        let id = CatalogId::new(profile_id, kind, [database.as_str(), schema_name.as_str(), name.as_str()]);
                        let qualified_name = QualifiedName { database: Some(database.clone()), schema: Some(schema_name.clone()), object: name };
                        let entry = if kind.is_relation() {
                            CatalogEntry::relation(id, schema_id.clone(), qualified_name, native_kind, OptionalMetadata::Unsupported, true)
                        } else {
                            CatalogEntry::object(id, schema_id.clone(), qualified_name, native_kind, OptionalMetadata::Unsupported, false)
                        }.map_err(|error| DatabaseError::internal(error.to_string()))?;
                        hits.push(CatalogSearchHit { entry, ancestors: vec![database_entry.clone(), schema_entry.clone()] });
                    }
                }
                }
            }
            hits.sort_by(|left, right| left.entry.qualified_name.object.to_lowercase().cmp(&right.entry.qualified_name.object.to_lowercase()).then_with(|| left.entry.id.native_path.cmp(&right.entry.id.native_path)));
            let total = hits.len();
            let truncated = total > request.limit;
            hits.truncate(request.limit);
            CatalogSearchPage::new(&request, hits, Some(total), truncated).map_err(|error| DatabaseError::internal(error.to_string()))
        }).await.map_err(|error| DatabaseError::internal(format!("DuckDB worker failed: {error}")))?
    }

    pub async fn preview_relation(
        &self,
        relation: &crate::db::catalog::CatalogId,
        scope: &crate::profile::CatalogScope,
        options: &crate::model::relation::RelationPreviewOptions,
        page: crate::model::pagination::PageRequest,
    ) -> Result<crate::db::RelationPreview, DatabaseError> {
        use crate::db::query::ResultSet;
        use crate::model::pagination::{ResultPagination, TotalRows};
        if relation.profile_id() != self.profile_id || !relation.kind.is_relation() {
            return Err(DatabaseError::configuration(
                "relation identity does not belong to this DuckDB profile",
            ));
        }
        let [database, schema, object, ..] = relation.native_path.as_slice() else {
            return Err(DatabaseError::configuration(
                "DuckDB relation identity is incomplete",
            ));
        };
        if !scope.allows_schema(database, schema) {
            return Err(DatabaseError::unsupported(
                "DuckDB relation is outside the selected catalog scope",
            ));
        }
        let quote = |value: &str| format!("\"{}\"", value.replace('"', "\"\""));
        let mut relation_sql = format!("{}.{}.{}", quote(database), quote(schema), quote(object));
        if self.path.is_none() {
            relation_sql = format!("{}.{}", quote(schema), quote(object));
        }
        let mut base_sql = format!("SELECT * FROM {relation_sql}");
        if let Some(where_clause) = &options.where_clause {
            if where_clause.trim().is_empty() || where_clause.contains(';') {
                return Err(DatabaseError::configuration(
                    "invalid DuckDB preview WHERE clause",
                ));
            }
            base_sql.push_str(" WHERE ");
            base_sql.push_str(where_clause);
        }
        if let Some(order_by) = &options.order_by_clause {
            if order_by.trim().is_empty() || order_by.contains(';') {
                return Err(DatabaseError::configuration(
                    "invalid DuckDB preview ORDER BY clause",
                ));
            }
            base_sql.push_str(" ORDER BY ");
            base_sql.push_str(order_by);
        }
        let connection = self.connection.clone();
        let sql_base = base_sql.clone();
        let execution_base_sql = sql_base.clone();
        let offset = page.offset;
        let limit = page.size.lookahead_limit();
        let resolve_total = page.resolve_total;
        let (columns, mut rows, total) = task::spawn_blocking(move || {
            let connection = connection
                .lock()
                .map_err(|_| DatabaseError::internal("DuckDB connection lock poisoned"))?;
            let total = if resolve_total {
                Some(
                    connection
                        .query_row(
                            &format!(
                                "SELECT count(*) FROM ({execution_base_sql}) AS __lazydb_count"
                            ),
                            [],
                            |row| row.get::<_, i64>(0),
                        )
                        .map_err(duck_error)? as u64,
                )
            } else {
                None
            };
            let sql = format!("{execution_base_sql} LIMIT {limit} OFFSET {offset}");
            let mut statement = connection.prepare(&sql).map_err(duck_error)?;
            let mut rows = statement.query([]).map_err(duck_error)?;
            let executed = rows.as_ref().expect("DuckDB query returns executed rows");
            let names = executed.column_names();
            let types = (0..names.len())
                .map(|index| executed.column_type(index).to_string())
                .collect::<Vec<_>>();
            let columns = names
                .into_iter()
                .zip(types)
                .map(|(name, type_name)| ColumnMeta { name, type_name })
                .collect::<Vec<_>>();
            let mut values = Vec::new();
            while let Some(row) = rows.next().map_err(duck_error)? {
                values.push(
                    (0..columns.len())
                        .map(|index| duck_value(row, index))
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
            Ok::<_, DatabaseError>((columns, values, total))
        })
        .await
        .map_err(|error| DatabaseError::internal(format!("DuckDB worker failed: {error}")))??;
        let fetched = rows.len();
        let has_next = fetched > page.size.get();
        rows.truncate(page.size.get());
        let visible = rows.len();
        let pagination = ResultPagination {
            page_size: page.size,
            offset,
            visible_rows: visible,
            has_next,
            total: total.map(TotalRows::Exact).unwrap_or_else(|| {
                if has_next {
                    TotalRows::LowerBound(offset + page.size.get() as u64 + 1)
                } else {
                    TotalRows::Exact(offset + visible as u64)
                }
            }),
        };
        let result = QueryOutcome::from_result_set(
            ResultSet {
                columns,
                rows,
                affected_rows: 0,
            },
            std::time::Duration::ZERO,
            std::time::Duration::ZERO,
        );
        Ok(crate::db::RelationPreview {
            sql: format!(
                "{base_sql} LIMIT {} OFFSET {}",
                page.size.lookahead_limit(),
                page.offset
            ),
            result,
            pagination,
            row_versions: None,
        })
    }

    pub async fn relation_ddl_with_scope(
        &self,
        relation: &crate::db::catalog::CatalogId,
        scope: &crate::profile::CatalogScope,
    ) -> Result<crate::db::catalog::RelationDdl, DatabaseError> {
        use crate::db::catalog::{
            CatalogEntry, CatalogId, CatalogKind, CatalogTarget, DdlProvenance, OptionalMetadata,
            QualifiedName,
        };
        if relation.profile_id() != self.profile_id || !relation.kind.is_relation() {
            return Err(DatabaseError::configuration(
                "relation identity does not belong to this DuckDB profile",
            ));
        }
        let [database, schema, name, ..] = relation.native_path.as_slice() else {
            return Err(DatabaseError::configuration(
                "DuckDB relation identity is incomplete",
            ));
        };
        if !scope.allows_schema(database, schema) {
            return Err(DatabaseError::unsupported(
                "DuckDB relation is outside the selected catalog scope",
            ));
        }
        let relation = relation.clone();
        let database = database.clone();
        let schema = schema.clone();
        let name = name.clone();
        let connection = self.connection.clone();
        let profile_id = self.profile_id;
        let (entry, sql) = task::spawn_blocking(move || {
            let connection = connection.lock().map_err(|_| DatabaseError::internal("DuckDB connection lock poisoned"))?;
            let (query, column, native_kind) = if relation.kind == CatalogKind::View {
                ("SELECT sql FROM duckdb_views() WHERE database_name = ? AND schema_name = ? AND view_name = ?", "view_name", "VIEW")
            } else {
                ("SELECT sql FROM duckdb_tables() WHERE database_name = ? AND schema_name = ? AND table_name = ?", "table_name", "BASE TABLE")
            };
            let _ = column;
            let sql = connection.query_row(query, [database.as_str(), schema.as_str(), name.as_str()], |row| row.get::<_, String>(0)).map_err(duck_error)?;
            let schema_id = CatalogId::new(profile_id, CatalogKind::Schema, [database.as_str(), schema.as_str()]);
            let entry = CatalogEntry::relation(relation, schema_id, QualifiedName { database: Some(database), schema: Some(schema), object: name }, native_kind, OptionalMetadata::Unsupported, true).map_err(|error| DatabaseError::internal(error.to_string()))?;
            Ok::<_, DatabaseError>((entry, sql))
        }).await.map_err(|error| DatabaseError::internal(format!("DuckDB worker failed: {error}")))??;
        let children = self
            .load_catalog_page(&crate::db::catalog::CatalogRequest {
                key: crate::db::catalog::CatalogRequestKey {
                    connection: crate::identity::ConnectionIdentity {
                        profile_id: self.profile_id,
                        generation: 0,
                    },
                    catalog_epoch: 0,
                    request_id: 1,
                    target: CatalogTarget::RelationChildren {
                        relation: entry.id.clone(),
                    },
                    cursor: None,
                },
                scope: scope.clone(),
                page_size: 500,
            })
            .await?;
        Ok(crate::db::catalog::RelationDdl {
            relation: entry,
            children,
            sql,
            provenance: DdlProvenance::NativeCatalog,
        })
    }

    pub async fn load_catalog_object_definition(
        &self,
        request: &crate::db::catalog_mutation::CatalogObjectDefinitionRequest,
    ) -> Result<crate::db::catalog_mutation::CatalogObjectDefinition, DatabaseError> {
        use crate::db::catalog::{CatalogKind, OptionalMetadata};
        use crate::db::catalog_mutation::{
            CatalogObjectDefinition, ColumnDefinition, TableDefinition, ViewDefinition, ViewOption,
        };
        request
            .validate()
            .map_err(|error| DatabaseError::configuration(error.to_string()))?;
        if request.connection.profile_id != self.profile_id
            || request.object.profile_id() != self.profile_id
        {
            return Err(DatabaseError::configuration(
                "DuckDB object definition request profile mismatch",
            ));
        }
        let (database, schema, name, index_relation) = if request.object.kind == CatalogKind::Index
        {
            let [database, schema, relation, index, ..] = request.object.native_path.as_slice()
            else {
                return Err(DatabaseError::configuration(
                    "DuckDB index identity is incomplete",
                ));
            };
            (
                database.clone(),
                schema.clone(),
                index.clone(),
                Some(relation.clone()),
            )
        } else {
            let [database, schema, name, ..] = request.object.native_path.as_slice() else {
                return Err(DatabaseError::configuration(
                    "DuckDB object identity is incomplete",
                ));
            };
            (database.clone(), schema.clone(), name.clone(), None)
        };
        if !self.catalog_scope.allows_schema(&database, &schema) {
            return Err(DatabaseError::configuration(
                "DuckDB object definition request is outside profile catalog scope",
            ));
        }
        let database = database.clone();
        let schema = schema.clone();
        let name = name.clone();
        let kind = request.object.kind;
        let connection = self.connection.clone();
        task::spawn_blocking(move || {
            let connection = connection.lock().map_err(|_| DatabaseError::internal("DuckDB connection lock poisoned"))?;
            if kind == CatalogKind::Index {
                let relation = index_relation.as_deref().ok_or_else(|| DatabaseError::configuration("DuckDB index relation identity is incomplete"))?;
                let row = connection.query_row(
                    "SELECT database_name, schema_name, table_name, is_unique, expressions FROM duckdb_indexes() WHERE index_name = ?",
                    [name.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, bool>(3)?, row.get::<_, String>(4)?)),
                ).map_err(duck_error)?;
                if row.0 != database || row.1 != schema || row.2 != relation {
                    return Err(DatabaseError::configuration("DuckDB index identity points to another relation"));
                }
                let expressions = row.4.trim().trim_start_matches('[').trim_end_matches(']');
                let columns = expressions.split(',').map(|expression| crate::db::catalog_mutation::IndexColumnDefinition {
                    expression: expression.trim().trim_matches('"').to_owned(), descending: false,
                    nulls_first: false, is_expression: false,
                }).collect();
                let baseline_fingerprint = format!("duckdb:index:{}:{}:{}:{}:{}", row.0, row.1, row.2, row.3, row.4);
                return Ok(CatalogObjectDefinition::Index(crate::db::catalog_mutation::IndexDefinition {
                    database, schema, relation: row.2, relation_kind: CatalogKind::Table, name,
                    unique: row.3, access_method: "ART".into(), columns, include_columns: Vec::new(),
                    predicate: OptionalMetadata::Unsupported, tablespace: OptionalMetadata::Unsupported,
                    baseline_fingerprint,
                }));
            }
            if kind == CatalogKind::View {
                let (query, comment) = connection.query_row("SELECT sql, comment FROM duckdb_views() WHERE database_name = ? AND schema_name = ? AND view_name = ?", [database.as_str(), schema.as_str(), name.as_str()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))).map_err(duck_error)?;
                let columns = connection.prepare("SELECT column_name FROM duckdb_columns() WHERE database_name = ? AND schema_name = ? AND table_name = ? ORDER BY column_index").map_err(duck_error)?
                    .query_map([database.as_str(), schema.as_str(), name.as_str()], |row| row.get::<_, String>(0)).map_err(duck_error)?.collect::<Result<Vec<_>, _>>().map_err(duck_error)?;
                let fingerprint = format!("duckdb:view:{query}:comment={comment:?}");
                return Ok(CatalogObjectDefinition::View(ViewDefinition {
                    database, schema, name, owner: String::new(), comment: OptionalMetadata::Supported(comment),
                    query: view_query_from_catalog(&query), output_columns: columns,
                    security_barrier: ViewOption::unavailable("DuckDB does not expose PostgreSQL security_barrier"),
                    security_invoker: ViewOption::unavailable("DuckDB does not expose PostgreSQL security_invoker"),
                    check_option: ViewOption::unavailable("DuckDB does not expose PostgreSQL view check options"),
                    baseline_fingerprint: fingerprint,
                    clickhouse_native: None,
                }));
            }
            if kind == CatalogKind::Sequence {
                let row = connection.query_row(
                    "SELECT start_value, min_value, max_value, increment_by, cycle, sql FROM duckdb_sequences() WHERE database_name = ? AND schema_name = ? AND sequence_name = ?",
                    [database.as_str(), schema.as_str(), name.as_str()],
                    |row| Ok((
                        row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<i64>>(2)?, row.get::<_, i64>(3)?,
                        row.get::<_, bool>(4)?, row.get::<_, String>(5)?,
                    )),
                ).map_err(duck_error)?;
                let bound = |value: Option<i64>| value.map_or(
                    crate::db::catalog_mutation::SequenceBound::NoLimit,
                    |value| crate::db::catalog_mutation::SequenceBound::Value(value.to_string()),
                );
                return Ok(CatalogObjectDefinition::Sequence(
                    crate::db::catalog_mutation::SequenceDefinition {
                        database, schema, name, owner: String::new(),
                        comment: OptionalMetadata::Unsupported,
                        data_type: "BIGINT".into(), start_value: row.0.to_string(),
                        min_value: bound(row.1), max_value: bound(row.2),
                        increment: row.3.to_string(), cycle: row.4,
                        cache: String::new(), owned_by: None,
                        baseline_fingerprint: format!("duckdb:sequence:{}", row.5),
                    },
                ));
            }
            if kind != CatalogKind::Table {
                return Err(DatabaseError::unsupported("DuckDB structured object definition is available only for tables, views, and sequences"));
            }
            let (ddl, table_comment) = connection.query_row("SELECT sql, comment FROM duckdb_tables() WHERE database_name = ? AND schema_name = ? AND table_name = ?", [database.as_str(), schema.as_str(), name.as_str()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))).map_err(duck_error)?;
            let mut statement = connection.prepare("SELECT column_name, data_type, is_nullable, column_default, comment FROM duckdb_columns() WHERE database_name = ? AND schema_name = ? AND table_name = ? ORDER BY column_index").map_err(duck_error)?;
            let columns = statement.query_map([database.as_str(), schema.as_str(), name.as_str()], |row| Ok(ColumnDefinition {
                name: row.get(0)?, ordinal_position: 0, native_type: row.get(1)?, nullable: row.get(2)?,
                default_expression: OptionalMetadata::Supported(row.get::<_, Option<String>>(3)?),
                identity: OptionalMetadata::Unsupported, generated_expression: OptionalMetadata::Unsupported,
                collation: OptionalMetadata::Unsupported, comment: OptionalMetadata::Supported(row.get::<_, Option<String>>(4)?),
            })).map_err(duck_error)?.collect::<Result<Vec<_>, _>>().map_err(duck_error)?;
            let columns: Vec<ColumnDefinition> = columns.into_iter().enumerate().map(|(index, mut column)| { column.ordinal_position = index as u32 + 1; column }).collect();
            let index_metadata = duckdb_index_metadata(&connection, &database, &schema, &name)?;
            let indexes = index_metadata.iter().map(|(index_name, unique, expressions, comment)| {
                let columns = expressions.trim().trim_start_matches('[').trim_end_matches(']').split(',').map(|expression| expression.trim().trim_matches('"').to_owned()).collect::<Vec<_>>();
                let mut sql = format!("{}:{}:{}", index_name, unique, columns.join(","));
                if let Some(comment) = comment { sql.push_str(&format!(":comment={comment}")); }
                sql
            }).collect::<Vec<_>>();
            let constraint_metadata = duckdb_constraint_metadata(&connection, &database, &schema, &name)?
                .into_iter().filter(|(_, kind, _)| kind != "NOT NULL").collect::<Vec<_>>();
            let constraints = constraint_metadata.iter().map(|(_, kind, text)| format!("{kind}:{text}")).collect::<Vec<_>>();
            let structured_constraints = constraint_metadata.iter().map(|(constraint_name, kind, text)| {
                let definition_kind = if kind == "PRIMARY KEY" {
                    let columns = constraint_columns(&connection, &database, &schema, &name, constraint_name)?;
                    crate::db::catalog_mutation::ConstraintDefinitionKind::PrimaryKey { columns }
                } else if kind == "UNIQUE" {
                    let columns = constraint_columns(&connection, &database, &schema, &name, constraint_name)?;
                    crate::db::catalog_mutation::ConstraintDefinitionKind::Unique { columns }
                } else if kind == "CHECK" {
                    let expression = connection.query_row("SELECT expression FROM duckdb_constraints() WHERE database_name=? AND schema_name=? AND table_name=? AND constraint_name=?", [&database, &schema, &name, constraint_name], |row| row.get::<_, Option<String>>(0)).map_err(duck_error)?.unwrap_or_default();
                    crate::db::catalog_mutation::ConstraintDefinitionKind::Check { expression, no_inherit: false }
                } else if kind == "FOREIGN KEY" {
                    let columns = constraint_columns(&connection, &database, &schema, &name, constraint_name)?;
                    let referenced_relation = connection.query_row("SELECT referenced_table FROM duckdb_constraints() WHERE database_name=? AND schema_name=? AND table_name=? AND constraint_name=?", [&database, &schema, &name, constraint_name], |row| row.get::<_, Option<String>>(0)).map_err(duck_error)?;
                    let mut referenced_columns_statement = connection.prepare("SELECT unnest(referenced_column_names) FROM duckdb_constraints() WHERE database_name=? AND schema_name=? AND table_name=? AND constraint_name=?").map_err(duck_error)?;
                    let referenced_columns = referenced_columns_statement.query_map([&database, &schema, &name, constraint_name], |row| row.get::<_, String>(0)).map_err(duck_error)?.collect::<Result<Vec<_>, _>>().map_err(duck_error)?;
                    crate::db::catalog_mutation::ConstraintDefinitionKind::ForeignKey { columns, referenced_schema: schema.clone(), referenced_relation: referenced_relation.unwrap_or_default(), referenced_columns, match_type: "SIMPLE".into(), on_update: "NO ACTION".into(), on_delete: "NO ACTION".into() }
                } else {
                    return Err(DatabaseError::unsupported(format!("DuckDB constraint kind {kind:?} cannot be represented by the structured editor")));
                };
                Ok(crate::db::catalog_mutation::ConstraintDefinition {
                    database: database.clone(), schema: schema.clone(), relation: name.clone(), relation_kind: CatalogKind::Table,
                    name: constraint_name.clone(), kind: definition_kind, deferrable: false, initially_deferred: false,
                    validated: true, comment: OptionalMetadata::Unsupported,
                    baseline_fingerprint: format!("duckdb:constraint:{constraint_name}:{kind}:{text}"),
                })
            }).collect::<Result<Vec<_>, DatabaseError>>()?;
            let column_fingerprint = columns.iter().map(|column| (column.name.clone(), match &column.comment { OptionalMetadata::Supported(value) => value.clone(), OptionalMetadata::Unsupported => None })).collect::<Vec<_>>();
            let baseline_fingerprint = duckdb_table_fingerprint(&ddl, &table_comment, &column_fingerprint, &index_metadata, &constraint_metadata);
            Ok(CatalogObjectDefinition::Table(TableDefinition {
                database, schema, name, owner: String::new(), comment: OptionalMetadata::Supported(table_comment),
                columns, indexes, constraints, structured_constraints,
                baseline_fingerprint,
            }))
        }).await.map_err(|error| DatabaseError::internal(format!("DuckDB worker failed: {error}")))?
    }

    pub async fn object_ddl(
        &self,
        kind: crate::db::catalog::CatalogKind,
        schema: &str,
        name: &str,
    ) -> Result<Option<String>, DatabaseError> {
        let database = self.database.clone();
        let schema = schema.to_owned();
        let name = name.to_owned();
        let connection = self.connection.clone();
        task::spawn_blocking(move || {
            let connection = connection.lock().map_err(|_| DatabaseError::internal("DuckDB connection lock poisoned"))?;
            let sql = match kind {
                crate::db::catalog::CatalogKind::Table => "SELECT sql FROM duckdb_tables() WHERE database_name = ? AND schema_name = ? AND table_name = ?",
                crate::db::catalog::CatalogKind::View => "SELECT sql FROM duckdb_views() WHERE database_name = ? AND schema_name = ? AND view_name = ?",
                crate::db::catalog::CatalogKind::Sequence => "SELECT sql FROM duckdb_sequences() WHERE database_name = ? AND schema_name = ? AND sequence_name = ?",
                crate::db::catalog::CatalogKind::Index => return Ok(None),
                _ => return Ok(None),
            };
            match connection.query_row(sql, [database.as_str(), schema.as_str(), name.as_str()], |row| row.get::<_, String>(0)) {
                Ok(ddl) => Ok(Some(ddl)),
                Err(error) if error.to_string().contains("No rows") => Ok(None),
                Err(error) => Err(duck_error(error)),
            }
        }).await.map_err(|error| DatabaseError::internal(format!("DuckDB worker failed: {error}")))?
    }

    pub async fn execute_catalog_mutation(
        &self,
        plan: &crate::db::catalog_mutation::CatalogMutationPlan,
    ) -> Result<QueryOutcome, DatabaseError> {
        plan.validate()
            .map_err(|error| DatabaseError::configuration(error.to_string()))?;
        if plan.request.mode == crate::db::catalog_mutation::CatalogMutationMode::Edit {
            let object = match &plan.request.anchor {
                crate::db::catalog_mutation::CatalogMutationAnchor::Catalog(object) => object,
                _ => {
                    return Err(DatabaseError::configuration(
                        "DuckDB edit requires a catalog object anchor",
                    ));
                }
            };
            if plan.impact.old_object_id != *object {
                return Err(DatabaseError::configuration(
                    "DuckDB catalog edit object identity changed after planning",
                ));
            }
            if object.kind == crate::db::catalog::CatalogKind::Index {
                let [database, schema, relation, name, ..] = object.native_path.as_slice() else {
                    return Err(DatabaseError::configuration(
                        "DuckDB index edit identity is incomplete",
                    ));
                };
                let connection = self.connection.clone();
                let database = database.clone();
                let schema = schema.clone();
                let relation = relation.clone();
                let name = name.clone();
                let expected = plan.baseline_fingerprint.clone();
                let statements = plan.statements().to_vec();
                return task::spawn_blocking(move || {
                    let connection = connection.lock().map_err(|_| DatabaseError::internal("DuckDB connection lock poisoned"))?;
                let row = connection.query_row(
                        "SELECT database_name, schema_name, table_name, is_unique, expressions FROM duckdb_indexes() WHERE index_name = ?",
                        [name.as_str()],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, bool>(3)?, row.get::<_, String>(4)?)),
                    ).map_err(duck_error)?;
                let actual = format!("duckdb:index:{}:{}:{}:{}:{}", row.0, row.1, row.2, row.3, row.4);
                    if row.0 != database || row.1 != schema || row.2 != relation || expected.as_deref() != Some(actual.as_str()) {
                        return Err(DatabaseError::configuration("DuckDB index edit baseline is stale"));
                    }
                    for statement in statements { connection.execute_batch(&statement).map_err(duck_error)?; }
                    let mut outcome = QueryOutcomeAccumulator::with_budget(QueryBudget::UNBOUNDED);
                    outcome.done(0);
                    Ok(outcome.finish())
                }).await.map_err(|error| DatabaseError::internal(format!("DuckDB worker failed: {error}")))?;
            }
            let [database, schema, name, ..] = object.native_path.as_slice() else {
                return Err(DatabaseError::configuration(
                    "DuckDB edit object identity is incomplete",
                ));
            };
            let kind = object.kind;
            let connection = self.connection.clone();
            let database = database.clone();
            let schema = schema.clone();
            let name = name.clone();
            let expected = plan.baseline_fingerprint.clone();
            let statements = plan.statements().to_vec();
            return task::spawn_blocking(move || {
                let connection = connection.lock().map_err(|_| DatabaseError::internal("DuckDB connection lock poisoned"))?;
                let sql = match kind {
                    crate::db::catalog::CatalogKind::Table => "SELECT sql, comment FROM duckdb_tables() WHERE database_name = ? AND schema_name = ? AND table_name = ?",
                    crate::db::catalog::CatalogKind::View => "SELECT sql, comment FROM duckdb_views() WHERE database_name = ? AND schema_name = ? AND view_name = ?",
                    _ => return Err(DatabaseError::unsupported("DuckDB edit is supported only for tables and views")),
                };
                let current = connection.query_row(sql, [database.as_str(), schema.as_str(), name.as_str()], |row| {
                    let ddl = row.get::<_, String>(0)?;
                    let comment = row.get::<_, Option<String>>(1)?;
                    let children = if kind == crate::db::catalog::CatalogKind::Table {
                        let mut columns = connection.prepare("SELECT column_name, comment FROM duckdb_columns() WHERE database_name = ? AND schema_name = ? AND table_name = ? ORDER BY column_index")?;
                        let columns = columns.query_map([database.as_str(), schema.as_str(), name.as_str()], |child| Ok((child.get::<_, String>(0)?, child.get::<_, Option<String>>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
                        let indexes = duckdb_index_metadata(&connection, &database, &schema, &name).map_err(|error| duckdb::Error::ToSqlConversionFailure(Box::new(error)))?;
                        let constraints = duckdb_constraint_metadata(&connection, &database, &schema, &name).map_err(|error| duckdb::Error::ToSqlConversionFailure(Box::new(error)))?.into_iter().filter(|(_, kind, _)| kind != "NOT NULL").collect::<Vec<_>>();
                        Ok::<_, duckdb::Error>(duckdb_table_fingerprint(&ddl, &comment, &columns, &indexes, &constraints))?
                    } else { String::new() };
                    Ok::<_, duckdb::Error>((ddl, comment, children))
                }).map_err(duck_error)?;
                let actual = match kind {
                    crate::db::catalog::CatalogKind::Table => current.2.clone(),
                    crate::db::catalog::CatalogKind::View => format!("duckdb:view:{}:comment={:?}", current.0, current.1),
                    _ => unreachable!(),
                };
                if expected.as_deref() != Some(actual.as_str()) {
                    return Err(DatabaseError::configuration("DuckDB catalog edit baseline is stale"));
                }
                for statement in statements {
                    connection.execute_batch(&statement).map_err(duck_error)?;
                }
                let mut outcome = QueryOutcomeAccumulator::with_budget(QueryBudget::UNBOUNDED);
                outcome.done(0);
                Ok(outcome.finish())
            }).await.map_err(|error| DatabaseError::internal(format!("DuckDB worker failed: {error}")))?;
        }
        self.execute(&plan.statements().join("; ")).await
    }
}

fn duck_value(
    row: &duckdb::Row<'_>,
    index: usize,
) -> Result<crate::db::value::CellValue, DatabaseError> {
    use crate::db::value::CellValue;
    let value_ref = row.get_ref(index).map_err(duck_error)?;
    if let duckdb::types::ValueRef::Text(value) = value_ref {
        return Ok(CellValue::Text(String::from_utf8_lossy(value).into_owned()));
    }
    if let duckdb::types::ValueRef::Time64(unit, value) = value_ref {
        let nanos_from_midnight = match unit {
            duckdb::types::TimeUnit::Second => value.saturating_mul(1_000_000_000),
            duckdb::types::TimeUnit::Millisecond => value.saturating_mul(1_000_000),
            duckdb::types::TimeUnit::Microsecond => value.saturating_mul(1_000),
            duckdb::types::TimeUnit::Nanosecond => value,
        };
        return Ok(chrono::NaiveTime::from_num_seconds_from_midnight_opt(
            nanos_from_midnight.div_euclid(1_000_000_000) as u32,
            nanos_from_midnight.rem_euclid(1_000_000_000) as u32,
        )
        .map(CellValue::Time)
        .unwrap_or_else(|| CellValue::Text(format!("{nanos_from_midnight} ns"))));
    }
    let value = duckdb::types::Value::from(value_ref);
    Ok(match value {
        duckdb::types::Value::Null => CellValue::Null,
        duckdb::types::Value::Boolean(value) => CellValue::Boolean(value),
        duckdb::types::Value::TinyInt(value) => CellValue::Integer(value.into()),
        duckdb::types::Value::SmallInt(value) => CellValue::Integer(value.into()),
        duckdb::types::Value::Int(value) => CellValue::Integer(value.into()),
        duckdb::types::Value::BigInt(value) => CellValue::Integer(value),
        duckdb::types::Value::HugeInt(value) => CellValue::Text(value.to_string()),
        duckdb::types::Value::UHugeInt(value) => CellValue::Text(value.to_string()),
        duckdb::types::Value::UTinyInt(value) => CellValue::Unsigned(value.into()),
        duckdb::types::Value::USmallInt(value) => CellValue::Unsigned(value.into()),
        duckdb::types::Value::UInt(value) => CellValue::Unsigned(value.into()),
        duckdb::types::Value::UBigInt(value) => CellValue::Unsigned(value),
        duckdb::types::Value::Float(value) => CellValue::Float(value.into()),
        duckdb::types::Value::Double(value) => CellValue::Float(value),
        duckdb::types::Value::Text(value) => CellValue::Text(value),
        duckdb::types::Value::Blob(value) | duckdb::types::Value::Geometry(value) => {
            CellValue::Bytes(value)
        }
        duckdb::types::Value::Decimal(value) => CellValue::Text(value.to_string()),
        duckdb::types::Value::Timestamp(unit, value) => {
            let (seconds, nanos) = match unit {
                duckdb::types::TimeUnit::Second => (value, 0),
                duckdb::types::TimeUnit::Millisecond => (
                    value.div_euclid(1_000),
                    (value.rem_euclid(1_000) * 1_000_000) as u32,
                ),
                duckdb::types::TimeUnit::Microsecond => (
                    value.div_euclid(1_000_000),
                    (value.rem_euclid(1_000_000) * 1_000) as u32,
                ),
                duckdb::types::TimeUnit::Nanosecond => (
                    value.div_euclid(1_000_000_000),
                    value.rem_euclid(1_000_000_000) as u32,
                ),
            };
            chrono::DateTime::from_timestamp(seconds, nanos)
                .map(|value| CellValue::Timestamp(value.fixed_offset()))
                .unwrap_or_else(|| CellValue::Text(format!("{value} {unit:?}")))
        }
        duckdb::types::Value::Date32(days) => {
            chrono::NaiveDate::from_num_days_from_ce_opt(days.saturating_add(719_163))
                .map(CellValue::Date)
                .unwrap_or_else(|| CellValue::Text(format!("date({days})")))
        }
        duckdb::types::Value::Time64(_, _) => {
            unreachable!("time values are decoded before owning conversion")
        }
        duckdb::types::Value::Interval {
            months,
            days,
            nanos,
        } => CellValue::Text(format!("{months} mons {days} days {nanos} ns")),
        duckdb::types::Value::List(_)
        | duckdb::types::Value::Array(_)
        | duckdb::types::Value::Struct(_)
        | duckdb::types::Value::Map(_)
        | duckdb::types::Value::Enum(_)
        | duckdb::types::Value::Union(_) => CellValue::Text(duck_value_text(&value)),
        other => CellValue::Unsupported {
            type_name: "DuckDB value".into(),
            preview: format!("{other:?}"),
        },
    })
}

fn quote_duck_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

type DuckDbIndexMetadata = (String, bool, String, Option<String>);
type DuckDbConstraintMetadata = (String, String, String);

fn duckdb_index_metadata(
    connection: &duckdb::Connection,
    database: &str,
    schema: &str,
    table: &str,
) -> Result<Vec<DuckDbIndexMetadata>, DatabaseError> {
    let mut statement = connection.prepare("SELECT index_name, is_unique, expressions, comment FROM duckdb_indexes() WHERE database_name = ? AND schema_name = ? AND table_name = ? ORDER BY index_name").map_err(duck_error)?;
    statement
        .query_map([database, schema, table], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(duck_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(duck_error)
}

fn duckdb_constraint_metadata(
    connection: &duckdb::Connection,
    database: &str,
    schema: &str,
    table: &str,
) -> Result<Vec<DuckDbConstraintMetadata>, DatabaseError> {
    let mut statement = connection.prepare("SELECT constraint_name, constraint_type, constraint_text FROM duckdb_constraints() WHERE database_name = ? AND schema_name = ? AND table_name = ? ORDER BY constraint_index").map_err(duck_error)?;
    statement
        .query_map([database, schema, table], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(duck_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(duck_error)
}

fn constraint_columns(
    connection: &duckdb::Connection,
    database: &str,
    schema: &str,
    table: &str,
    constraint: &str,
) -> Result<Vec<String>, DatabaseError> {
    let mut statement = connection.prepare(
        "SELECT unnest(constraint_column_names) FROM duckdb_constraints() WHERE database_name=? AND schema_name=? AND table_name=? AND constraint_name=? ORDER BY constraint_column_indexes",
    ).map_err(duck_error)?;
    statement
        .query_map([database, schema, table, constraint], |row| row.get(0))
        .map_err(duck_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(duck_error)
}

fn duckdb_table_fingerprint(
    ddl: &str,
    table_comment: &Option<String>,
    columns: &[(String, Option<String>)],
    indexes: &[DuckDbIndexMetadata],
    constraints: &[DuckDbConstraintMetadata],
) -> String {
    format!(
        "duckdb:table:{ddl}:comment={table_comment:?}:columns={columns:?}:indexes={indexes:?}:constraints={constraints:?}"
    )
}

fn duck_sql_literal_or_null(value: &str) -> String {
    if value.is_empty() {
        "NULL".into()
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}

fn view_query_from_catalog(sql: &str) -> String {
    let upper = sql.to_ascii_uppercase();
    upper
        .find(" AS ")
        .map(|offset| sql[offset + 4..].trim().to_owned())
        .unwrap_or_else(|| sql.to_owned())
}

fn duckdb_select_query(statement: &str) -> bool {
    let mut sql = statement.trim_start();
    loop {
        if let Some(rest) = sql.strip_prefix("--") {
            sql = rest
                .split_once('\n')
                .map_or("", |(_, rest)| rest)
                .trim_start();
            continue;
        }
        if let Some(rest) = sql.strip_prefix("/*") {
            let Some((_, rest)) = rest.split_once("*/") else {
                return false;
            };
            sql = rest.trim_start();
            continue;
        }
        break;
    }
    let keyword = sql
        .split(|character: char| !character.is_ascii_alphabetic())
        .next()
        .unwrap_or_default();
    keyword.eq_ignore_ascii_case("SELECT")
        || keyword.eq_ignore_ascii_case("WITH")
        || keyword.eq_ignore_ascii_case("FROM")
}

fn duckdb_time_ns_text_query(
    statement: &str,
    output_columns: &[(String, String)],
) -> Result<String, DatabaseError> {
    if !output_columns.iter().any(|(_, type_name)| {
        type_name.eq_ignore_ascii_case("TIME_NS") || type_name.eq_ignore_ascii_case("Time64(ns)")
    }) {
        return Ok(statement.to_owned());
    }
    let mut names = std::collections::HashSet::new();
    if output_columns
        .iter()
        .any(|(name, _)| name.trim().is_empty() || !names.insert(name))
    {
        return Err(DatabaseError::unsupported(
            "DuckDB TIME_NS decoding requires unique, non-empty output column names",
        ));
    }
    let projection = output_columns
        .iter()
        .map(|(name, type_name)| {
            if !type_name.eq_ignore_ascii_case("TIME_NS")
                && !type_name.eq_ignore_ascii_case("Time64(ns)")
            {
                return Ok(quote_duck_identifier(name));
            }
            Ok(format!(
                "CAST({alias} AS VARCHAR) AS {alias}",
                alias = quote_duck_identifier(name),
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(format!(
        "SELECT {} FROM ({}) AS {}",
        projection.join(", "),
        statement,
        quote_duck_identifier("__lazydb_time_ns")
    ))
}

fn duck_parameter(
    value: &crate::db::value::CellValue,
) -> Result<duckdb::types::Value, crate::db::transaction::TransactionError> {
    use crate::db::{transaction::TransactionError, value::CellValue};
    use duckdb::types::Value;
    Ok(match value {
        CellValue::Null => Value::Null,
        CellValue::Boolean(value) => Value::Boolean(*value),
        CellValue::Integer(value) => Value::BigInt(*value),
        CellValue::Unsigned(value) => Value::UBigInt(*value),
        CellValue::Float(value) => Value::Double(*value),
        CellValue::Text(value) => Value::Text(value.clone()),
        CellValue::Bytes(value) => Value::Blob(value.clone()),
        CellValue::Date(value) => Value::Date32(value.num_days_from_ce() - 719_163),
        CellValue::Time(value) => Value::Time64(
            duckdb::types::TimeUnit::Microsecond,
            value.num_seconds_from_midnight() as i64 * 1_000_000
                + value.nanosecond() as i64 / 1_000,
        ),
        CellValue::DateTime(value) => Value::Timestamp(
            duckdb::types::TimeUnit::Microsecond,
            value.and_utc().timestamp_micros(),
        ),
        CellValue::Timestamp(value) => Value::Timestamp(
            duckdb::types::TimeUnit::Microsecond,
            value.timestamp_micros(),
        ),
        CellValue::MySqlGeometry { .. } | CellValue::Unsupported { .. } => {
            return Err(TransactionError(
                "DuckDB relation mutation cannot bind this cell value".into(),
            ));
        }
    })
}

fn duck_parameter_input(
    value: &InputValue,
) -> Result<duckdb::types::Value, crate::db::transaction::TransactionError> {
    match value {
        InputValue::Null => duck_parameter(&crate::db::value::CellValue::Null),
        InputValue::Value(value) => duck_parameter(value),
        InputValue::Default => Err(crate::db::transaction::TransactionError(
            "DEFAULT must be omitted from DuckDB insert values".into(),
        )),
    }
}

fn decode_duck_row(row: &duckdb::Row<'_>) -> duckdb::Result<Vec<crate::db::value::CellValue>> {
    (0..row.as_ref().column_count())
        .map(|index| {
            duck_value(row, index)
                .map_err(|error| duckdb::Error::ToSqlConversionFailure(Box::new(error)))
        })
        .collect()
}

fn duck_value_text(value: &duckdb::types::Value) -> String {
    use duckdb::types::Value;
    match value {
        Value::Null => "NULL".into(),
        Value::Boolean(value) => value.to_string(),
        Value::TinyInt(value) => value.to_string(),
        Value::SmallInt(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::HugeInt(value) => value.to_string(),
        Value::UHugeInt(value) => value.to_string(),
        Value::UTinyInt(value) => value.to_string(),
        Value::USmallInt(value) => value.to_string(),
        Value::UInt(value) => value.to_string(),
        Value::UBigInt(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::Double(value) => value.to_string(),
        Value::Decimal(value) => value.to_string(),
        Value::Timestamp(unit, value) => format!("{value} {unit:?}"),
        Value::Text(value) => value.clone(),
        Value::Blob(value) | Value::Geometry(value) => format!(
            "\\x{}",
            value
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ),
        Value::Date32(value) => value.to_string(),
        Value::Time64(unit, value) => format!("{value} {unit:?}"),
        Value::Interval {
            months,
            days,
            nanos,
        } => format!("{months} mons {days} days {nanos} ns"),
        Value::List(values) | Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(duck_value_text)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Enum(value) => value.clone(),
        Value::Struct(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!("{key}: {}", duck_value_text(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Map(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!("{}: {}", duck_value_text(key), duck_value_text(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Union(value) => duck_value_text(value),
        _ => "<unsupported DuckDB value>".into(),
    }
}

fn intersect_catalog_scopes(
    profile: &crate::profile::CatalogScope,
    request: &crate::profile::CatalogScope,
) -> crate::profile::CatalogScope {
    use crate::profile::{CatalogScope, CatalogSelection, DatabaseScope};
    let databases = match (&profile.databases, &request.databases) {
        (CatalogSelection::All, CatalogSelection::All) => CatalogSelection::All,
        (CatalogSelection::All, CatalogSelection::Selected(selected)) => {
            CatalogSelection::Selected(selected.clone())
        }
        (CatalogSelection::Selected(selected), CatalogSelection::All) => {
            CatalogSelection::Selected(selected.clone())
        }
        (CatalogSelection::Selected(profile), CatalogSelection::Selected(request)) => {
            let mut intersection = Vec::new();
            for profile_database in profile {
                let Some(request_database) = request
                    .iter()
                    .find(|item| item.name == profile_database.name)
                else {
                    continue;
                };
                let schemas = match (&profile_database.schemas, &request_database.schemas) {
                    (CatalogSelection::All, CatalogSelection::All) => CatalogSelection::All,
                    (CatalogSelection::All, selected) | (selected, CatalogSelection::All) => {
                        selected.clone()
                    }
                    (CatalogSelection::Selected(left), CatalogSelection::Selected(right)) => {
                        CatalogSelection::Selected(
                            left.iter()
                                .filter(|schema| right.contains(schema))
                                .cloned()
                                .collect(),
                        )
                    }
                };
                if matches!(&schemas, CatalogSelection::Selected(schemas) if schemas.is_empty()) {
                    continue;
                }
                intersection.push(DatabaseScope {
                    name: profile_database.name.clone(),
                    schemas,
                });
            }
            CatalogSelection::Selected(intersection)
        }
    };
    CatalogScope { databases }
}

fn duck_operation_error(mut error: DatabaseError, cancelled: bool) -> DatabaseError {
    // Some DuckDB C API paths expose only a generic failure code on interrupt.
    if cancelled && error.category == ErrorCategory::Sql {
        error.category = ErrorCategory::Cancelled;
        error.code = Some("cancelled".into());
    }
    error
}

fn duck_error(error: duckdb::Error) -> DatabaseError {
    let interrupted = matches!(
        &error,
        duckdb::Error::DuckDBFailure(native, _)
            if native.extended_code == duckdb::ffi::duckdb_error_type_DUCKDB_ERROR_INTERRUPT
    );
    DatabaseError {
        category: if interrupted {
            ErrorCategory::Cancelled
        } else {
            ErrorCategory::Sql
        },
        code: Some(
            if interrupted {
                "cancelled"
            } else {
                "duckdb_error"
            }
            .into(),
        ),
        message: crate::security::sanitize_terminal_text(&error.to_string()),
        diagnostic: None,
    }
}

#[cfg(test)]
mod transaction_tests {
    use super::*;
    use crate::db::{DatabaseConnection, transaction::TransactionBackend};

    #[tokio::test]
    async fn budget_drain_consumes_remaining_statements_without_retaining_them() {
        let profile = crate::profile::import_connection_url("duckdb::memory:", None)
            .unwrap()
            .profile;
        let adapter = DuckDbAdapter::connect(&profile).await.unwrap();
        let outcome = adapter
            .execute_with_budget(
                "SELECT * FROM range(100); SELECT 7 AS next",
                QueryBudget {
                    max_rows: 2,
                    max_bytes: usize::MAX,
                },
            )
            .await
            .unwrap();
        assert!(outcome.stats.truncated);
        assert_eq!(outcome.stats.fetched_row_count, 3);
        assert_eq!(outcome.result_sets[0].rows.len(), 2);
        assert!(outcome.result_sets[1].rows.is_empty());
        adapter.close().await;
    }

    #[tokio::test]
    async fn dml_returning_reports_returned_row_count() {
        let profile = crate::profile::import_connection_url("duckdb::memory:", None)
            .unwrap()
            .profile;
        let adapter = DuckDbAdapter::connect(&profile).await.unwrap();
        adapter
            .execute("CREATE TABLE returned (id INTEGER)")
            .await
            .unwrap();
        let inserted = adapter
            .execute("INSERT INTO returned VALUES (1), (2) RETURNING id")
            .await
            .unwrap();
        assert_eq!(inserted.result_sets[0].affected_rows, 2);
        let updated = adapter
            .execute("UPDATE returned SET id = id + 10 RETURNING id")
            .await
            .unwrap();
        assert_eq!(updated.result_sets[0].affected_rows, 2);
        let deleted = adapter
            .execute("DELETE FROM returned WHERE id = 11 RETURNING id")
            .await
            .unwrap();
        assert_eq!(deleted.result_sets[0].affected_rows, 1);
        adapter.close().await;
    }

    #[test]
    fn native_interrupt_errors_are_cancelled_while_sql_errors_remain_sql() {
        for (native_code, category, code) in [
            (
                duckdb::ffi::duckdb_error_type_DUCKDB_ERROR_INTERRUPT,
                ErrorCategory::Cancelled,
                "cancelled",
            ),
            (
                duckdb::ffi::duckdb_error_type_DUCKDB_ERROR_PARSER,
                ErrorCategory::Sql,
                "duckdb_error",
            ),
        ] {
            let error = duck_error(duckdb::Error::DuckDBFailure(
                duckdb::ffi::Error::new(native_code),
                Some("native diagnostic".into()),
            ));
            assert_eq!(error.category, category);
            assert_eq!(error.code.as_deref(), Some(code));
            assert_eq!(error.message, "native diagnostic");
        }
    }

    #[test]
    fn operation_cancellation_classifies_generic_native_failures() {
        for (cancelled, expected) in [
            (true, ErrorCategory::Cancelled),
            (false, ErrorCategory::Sql),
        ] {
            let native = duckdb::Error::DuckDBFailure(
                duckdb::ffi::Error::new(duckdb::ffi::DuckDBError),
                Some("INTERRUPT Error: Interrupted!".into()),
            );
            let error = duck_operation_error(duck_error(native), cancelled);
            assert_eq!(error.category, expected);
            assert_eq!(error.message, "INTERRUPT Error: Interrupted!");
        }
        let error = duck_operation_error(DatabaseError::internal("worker failed"), true);
        assert_eq!(error.category, ErrorCategory::Internal);
    }

    #[derive(Clone, Copy)]
    enum CancelStage {
        Describe,
        Prepare,
        Execute,
    }

    struct QueryGate {
        stage: CancelStage,
        binds: std::sync::atomic::AtomicUsize,
        reached: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl QueryGate {
        fn wait(&self) -> Result<(), Box<dyn std::error::Error>> {
            if let Some(reached) = self.reached.lock().unwrap().take() {
                let _ = reached.send(());
                self.release
                    .lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(10))?;
            }
            Ok(())
        }
    }

    struct GatedTable;

    impl duckdb::vtab::VTab for GatedTable {
        type InitData = std::sync::atomic::AtomicBool;
        type BindData = Arc<QueryGate>;

        fn bind(
            bind: &duckdb::vtab::BindInfo,
        ) -> Result<Self::BindData, Box<dyn std::error::Error>> {
            // Registration owns an Arc<QueryGate> for the lifetime of these callbacks.
            let gate = unsafe { &*bind.get_extra_info::<Arc<QueryGate>>() }.clone();
            let binding = gate.binds.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if matches!(
                (gate.stage, binding),
                (CancelStage::Describe, 0) | (CancelStage::Prepare, 1)
            ) {
                gate.wait()?;
            }
            bind.add_result_column(
                "answer",
                duckdb::core::LogicalTypeHandle::from(duckdb::core::LogicalTypeId::Bigint),
            );
            Ok(gate)
        }

        fn init(_: &duckdb::vtab::InitInfo) -> Result<Self::InitData, Box<dyn std::error::Error>> {
            Ok(std::sync::atomic::AtomicBool::new(false))
        }

        fn func(
            func: &duckdb::vtab::TableFunctionInfo<Self>,
            output: &mut duckdb::core::DataChunkHandle,
        ) -> Result<(), Box<dyn std::error::Error>> {
            if func
                .get_init_data()
                .swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                output.set_len(0);
                return Ok(());
            }
            let gate = func.get_bind_data();
            if matches!(gate.stage, CancelStage::Execute) {
                gate.wait()?;
            }
            // The declared BIGINT output has capacity for at least this one row.
            unsafe { output.flat_vector(0).as_mut_slice::<i64>()[0] = 42 };
            output.set_len(1);
            Ok(())
        }
    }

    fn install_query_gate(
        adapter: &DuckDbAdapter,
        stage: CancelStage,
    ) -> (
        tokio::sync::oneshot::Receiver<()>,
        std::sync::mpsc::Sender<()>,
    ) {
        let (reached, receiver) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let gate = Arc::new(QueryGate {
            stage,
            binds: std::sync::atomic::AtomicUsize::new(0),
            reached: Mutex::new(Some(reached)),
            release: Mutex::new(wait),
        });
        adapter
            .connection
            .lock()
            .unwrap()
            .register_table_function_with_extra_info::<GatedTable, _>("cancel_gate", &gate)
            .unwrap();
        (receiver, release)
    }

    async fn assert_answer_42(adapter: &DuckDbAdapter) {
        let result = adapter.execute("SELECT 42").await.unwrap();
        assert_eq!(
            result.result_sets[0].rows[0][0],
            crate::db::value::CellValue::Integer(42)
        );
    }

    async fn cancel_at_stage(stage: CancelStage) {
        let profile = crate::profile::import_connection_url("duckdb::memory:", None)
            .unwrap()
            .profile;
        let adapter = DuckDbAdapter::connect(&profile).await.unwrap();
        let (reached, release) = install_query_gate(&adapter, stage);
        let (started, operation) = tokio::sync::oneshot::channel();
        let query_adapter = adapter.clone();
        let query = tokio::spawn(async move {
            query_adapter
                .execute_with_budget_cancellable(
                    "SELECT * FROM cancel_gate()",
                    QueryBudget::UNBOUNDED,
                    uuid::Uuid::new_v4(),
                    Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    move |handle| {
                        let _ = started.send(handle);
                    },
                )
                .await
        });
        let handle = operation.await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), reached)
            .await
            .unwrap()
            .unwrap();
        handle.cancel();
        release.send(()).unwrap();
        let error = tokio::time::timeout(std::time::Duration::from_secs(5), query)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.category, ErrorCategory::Cancelled);
        assert_eq!(error.code.as_deref(), Some("cancelled"));
        assert_answer_42(&adapter).await;
        adapter.close().await;
    }

    #[tokio::test]
    async fn cancellation_during_describe_binding_preserves_session() {
        cancel_at_stage(CancelStage::Describe).await;
    }

    #[tokio::test]
    async fn cancellation_during_statement_preparation_preserves_session() {
        cancel_at_stage(CancelStage::Prepare).await;
    }

    #[tokio::test]
    async fn cancellation_during_native_execution_preserves_session() {
        cancel_at_stage(CancelStage::Execute).await;
    }

    async fn assert_retired_operation_cannot_cancel(sql: &str, cancel_on_start: bool) {
        let profile = crate::profile::import_connection_url("duckdb::memory:", None)
            .unwrap()
            .profile;
        let adapter = DuckDbAdapter::connect(&profile).await.unwrap();
        let (started, previous) = tokio::sync::oneshot::channel();
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let result = adapter
            .execute_with_budget_cancellable(
                sql,
                QueryBudget::UNBOUNDED,
                uuid::Uuid::new_v4(),
                cancelled.clone(),
                move |handle| {
                    if cancel_on_start {
                        handle.cancel();
                    }
                    let _ = started.send(handle);
                },
            )
            .await;
        if cancel_on_start {
            assert_eq!(result.unwrap_err().category, ErrorCategory::Cancelled);
        } else if sql == "SELECT 1" {
            result.unwrap();
        } else {
            assert_eq!(result.unwrap_err().category, ErrorCategory::Sql);
        }
        let was_cancelled = cancelled.load(std::sync::atomic::Ordering::SeqCst);
        let previous = previous.await.unwrap();
        let (reached, release) = install_query_gate(&adapter, CancelStage::Execute);
        let query_adapter = adapter.clone();
        let query =
            tokio::spawn(async move { query_adapter.execute("SELECT * FROM cancel_gate()").await });
        tokio::time::timeout(std::time::Duration::from_secs(5), reached)
            .await
            .unwrap()
            .unwrap();
        previous.cancel();
        release.send(()).unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), query)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            result.result_sets[0].rows[0][0],
            crate::db::value::CellValue::Integer(42)
        );
        assert_eq!(
            cancelled.load(std::sync::atomic::Ordering::SeqCst),
            was_cancelled
        );
        assert_answer_42(&adapter).await;
        adapter.close().await;
    }

    #[tokio::test]
    async fn completed_operation_cannot_cancel_next_native_query() {
        assert_retired_operation_cannot_cancel("SELECT 1", false).await;
    }

    #[tokio::test]
    async fn failed_operation_cannot_cancel_next_native_query() {
        assert_retired_operation_cannot_cancel("SELECT missing_column", false).await;
    }

    #[tokio::test]
    async fn cancelled_before_execution_cannot_cancel_next_native_query() {
        assert_retired_operation_cannot_cancel("SELECT 1", true).await;
    }

    #[tokio::test]
    async fn aborted_waiter_keeps_native_cancellation_active_until_worker_finishes() {
        let profile = crate::profile::import_connection_url("duckdb::memory:", None)
            .unwrap()
            .profile;
        let adapter = DuckDbAdapter::connect(&profile).await.unwrap();
        let (reached, release) = install_query_gate(&adapter, CancelStage::Execute);
        let (started, operation) = tokio::sync::oneshot::channel();
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let query_adapter = adapter.clone();
        let query_cancelled = cancelled.clone();
        let query = tokio::spawn(async move {
            query_adapter
                .execute_with_budget_cancellable(
                    "SELECT * FROM cancel_gate()",
                    QueryBudget::UNBOUNDED,
                    uuid::Uuid::new_v4(),
                    query_cancelled,
                    move |handle| {
                        let _ = started.send(handle);
                    },
                )
                .await
        });
        let operation = operation.await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), reached)
            .await
            .unwrap()
            .unwrap();
        query.abort();
        assert!(query.await.unwrap_err().is_cancelled());
        assert!(adapter.operation_gate.clone().try_acquire_owned().is_err());
        operation.cancel();
        assert!(cancelled.load(std::sync::atomic::Ordering::SeqCst));
        release.send(()).unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            assert_answer_42(&adapter),
        )
        .await
        .unwrap();
        assert!(!*operation.active.lock().unwrap());
        let permit = adapter.operation_gate.clone().try_acquire_owned().unwrap();
        drop(permit);
        adapter.close().await;
    }

    #[tokio::test]
    async fn operation_interrupt_cancels_native_query_and_session_remains_usable() {
        let profile = crate::profile::import_connection_url("duckdb::memory:", Some("cancel-test"))
            .unwrap()
            .profile;
        let adapter = DuckDbAdapter::connect(&profile).await.unwrap();
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel_copy = Arc::clone(&cancelled);
        let (started_tx, mut started_rx) = tokio::sync::oneshot::channel();
        let query_adapter = adapter.clone();
        let query = query_adapter.execute_with_budget_cancellable(
            "SELECT sum(i) FROM range(5000000000) t(i)",
            QueryBudget::UNBOUNDED,
            uuid::Uuid::new_v4(),
            cancelled,
            move |operation| {
                let _ = started_tx.send(operation);
            },
        );
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::pin!(query);
            tokio::select! {
                operation = &mut started_rx => {
                    let operation = operation.expect("operation handle must be registered");
                    operation.cancel();
                }
                result = &mut query => panic!("native query completed before cancellation: {result:?}"),
            }
            query.await
        })
        .await
        .expect("cancelled native query did not finish")
        .unwrap_err();
        assert_eq!(result.category, crate::db::ErrorCategory::Cancelled);
        assert!(cancel_copy.load(std::sync::atomic::Ordering::SeqCst));
        let next = adapter.execute("SELECT 42").await.unwrap();
        assert_eq!(
            next.result_sets[0].rows[0][0],
            crate::db::value::CellValue::Integer(42)
        );
        adapter.close().await;
    }

    #[tokio::test]
    async fn structured_create_drop_plans_execute_on_native_database() {
        let profile = crate::profile::import_connection_url("duckdb::memory:", None)
            .unwrap()
            .profile;
        let database = DatabaseConnection::connect(&profile, None).await.unwrap();
        let connection = crate::identity::ConnectionIdentity {
            profile_id: profile.id,
            generation: 1,
        };
        let schema_id = crate::db::catalog::CatalogId::new(
            profile.id,
            crate::db::catalog::CatalogKind::Schema,
            ["memory", "main"],
        );
        let request = crate::db::catalog_mutation::CatalogMutationRequest::new(
            connection,
            1,
            0,
            crate::db::catalog_mutation::CatalogMutationMode::Create,
            crate::db::catalog_mutation::CatalogMutationAnchor::Catalog(schema_id.clone()),
            crate::db::catalog_mutation::CatalogObjectType::Catalog(
                crate::db::catalog::CatalogKind::Table,
            ),
        )
        .unwrap()
        .with_current_database("memory");
        let mut draft = crate::model::catalog_editor::TableDraft::new_for_database(
            "main",
            DatabaseKind::DuckDb,
        );
        draft.name.set("plan_table");
        draft.columns[0].name.set("id");
        draft.columns[0].native_type.set("INTEGER");
        let plan = database
            .plan_catalog_mutation(
                request,
                crate::model::catalog_editor::CatalogDraft::Table(draft),
                None,
            )
            .unwrap();
        database.execute_catalog_mutation(&plan).await.unwrap();
        let entry = crate::db::catalog::CatalogEntry::relation(
            crate::db::catalog::CatalogId::new(
                profile.id,
                crate::db::catalog::CatalogKind::Table,
                ["memory", "main", "plan_table"],
            ),
            schema_id,
            crate::db::catalog::QualifiedName {
                database: Some("memory".into()),
                schema: Some("main".into()),
                object: "plan_table".into(),
            },
            "BASE TABLE",
            crate::db::catalog::OptionalMetadata::Unsupported,
            false,
        )
        .unwrap();
        let drop = database
            .plan_catalog_drop(
                crate::db::catalog_drop::CatalogDropRequest::new(connection, entry.id.clone(), 2),
                &entry,
            )
            .unwrap();
        assert_eq!(drop.sql(), "DROP TABLE \"main\".\"plan_table\"");
        database.execute_catalog_drop(&drop).await.unwrap();
        assert!(
            database
                .execute("SELECT * FROM main.plan_table")
                .await
                .is_err()
        );
        database.close().await;
    }

    #[tokio::test]
    async fn relation_mutations_insert_update_delete_and_detect_conflicts() {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            relation_mutation_scenario(),
        )
        .await
        .expect("DuckDB relation mutation scenario exceeded five seconds");
    }

    #[tokio::test]
    async fn readonly_file_connection_rejects_relation_mutation_before_transaction_start() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("readonly-mutation.duckdb");
        let profile = crate::profile::import_connection_url(
            &format!("duckdb://{}", path.display()),
            Some("readonly-mutation"),
        )
        .unwrap()
        .profile;
        let writer = DuckDbAdapter::connect(&profile).await.unwrap();
        writer
            .execute("CREATE TABLE protected (id INTEGER PRIMARY KEY, value VARCHAR); INSERT INTO protected VALUES (1, 'safe')")
            .await
            .unwrap();
        writer.close().await;

        let mut readonly_profile = profile.clone();
        readonly_profile.read_only = true;
        let readonly = DuckDbAdapter::connect(&readonly_profile).await.unwrap();
        let error = match readonly.transaction_backend().await {
            Ok(_) => panic!("read-only file must not create a mutation transaction worker"),
            Err(error) => error,
        };
        assert!(error.message.contains("writable connection"));
        let state = readonly
            .execute("SELECT value FROM protected WHERE id = 1")
            .await
            .unwrap();
        assert_eq!(
            state.result_sets[0].rows[0][0],
            crate::db::value::CellValue::Text("safe".into())
        );
        readonly.close().await;
    }

    async fn relation_mutation_scenario() {
        use crate::{
            db::{
                catalog::{CatalogId, CatalogKind},
                mutation::*,
            },
            identity::ConnectionIdentity,
            model::{execution_target::ExecutionTarget, relation::RelationKey},
        };
        let profile =
            crate::profile::import_connection_url("duckdb::memory:", Some("mutation-test"))
                .unwrap()
                .profile;
        let owner = DuckDbAdapter::connect(&profile).await.unwrap();
        owner.execute("CREATE TABLE editable (id INTEGER PRIMARY KEY, label VARCHAR NOT NULL); INSERT INTO editable VALUES (1, 'one')").await.unwrap();
        let mut transaction = owner
            .database_clone()
            .unwrap()
            .transaction_backend()
            .await
            .unwrap();
        transaction.begin().await.unwrap();
        let relation = CatalogId::new(
            profile.id,
            CatalogKind::Table,
            ["mutation-test", "main", "editable"],
        );
        let base = |operation| RelationMutationRequest {
            tab_id: uuid::Uuid::new_v4(),
            tab_generation: 1,
            edit_generation: 1,
            row_id: crate::model::relation_edit::EditableRowId(1),
            connection: ConnectionIdentity {
                profile_id: profile.id,
                generation: 1,
            },
            target: ExecutionTarget::from_profile(&profile),
            relation: relation.clone(),
            relation_key: RelationKey {
                profile_id: profile.id,
                object_id: relation.clone(),
            },
            scope: crate::profile::CatalogScope::for_profile(
                crate::profile::DatabaseKind::DuckDb,
                "mutation-test",
                Some("main"),
            ),
            metadata: MetadataFingerprint {
                relation: "editable".into(),
                columns: vec![
                    ("id".into(), "INTEGER".into(), false),
                    ("label".into(), "VARCHAR".into(), false),
                ],
                primary_key: vec!["id".into()],
            },
            operation,
        };
        let inserted = transaction
            .relation_mutation(base(RelationMutation::InsertRow(InsertRowMutation {
                columns: vec![0, 1],
                values: vec![
                    InputValue::Value(crate::db::value::CellValue::Integer(2)),
                    InputValue::Value(crate::db::value::CellValue::Text("two".into())),
                ],
            })))
            .await
            .unwrap();
        assert!(
            matches!(inserted, MutationResult::Inserted { ref row, .. } if row[0] == crate::db::value::CellValue::Integer(2))
        );
        let update = UpdateCellMutation {
            row: RowLocator {
                columns: vec![0],
                values: vec![crate::db::value::CellValue::Integer(2)],
            },
            column: 1,
            original: crate::db::value::CellValue::Text("two".into()),
            value: InputValue::Value(crate::db::value::CellValue::Text("updated".into())),
        };
        let updated = transaction
            .relation_mutation(base(RelationMutation::UpdateCell(update)))
            .await
            .unwrap();
        assert!(
            matches!(updated, MutationResult::Updated { ref row, .. } if row[1] == crate::db::value::CellValue::Text("updated".into()))
        );
        let conflict = UpdateCellMutation {
            row: RowLocator {
                columns: vec![0],
                values: vec![crate::db::value::CellValue::Integer(2)],
            },
            column: 1,
            original: crate::db::value::CellValue::Text("stale".into()),
            value: InputValue::Value(crate::db::value::CellValue::Text("lost".into())),
        };
        assert!(
            transaction
                .relation_mutation(base(RelationMutation::UpdateCell(conflict)))
                .await
                .is_err()
        );
        let delete = DeleteRowMutation {
            row_id: crate::model::relation_edit::EditableRowId(2),
            row: RowLocator {
                columns: vec![0],
                values: vec![crate::db::value::CellValue::Integer(2)],
            },
            original: vec![
                crate::db::value::CellValue::Integer(2),
                crate::db::value::CellValue::Text("updated".into()),
            ],
            version: None,
        };
        assert!(matches!(
            transaction
                .relation_mutation(base(RelationMutation::DeleteRows(vec![delete])))
                .await
                .unwrap(),
            MutationResult::Deleted { rows: 1 }
        ));
        transaction.rollback().await.unwrap();
        drop(transaction);

        let mut stale_transaction = owner
            .database_clone()
            .unwrap()
            .transaction_backend()
            .await
            .unwrap();
        stale_transaction.begin().await.unwrap();
        stale_transaction
            .execute("ALTER TABLE editable ADD COLUMN extra BOOLEAN")
            .await
            .unwrap();
        let stale_insert = stale_transaction
            .relation_mutation(base(RelationMutation::InsertRow(InsertRowMutation {
                columns: vec![0, 1],
                values: vec![
                    InputValue::Value(crate::db::value::CellValue::Integer(9)),
                    InputValue::Value(crate::db::value::CellValue::Text("stale".into())),
                ],
            })))
            .await;
        assert!(stale_insert.is_err());
        let missing_profile = {
            let mut request = base(RelationMutation::InsertRow(InsertRowMutation {
                columns: vec![0, 1],
                values: vec![
                    InputValue::Value(crate::db::value::CellValue::Integer(10)),
                    InputValue::Value(crate::db::value::CellValue::Text("wrong profile".into())),
                ],
            }));
            request.connection.profile_id = uuid::Uuid::new_v4();
            request
        };
        assert!(
            stale_transaction
                .relation_mutation(missing_profile)
                .await
                .is_err()
        );

        let outside_scope = {
            let mut request = base(RelationMutation::InsertRow(InsertRowMutation {
                columns: vec![0, 1],
                values: vec![
                    InputValue::Value(crate::db::value::CellValue::Integer(12)),
                    InputValue::Value(crate::db::value::CellValue::Text("outside scope".into())),
                ],
            }));
            request.scope = crate::profile::CatalogScope::for_profile(
                crate::profile::DatabaseKind::DuckDb,
                "other-catalog",
                Some("main"),
            );
            request
        };
        assert!(
            stale_transaction
                .relation_mutation(outside_scope)
                .await
                .is_err()
        );
        let unchanged = stale_transaction
            .execute("SELECT count(*) FROM editable WHERE id IN (9, 10, 12)")
            .await
            .unwrap();
        assert_eq!(
            unchanged.result_sets[0].rows[0][0],
            crate::db::value::CellValue::Integer(0)
        );
        stale_transaction.rollback().await.unwrap();
        drop(stale_transaction);

        owner.close().await;
    }

    #[tokio::test]
    async fn manual_transaction_clone_commits_and_rolls_back_shared_memory_database() {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            manual_transaction_clone_scenario(),
        )
        .await
        .expect("DuckDB transaction/session scenario exceeded five seconds");
    }

    async fn manual_transaction_clone_scenario() {
        let profile = crate::profile::import_connection_url("duckdb::memory:", Some("tx-test"))
            .unwrap()
            .profile;
        let owner = DuckDbAdapter::connect(&profile).await.unwrap();
        owner
            .execute("CREATE TABLE tx_values (id INTEGER)")
            .await
            .unwrap();

        let session = owner.database_clone().unwrap();
        session
            .execute("CREATE TEMP TABLE session_values (id INTEGER)")
            .await
            .unwrap();
        let mut transaction = session.transaction_backend().await.unwrap();
        transaction.begin().await.unwrap();
        transaction
            .execute("INSERT INTO tx_values VALUES (1)")
            .await
            .unwrap();
        transaction
            .execute("INSERT INTO session_values VALUES (10)")
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        assert_eq!(transaction.depth(), 0);
        drop(transaction);

        let session = owner.database_clone().unwrap();
        let mut transaction = session.transaction_backend().await.unwrap();
        transaction.begin().await.unwrap();
        transaction
            .execute("INSERT INTO tx_values VALUES (2)")
            .await
            .unwrap();
        transaction.rollback().await.unwrap();
        assert_eq!(transaction.depth(), 0);
        drop(transaction);

        let outcome = owner
            .execute("SELECT id FROM tx_values ORDER BY id")
            .await
            .unwrap();
        assert_eq!(outcome.result_sets[0].rows.len(), 1);
        assert_eq!(
            outcome.result_sets[0].rows[0][0],
            crate::db::value::CellValue::Integer(1)
        );
        let session = owner.database_clone().unwrap();
        session
            .execute("CREATE TEMP TABLE session_values (id INTEGER)")
            .await
            .unwrap();
        let mut transaction = session.transaction_backend().await.unwrap();
        transaction.begin().await.unwrap();
        transaction
            .execute("INSERT INTO session_values VALUES (10)")
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        // The backend owns the session lease until it is dropped, as in the
        // transaction worker. Release it before executing through this session.
        drop(transaction);
        let session_rows = session
            .execute("SELECT id FROM session_values")
            .await
            .unwrap();
        assert_eq!(
            session_rows.result_sets[0].rows[0][0],
            crate::db::value::CellValue::Integer(10)
        );
        assert!(owner.execute("SELECT * FROM session_values").await.is_err());
    }
}
