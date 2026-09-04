#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextInputEdit {
    Insert(char),
    Backspace,
    DeletePreviousWord,
    DeleteToStart,
    Clear,
    Delete,
    MoveLeft,
    MoveRight,
    MoveHome,
    MoveEnd,
    Undo,
    Redo,
}

const HISTORY_LIMIT: usize = 100;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Snapshot {
    value: String,
    cursor: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EditGroup {
    Insert,
    Backspace,
    Delete,
}

#[derive(Clone, Debug, Default)]
struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    group: Option<(EditGroup, Snapshot)>,
}

#[derive(Clone, Debug, Default)]
pub struct TextInput {
    value: String,
    cursor: usize,
    history: History,
}

impl PartialEq for TextInput {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value && self.cursor == other.cursor
    }
}

impl Eq for TextInput {}

impl History {
    fn push_undo(&mut self, snapshot: Snapshot) {
        if self.undo.last() == Some(&snapshot) {
            return;
        }
        if self.undo.len() == HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(snapshot);
    }

    fn push_redo(&mut self, snapshot: Snapshot) {
        if self.redo.len() == HISTORY_LIMIT {
            self.redo.remove(0);
        }
        self.redo.push(snapshot);
    }
}

impl TextInput {
    pub fn apply(&mut self, edit: TextInputEdit) -> bool {
        let before = self.snapshot();
        match edit {
            TextInputEdit::Insert(character) => self.insert(character),
            TextInputEdit::Backspace => self.backspace(),
            TextInputEdit::DeletePreviousWord => self.delete_previous_word(),
            TextInputEdit::DeleteToStart => self.delete_to_start(),
            TextInputEdit::Clear => self.clear(),
            TextInputEdit::Delete => self.delete(),
            TextInputEdit::MoveLeft => self.move_left(),
            TextInputEdit::MoveRight => self.move_right(),
            TextInputEdit::MoveHome => self.move_home(),
            TextInputEdit::MoveEnd => self.move_end(),
            TextInputEdit::Undo => return self.undo(),
            TextInputEdit::Redo => return self.redo(),
        }
        self.snapshot() != before
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn set(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.cursor = self.value.chars().count();
        self.history = History::default();
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn insert(&mut self, character: char) {
        let before = self.snapshot();
        let byte_index = self.byte_index(self.cursor);
        self.value.insert(byte_index, character);
        self.cursor += 1;
        self.record_grouped(EditGroup::Insert, before);
    }

    pub fn paste(&mut self, text: impl AsRef<str>) {
        let text = text.as_ref();
        if text.is_empty() {
            return;
        }
        let before = self.snapshot();
        let byte_index = self.byte_index(self.cursor);
        self.value.insert_str(byte_index, text);
        self.cursor += text.chars().count();
        self.record_atomic(before);
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }

        let before = self.snapshot();
        let start = self.byte_index(self.cursor - 1);
        let end = self.byte_index(self.cursor);
        self.value.replace_range(start..end, "");
        self.cursor -= 1;
        self.record_grouped(EditGroup::Backspace, before);
    }

    pub fn delete_previous_word(&mut self) {
        let before = self.snapshot();
        let mut start = self.cursor;
        while start > 0
            && self.value[..self.byte_index(start)]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
        {
            start -= 1;
        }
        while start > 0
            && !self.value[..self.byte_index(start)]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
        {
            start -= 1;
        }
        if start == self.cursor {
            return;
        }
        let byte_start = self.byte_index(start);
        let byte_end = self.byte_index(self.cursor);
        self.value.replace_range(byte_start..byte_end, "");
        self.cursor = start;
        self.record_atomic(before);
    }

    pub fn delete_to_start(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let before = self.snapshot();
        let end = self.byte_index(self.cursor);
        self.value.replace_range(..end, "");
        self.cursor = 0;
        self.record_atomic(before);
    }

    pub fn clear(&mut self) {
        if self.value.is_empty() {
            return;
        }
        let before = self.snapshot();
        self.value.clear();
        self.cursor = 0;
        self.record_atomic(before);
    }

    pub fn delete(&mut self) {
        let start = self.byte_index(self.cursor);
        if start == self.value.len() {
            return;
        }

        let before = self.snapshot();
        let end = self.byte_index(self.cursor + 1);
        self.value.replace_range(start..end, "");
        self.record_grouped(EditGroup::Delete, before);
    }

    pub fn move_left(&mut self) {
        self.finish_edit_group();
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        self.finish_edit_group();
        self.cursor = (self.cursor + 1).min(self.value.chars().count());
    }

    pub fn move_home(&mut self) {
        self.finish_edit_group();
        self.cursor = 0;
    }

    pub fn move_end(&mut self) {
        self.finish_edit_group();
        self.cursor = self.value.chars().count();
    }

    pub fn replace(&mut self, range: crate::sql::TextRange, replacement: &str) {
        let start = self.byte_index(range.start.min(self.value.chars().count()));
        let end = self.byte_index(range.end.min(self.value.chars().count()));
        if start > end {
            return;
        }
        let before = self.snapshot();
        self.value.replace_range(start..end, replacement);
        self.cursor = range.start + replacement.chars().count();
        if self.snapshot() != before {
            self.record_atomic(before);
        }
    }

    pub(crate) fn edit_string(&mut self, edit: impl FnOnce(&mut String)) -> bool {
        let before = self.snapshot();
        let mut value = self.value.clone();
        edit(&mut value);
        if value == self.value {
            return false;
        }
        self.value = value;
        self.cursor = self.value.chars().count();
        self.record_atomic(before);
        true
    }

    pub fn undo(&mut self) -> bool {
        self.finish_edit_group();
        let Some(previous) = self.history.undo.pop() else {
            return false;
        };
        let current = self.snapshot();
        self.history.push_redo(current);
        self.restore(previous);
        true
    }

    pub fn redo(&mut self) -> bool {
        self.finish_edit_group();
        let Some(next) = self.history.redo.pop() else {
            return false;
        };
        let current = self.snapshot();
        self.history.push_undo(current);
        self.restore(next);
        true
    }

    pub fn finish_edit_group(&mut self) {
        if let Some((_, start)) = self.history.group.take() {
            self.history.push_undo(start);
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            value: self.value.clone(),
            cursor: self.cursor,
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.value = snapshot.value;
        self.cursor = snapshot.cursor.min(self.value.chars().count());
    }

    fn record_grouped(&mut self, group: EditGroup, before: Snapshot) {
        if !self
            .history
            .group
            .as_ref()
            .is_some_and(|(current, _)| *current == group)
        {
            self.finish_edit_group();
            self.history.group = Some((group, before));
        }
        self.history.redo.clear();
    }

    fn record_atomic(&mut self, before: Snapshot) {
        self.finish_edit_group();
        self.history.push_undo(before);
        self.history.redo.clear();
    }

    fn byte_index(&self, character_index: usize) -> usize {
        self.value
            .char_indices()
            .nth(character_index)
            .map_or(self.value.len(), |(byte_index, _)| byte_index)
    }
}

impl From<&str> for TextInput {
    fn from(value: &str) -> Self {
        let mut input = Self::default();
        input.set(value);
        input
    }
}

impl From<String> for TextInput {
    fn from(value: String) -> Self {
        let mut input = Self::default();
        input.set(value);
        input
    }
}

#[cfg(test)]
mod tests {
    use super::{TextInput, TextInputEdit};

    #[test]
    fn applies_shared_edit_commands() {
        let mut input = TextInput::from("alpha beta");

        input.apply(TextInputEdit::MoveHome);
        input.apply(TextInputEdit::MoveRight);
        input.apply(TextInputEdit::Insert('-'));
        input.apply(TextInputEdit::MoveEnd);
        input.apply(TextInputEdit::DeletePreviousWord);

        assert_eq!(input.value(), "a-lpha ");
        assert_eq!(input.cursor(), 7);
    }

    #[test]
    fn deletes_previous_word_and_leading_whitespace() {
        let mut input = TextInput::from("alpha beta   ");

        input.delete_previous_word();

        assert_eq!(input.value(), "alpha ");
        assert_eq!(input.cursor(), 6);
    }

    #[test]
    fn deletes_only_from_cursor_to_start() {
        let mut input = TextInput::from("alpha beta");
        for _ in 0..4 {
            input.move_left();
        }

        input.delete_to_start();

        assert_eq!(input.value(), "beta");
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn word_deletion_preserves_unicode_boundaries() {
        let mut input = TextInput::from("你好 world");

        input.delete_previous_word();
        input.delete_previous_word();

        assert_eq!(input.value(), "");
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn groups_contiguous_typing_and_restores_cursor() {
        let mut input = TextInput::default();
        for character in "hello".chars() {
            input.insert(character);
        }

        assert!(input.undo());
        assert_eq!(input.value(), "");
        assert_eq!(input.cursor(), 0);
        assert!(input.redo());
        assert_eq!(input.value(), "hello");
        assert_eq!(input.cursor(), 5);
    }

    #[test]
    fn movement_and_edit_kind_changes_split_groups() {
        let mut input = TextInput::from("ab");
        input.insert('c');
        input.move_left();
        input.insert('X');
        input.backspace();

        assert!(input.undo());
        assert_eq!(input.value(), "abXc");
        assert!(input.undo());
        assert_eq!(input.value(), "abc");
        assert!(input.undo());
        assert_eq!(input.value(), "ab");
    }

    #[test]
    fn paste_and_word_deletion_are_atomic() {
        let mut input = TextInput::from("alpha ");
        input.paste("beta gamma");
        input.delete_previous_word();

        assert_eq!(input.value(), "alpha beta ");
        assert!(input.undo());
        assert_eq!(input.value(), "alpha beta gamma");
        assert!(input.undo());
        assert_eq!(input.value(), "alpha ");
    }

    #[test]
    fn new_edit_after_undo_discards_redo_branch() {
        let mut input = TextInput::default();
        input.paste("old");
        assert!(input.undo());
        input.insert('n');

        assert!(!input.redo());
        assert_eq!(input.value(), "n");
    }

    #[test]
    fn set_resets_history_and_visible_equality_ignores_history() {
        let mut edited = TextInput::from("same");
        edited.insert('!');
        edited.set("same");
        let fresh = TextInput::from("same");

        assert_eq!(edited, fresh);
        assert!(!edited.undo());
    }

    #[test]
    fn history_is_unicode_safe_and_bounded() {
        let mut input = TextInput::default();
        input.paste("你🙂");
        input.move_left();
        input.insert('好');
        assert!(input.undo());
        assert_eq!(input.value(), "你🙂");
        assert_eq!(input.cursor(), 1);

        input.set("");
        for index in 0..105 {
            input.paste(index.to_string());
        }
        for _ in 0..100 {
            assert!(input.undo());
        }
        assert!(!input.undo());
    }
}
