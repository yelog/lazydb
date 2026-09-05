#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EditorMode {
    Normal,
    #[default]
    Insert,
    Replace,
    VisualChar,
    VisualLine,
    VisualBlock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EditorPosition {
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EditorViewport {
    pub width: usize,
    pub height: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditorSelectionShape {
    Char,
    Line,
    Block,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditorSelection {
    pub start: EditorPosition,
    pub end: EditorPosition,
    pub shape: EditorSelectionShape,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditorRenderSpan {
    pub text: String,
    pub source_start: usize,
    pub source_end: usize,
    pub kind: EditorHighlightKind,
    pub current_statement: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EditorHighlightKind {
    #[default]
    Plain,
    Keyword,
    Identifier,
    Relation,
    RelationAlias,
    Column,
    Function,
    String,
    Number,
    Comment,
    Operator,
    Punctuation,
    Parameter,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditorRenderLine {
    pub line: usize,
    pub spans: Vec<EditorRenderSpan>,
    /// Display-cell offset for every source character boundary, including the end.
    pub source_to_display_cells: Vec<usize>,
    pub current_statement: bool,
    pub statement_background_cells: Option<(usize, usize)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditorPromptKind {
    SearchForward,
    SearchBackward,
    Command,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditorPromptSnapshot {
    pub kind: EditorPromptKind,
    pub prefix: String,
    pub text: String,
    pub cursor: usize,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditorRenderSnapshot {
    pub revision: u64,
    pub mode: EditorMode,
    pub first_line: usize,
    pub total_lines: usize,
    pub viewport: EditorViewport,
    pub horizontal_offset: usize,
    pub lines: Vec<EditorRenderLine>,
    pub cursor: EditorPosition,
    pub cursor_screen_cell: Option<(u16, u16)>,
    pub selections: Vec<EditorSelection>,
    pub selection_cells: Vec<(usize, usize, usize)>,
    pub prompt: Option<EditorPromptSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditorBuffer {
    lines: Vec<String>,
    pub row: usize,
    pub column: usize,
    pub mode: EditorMode,
}

impl Default for EditorBuffer {
    fn default() -> Self {
        Self {
            lines: vec![String::new()],
            row: 0,
            column: 0,
            mode: EditorMode::Insert,
        }
    }
}

impl EditorBuffer {
    pub fn from_text(value: impl AsRef<str>) -> Self {
        let mut editor = Self::default();
        editor.set_text(value);
        editor
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn set_text(&mut self, value: impl AsRef<str>) {
        self.lines = value.as_ref().split('\n').map(str::to_owned).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.row = self.row.min(self.lines.len() - 1);
        self.column = self.column.min(self.current_line_chars());
    }

    pub fn insert(&mut self, character: char) {
        let byte = char_to_byte(&self.lines[self.row], self.column);
        self.lines[self.row].insert(byte, character);
        self.column += 1;
    }

    pub fn newline(&mut self) {
        let byte = char_to_byte(&self.lines[self.row], self.column);
        let suffix = self.lines[self.row].split_off(byte);
        self.row += 1;
        self.lines.insert(self.row, suffix);
        self.column = 0;
    }

    pub fn backspace(&mut self) {
        if self.column > 0 {
            let end = char_to_byte(&self.lines[self.row], self.column);
            let start = char_to_byte(&self.lines[self.row], self.column - 1);
            self.lines[self.row].replace_range(start..end, "");
            self.column -= 1;
        } else if self.row > 0 {
            let current = self.lines.remove(self.row);
            self.row -= 1;
            self.column = self.lines[self.row].chars().count();
            self.lines[self.row].push_str(&current);
        }
    }

    pub fn delete(&mut self) {
        let line_len = self.current_line_chars();
        if self.column < line_len {
            let start = char_to_byte(&self.lines[self.row], self.column);
            let end = char_to_byte(&self.lines[self.row], self.column + 1);
            self.lines[self.row].replace_range(start..end, "");
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    pub fn move_left(&mut self) {
        if self.column > 0 {
            self.column -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.column = self.current_line_chars();
        }
    }

    pub fn move_right(&mut self) {
        if self.column < self.current_line_chars() {
            self.column += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.column = 0;
        }
    }

    pub fn move_up(&mut self) {
        self.row = self.row.saturating_sub(1);
        self.column = self.column.min(self.current_line_chars());
    }

    pub fn move_down(&mut self) {
        self.row = (self.row + 1).min(self.lines.len() - 1);
        self.column = self.column.min(self.current_line_chars());
    }

    pub fn move_home(&mut self) {
        self.column = 0;
    }

    pub fn move_end(&mut self) {
        self.column = self.current_line_chars();
    }

    fn current_line_chars(&self) -> usize {
        self.lines[self.row].chars().count()
    }
}

fn char_to_byte(value: &str, character_index: usize) -> usize {
    value
        .char_indices()
        .nth(character_index)
        .map(|(index, _)| index)
        .unwrap_or(value.len())
}

#[cfg(test)]
mod tests {
    use super::EditorBuffer;

    #[test]
    fn edits_unicode_by_character_position() {
        let mut editor = EditorBuffer::from_text("数据");
        editor.move_end();
        editor.backspace();
        editor.insert('库');

        assert_eq!(editor.text(), "数库");
        assert_eq!(editor.column, 2);
    }

    #[test]
    fn joins_lines_when_backspacing_at_line_start() {
        let mut editor = EditorBuffer::from_text("select\n1");
        editor.row = 1;
        editor.column = 0;
        editor.backspace();

        assert_eq!(editor.text(), "select1");
        assert_eq!((editor.row, editor.column), (0, 6));
    }
}
