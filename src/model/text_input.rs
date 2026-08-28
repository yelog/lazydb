#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextInput {
    value: String,
    cursor: usize,
}

impl TextInput {
    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn set(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.cursor = self.value.chars().count();
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn insert(&mut self, character: char) {
        let byte_index = self.byte_index(self.cursor);
        self.value.insert(byte_index, character);
        self.cursor += 1;
    }

    pub fn paste(&mut self, text: impl AsRef<str>) {
        let text = text.as_ref();
        let byte_index = self.byte_index(self.cursor);
        self.value.insert_str(byte_index, text);
        self.cursor += text.chars().count();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }

        let start = self.byte_index(self.cursor - 1);
        let end = self.byte_index(self.cursor);
        self.value.replace_range(start..end, "");
        self.cursor -= 1;
    }

    pub fn delete_previous_word(&mut self) {
        while self.cursor > 0
            && self.value[..self.byte_index(self.cursor)]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
        {
            self.backspace();
        }
        while self.cursor > 0
            && !self.value[..self.byte_index(self.cursor)]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
        {
            self.backspace();
        }
    }

    pub fn delete_to_start(&mut self) {
        let end = self.byte_index(self.cursor);
        self.value.replace_range(..end, "");
        self.cursor = 0;
    }

    pub fn delete(&mut self) {
        let start = self.byte_index(self.cursor);
        if start == self.value.len() {
            return;
        }

        let end = self.byte_index(self.cursor + 1);
        self.value.replace_range(start..end, "");
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.value.chars().count());
    }

    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor = self.value.chars().count();
    }

    pub fn replace(&mut self, range: crate::sql::TextRange, replacement: &str) {
        let start = self.byte_index(range.start.min(self.value.chars().count()));
        let end = self.byte_index(range.end.min(self.value.chars().count()));
        if start > end {
            return;
        }
        self.value.replace_range(start..end, replacement);
        self.cursor = range.start + replacement.chars().count();
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
    use super::TextInput;

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
}
