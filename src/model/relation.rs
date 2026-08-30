use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use super::relation_edit::RelationEditSession;
use super::transaction::TransactionState;
use super::{data_query::DataQueryOptions, data_query::DataQueryState, tab::DataGridState};
use crate::db::{
    RelationPreview,
    catalog::{CatalogId, CatalogKind, QualifiedName, RelationDdl},
};
use crate::identity::ConnectionIdentity;
use crate::profile::CatalogScope;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RelationKey {
    pub profile_id: Uuid,
    pub object_id: CatalogId,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationView {
    #[default]
    Data,
    Ddl,
}

pub type RelationPreviewOptions = DataQueryOptions;
pub type RelationQueryInput = crate::model::data_query::DataQueryInput;
pub type RelationQueryState = DataQueryState;

pub fn automatic_relation_column_widths(result: &crate::db::query::ResultSet) -> Vec<u16> {
    result
        .columns
        .iter()
        .enumerate()
        .map(|(column_index, column)| {
            let header = crate::security::sanitize_terminal_text(&column.name);
            let content = result
                .rows
                .iter()
                .filter_map(|row| row.get(column_index))
                .map(|value| value.preview(40).text)
                .map(|text| UnicodeWidthStr::width(text.as_str()))
                .max()
                .unwrap_or(0);
            (UnicodeWidthStr::width(header.as_str()).max(content) + 2).clamp(6, 40) as u16
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RelationRequestKind {
    Preview,
    Ddl,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RelationRequest {
    pub tab_id: Uuid,
    pub tab_generation: u64,
    pub request_id: u64,
    pub connection: ConnectionIdentity,
    pub relation: RelationKey,
    pub kind: RelationRequestKind,
    pub scope: CatalogScope,
    pub options: RelationPreviewOptions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelationSnapshotProvenance {
    Live,
    OfflineSnapshot,
    ProfileDeletedSnapshot,
    OutOfScopeSnapshot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotAttribution {
    pub connection: ConnectionIdentity,
    pub profile_id: Uuid,
    pub scope: CatalogScope,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OwnedSnapshot<T> {
    pub value: T,
    pub attribution: SnapshotAttribution,
}

impl<T> OwnedSnapshot<T> {
    pub fn new(value: T, connection: ConnectionIdentity, scope: CatalogScope) -> Self {
        Self {
            value,
            attribution: SnapshotAttribution {
                connection,
                profile_id: connection.profile_id,
                scope,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RelationLoad<T> {
    Empty,
    Loading {
        request: RelationRequest,
        previous: Option<OwnedSnapshot<T>>,
    },
    Ready(OwnedSnapshot<T>),
    Failed {
        message: String,
        previous: Option<OwnedSnapshot<T>>,
    },
    Cancelled {
        previous: Option<OwnedSnapshot<T>>,
    },
}

pub type RelationPreviewLoad = RelationLoad<RelationPreview>;
pub type RelationDdlLoad = RelationLoad<RelationDdl>;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DdlViewportState {
    pub row_offset: usize,
    pub column_offset: usize,
    pub visible_rows: usize,
    pub visible_columns: usize,
    pub total_rows: usize,
    pub max_line_width: usize,
}

impl DdlViewportState {
    pub fn max_row_offset(&self) -> usize {
        self.total_rows.saturating_sub(self.visible_rows)
    }

    pub fn max_column_offset(&self) -> usize {
        self.max_line_width.saturating_sub(self.visible_columns)
    }

    pub fn clamp(&mut self) {
        self.row_offset = self.row_offset.min(self.max_row_offset());
        self.column_offset = self.column_offset.min(self.max_column_offset());
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RelationSnapshot {
    Preview(RelationPreview),
    Ddl(Box<RelationDdl>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationDescriptor {
    pub key: RelationKey,
    pub qualified_name: QualifiedName,
    pub kind: CatalogKind,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RelationTab {
    pub id: Uuid,
    pub generation: u64,
    pub next_request_id: u64,
    pub descriptor: RelationDescriptor,
    pub view: RelationView,
    pub data: RelationPreviewLoad,
    pub ddl: RelationDdlLoad,
    pub ddl_viewport: DdlViewportState,
    pub grid: DataGridState,
    pub query: RelationQueryState,
    pub edit: Option<RelationEditSession>,
    pub transaction_state: TransactionState,
    pub transaction_generation: u64,
    pub transaction_snapshot: Option<RelationEditSession>,
}

impl RelationTab {
    pub fn provenance(
        &self,
        view: RelationView,
        active: Option<ConnectionIdentity>,
        profile: Option<&crate::profile::ConnectionProfile>,
    ) -> Option<RelationSnapshotProvenance> {
        let snapshot_connection = match view {
            RelationView::Data => relation_snapshot(&self.data)?.attribution.connection,
            RelationView::Ddl => relation_snapshot(&self.ddl)?.attribution.connection,
        };
        let Some(profile) = profile else {
            return Some(RelationSnapshotProvenance::ProfileDeletedSnapshot);
        };
        let in_scope = self
            .descriptor
            .qualified_name
            .database
            .as_deref()
            .is_none_or(|database| profile.catalog_scope.allows_database(database))
            && self
                .descriptor
                .qualified_name
                .schema
                .as_deref()
                .is_none_or(|schema| {
                    self.descriptor
                        .qualified_name
                        .database
                        .as_deref()
                        .is_none_or(|database| {
                            profile.catalog_scope.allows_schema(database, schema)
                        })
                });
        if !in_scope {
            Some(RelationSnapshotProvenance::OutOfScopeSnapshot)
        } else if active == Some(snapshot_connection) {
            Some(RelationSnapshotProvenance::Live)
        } else {
            Some(RelationSnapshotProvenance::OfflineSnapshot)
        }
    }
}

fn relation_snapshot<T>(load: &RelationLoad<T>) -> Option<&OwnedSnapshot<T>> {
    match load {
        RelationLoad::Ready(snapshot) => Some(snapshot),
        RelationLoad::Loading { previous, .. }
        | RelationLoad::Failed { previous, .. }
        | RelationLoad::Cancelled { previous } => previous.as_ref(),
        RelationLoad::Empty => None,
    }
}

impl RelationTab {
    pub fn new(title: impl Into<String>) -> Self {
        let title = title.into();
        let profile_id = Uuid::nil();
        let object_id = CatalogId::new(profile_id, CatalogKind::Table, [title.clone()]);
        Self::with_descriptor(
            RelationDescriptor {
                key: RelationKey {
                    profile_id,
                    object_id,
                },
                qualified_name: QualifiedName {
                    database: None,
                    schema: None,
                    object: title.clone(),
                },
                kind: CatalogKind::Table,
                title,
            },
            RelationView::Data,
        )
    }

    pub fn with_descriptor(descriptor: RelationDescriptor, view: RelationView) -> Self {
        Self {
            id: Uuid::new_v4(),
            generation: 0,
            next_request_id: 1,
            descriptor,
            view,
            data: RelationLoad::Empty,
            ddl: RelationLoad::Empty,
            ddl_viewport: DdlViewportState::default(),
            grid: DataGridState::default(),
            query: RelationQueryState {
                capability: crate::model::data_query::DataQueryCapability::Relation,
                ..RelationQueryState::default()
            },
            edit: None,
            transaction_state: TransactionState::Idle,
            transaction_generation: 0,
            transaction_snapshot: None,
        }
    }

    pub fn restored(id: Uuid, descriptor: RelationDescriptor, view: RelationView) -> Self {
        let mut tab = Self::with_descriptor(descriptor, view);
        tab.id = id;
        tab
    }

    pub fn title(&self) -> &str {
        &self.descriptor.title
    }
}
