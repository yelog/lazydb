#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecordViewState {
    pub field_offset: usize,
    pub visible_fields: usize,
}

impl RecordViewState {
    pub fn move_fields(&mut self, delta: isize, field_count: usize, viewport_rows: usize) {
        if field_count == 0 {
            self.field_offset = 0;
            return;
        }
        if viewport_rows == 0 {
            return;
        }
        let max_offset = field_count.saturating_sub(viewport_rows.min(field_count));
        self.field_offset = if delta.is_negative() {
            self.field_offset.saturating_sub(delta.unsigned_abs())
        } else {
            self.field_offset.saturating_add(delta as usize)
        }
        .min(max_offset);
    }

    pub fn jump_first(&mut self) {
        self.field_offset = 0;
    }

    pub fn jump_last(&mut self, field_count: usize, viewport_rows: usize) {
        if viewport_rows == 0 {
            return;
        }
        self.field_offset = field_count.saturating_sub(viewport_rows.min(field_count));
    }

    pub fn clamp(&mut self, field_count: usize, viewport_rows: usize) {
        self.visible_fields = viewport_rows;
        self.field_offset = self
            .field_offset
            .min(field_count.saturating_sub(viewport_rows.min(field_count)));
    }
}

#[cfg(test)]
mod tests {
    use super::RecordViewState;

    #[test]
    fn field_navigation_clamps_to_the_visible_range() {
        let mut state = RecordViewState::default();
        state.move_fields(4, 10, 3);
        assert_eq!(state.field_offset, 4);

        state.move_fields(20, 10, 3);
        assert_eq!(state.field_offset, 7);

        state.move_fields(-20, 10, 3);
        assert_eq!(state.field_offset, 0);
    }

    #[test]
    fn zero_capacity_is_safe_and_last_field_uses_the_viewport() {
        let mut state = RecordViewState::default();
        state.move_fields(5, 10, 0);
        assert_eq!(state.field_offset, 0);

        state.jump_last(10, 3);
        assert_eq!(state.field_offset, 7);
        state.jump_last(10, 0);
        assert_eq!(state.field_offset, 7);
        state.clamp(2, 3);
        assert_eq!(state.field_offset, 0);
    }
}
