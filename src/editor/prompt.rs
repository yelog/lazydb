use crate::model::editor::EditorPromptKind;
use crate::model::text_input::TextInput;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PromptSession {
    pub(super) kind: EditorPromptKind,
    pub(super) input: TextInput,
    pub(super) error: Option<String>,
    pub(super) history_index: Option<usize>,
}

impl PromptSession {
    pub(super) fn new(kind: EditorPromptKind) -> Self {
        Self {
            kind,
            input: TextInput::default(),
            error: None,
            history_index: None,
        }
    }

    pub(super) fn insert(&mut self, value: &str) {
        self.input.paste(value);
        self.error = None;
    }

    pub(super) fn backspace(&mut self) {
        self.input.backspace();
        self.error = None;
    }

    pub(super) fn delete_previous_word(&mut self) {
        self.input.delete_previous_word();
        self.error = None;
    }

    pub(super) fn byte_cursor(&self) -> usize {
        self.input
            .value()
            .char_indices()
            .nth(self.input.cursor())
            .map_or(self.input.value().len(), |(offset, _)| offset)
    }
}
