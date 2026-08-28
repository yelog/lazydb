#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SqlEditorListState {
    pub query: String,
    pub selected: usize,
}

impl SqlEditorListState {
    pub fn insert(&mut self, value: char) {
        self.query.push(value);
        self.selected = 0;
    }
    pub fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
    }
    pub fn move_selection(&mut self, delta: isize, count: usize) {
        if count == 0 {
            self.selected = 0;
            return;
        }
        self.selected = if delta < 0 {
            self.selected
                .checked_sub(delta.unsigned_abs())
                .unwrap_or(count - 1)
        } else {
            (self.selected + delta as usize) % count
        };
    }

    pub fn matches(name: &str, query: &str) -> bool {
        name.to_lowercase().contains(&query.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_selection_wraps_and_filters_case_insensitively() {
        let mut state = SqlEditorListState::default();
        state.insert('B');
        assert!(SqlEditorListState::matches("backup", &state.query));
        assert!(!SqlEditorListState::matches("console", &state.query));
        state.move_selection(-1, 2);
        assert_eq!(state.selected, 1);
        state.move_selection(1, 2);
        assert_eq!(state.selected, 0);
    }
}
