use std::collections::BTreeSet;

use uuid::Uuid;

use crate::{
    identity::ConnectionIdentity,
    model::{execution_target::ExecutionTarget, relation::RelationKey},
    profile::CatalogScope,
};

use super::{
    catalog::{CatalogId, CatalogKind, RelationStructure},
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
pub struct EditMetadata {
    pub fingerprint: MetadataFingerprint,
    pub primary_key_columns: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditDisabledReason {
    ReadOnlyConnection,
    NotATable,
    SnapshotNotLive,
    MissingStructure,
    MissingPrimaryKey,
    MissingPrimaryKeyColumn(String),
    UnsupportedRowValue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditableRelationCapability {
    Editable(EditMetadata),
    ReadOnly(EditDisabledReason),
}

pub fn metadata_fingerprint(structure: &RelationStructure) -> MetadataFingerprint {
    let columns = structure
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
    let primary_key = structure
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
        relation: structure.relation.qualified_name.object.clone(),
        columns,
        primary_key,
    }
}

pub fn editable_capability(
    kind: CatalogKind,
    read_only: bool,
    live: bool,
    structure: Option<&RelationStructure>,
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
    let Some(structure) = structure else {
        return EditableRelationCapability::ReadOnly(EditDisabledReason::MissingStructure);
    };
    let fingerprint = metadata_fingerprint(structure);
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputValue {
    Value(String),
    Null,
    Default,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RowLocator {
    pub columns: Vec<usize>,
    pub values: Vec<CellValue>,
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
    pub row: RowLocator,
    pub original: Vec<CellValue>,
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
    Updated { row: Vec<CellValue> },
    Deleted { rows: usize },
    Inserted { row: Vec<CellValue> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangedColumns(pub BTreeSet<usize>);

#[cfg(test)]
mod tests {
    use super::{InputValue, MetadataFingerprint};

    #[test]
    fn input_values_keep_literal_and_sql_values_distinct() {
        assert_ne!(InputValue::Value("NULL".into()), InputValue::Null);
        assert_ne!(InputValue::Value("DEFAULT".into()), InputValue::Default);
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
}
