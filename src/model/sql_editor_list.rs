use uuid::Uuid;

use crate::model::text_input::TextInput;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum SqlEditorListMode {
    #[default]
    Browse,
    Search,
    Rename {
        console_id: Uuid,
        input: TextInput,
        error: Option<String>,
    },
    DeleteConfirm {
        console_id: Uuid,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SqlEditorListState {
    pub query: TextInput,
    pub selected_id: Option<Uuid>,
    pub mode: SqlEditorListMode,
}

impl SqlEditorListState {
    pub fn new(selected_id: Option<Uuid>) -> Self {
        Self {
            selected_id,
            ..Self::default()
        }
    }

    pub fn visible_query(&self) -> &str {
        self.query.value()
    }

    pub fn insert(&mut self, value: char) {
        self.input_mut().insert(value);
    }

    pub fn backspace(&mut self) {
        self.input_mut().backspace();
    }

    pub fn delete_previous_word(&mut self) {
        self.input_mut().delete_previous_word();
    }

    pub fn delete_to_start(&mut self) {
        self.input_mut().delete_to_start();
    }

    pub fn delete(&mut self) {
        self.input_mut().delete();
    }

    pub fn move_left(&mut self) {
        self.input_mut().move_left();
    }

    pub fn move_right(&mut self) {
        self.input_mut().move_right();
    }

    pub fn move_home(&mut self) {
        self.input_mut().move_home();
    }

    pub fn move_end(&mut self) {
        self.input_mut().move_end();
    }

    pub fn undo(&mut self) {
        self.input_mut().undo();
    }

    pub fn redo(&mut self) {
        self.input_mut().redo();
    }

    fn input_mut(&mut self) -> &mut TextInput {
        match &mut self.mode {
            SqlEditorListMode::Rename { input, .. } => input,
            SqlEditorListMode::Browse
            | SqlEditorListMode::Search
            | SqlEditorListMode::DeleteConfirm { .. } => &mut self.query,
        }
    }

    pub fn move_selection(&mut self, delta: isize, visible_ids: &[Uuid]) {
        self.input_mut().finish_edit_group();
        if visible_ids.is_empty() {
            self.reconcile_selection(visible_ids);
            return;
        }
        self.reconcile_selection(visible_ids);

        let current = visible_ids
            .iter()
            .position(|id| Some(*id) == self.selected_id)
            .unwrap_or(0);
        let next = if delta == isize::MIN {
            0
        } else if delta == isize::MAX {
            visible_ids.len() - 1
        } else if delta < 0 {
            (current + visible_ids.len() - delta.unsigned_abs() % visible_ids.len())
                % visible_ids.len()
        } else {
            (current + delta as usize) % visible_ids.len()
        };
        self.selected_id = Some(visible_ids[next]);
    }

    pub fn reconcile_selection(&mut self, visible_ids: &[Uuid]) {
        if visible_ids.is_empty() {
            self.selected_id = None;
            return;
        }
        if visible_ids
            .iter()
            .position(|id| Some(*id) == self.selected_id)
            .is_some()
        {
        } else {
            self.selected_id = Some(visible_ids[0]);
        }
    }

    pub fn start_search(&mut self) {
        self.mode = SqlEditorListMode::Search;
    }

    /// Returns true when the manager was already in Browse mode and should close.
    pub fn cancel_mode(&mut self) -> bool {
        match self.mode {
            SqlEditorListMode::Browse => true,
            SqlEditorListMode::Search => {
                self.query = TextInput::default();
                self.mode = SqlEditorListMode::Browse;
                false
            }
            SqlEditorListMode::Rename { .. } | SqlEditorListMode::DeleteConfirm { .. } => {
                self.mode = SqlEditorListMode::Browse;
                false
            }
        }
    }

    pub fn matches(name: &str, query: &str) -> bool {
        name.to_lowercase().contains(&query.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_list_starts_in_browse_mode_and_tracks_selection_by_id() {
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        let mut state = SqlEditorListState::new(Some(second));

        assert_eq!(state.mode, SqlEditorListMode::Browse);
        assert_eq!(state.selected_id, Some(second));

        state.move_selection(1, &[first, second]);
        assert_eq!(state.selected_id, Some(first));
        state.move_selection(-1, &[first, second]);
        assert_eq!(state.selected_id, Some(second));
    }

    #[test]
    fn reconcile_selection_handles_empty_and_non_visible_ids() {
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        let mut state = SqlEditorListState::new(Some(Uuid::from_u128(9)));

        state.reconcile_selection(&[first, second]);
        assert_eq!(state.selected_id, Some(first));
        state.reconcile_selection(&[]);
        assert_eq!(state.selected_id, None);
    }

    #[test]
    fn selection_wraps_up_from_first_item() {
        let first = Uuid::from_u128(1);
        let last = Uuid::from_u128(3);
        let mut state = SqlEditorListState::new(Some(first));

        state.move_selection(-1, &[first, Uuid::from_u128(2), last]);
        assert_eq!(state.selected_id, Some(last));
    }

    #[test]
    fn cancelling_search_clears_query_before_closing_the_manager() {
        let mut state = SqlEditorListState::default();
        state.query.set("backup");
        state.start_search();

        assert!(!state.cancel_mode());
        assert_eq!(state.mode, SqlEditorListMode::Browse);
        assert_eq!(state.visible_query(), "");
    }

    #[test]
    fn cancelling_rename_returns_to_browse_without_clearing_search() {
        let id = Uuid::from_u128(1);
        let mut state = SqlEditorListState::default();
        state.query.set("backup");
        state.mode = SqlEditorListMode::Rename {
            console_id: id,
            input: TextInput::from("renamed"),
            error: None,
        };

        assert!(!state.cancel_mode());
        assert_eq!(state.mode, SqlEditorListMode::Browse);
        assert_eq!(state.visible_query(), "backup");
    }

    #[test]
    fn list_selection_wraps_and_filters_case_insensitively() {
        let mut state = SqlEditorListState::default();
        state.insert('B');
        assert!(SqlEditorListState::matches("backup", state.visible_query()));
        assert!(!SqlEditorListState::matches(
            "console",
            state.visible_query()
        ));
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        state.selected_id = Some(first);
        state.move_selection(-1, &[first, second]);
        assert_eq!(state.selected_id, Some(second));
        state.move_selection(1, &[first, second]);
        assert_eq!(state.selected_id, Some(first));
    }

    #[test]
    fn input_undo_and_redo_restore_the_active_search_text() {
        let mut state = SqlEditorListState::new(None);
        state.start_search();
        state.insert('a');
        state.insert('b');
        state.undo();
        assert_eq!(state.visible_query(), "");
        state.redo();
        assert_eq!(state.visible_query(), "ab");
    }
}
