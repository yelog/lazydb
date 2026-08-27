use std::collections::BTreeSet;

use crate::db::mutation::RelationMutationRequest;
use crate::db::value::CellValue;
use crate::model::text_input::TextInput;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EditableRowId(pub u64);

#[derive(Clone, Debug, Default, PartialEq)]
pub enum RelationGridMode {
    #[default]
    Browse,
    EditCell(CellEditorState),
    VisualLine {
        anchor: usize,
    },
    Busy,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CellEditorState {
    pub row: usize,
    pub column: usize,
    pub input: TextInput,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditableRowState {
    Clean,
    Updated { changed_columns: BTreeSet<usize> },
    InsertDraft,
    Inserted,
    Deleted,
    Conflict { message: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditableRow {
    pub id: EditableRowId,
    pub original: Vec<CellValue>,
    pub current: Vec<CellValue>,
    pub state: EditableRowState,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RelationMutationHistory {
    pub forward: RelationMutationRequest,
    pub inverse: RelationMutationRequest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PendingMutationHistory {
    Undo,
    Redo,
}

impl EditableRow {
    pub fn new(id: EditableRowId, values: Vec<CellValue>) -> Self {
        Self {
            id,
            original: values.clone(),
            current: values,
            state: EditableRowState::Clean,
        }
    }

    pub fn update_cell(&mut self, column: usize, value: CellValue) -> bool {
        if matches!(self.state, EditableRowState::Deleted) {
            return false;
        }
        let Some(current) = self.current.get_mut(column) else {
            return false;
        };
        *current = value;
        if matches!(
            self.state,
            EditableRowState::InsertDraft | EditableRowState::Inserted
        ) {
            return true;
        }
        let changed_columns = self
            .current
            .iter()
            .enumerate()
            .filter_map(|(index, value)| (self.original.get(index) != Some(value)).then_some(index))
            .collect::<BTreeSet<_>>();
        self.state = if changed_columns.is_empty() {
            EditableRowState::Clean
        } else {
            EditableRowState::Updated { changed_columns }
        };
        true
    }

    pub fn mark_deleted(&mut self) -> bool {
        if matches!(self.state, EditableRowState::Deleted) {
            return false;
        }
        self.state = EditableRowState::Deleted;
        true
    }

    pub fn mark_inserted(&mut self, values: Vec<CellValue>) {
        self.current = values.clone();
        self.original = values;
        self.state = EditableRowState::Inserted;
    }

    pub fn mark_conflict(&mut self, message: impl Into<String>) {
        self.state = EditableRowState::Conflict {
            message: message.into(),
        };
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RelationEditSession {
    pub mode: RelationGridMode,
    pub rows: Vec<EditableRow>,
    pub yank: Option<Vec<CellValue>>,
    pub undo_depth: usize,
    pub redo_depth: usize,
    next_row_id: u64,
    undo: Vec<Vec<EditableRow>>,
    redo: Vec<Vec<EditableRow>>,
    pub mutation_undo: Vec<RelationMutationHistory>,
    pub mutation_redo: Vec<RelationMutationHistory>,
    pub pending_mutation_history: Option<PendingMutationHistory>,
}

impl RelationEditSession {
    pub fn from_rows(rows: Vec<Vec<CellValue>>) -> Self {
        let mut session = Self::default();
        session.rows = rows
            .into_iter()
            .map(|values| {
                let id = session.allocate_id();
                EditableRow::new(id, values)
            })
            .collect();
        session
    }

    pub fn allocate_id(&mut self) -> EditableRowId {
        self.next_row_id = self.next_row_id.saturating_add(1);
        EditableRowId(self.next_row_id)
    }

    pub fn visual_range(&self, cursor: usize) -> Option<(usize, usize)> {
        let RelationGridMode::VisualLine { anchor } = self.mode else {
            return None;
        };
        Some((anchor.min(cursor), anchor.max(cursor)))
    }

    pub fn insert_row(&mut self, position: usize, values: Vec<CellValue>) -> EditableRowId {
        self.record_change();
        let id = self.allocate_id();
        let position = position.min(self.rows.len());
        let mut row = EditableRow::new(id, values);
        row.state = EditableRowState::InsertDraft;
        self.rows.insert(position, row);
        id
    }

    pub fn yank_row(&mut self, row: usize) -> bool {
        let Some(row) = self.rows.get(row) else {
            return false;
        };
        self.yank = Some(row.current.clone());
        true
    }

    pub fn update_cell(&mut self, row: usize, column: usize, value: CellValue) -> bool {
        if self.rows.get(row).is_none() {
            return false;
        }
        self.record_change();
        self.rows[row].update_cell(column, value)
    }

    pub fn delete_rows(&mut self, range: std::ops::RangeInclusive<usize>) -> bool {
        let mut changed = false;
        self.record_change();
        for index in range {
            if let Some(row) = self.rows.get_mut(index) {
                changed |= row.mark_deleted();
            }
        }
        if !changed {
            self.undo.pop();
        }
        changed
    }

    pub fn paste_row(&mut self, position: usize) -> bool {
        let Some(values) = self.yank.clone() else {
            return false;
        };
        self.insert_row(position, values);
        true
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.rows.clone());
        self.rows = previous;
        self.sync_history_depth();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.rows.clone());
        self.rows = next;
        self.sync_history_depth();
        true
    }

    pub fn sync_history_depth(&mut self) {
        self.undo_depth = self.undo.len();
        self.redo_depth = self.redo.len();
    }

    pub fn record_mutation(&mut self, history: RelationMutationHistory) {
        self.mutation_undo.push(history);
        self.mutation_redo.clear();
    }

    pub fn pending_mutation(
        &mut self,
        direction: PendingMutationHistory,
    ) -> Option<RelationMutationRequest> {
        let request = match direction {
            PendingMutationHistory::Undo => self.mutation_undo.last()?.inverse.clone(),
            PendingMutationHistory::Redo => self.mutation_redo.last()?.forward.clone(),
        };
        self.pending_mutation_history = Some(direction);
        Some(request)
    }

    pub fn complete_mutation(&mut self) -> bool {
        let Some(direction) = self.pending_mutation_history.take() else {
            return false;
        };
        match direction {
            PendingMutationHistory::Undo => {
                let Some(history) = self.mutation_undo.pop() else {
                    return false;
                };
                self.mutation_redo.push(history);
            }
            PendingMutationHistory::Redo => {
                let Some(history) = self.mutation_redo.pop() else {
                    return false;
                };
                self.mutation_undo.push(history);
            }
        }
        true
    }

    fn record_change(&mut self) {
        self.undo.push(self.rows.clone());
        self.redo.clear();
        self.sync_history_depth();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EditableRow, EditableRowId, EditableRowState, PendingMutationHistory, RelationEditSession,
        RelationGridMode, RelationMutationHistory,
    };
    use crate::db::value::CellValue;

    fn row() -> EditableRow {
        EditableRow::new(
            EditableRowId(1),
            vec![CellValue::Integer(1), CellValue::Text("old".into())],
        )
    }

    #[test]
    fn changing_a_cell_marks_only_changed_columns() {
        let mut row = row();
        assert!(row.update_cell(1, CellValue::Text("new".into())));
        assert_eq!(
            row.state,
            EditableRowState::Updated {
                changed_columns: [1].into_iter().collect()
            }
        );
        assert!(row.update_cell(1, CellValue::Text("old".into())));
        assert_eq!(row.state, EditableRowState::Clean);
    }

    #[test]
    fn visual_line_range_is_inclusive_and_direction_independent() {
        let mut session = RelationEditSession::from_rows(vec![vec![CellValue::Integer(1)]; 5]);
        session.mode = RelationGridMode::VisualLine { anchor: 3 };
        assert_eq!(session.visual_range(1), Some((1, 3)));
        assert_eq!(session.visual_range(4), Some((3, 4)));
    }

    #[test]
    fn inserted_rows_receive_stable_ids_and_yank_is_structured() {
        let mut session = RelationEditSession::from_rows(vec![vec![CellValue::Integer(1)]]);
        let first = session.rows[0].id;
        let inserted = session.insert_row(0, vec![CellValue::Integer(2)]);
        assert_ne!(first, inserted);
        assert!(session.yank_row(0));
        assert_eq!(session.yank, Some(vec![CellValue::Integer(2)]));
        assert_ne!(session.rows[1].id, session.rows[0].id);
    }

    #[test]
    fn deleted_rows_cannot_be_updated_or_deleted_twice() {
        let mut row = row();
        assert!(row.mark_deleted());
        assert!(!row.mark_deleted());
        assert!(!row.update_cell(1, CellValue::Text("no".into())));
    }

    #[test]
    fn typed_mutation_history_moves_only_after_success() {
        let request = crate::db::mutation::RelationMutationRequest {
            tab_id: uuid::Uuid::nil(),
            tab_generation: 1,
            edit_generation: 1,
            row_id: EditableRowId(1),
            connection: crate::identity::ConnectionIdentity {
                profile_id: uuid::Uuid::nil(),
                generation: 1,
            },
            target: crate::model::execution_target::ExecutionTarget {
                profile_id: uuid::Uuid::nil(),
                database: "db".into(),
                schema: None,
            },
            relation: crate::db::catalog::CatalogId::new(
                uuid::Uuid::nil(),
                crate::db::catalog::CatalogKind::Table,
                ["db", "items"],
            ),
            relation_key: crate::model::relation::RelationKey {
                profile_id: uuid::Uuid::nil(),
                object_id: crate::db::catalog::CatalogId::new(
                    uuid::Uuid::nil(),
                    crate::db::catalog::CatalogKind::Table,
                    ["db", "items"],
                ),
            },
            scope: crate::profile::CatalogScope::for_profile(
                crate::profile::DatabaseKind::Sqlite,
                "db",
                None,
            ),
            metadata: crate::db::mutation::MetadataFingerprint {
                relation: "items".into(),
                columns: vec![("id".into(), "INTEGER".into(), false)],
                primary_key: vec!["id".into()],
            },
            operation: crate::db::mutation::RelationMutation::DeleteRows(Vec::new()),
        };
        let mut session = RelationEditSession::default();
        session.record_mutation(RelationMutationHistory {
            forward: request.clone(),
            inverse: request,
        });
        assert_eq!(session.mutation_undo.len(), 1);
        assert!(
            session
                .pending_mutation(PendingMutationHistory::Undo)
                .is_some()
        );
        assert_eq!(session.mutation_undo.len(), 1);
        assert!(session.complete_mutation());
        assert!(session.mutation_undo.is_empty());
        assert_eq!(session.mutation_redo.len(), 1);
    }
}
