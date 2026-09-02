use std::collections::BTreeSet;

use uuid::Uuid;

use crate::{
    identity::ConnectionIdentity,
    model::{execution_target::ExecutionTarget, relation::RelationKey},
    profile::CatalogScope,
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
    use uuid::Uuid;

    use super::{
        EditableRelationCapability, InputValue, MetadataFingerprint, editable_capability,
        metadata_fingerprint,
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
