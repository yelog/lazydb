use crate::model::editor::EditorPromptKind;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PromptSession {
    pub(super) kind: EditorPromptKind,
    pub(super) text: String,
    pub(super) cursor: usize,
    pub(super) error: Option<String>,
    pub(super) history_index: Option<usize>,
}

impl PromptSession {
    pub(super) fn new(kind: EditorPromptKind) -> Self {
        Self {
            kind,
            text: String::new(),
            cursor: 0,
            error: None,
            history_index: None,
        }
    }

    pub(super) fn insert(&mut self, value: &str) {
        let offset = self
            .text
            .char_indices()
            .nth(self.cursor)
            .map_or(self.text.len(), |(offset, _)| offset);
        self.text.insert_str(offset, value);
        self.cursor += value.chars().count();
        self.error = None;
    }

    pub(super) fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let end = self
            .text
            .char_indices()
            .nth(self.cursor)
            .map_or(self.text.len(), |(offset, _)| offset);
        let start = self.text[..end]
            .char_indices()
            .last()
            .map_or(0, |(offset, _)| offset);
        self.text.replace_range(start..end, "");
        self.cursor -= 1;
        self.error = None;
    }

    pub(super) fn delete_previous_word(&mut self) {
        while self.cursor > 0
            && self.text[..self.byte_cursor()]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
        {
            self.backspace();
        }
        while self.cursor > 0
            && !self.text[..self.byte_cursor()]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
        {
            self.backspace();
        }
    }

    pub(super) fn byte_cursor(&self) -> usize {
        self.text
            .char_indices()
            .nth(self.cursor)
            .map_or(self.text.len(), |(offset, _)| offset)
    }
}
