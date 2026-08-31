#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecordViewState {
    pub selected_field: usize,
    pub field_offset: usize,
    pub visible_fields: usize,
}

impl RecordViewState {
    pub fn move_fields(&mut self, delta: isize, field_count: usize, viewport_rows: usize) {
        if field_count == 0 {
            self.selected_field = 0;
            self.field_offset = 0;
            return;
        }
        if viewport_rows == 0 {
            return;
        }
        self.selected_field = if delta.is_negative() {
            self.selected_field.saturating_sub(delta.unsigned_abs())
        } else {
            self.selected_field.saturating_add(delta as usize)
        }
        .min(field_count - 1);
        self.reveal_selection(field_count, viewport_rows);
    }

    pub fn jump_first(&mut self) {
        self.selected_field = 0;
        self.field_offset = 0;
    }

    pub fn jump_last(&mut self, field_count: usize, viewport_rows: usize) {
        if viewport_rows == 0 {
            return;
        }
        if field_count == 0 {
            self.jump_first();
            return;
        }
        self.selected_field = field_count - 1;
        self.reveal_selection(field_count, viewport_rows);
    }

    pub fn clamp(&mut self, field_count: usize, viewport_rows: usize) {
        self.visible_fields = viewport_rows;
        if field_count == 0 {
            self.jump_first();
            return;
        }
        self.selected_field = self.selected_field.min(field_count - 1);
        self.reveal_selection(field_count, viewport_rows);
    }

    fn reveal_selection(&mut self, field_count: usize, viewport_rows: usize) {
        if viewport_rows == 0 {
            return;
        }
        let visible_fields = viewport_rows.min(field_count);
        let max_offset = field_count.saturating_sub(visible_fields);
        self.field_offset = self.field_offset.min(max_offset);
        if self.selected_field < self.field_offset {
            self.field_offset = self.selected_field;
        } else if self.selected_field >= self.field_offset.saturating_add(visible_fields) {
            self.field_offset = self.selected_field + 1 - visible_fields;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RecordViewState;

    #[test]
    fn field_navigation_selects_fields_and_keeps_them_visible() {
        let mut state = RecordViewState::default();
        state.move_fields(4, 10, 3);
        assert_eq!(state.selected_field, 4);
        assert_eq!(state.field_offset, 2);

        state.move_fields(20, 10, 3);
        assert_eq!(state.selected_field, 9);
        assert_eq!(state.field_offset, 7);

        state.move_fields(-20, 10, 3);
        assert_eq!(state.selected_field, 0);
        assert_eq!(state.field_offset, 0);
    }

    #[test]
    fn navigation_changes_selection_when_all_fields_are_visible() {
        let mut state = RecordViewState::default();
        state.move_fields(1, 3, 10);
        assert_eq!(state.selected_field, 1);
        assert_eq!(state.field_offset, 0);

        state.jump_last(3, 10);
        assert_eq!(state.selected_field, 2);
        assert_eq!(state.field_offset, 0);

        state.jump_first();
        assert_eq!(state.selected_field, 0);
        assert_eq!(state.field_offset, 0);
    }

    #[test]
    fn zero_capacity_is_safe_and_clamp_handles_fewer_fields() {
        let mut state = RecordViewState::default();
        state.move_fields(5, 10, 0);
        assert_eq!(state.selected_field, 0);
        assert_eq!(state.field_offset, 0);

        state.jump_last(10, 3);
        assert_eq!(state.selected_field, 9);
        assert_eq!(state.field_offset, 7);
        state.jump_last(10, 0);
        assert_eq!(state.field_offset, 7);
        state.clamp(2, 3);
        assert_eq!(state.selected_field, 1);
        assert_eq!(state.field_offset, 0);

        state.clamp(0, 3);
        assert_eq!(state.selected_field, 0);
        assert_eq!(state.field_offset, 0);
    }
}
