use std::collections::BTreeSet;

use uuid::Uuid;

use crate::{
    identity::ConnectionIdentity,
    model::{execution_target::ExecutionTarget, relation::RelationKey},
    profile::{CatalogScope, DatabaseKind},
};

use super::{
    catalog::{CatalogId, CatalogKind, RelationDdl},
    query::ResultSet,
    value::CellValue,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataFingerprint {
    pub relation: String,
    pub columns: Vec<(String, String, bool)>,
    pub primary_key: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationColumnMapping {
    metadata_to_result: Vec<usize>,
    result_to_metadata: Vec<usize>,
}

impl RelationColumnMapping {
    pub fn metadata_to_result(&self) -> &[usize] {
        &self.metadata_to_result
    }

    pub fn result_to_metadata(&self) -> &[usize] {
        &self.result_to_metadata
    }

    pub fn result_column_for_metadata(&self, column: usize) -> Option<usize> {
        self.metadata_to_result.get(column).copied()
    }

    pub fn metadata_column_for_result(&self, column: usize) -> Option<usize> {
        self.result_to_metadata.get(column).copied()
    }
}

pub fn relation_column_mapping(
    metadata: &MetadataFingerprint,
    result_columns: &[String],
) -> Option<RelationColumnMapping> {
    let metadata_to_result = metadata
        .columns
        .iter()
        .map(|(name, _, _)| result_columns.iter().position(|column| column == name))
        .collect::<Option<Vec<_>>>()?;
    let mut result_to_metadata = vec![usize::MAX; result_columns.len()];
    for (metadata_column, result_column) in metadata_to_result.iter().copied().enumerate() {
        let slot = result_to_metadata.get_mut(result_column)?;
        if *slot != usize::MAX {
            return None;
        }
        *slot = metadata_column;
    }
    if result_to_metadata.contains(&usize::MAX) {
        return None;
    }
    Some(RelationColumnMapping {
        metadata_to_result,
        result_to_metadata,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InsertResultStrategy {
    Returning,
    Output,
    LookupByPrimaryKey,
    LookupByRowId,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MutationCapability<T> {
    Available(T),
    Unavailable(EditDisabledReason),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationMutationCapabilities {
    pub insert: MutationCapability<InsertResultStrategy>,
    pub update: MutationCapability<Vec<usize>>,
    pub delete: MutationCapability<Vec<usize>>,
}

impl RelationMutationCapabilities {
    pub fn allows_keyless_insert(&self) -> bool {
        matches!(
            self.insert,
            MutationCapability::Available(
                InsertResultStrategy::Returning
                    | InsertResultStrategy::Output
                    | InsertResultStrategy::LookupByRowId
            )
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditMetadata {
    pub fingerprint: MetadataFingerprint,
    pub primary_key_columns: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditDisabledReason {
    ReadOnlyConnection,
    NotATable,
    SnapshotNotLive,
    MissingDdl,
    MissingPrimaryKey,
    MissingPrimaryKeyColumn(String),
    GeneratedColumn(String),
    UnsupportedRowValue,
    UnsupportedInsertResultStrategy,
    UnsupportedDriver,
    UnsupportedDatabase,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditableRelationCapability {
    Editable(EditMetadata),
    ReadOnly(EditDisabledReason),
}

pub fn metadata_fingerprint(ddl: &RelationDdl) -> MetadataFingerprint {
    let columns = ddl
        .children
        .entries
        .iter()
        .filter_map(|entry| {
            let super::catalog::CatalogMetadata::Column(column) = &entry.metadata else {
                return None;
            };
            Some((
                entry.qualified_name.object.clone(),
                column.native_type.clone(),
                column.nullable,
            ))
        })
        .collect();
    let primary_key = ddl
        .children
        .entries
        .iter()
        .find_map(|entry| match &entry.metadata {
            super::catalog::CatalogMetadata::Constraint(
                super::catalog::ConstraintMetadata::PrimaryKey { columns },
            ) => Some(columns.clone()),
            _ => None,
        })
        .unwrap_or_default();
    MetadataFingerprint {
        relation: ddl.relation.qualified_name.object.clone(),
        columns,
        primary_key,
    }
}

pub fn relation_mutation_capabilities(
    kind: DatabaseKind,
    metadata: &MetadataFingerprint,
) -> RelationMutationCapabilities {
    if kind == DatabaseKind::CockroachDb {
        return RelationMutationCapabilities {
            insert: MutationCapability::Unavailable(EditDisabledReason::UnsupportedDriver),
            update: MutationCapability::Unavailable(EditDisabledReason::UnsupportedDriver),
            delete: MutationCapability::Unavailable(EditDisabledReason::UnsupportedDriver),
        };
    }
    if kind == DatabaseKind::TiDb {
        return RelationMutationCapabilities {
            insert: MutationCapability::Unavailable(EditDisabledReason::UnsupportedDatabase),
            update: MutationCapability::Unavailable(EditDisabledReason::UnsupportedDatabase),
            delete: MutationCapability::Unavailable(EditDisabledReason::UnsupportedDatabase),
        };
    }
    let primary_key_columns = metadata
        .primary_key
        .iter()
        .map(|name| {
            metadata
                .columns
                .iter()
                .position(|(column, _, _)| column == name)
        })
        .collect::<Option<Vec<_>>>();
    let existing_row_capability = match primary_key_columns {
        Some(columns) if !columns.is_empty() => MutationCapability::Available(columns),
        Some(_) | None => MutationCapability::Unavailable(EditDisabledReason::MissingPrimaryKey),
    };
    let insert_strategy = match kind {
        DatabaseKind::Postgres | DatabaseKind::MariaDb => InsertResultStrategy::Returning,
        DatabaseKind::SqlServer => InsertResultStrategy::Output,
        DatabaseKind::Sqlite => InsertResultStrategy::LookupByRowId,
        DatabaseKind::DuckDb if !metadata.primary_key.is_empty() => {
            InsertResultStrategy::LookupByPrimaryKey
        }
        DatabaseKind::MySql if !metadata.primary_key.is_empty() => {
            InsertResultStrategy::LookupByPrimaryKey
        }
        _ => InsertResultStrategy::Unsupported,
    };
    let insert = match insert_strategy {
        InsertResultStrategy::Unsupported => {
            MutationCapability::Unavailable(EditDisabledReason::UnsupportedInsertResultStrategy)
        }
        strategy => MutationCapability::Available(strategy),
    };
    RelationMutationCapabilities {
        insert,
        update: existing_row_capability.clone(),
        delete: existing_row_capability,
    }
}

pub fn editable_capability(
    kind: CatalogKind,
    read_only: bool,
    live: bool,
    ddl: Option<&RelationDdl>,
    result: &ResultSet,
) -> EditableRelationCapability {
    if read_only {
        return EditableRelationCapability::ReadOnly(EditDisabledReason::ReadOnlyConnection);
    }
    if kind != CatalogKind::Table {
        return EditableRelationCapability::ReadOnly(EditDisabledReason::NotATable);
    }
    if !live {
        return EditableRelationCapability::ReadOnly(EditDisabledReason::SnapshotNotLive);
    }
    let Some(ddl) = ddl else {
        return EditableRelationCapability::ReadOnly(EditDisabledReason::MissingDdl);
    };
    let fingerprint = metadata_fingerprint(ddl);
    if let Some(column) = ddl.children.entries.iter().find_map(|entry| {
        let super::catalog::CatalogMetadata::Column(metadata) = &entry.metadata else {
            return None;
        };
        let generated = metadata.identity
            == super::catalog::OptionalMetadata::Supported(Some(true))
            || metadata.generated_expression.is_supported()
                && matches!(
                    &metadata.generated_expression,
                    super::catalog::OptionalMetadata::Supported(Some(_))
                )
            || metadata.native_type.eq_ignore_ascii_case("rowversion")
            || metadata.native_type.eq_ignore_ascii_case("timestamp");
        generated.then(|| entry.qualified_name.object.clone())
    }) {
        return EditableRelationCapability::ReadOnly(EditDisabledReason::GeneratedColumn(column));
    }
    if fingerprint.primary_key.is_empty() {
        return EditableRelationCapability::ReadOnly(EditDisabledReason::MissingPrimaryKey);
    }
    let primary_key_columns = fingerprint
        .primary_key
        .iter()
        .map(|name| {
            result
                .columns
                .iter()
                .position(|column| column.name == *name)
                .ok_or_else(|| EditDisabledReason::MissingPrimaryKeyColumn(name.clone()))
        })
        .collect::<Result<Vec<_>, _>>();
    let Ok(primary_key_columns) = primary_key_columns else {
        return EditableRelationCapability::ReadOnly(primary_key_columns.unwrap_err());
    };
    if result
        .rows
        .iter()
        .flatten()
        .any(|value| matches!(value, CellValue::Unsupported { .. }))
    {
        return EditableRelationCapability::ReadOnly(EditDisabledReason::UnsupportedRowValue);
    }
    EditableRelationCapability::Editable(EditMetadata {
        fingerprint,
        primary_key_columns,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputValue {
    Value(CellValue),
    Null,
    Default,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RowLocator {
    pub columns: Vec<usize>,
    pub values: Vec<CellValue>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowVersion {
    PostgresXmin(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct UpdateCellMutation {
    pub row: RowLocator,
    pub column: usize,
    pub original: CellValue,
    pub value: InputValue,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeleteRowMutation {
    pub row_id: crate::model::relation_edit::EditableRowId,
    pub row: RowLocator,
    pub original: Vec<CellValue>,
    pub version: Option<RowVersion>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InsertRowMutation {
    /// Columns supplied by the draft. Columns not listed here are omitted so
    /// generated columns and database defaults can be evaluated by the server.
    pub columns: Vec<usize>,
    pub values: Vec<InputValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RelationMutation {
    UpdateCell(UpdateCellMutation),
    DeleteRows(Vec<DeleteRowMutation>),
    InsertRow(InsertRowMutation),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RelationMutationRequest {
    pub tab_id: Uuid,
    pub tab_generation: u64,
    pub edit_generation: u64,
    pub row_id: crate::model::relation_edit::EditableRowId,
    pub connection: ConnectionIdentity,
    pub target: ExecutionTarget,
    pub relation: CatalogId,
    pub relation_key: RelationKey,
    pub scope: CatalogScope,
    pub metadata: MetadataFingerprint,
    pub operation: RelationMutation,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MutationResult {
    Updated {
        row: Vec<CellValue>,
        version: Option<RowVersion>,
    },
    Deleted {
        rows: usize,
    },
    Inserted {
        row: Vec<CellValue>,
        version: Option<RowVersion>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangedColumns(pub BTreeSet<usize>);

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{
        EditableRelationCapability, InputValue, InsertResultStrategy, MetadataFingerprint,
        MutationCapability, editable_capability, metadata_fingerprint, relation_column_mapping,
        relation_mutation_capabilities,
    };
    use crate::{
        db::{
            catalog::{
                CatalogCount, CatalogEntry, CatalogId, CatalogKind, CatalogMetadata, CatalogPage,
                CatalogRequest, CatalogRequestKey, CatalogTarget, ColumnMetadata,
                ConstraintMetadata, DdlProvenance, OptionalMetadata, QualifiedName, RelationDdl,
            },
            query::{ColumnMeta, ResultSet},
            value::CellValue,
        },
        identity::ConnectionIdentity,
        profile::{CatalogScope, DatabaseKind},
    };

    #[test]
    fn input_values_keep_literal_and_sql_values_distinct() {
        assert_ne!(
            InputValue::Value(CellValue::Text("NULL".into())),
            InputValue::Null
        );
        assert_ne!(
            InputValue::Value(CellValue::Text("DEFAULT".into())),
            InputValue::Default
        );
    }

    #[test]
    fn metadata_fingerprint_is_ordered_and_comparable() {
        let left = MetadataFingerprint {
            relation: "users".into(),
            columns: vec![("id".into(), "integer".into(), false)],
            primary_key: vec!["id".into()],
        };
        assert_eq!(left, left.clone());
    }

    #[test]
    fn relation_ddl_metadata_fingerprint_preserves_columns_and_primary_key() {
        let ddl = relation_ddl_with_primary_key();

        assert_eq!(
            metadata_fingerprint(&ddl),
            MetadataFingerprint {
                relation: "users".into(),
                columns: vec![
                    ("id".into(), "integer".into(), false),
                    ("name".into(), "text".into(), true),
                ],
                primary_key: vec!["id".into()],
            }
        );
    }

    #[test]
    fn relation_with_primary_key_metadata_is_editable() {
        let ddl = relation_ddl_with_primary_key();
        let result = ResultSet {
            columns: vec![
                ColumnMeta {
                    name: "id".into(),
                    type_name: "integer".into(),
                },
                ColumnMeta {
                    name: "name".into(),
                    type_name: "text".into(),
                },
            ],
            rows: vec![vec![CellValue::Integer(1), CellValue::Text("Ada".into())]],
            affected_rows: 0,
        };

        assert_eq!(
            editable_capability(CatalogKind::Table, false, true, Some(&ddl), &result),
            EditableRelationCapability::Editable(super::EditMetadata {
                fingerprint: metadata_fingerprint(&ddl),
                primary_key_columns: vec![0],
            })
        );
    }

    #[test]
    fn cockroach_grid_mutations_remain_disabled_even_with_a_primary_key() {
        for primary_key in [Vec::new(), vec!["id".into()]] {
            let metadata = MetadataFingerprint {
                relation: "items".into(),
                columns: vec![("id".into(), "integer".into(), false)],
                primary_key,
            };
            let capabilities = relation_mutation_capabilities(DatabaseKind::CockroachDb, &metadata);
            assert_eq!(
                capabilities.insert,
                MutationCapability::Unavailable(super::EditDisabledReason::UnsupportedDriver)
            );
            assert_eq!(
                capabilities.update,
                MutationCapability::Unavailable(super::EditDisabledReason::UnsupportedDriver)
            );
            assert_eq!(
                capabilities.delete,
                MutationCapability::Unavailable(super::EditDisabledReason::UnsupportedDriver)
            );
            assert!(!capabilities.allows_keyless_insert());
        }
    }

    #[test]
    fn mariadb_keyless_insert_is_available_but_existing_row_mutations_are_not() {
        let metadata = MetadataFingerprint {
            relation: "test1".into(),
            columns: vec![
                ("name".into(), "text".into(), true),
                ("id".into(), "text".into(), true),
            ],
            primary_key: Vec::new(),
        };
        let capabilities = relation_mutation_capabilities(DatabaseKind::MariaDb, &metadata);
        assert_eq!(
            capabilities.insert,
            MutationCapability::Available(InsertResultStrategy::Returning)
        );
        assert!(capabilities.allows_keyless_insert());
        assert!(matches!(
            capabilities.update,
            MutationCapability::Unavailable(super::EditDisabledReason::MissingPrimaryKey)
        ));
        assert!(matches!(
            capabilities.delete,
            MutationCapability::Unavailable(super::EditDisabledReason::MissingPrimaryKey)
        ));
    }

    #[test]
    fn insert_result_strategy_does_not_make_mysql_keyless_insert_look_supported() {
        let metadata = MetadataFingerprint {
            relation: "test1".into(),
            columns: vec![("value".into(), "text".into(), true)],
            primary_key: Vec::new(),
        };
        let capabilities = relation_mutation_capabilities(DatabaseKind::MySql, &metadata);
        assert!(!capabilities.allows_keyless_insert());
        assert!(matches!(
            capabilities.insert,
            MutationCapability::Unavailable(
                super::EditDisabledReason::UnsupportedInsertResultStrategy
            )
        ));
    }

    #[test]
    fn relation_column_mapping_handles_reordered_result_columns() {
        let metadata = MetadataFingerprint {
            relation: "items".into(),
            columns: vec![
                ("name".into(), "text".into(), true),
                ("id".into(), "integer".into(), false),
            ],
            primary_key: vec!["id".into()],
        };
        let mapping = relation_column_mapping(&metadata, &["id".into(), "name".into()]).unwrap();
        assert_eq!(mapping.metadata_to_result(), &[1, 0]);
        assert_eq!(mapping.result_to_metadata(), &[1, 0]);
        assert_eq!(mapping.result_column_for_metadata(0), Some(1));
        assert_eq!(mapping.metadata_column_for_result(0), Some(1));
    }

    fn relation_ddl_with_primary_key() -> RelationDdl {
        let profile_id = Uuid::new_v4();
        let relation_id = CatalogId::new(profile_id, CatalogKind::Table, ["db", "public", "users"]);
        let schema_id = CatalogId::new(profile_id, CatalogKind::Schema, ["db", "public"]);
        let qualified_name = QualifiedName {
            database: Some("db".into()),
            schema: Some("public".into()),
            object: "users".into(),
        };
        let relation = CatalogEntry::relation(
            relation_id.clone(),
            schema_id,
            qualified_name.clone(),
            "table",
            OptionalMetadata::Unsupported,
            true,
        )
        .unwrap();
        let entries = vec![
            CatalogEntry::relation_child(
                CatalogId::new(
                    profile_id,
                    CatalogKind::Column,
                    ["db", "public", "users", "id"],
                ),
                relation_id.clone(),
                QualifiedName {
                    object: "id".into(),
                    ..qualified_name.clone()
                },
                "integer",
                OptionalMetadata::Unsupported,
                CatalogMetadata::Column(ColumnMetadata::new(1, "integer", false)),
            )
            .unwrap(),
            CatalogEntry::relation_child(
                CatalogId::new(
                    profile_id,
                    CatalogKind::Column,
                    ["db", "public", "users", "name"],
                ),
                relation_id.clone(),
                QualifiedName {
                    object: "name".into(),
                    ..qualified_name.clone()
                },
                "text",
                OptionalMetadata::Unsupported,
                CatalogMetadata::Column(ColumnMetadata::new(2, "text", true)),
            )
            .unwrap(),
            CatalogEntry::relation_child(
                CatalogId::new(
                    profile_id,
                    CatalogKind::PrimaryKey,
                    ["db", "public", "users", "users_pkey"],
                ),
                relation_id.clone(),
                QualifiedName {
                    object: "users_pkey".into(),
                    ..qualified_name
                },
                "primary_key",
                OptionalMetadata::Unsupported,
                CatalogMetadata::Constraint(ConstraintMetadata::PrimaryKey {
                    columns: vec!["id".into()],
                }),
            )
            .unwrap(),
        ];
        let request = CatalogRequest {
            key: CatalogRequestKey {
                connection: ConnectionIdentity {
                    profile_id,
                    generation: 1,
                },
                catalog_epoch: 1,
                request_id: 1,
                target: CatalogTarget::RelationChildren {
                    relation: relation_id,
                },
                cursor: None,
            },
            scope: CatalogScope::for_profile(DatabaseKind::Postgres, "db", None),
            page_size: 100,
        };
        let children = CatalogPage::new(&request, entries, CatalogCount::Exact(3), None).unwrap();
        RelationDdl {
            relation,
            children,
            sql: "CREATE TABLE users (id integer PRIMARY KEY, name text)".into(),
            provenance: DdlProvenance::NativeCatalog,
        }
    }
}
