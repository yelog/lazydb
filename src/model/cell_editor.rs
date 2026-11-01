use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde::de::IgnoredAny;

use crate::{db::value::CellValue, model::text_input::TextInput, profile::DatabaseKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CellEditorKind {
    Boolean,
    Date,
    Time,
    DateTime,
    Timestamp,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ColumnEditorDescription {
    pub(crate) kind: CellEditorKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CellEditorPresence {
    Unprovided,
    Null,
    Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CellEditorContent {
    Text(TextInput),
    Typed {
        kind: CellEditorKind,
        draft: TypedDraft,
    },
}

#[derive(Clone, Debug)]
pub struct CellEditorBuffer {
    pub presence: CellEditorPresence,
    pub content: CellEditorContent,
    pub presence_history: Vec<CellEditorSnapshot>,
    pub presence_redo: Vec<CellEditorSnapshot>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CellEditorSnapshot {
    presence: CellEditorPresence,
    content: CellEditorContent,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypedDraft {
    Boolean(TextInput),
    Temporal(TemporalDraft),
    Json(JsonBuffer),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JsonBuffer {
    value: String,
    cursor: usize,
    anchor: Option<usize>,
    sql_null: bool,
    history: Vec<JsonEditSnapshot>,
    redo: Vec<JsonEditSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct JsonEditSnapshot {
    value: String,
    cursor: usize,
    anchor: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JsonTokenKind {
    String,
    Number,
    Boolean,
    Null,
    Punctuation,
    Whitespace,
    Invalid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JsonToken {
    pub start: usize,
    pub end: usize,
    pub kind: JsonTokenKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JsonValidationError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl JsonBuffer {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.chars().count();
        Self {
            value,
            cursor,
            anchor: None,
            sql_null: false,
            history: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    fn without_history(&self) -> Self {
        Self {
            value: self.value.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
            sql_null: self.sql_null,
            history: Vec::new(),
            redo: Vec::new(),
        }
    }
    pub fn is_sql_null(&self) -> bool {
        self.sql_null
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.anchor
            .map(|anchor| (anchor.min(self.cursor), anchor.max(self.cursor)))
            .filter(|(start, end)| start != end)
    }
    pub fn selected_text(&self) -> Option<&str> {
        let (start, end) = self.selection()?;
        Some(&self.value[self.byte_index(start)..self.byte_index(end)])
    }
    pub fn begin_selection(&mut self, anchor: usize) {
        let anchor = anchor.min(self.value.chars().count());
        self.anchor = Some(anchor);
        self.cursor = anchor;
    }
    pub fn extend_selection(&mut self, cursor: usize) {
        self.cursor = cursor.min(self.value.chars().count());
    }

    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }
    pub fn line(&self) -> usize {
        self.value[..self.byte_index(self.cursor)]
            .matches('\n')
            .count()
    }
    pub fn column(&self) -> usize {
        self.value[..self.byte_index(self.cursor)]
            .rsplit('\n')
            .next()
            .unwrap_or_default()
            .chars()
            .count()
    }

    pub fn insert(&mut self, character: char) {
        self.record();
        self.remove_selection();
        let index = self.byte_index(self.cursor);
        self.value.insert(index, character);
        self.cursor += 1;
    }

    pub fn paste(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.record();
        self.remove_selection();
        let index = self.byte_index(self.cursor);
        self.value.insert_str(index, text);
        self.cursor += text.chars().count();
        self.sql_null = false;
    }

    pub fn backspace(&mut self) {
        if self.selection().is_some() {
            self.record();
            self.remove_selection();
            return;
        }
        if self.cursor == 0 {
            return;
        }
        self.record();
        let end = self.byte_index(self.cursor);
        let start = self.byte_index(self.cursor - 1);
        self.value.replace_range(start..end, "");
        self.cursor -= 1;
    }

    pub fn delete(&mut self) {
        if self.selection().is_some() {
            self.record();
            self.remove_selection();
            return;
        }
        if self.cursor >= self.value.chars().count() {
            return;
        }
        self.record();
        let start = self.byte_index(self.cursor);
        let end = self.byte_index(self.cursor + 1);
        self.value.replace_range(start..end, "");
    }

    pub fn move_left(&mut self) {
        self.anchor = None;
        self.cursor = self.cursor.saturating_sub(1);
    }
    pub fn move_right(&mut self) {
        self.anchor = None;
        self.cursor = (self.cursor + 1).min(self.value.chars().count());
    }
    pub fn move_home(&mut self) {
        self.anchor = None;
        self.cursor = self.line_start(self.cursor);
    }
    pub fn move_end(&mut self) {
        self.anchor = None;
        self.cursor = self.line_end(self.cursor);
    }
    pub fn move_up(&mut self) {
        self.move_vertical(-1);
    }
    pub fn move_down(&mut self) {
        self.move_vertical(1);
    }
    pub fn undo(&mut self) {
        if let Some(snapshot) = self.history.pop() {
            self.redo.push(self.snapshot());
            self.restore(snapshot);
        }
    }
    pub fn redo(&mut self) {
        if let Some(snapshot) = self.redo.pop() {
            self.history.push(self.snapshot());
            self.restore(snapshot);
        }
    }

    pub fn apply(&mut self, edit: crate::model::text_input::TextInputEdit) -> bool {
        let before = self.without_history();
        match edit {
            crate::model::text_input::TextInputEdit::Insert(c) => self.insert(c),
            crate::model::text_input::TextInputEdit::Backspace => self.backspace(),
            crate::model::text_input::TextInputEdit::Delete => self.delete(),
            crate::model::text_input::TextInputEdit::MoveLeft => self.move_left(),
            crate::model::text_input::TextInputEdit::MoveRight => self.move_right(),
            crate::model::text_input::TextInputEdit::MoveHome => self.move_home(),
            crate::model::text_input::TextInputEdit::MoveEnd => self.move_end(),
            crate::model::text_input::TextInputEdit::DeletePreviousWord => {
                if self.cursor == 0 {
                    return false;
                }
                self.record();
                let end = self.byte_index(self.cursor);
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
                self.value.replace_range(self.byte_index(start)..end, "");
                self.cursor = start;
                self.anchor = None;
            }
            crate::model::text_input::TextInputEdit::DeleteToStart => {
                if self.cursor == 0 {
                    return false;
                }
                self.record();
                self.value.replace_range(..self.byte_index(self.cursor), "");
                self.cursor = 0;
                self.anchor = None;
            }
            crate::model::text_input::TextInputEdit::Clear => {
                if self.value.is_empty() {
                    return false;
                }
                self.record();
                self.value.clear();
                self.cursor = 0;
                self.anchor = None;
            }
            crate::model::text_input::TextInputEdit::Undo => {
                return {
                    self.undo();
                    true
                };
            }
            crate::model::text_input::TextInputEdit::Redo => {
                return {
                    self.redo();
                    true
                };
            }
        }
        self.without_history() != before
    }

    pub fn validate(&self) -> Result<(), JsonValidationError> {
        validate_json(self.value())
    }
    pub fn format(&self) -> Result<String, JsonValidationError> {
        format_json(self.value())
    }
    pub fn tokens(&self) -> Vec<JsonToken> {
        tokenize_json(self.value())
    }
    pub fn replace_value(&mut self, value: String) {
        self.record();
        self.value = value;
        self.cursor = self.value.chars().count();
        self.anchor = None;
    }

    fn record(&mut self) {
        self.history.push(self.snapshot());
        self.redo.clear();
    }
    fn snapshot(&self) -> JsonEditSnapshot {
        JsonEditSnapshot {
            value: self.value.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
        }
    }
    fn restore(&mut self, snapshot: JsonEditSnapshot) {
        self.value = snapshot.value;
        self.cursor = snapshot.cursor;
        self.anchor = snapshot.anchor;
    }
    fn remove_selection(&mut self) {
        let Some((start, end)) = self.selection() else {
            self.anchor = None;
            return;
        };
        self.value
            .replace_range(self.byte_index(start)..self.byte_index(end), "");
        self.cursor = start;
        self.anchor = None;
    }
    fn byte_index(&self, cursor: usize) -> usize {
        self.value
            .char_indices()
            .nth(cursor.min(self.value.chars().count()))
            .map_or(self.value.len(), |(index, _)| index)
    }
    fn line_start(&self, cursor: usize) -> usize {
        self.value[..self.byte_index(cursor)]
            .rfind('\n')
            .map_or(0, |index| self.value[..index].chars().count() + 1)
    }
    fn line_end(&self, cursor: usize) -> usize {
        self.value[self.byte_index(cursor)..].find('\n').map_or(
            self.value.chars().count(),
            |index| {
                cursor
                    + self.value[self.byte_index(cursor)..self.byte_index(cursor) + index]
                        .chars()
                        .count()
            },
        )
    }
    fn move_vertical(&mut self, direction: isize) {
        self.anchor = None;
        let column = self.column();
        let line = self.line().saturating_add_signed(direction);
        let start = self
            .value
            .split_inclusive('\n')
            .take(line)
            .map(str::len)
            .sum::<usize>();
        let target = self.value[start..].split('\n').next().unwrap_or_default();
        self.cursor = self.value[..start].chars().count() + column.min(target.chars().count());
    }
}

pub fn validate_json(value: &str) -> Result<(), JsonValidationError> {
    serde_json::from_str::<IgnoredAny>(value)
        .map(|_| ())
        .map_err(|error| {
            let line = error.line();
            let column = error.column();
            JsonValidationError {
                message: error.to_string(),
                line,
                column,
            }
        })
}

pub fn format_json(value: &str) -> Result<String, JsonValidationError> {
    validate_json(value)?;
    let tokens = tokenize_json(value)
        .into_iter()
        .filter(|token| token.kind != JsonTokenKind::Whitespace)
        .collect::<Vec<_>>();
    let mut output = String::new();
    let mut indent = 0usize;
    let mut line_start = true;
    for (position, token) in tokens.iter().enumerate() {
        let text = &value[token.start..token.end];
        match text {
            "{" | "[" => {
                output.push_str(text);
                indent += 1;
                if tokens.get(position + 1).is_some_and(|next| {
                    &value[next.start..next.end] != (if text == "{" { "}" } else { "]" })
                }) {
                    output.push('\n');
                    line_start = true;
                }
            }
            "}" | "]" => {
                indent = indent.saturating_sub(1);
                if !line_start {
                    output.push('\n');
                }
                output.push_str(&"  ".repeat(indent));
                output.push_str(text);
                line_start = false;
            }
            "," => {
                output.push(',');
                output.push('\n');
                line_start = true;
            }
            ":" => output.push_str(": "),
            _ => {
                if line_start {
                    output.push_str(&"  ".repeat(indent));
                    line_start = false;
                }
                output.push_str(text);
            }
        }
    }
    Ok(output)
}

pub fn tokenize_json(value: &str) -> Vec<JsonToken> {
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < value.len() {
        let start = index;
        let byte = value.as_bytes()[index];
        let (kind, end) = match byte {
            b' ' | b'\t' | b'\r' | b'\n' => (
                JsonTokenKind::Whitespace,
                value[index..]
                    .find(|c: char| !c.is_whitespace())
                    .map_or(value.len(), |offset| index + offset),
            ),
            b'"' => {
                let mut cursor = index + 1;
                let mut escaped = false;
                while cursor < value.len() {
                    let character = value.as_bytes()[cursor];
                    if !escaped && character == b'"' {
                        cursor += 1;
                        break;
                    }
                    escaped = !escaped && character == b'\\';
                    if character != b'\\' {
                        escaped = false;
                    }
                    cursor += 1;
                }
                (JsonTokenKind::String, cursor)
            }
            b'{' | b'}' | b'[' | b']' | b':' | b',' => (JsonTokenKind::Punctuation, index + 1),
            b'-' | b'0'..=b'9' => (
                JsonTokenKind::Number,
                value[index..]
                    .find(|c: char| {
                        !(c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'))
                    })
                    .map_or(value.len(), |offset| index + offset),
            ),
            _ if value[index..].starts_with("true") || value[index..].starts_with("false") => (
                JsonTokenKind::Boolean,
                index
                    + if value[index..].starts_with("true") {
                        4
                    } else {
                        5
                    },
            ),
            _ if value[index..].starts_with("null") => (JsonTokenKind::Null, index + 4),
            _ => (
                JsonTokenKind::Invalid,
                index + value[index..].chars().next().map_or(1, char::len_utf8),
            ),
        };
        tokens.push(JsonToken { start, end, kind });
        index = end.max(start + 1);
    }
    tokens
}

#[derive(Clone, Debug, PartialEq)]
pub struct TemporalDraft {
    kind: CellEditorKind,
    input: TextInput,
    segment: usize,
    fraction_width: usize,
}

impl TemporalDraft {
    pub fn date(value: NaiveDate) -> Self {
        Self::new(CellEditorKind::Date, value.format("%Y-%m-%d").to_string())
    }

    pub fn from_time(value: NaiveTime) -> Self {
        Self::new(CellEditorKind::Time, format_time(value))
    }

    pub fn from_datetime(value: NaiveDateTime) -> Self {
        Self::new(CellEditorKind::DateTime, format_datetime(value))
    }

    pub fn from_timestamp(value: DateTime<FixedOffset>) -> Self {
        let mut draft = Self::new(CellEditorKind::Timestamp, format_timestamp(value));
        draft.fraction_width = value.nanosecond().to_string().len().max(1);
        draft
    }

    fn new(kind: CellEditorKind, value: String) -> Self {
        let fraction_width = value
            .split_once('.')
            .and_then(|(_, fraction)| fraction.split_once(' ').or(Some((fraction, ""))))
            .map(|(fraction, _)| fraction.len())
            .unwrap_or(0);
        Self {
            kind,
            input: TextInput::from(value),
            segment: 0,
            fraction_width,
        }
    }

    pub fn from_kind_and_text(kind: CellEditorKind, value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        let value = if value.is_empty() {
            match kind {
                CellEditorKind::Date => "1970-01-01".into(),
                CellEditorKind::Time => "00:00:00".into(),
                CellEditorKind::DateTime => "1970-01-01 00:00:00".into(),
                CellEditorKind::Timestamp => "1970-01-01 00:00:00 +00:00".into(),
                _ => return None,
            }
        } else {
            value
        };
        Some(Self::new(kind, value))
    }

    pub fn kind(&self) -> CellEditorKind {
        self.kind
    }

    pub fn input(&self) -> &TextInput {
        &self.input
    }

    pub fn input_mut(&mut self) -> &mut TextInput {
        &mut self.input
    }

    pub fn render(&self) -> &str {
        self.input.value()
    }

    pub fn set_segment(&mut self, segment: usize, value: &str) {
        self.segment = segment.min(self.segment_count().saturating_sub(1));
        let Some((start, end)) = self.segment_range(self.segment) else {
            return;
        };
        let width = end - start;
        let value = value.chars().take(width).collect::<String>();
        let mut text = self.input.value().to_owned();
        text.replace_range(start..end, &format!("{value:0>width$}"));
        self.input.set(text);
    }

    pub fn move_segment(&mut self, direction: isize) {
        self.segment = self
            .segment
            .saturating_add_signed(direction)
            .min(self.segment_count().saturating_sub(1));
        if let Some((start, _)) = self.segment_range(self.segment) {
            self.input.set_cursor(start);
        }
    }

    pub fn shift_month(&mut self, direction: isize) {
        if self.kind != CellEditorKind::Date
            && self.kind != CellEditorKind::DateTime
            && self.kind != CellEditorKind::Timestamp
        {
            return;
        }
        let date = match self.kind {
            CellEditorKind::Date => self.parse_date().ok(),
            CellEditorKind::DateTime => self.parse_datetime().ok().map(|value| value.date()),
            CellEditorKind::Timestamp => {
                self.parse_timestamp().ok().map(|value| value.date_naive())
            }
            _ => None,
        };
        let Some(date) = date else { return };
        let month = date.month0() as isize + direction;
        let year = date.year() + month.div_euclid(12) as i32;
        let month = month.rem_euclid(12) as u32 + 1;
        let day = date.day().min(days_in_month(year, month));
        let Some(date) = NaiveDate::from_ymd_opt(year, month, day) else {
            return;
        };
        let value = match self.kind {
            CellEditorKind::Date => date.format("%Y-%m-%d").to_string(),
            CellEditorKind::DateTime => self
                .parse_datetime()
                .ok()
                .map(|value| NaiveDateTime::new(date, value.time()))
                .map(format_datetime)
                .unwrap_or_else(|| date.format("%Y-%m-%d").to_string()),
            CellEditorKind::Timestamp => self
                .parse_timestamp()
                .ok()
                .and_then(|value| value.with_year(year))
                .and_then(|value| value.with_month(month))
                .and_then(|value| value.with_day(day))
                .map(format_timestamp)
                .unwrap_or_else(|| date.format("%Y-%m-%d").to_string()),
            _ => return,
        };
        self.input.set(value);
    }

    pub fn insert(&mut self, character: char) {
        if !(character.is_ascii_digit()
            || self.kind == CellEditorKind::Timestamp && character == ':')
        {
            return;
        }
        let Some((start, end)) = self.segment_range(self.segment) else {
            return;
        };
        let mut text = self.input.value().to_owned();
        let cursor = self.input.cursor().clamp(start, end.saturating_sub(1));
        text.replace_range(cursor..cursor + 1, &character.to_string());
        self.input.set(text);
        self.input.set_cursor((cursor + 1).min(end));
    }

    pub fn parse(&self) -> Result<CellValue, String> {
        match self.kind {
            CellEditorKind::Date => self.parse_date().map(CellValue::Date),
            CellEditorKind::Time => self.parse_time().map(CellValue::Time),
            CellEditorKind::DateTime => self.parse_datetime().map(CellValue::DateTime),
            CellEditorKind::Timestamp => self.parse_timestamp().map(CellValue::Timestamp),
            _ => Err("not a temporal draft".into()),
        }
    }

    pub fn parse_date(&self) -> Result<NaiveDate, String> {
        NaiveDate::parse_from_str(self.render(), "%Y-%m-%d")
            .map_err(|_| "invalid date; expected YYYY-MM-DD".into())
    }

    pub fn parse_time(&self) -> Result<NaiveTime, String> {
        NaiveTime::parse_from_str(self.render(), "%H:%M:%S%.f")
            .map_err(|_| "invalid time; expected HH:MM:SS[.fraction]".into())
    }

    pub fn parse_datetime(&self) -> Result<NaiveDateTime, String> {
        NaiveDateTime::parse_from_str(self.render(), "%Y-%m-%d %H:%M:%S%.f")
            .map_err(|_| "invalid datetime; expected YYYY-MM-DD HH:MM:SS[.fraction]".into())
    }

    pub fn parse_timestamp(&self) -> Result<DateTime<FixedOffset>, String> {
        DateTime::parse_from_str(self.render(), "%Y-%m-%d %H:%M:%S%.f %:z")
            .map_err(|_| "invalid timestamp; expected YYYY-MM-DD HH:MM:SS[.fraction] ±HH:MM".into())
    }

    pub fn error(&self) -> Option<String> {
        self.parse().err()
    }

    pub fn calendar_label(&self) -> Option<String> {
        let date = match self.kind {
            CellEditorKind::Date => self.parse_date().ok(),
            CellEditorKind::DateTime => self.parse_datetime().ok().map(|value| value.date()),
            CellEditorKind::Timestamp => {
                self.parse_timestamp().ok().map(|value| value.date_naive())
            }
            CellEditorKind::Time => None,
            _ => None,
        }?;
        Some(date.format("%B %Y  [ and ] month").to_string())
    }

    fn segment_count(&self) -> usize {
        match self.kind {
            CellEditorKind::Date => 3,
            CellEditorKind::Time => 3,
            CellEditorKind::DateTime => 6,
            CellEditorKind::Timestamp => 8,
            _ => 0,
        }
    }

    fn segment_range(&self, segment: usize) -> Option<(usize, usize)> {
        let ranges = match self.kind {
            CellEditorKind::Date => vec![(0, 4), (5, 7), (8, 10)],
            CellEditorKind::Time => vec![(0, 2), (3, 5), (6, 8)],
            CellEditorKind::DateTime => vec![(0, 4), (5, 7), (8, 10), (11, 13), (14, 16), (17, 19)],
            CellEditorKind::Timestamp => {
                let fraction_end = if self.fraction_width == 0 {
                    19
                } else {
                    20 + self.fraction_width
                };
                let offset_start = if self.fraction_width == 0 {
                    20
                } else {
                    fraction_end + 1
                };
                vec![
                    (0, 4),
                    (5, 7),
                    (8, 10),
                    (11, 13),
                    (14, 16),
                    (17, 19),
                    (20, fraction_end),
                    (offset_start, offset_start + 6),
                ]
            }
            _ => Vec::new(),
        };
        ranges.get(segment).copied()
    }
}

impl Default for CellEditorBuffer {
    fn default() -> Self {
        Self::text(CellEditorPresence::Value, TextInput::default())
    }
}

impl PartialEq for CellEditorBuffer {
    fn eq(&self, other: &Self) -> bool {
        self.presence == other.presence && self.content == other.content
    }
}

impl CellEditorBuffer {
    fn text(presence: CellEditorPresence, input: TextInput) -> Self {
        Self {
            presence,
            content: CellEditorContent::Text(input),
            presence_history: Vec::new(),
            presence_redo: Vec::new(),
        }
    }

    fn typed(presence: CellEditorPresence, kind: CellEditorKind, draft: TypedDraft) -> Self {
        Self {
            presence,
            content: CellEditorContent::Typed { kind, draft },
            presence_history: Vec::new(),
            presence_redo: Vec::new(),
        }
    }

    pub(crate) fn from_value(
        value: &CellValue,
        description: Option<ColumnEditorDescription>,
    ) -> Self {
        match value {
            CellValue::Null => description.map_or_else(
                || Self::text(CellEditorPresence::Null, TextInput::from("null")),
                |description| Self::typed_null(description.kind),
            ),
            _ => description.map_or_else(
                || {
                    Self::text(
                        CellEditorPresence::Value,
                        TextInput::from(value.clipboard_text()),
                    )
                },
                |description| Self::typed_from_value(description.kind, value),
            ),
        }
    }

    pub(crate) fn unprovided() -> Self {
        Self::text(CellEditorPresence::Unprovided, TextInput::default())
    }

    pub(crate) fn typed_unprovided(kind: CellEditorKind) -> Self {
        Self::typed_from_text(CellEditorPresence::Unprovided, kind, "")
    }

    pub(crate) fn typed_null(kind: CellEditorKind) -> Self {
        Self::typed_from_text(CellEditorPresence::Null, kind, "")
    }

    pub(crate) fn presence(&self) -> CellEditorPresence {
        self.presence.clone()
    }

    pub(crate) fn is_null(&self) -> bool {
        self.presence == CellEditorPresence::Null
    }

    #[cfg(test)]
    pub(crate) fn is_boolean(&self) -> bool {
        matches!(
            self.content,
            CellEditorContent::Typed {
                kind: CellEditorKind::Boolean,
                ..
            }
        )
    }

    #[cfg(test)]
    pub(crate) fn temporal_draft(&self) -> Option<&TemporalDraft> {
        match &self.content {
            CellEditorContent::Typed {
                draft: TypedDraft::Temporal(draft),
                ..
            } => Some(draft),
            _ => None,
        }
    }

    pub(crate) fn activate_value(&mut self) {
        self.presence = CellEditorPresence::Value;
    }

    pub(crate) fn set_unprovided(&mut self) {
        self.presence = CellEditorPresence::Unprovided;
    }

    pub(crate) fn set_null(&mut self) {
        self.presence = CellEditorPresence::Null;
    }

    fn snapshot(&self) -> CellEditorSnapshot {
        let content = match &self.content {
            CellEditorContent::Text(input) => CellEditorContent::Text(input.without_history()),
            CellEditorContent::Typed { kind, draft } => CellEditorContent::Typed {
                kind: *kind,
                draft: match draft {
                    TypedDraft::Boolean(input) => TypedDraft::Boolean(input.without_history()),
                    TypedDraft::Temporal(draft) => {
                        let mut draft = draft.clone();
                        draft.input = draft.input.without_history();
                        TypedDraft::Temporal(draft)
                    }
                    TypedDraft::Json(buffer) => TypedDraft::Json(buffer.without_history()),
                },
            },
        };
        CellEditorSnapshot {
            presence: self.presence.clone(),
            content,
        }
    }

    fn restore_snapshot(&mut self, snapshot: CellEditorSnapshot) {
        self.presence = snapshot.presence;
        self.content = snapshot.content;
    }

    fn record_presence_transition(&mut self, before: CellEditorSnapshot) {
        if self.presence_history.len() == 100 {
            self.presence_history.remove(0);
        }
        self.presence_history.push(before);
        self.presence_redo.clear();
    }

    pub(crate) fn apply_text_edit(
        &mut self,
        edit: crate::model::text_input::TextInputEdit,
    ) -> bool {
        if matches!(edit, crate::model::text_input::TextInputEdit::Undo) {
            return self.undo();
        }
        if matches!(edit, crate::model::text_input::TextInputEdit::Redo) {
            return self.redo();
        }
        self.finish_edit_group();
        let before = self.snapshot();
        let changed = match &mut self.content {
            CellEditorContent::Text(input) => input.apply(edit),
            CellEditorContent::Typed { draft, .. } => match draft {
                TypedDraft::Boolean(input) => input.apply(edit),
                TypedDraft::Temporal(draft) => match edit {
                    crate::model::text_input::TextInputEdit::Insert(character) => {
                        let before = draft.render().to_owned();
                        draft.insert(character);
                        draft.render() != before
                    }
                    _ => draft.input_mut().apply(edit),
                },
                TypedDraft::Json(buffer) => buffer.apply(edit),
            },
        };
        let is_navigation = matches!(
            edit,
            crate::model::text_input::TextInputEdit::MoveLeft
                | crate::model::text_input::TextInputEdit::MoveRight
                | crate::model::text_input::TextInputEdit::MoveHome
                | crate::model::text_input::TextInputEdit::MoveEnd
        );
        if changed && !is_navigation {
            self.record_presence_transition(before);
            self.presence = CellEditorPresence::Value;
        }
        changed
    }

    pub(crate) fn paste(&mut self, text: &str) -> bool {
        let old_presence = self.presence.clone();
        if text.is_empty() {
            return false;
        }
        if matches!(
            &self.content,
            CellEditorContent::Typed {
                draft: TypedDraft::Boolean(_) | TypedDraft::Temporal(_),
                ..
            }
        ) && text.trim().is_empty()
        {
            return false;
        }
        self.finish_edit_group();
        let before = self.snapshot();
        let changed = match &mut self.content {
            CellEditorContent::Text(input) => {
                let old_value = input.value().to_owned();
                let old_cursor = input.cursor();
                let old_selection = input.selection_range();
                if old_presence != CellEditorPresence::Value {
                    input.set(text);
                } else {
                    input.paste(text);
                }
                input.value() != old_value
                    || input.cursor() != old_cursor
                    || input.selection_range() != old_selection
            }
            CellEditorContent::Typed { draft, .. } => match draft {
                TypedDraft::Boolean(input) => {
                    let value = text.trim();
                    if value.is_empty() || input.value() == value {
                        false
                    } else {
                        input.set(value);
                        true
                    }
                }
                TypedDraft::Temporal(draft) => {
                    let value = text.trim();
                    if value.is_empty() || draft.render() == value {
                        false
                    } else if let Some(replacement) =
                        TemporalDraft::from_kind_and_text(draft.kind(), value)
                    {
                        *draft = replacement;
                        true
                    } else {
                        false
                    }
                }
                TypedDraft::Json(buffer) => {
                    let old_value = buffer.value().to_owned();
                    let old_cursor = buffer.cursor();
                    let old_selection = buffer.selection();
                    let old_sql_null = buffer.is_sql_null();
                    if old_presence != CellEditorPresence::Value {
                        *buffer = JsonBuffer::new(text);
                    } else {
                        buffer.paste(text);
                    }
                    buffer.value() != old_value
                        || buffer.cursor() != old_cursor
                        || buffer.selection() != old_selection
                        || buffer.is_sql_null() != old_sql_null
                }
            },
        };
        let presence_changed = old_presence != CellEditorPresence::Value;
        if !changed && !presence_changed {
            return false;
        }

        if changed {
            match &mut self.content {
                CellEditorContent::Text(input) => *input = input.without_history(),
                CellEditorContent::Typed { draft, .. } => match draft {
                    TypedDraft::Boolean(input) => *input = input.without_history(),
                    TypedDraft::Temporal(draft) => {
                        *draft.input_mut() = draft.input().without_history();
                    }
                    TypedDraft::Json(buffer) => *buffer = buffer.without_history(),
                },
            }
        }
        self.presence = CellEditorPresence::Value;
        self.record_presence_transition(before);
        true
    }

    pub(crate) fn undo(&mut self) -> bool {
        if let Some(snapshot) = self.presence_history.last().cloned()
            && self.snapshot() != snapshot
        {
            let current = self.snapshot();
            self.presence_history.pop();
            self.presence_redo.push(current);
            self.restore_snapshot(snapshot);
            return true;
        }
        let before = self.snapshot();
        let changed = match &mut self.content {
            CellEditorContent::Text(input) => input.undo(),
            CellEditorContent::Typed { draft, .. } => match draft {
                TypedDraft::Boolean(input) => input.undo(),
                TypedDraft::Temporal(draft) => draft.input_mut().undo(),
                TypedDraft::Json(buffer) => {
                    buffer.undo();
                    true
                }
            },
        };
        if !changed {
            return false;
        }
        if let Some(snapshot) = self.presence_history.last().cloned()
            && self.snapshot().content == snapshot.content
        {
            self.presence_history.pop();
            self.presence_redo.push(before);
            self.restore_snapshot(snapshot);
        }
        true
    }

    pub(crate) fn redo(&mut self) -> bool {
        if let Some(snapshot) = self.presence_redo.last().cloned() {
            let current = self.snapshot();
            self.presence_redo.pop();
            self.presence_history.push(current);
            self.restore_snapshot(snapshot);
            return true;
        }
        let changed = match &mut self.content {
            CellEditorContent::Text(input) => input.redo(),
            CellEditorContent::Typed { draft, .. } => match draft {
                TypedDraft::Boolean(input) => input.redo(),
                TypedDraft::Temporal(draft) => draft.input_mut().redo(),
                TypedDraft::Json(buffer) => {
                    buffer.redo();
                    true
                }
            },
        };
        if !changed {
            return false;
        }
        true
    }

    pub(crate) fn finish_edit_group(&mut self) {
        match &mut self.content {
            CellEditorContent::Text(input) => input.finish_edit_group(),
            CellEditorContent::Typed { draft, .. } => match draft {
                TypedDraft::Boolean(input) => input.finish_edit_group(),
                TypedDraft::Temporal(draft) => draft.input.finish_edit_group(),
                TypedDraft::Json(_) => {}
            },
        }
    }

    pub(crate) fn input(&self) -> Option<&TextInput> {
        match self {
            Self {
                content: CellEditorContent::Text(input),
                ..
            } => Some(input),
            Self {
                content: CellEditorContent::Typed { draft, .. },
                ..
            } => match draft {
                TypedDraft::Boolean(input) => Some(input),
                TypedDraft::Temporal(draft) => Some(draft.input()),
                TypedDraft::Json(_) => None,
            },
        }
    }

    pub(crate) fn input_mut(&mut self) -> &mut TextInput {
        match &mut self.content {
            CellEditorContent::Text(input) => input,
            CellEditorContent::Typed { draft, .. } => match draft {
                TypedDraft::Boolean(input) => input,
                TypedDraft::Temporal(draft) => draft.input_mut(),
                TypedDraft::Json(_) => unreachable!("JSON uses its multiline buffer API"),
            },
        }
    }

    pub(crate) fn json_buffer(&self) -> Option<&JsonBuffer> {
        match self {
            Self {
                content:
                    CellEditorContent::Typed {
                        draft: TypedDraft::Json(buffer),
                        ..
                    },
                ..
            } => Some(buffer),
            _ => None,
        }
    }

    pub(crate) fn json_buffer_mut(&mut self) -> Option<&mut JsonBuffer> {
        match self {
            Self {
                content:
                    CellEditorContent::Typed {
                        draft: TypedDraft::Json(buffer),
                        ..
                    },
                ..
            } => Some(buffer),
            _ => None,
        }
    }

    pub fn value(&self) -> Option<&str> {
        match self {
            Self {
                content: CellEditorContent::Text(input),
                ..
            } => Some(input.value()),
            Self {
                content: CellEditorContent::Typed { draft, .. },
                ..
            } => Some(match draft {
                TypedDraft::Boolean(input) => input.value(),
                TypedDraft::Temporal(draft) => draft.render(),
                TypedDraft::Json(buffer) => buffer.value(),
            }),
        }
    }

    pub(crate) fn is_unprovided(&self) -> bool {
        self.presence == CellEditorPresence::Unprovided
    }

    /// Parse a typed draft without consulting the value that originally populated it.
    pub(crate) fn typed_value(&self) -> Option<Result<CellValue, String>> {
        let result = match &self.content {
            CellEditorContent::Typed { kind, draft } => match (kind, draft) {
                (CellEditorKind::Boolean, TypedDraft::Boolean(input)) => {
                    match input.value().trim().to_ascii_lowercase().as_str() {
                        "true" | "t" => Ok(CellValue::Boolean(true)),
                        "false" | "f" => Ok(CellValue::Boolean(false)),
                        _ => Err("invalid boolean".into()),
                    }
                }
                (_, TypedDraft::Temporal(draft)) => draft.parse(),
                (_, TypedDraft::Json(json)) => json
                    .validate()
                    .map(|_| {
                        if json.is_sql_null() {
                            CellValue::Null
                        } else {
                            CellValue::Text(json.value().to_owned())
                        }
                    })
                    .map_err(|error| {
                        format!("{} at {}:{}", error.message, error.line, error.column)
                    }),
                _ => return None,
            },
            CellEditorContent::Text(_) => return None,
        };
        Some(result)
    }

    /// Extract the value represented by the editor's presence state.
    pub(crate) fn value_for_presence(&self) -> Result<Option<CellValue>, String> {
        match self.presence {
            CellEditorPresence::Unprovided => Ok(None),
            CellEditorPresence::Null => Ok(Some(CellValue::Null)),
            CellEditorPresence::Value => self
                .typed_value()
                .unwrap_or_else(|| Ok(CellValue::Text(self.value().unwrap_or_default().into())))
                .map(Some),
        }
    }

    pub(crate) fn boolean_selection(&self) -> Option<bool> {
        let Self {
            content:
                CellEditorContent::Typed {
                    kind: CellEditorKind::Boolean,
                    draft: TypedDraft::Boolean(input),
                },
            ..
        } = self
        else {
            return None;
        };
        match input.value().trim().to_ascii_lowercase().as_str() {
            "true" | "t" => Some(true),
            "false" | "f" => Some(false),
            _ => None,
        }
    }

    pub(crate) fn set_boolean(&mut self, value: bool) {
        let before = self.snapshot();
        if let Self {
            content:
                CellEditorContent::Typed {
                    kind: CellEditorKind::Boolean,
                    draft: TypedDraft::Boolean(input),
                },
            ..
        } = self
        {
            let next = if value { "true" } else { "false" };
            if input.value() != next {
                *input = TextInput::from(next);
                self.presence = CellEditorPresence::Value;
                self.record_presence_transition(before);
            }
        }
    }

    pub(crate) fn move_boolean(&mut self, direction: isize) {
        if direction != 0
            && let Some(current) = self.boolean_selection()
        {
            self.set_boolean(!current);
        }
    }

    fn typed_from_value(kind: CellEditorKind, value: &CellValue) -> Self {
        let draft = match (kind, value) {
            (CellEditorKind::Boolean, _) => {
                TypedDraft::Boolean(TextInput::from(value.clipboard_text()))
            }
            (CellEditorKind::Json, CellValue::Text(value)) => {
                TypedDraft::Json(JsonBuffer::new(value))
            }
            (CellEditorKind::Json, _) => TypedDraft::Json(JsonBuffer::new(value.clipboard_text())),
            (CellEditorKind::Date, CellValue::Date(value)) => {
                TypedDraft::Temporal(TemporalDraft::date(*value))
            }
            (CellEditorKind::Time, CellValue::Time(value)) => {
                TypedDraft::Temporal(TemporalDraft::from_time(*value))
            }
            (CellEditorKind::DateTime, CellValue::DateTime(value)) => {
                TypedDraft::Temporal(TemporalDraft::from_datetime(*value))
            }
            (CellEditorKind::Timestamp, CellValue::Timestamp(value)) => {
                TypedDraft::Temporal(TemporalDraft::from_timestamp(*value))
            }
            _ => {
                return Self::typed_from_text(
                    CellEditorPresence::Value,
                    kind,
                    value.clipboard_text(),
                );
            }
        };
        Self::typed(CellEditorPresence::Value, kind, draft)
    }

    fn typed_from_text(
        presence: CellEditorPresence,
        kind: CellEditorKind,
        text: impl Into<String>,
    ) -> Self {
        let text = text.into();
        let input = TextInput::from(text.clone());
        let draft = match kind {
            CellEditorKind::Boolean => TypedDraft::Boolean(input),
            CellEditorKind::Json => TypedDraft::Json(JsonBuffer {
                sql_null: presence == CellEditorPresence::Null,
                ..JsonBuffer::new(if text.is_empty() { "null" } else { &text })
            }),
            CellEditorKind::Date
            | CellEditorKind::Time
            | CellEditorKind::DateTime
            | CellEditorKind::Timestamp => TypedDraft::Temporal(
                TemporalDraft::from_kind_and_text(kind, text).expect("temporal kind"),
            ),
        };
        Self::typed(presence, kind, draft)
    }

    pub(crate) fn temporal_move_to(&mut self, segment: isize) {
        if let Self {
            content:
                CellEditorContent::Typed {
                    draft: TypedDraft::Temporal(draft),
                    ..
                },
            ..
        } = self
        {
            draft.move_segment(segment);
        }
    }

    pub(crate) fn temporal_shift_month(&mut self, direction: isize) {
        let before = self.snapshot();
        if let Self {
            content:
                CellEditorContent::Typed {
                    draft: TypedDraft::Temporal(draft),
                    ..
                },
            ..
        } = self
        {
            let old = draft.render().to_owned();
            draft.shift_month(direction);
            if draft.render() != old {
                self.presence = CellEditorPresence::Value;
                self.record_presence_transition(before);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn temporal_insert(&mut self, character: char) {
        let before = self.snapshot();
        if let Self {
            content:
                CellEditorContent::Typed {
                    draft: TypedDraft::Temporal(draft),
                    ..
                },
            ..
        } = self
        {
            let old = draft.render().to_owned();
            draft.insert(character);
            if draft.render() != old {
                self.presence = CellEditorPresence::Value;
                self.record_presence_transition(before);
            }
        }
    }

    pub(crate) fn replace_json_value(&mut self, value: String) -> bool {
        let before = self.snapshot();
        let Some(buffer) = self.json_buffer_mut() else {
            return false;
        };
        if buffer.value() == value {
            return false;
        }
        buffer.replace_value(value);
        if self.presence != CellEditorPresence::Value {
            self.presence = CellEditorPresence::Value;
            self.record_presence_transition(before);
        }
        true
    }
}

pub(crate) fn classify_column_type(
    database_kind: DatabaseKind,
    type_name: &str,
) -> Option<ColumnEditorDescription> {
    let type_name = normalize_type_name(type_name);
    let base_name = type_name.split('(').next().unwrap_or(&type_name).trim();

    let kind = match database_kind {
        DatabaseKind::Postgres | DatabaseKind::CockroachDb => match base_name {
            "bool" | "boolean" => Some(CellEditorKind::Boolean),
            "date" => Some(CellEditorKind::Date),
            "time" => Some(CellEditorKind::Time),
            "timestamp" => Some(CellEditorKind::DateTime),
            "timestamptz" => Some(CellEditorKind::Timestamp),
            "json" | "jsonb" => Some(CellEditorKind::Json),
            _ => None,
        },
        DatabaseKind::SqlServer => match base_name {
            "bit" => Some(CellEditorKind::Boolean),
            "date" => Some(CellEditorKind::Date),
            "time" => Some(CellEditorKind::Time),
            "datetime2" => Some(CellEditorKind::DateTime),
            "datetimeoffset" => Some(CellEditorKind::Timestamp),
            _ => None,
        },
        DatabaseKind::MySql
        | DatabaseKind::MariaDb
        | DatabaseKind::TiDb
        | DatabaseKind::Oracle
        | DatabaseKind::Sqlite
        | DatabaseKind::DuckDb => None,
        DatabaseKind::Redis | DatabaseKind::Valkey | DatabaseKind::MongoDb => None,
        DatabaseKind::ClickHouse | DatabaseKind::Snowflake | DatabaseKind::Elasticsearch => None,
    }?;

    Some(ColumnEditorDescription { kind })
}

fn normalize_type_name(type_name: &str) -> String {
    type_name.trim().to_ascii_lowercase()
}

fn format_time(value: NaiveTime) -> String {
    let base = value.format("%H:%M:%S").to_string();
    format_fraction(base, value.nanosecond())
}

fn format_datetime(value: NaiveDateTime) -> String {
    format_fraction(
        value.format("%Y-%m-%d %H:%M:%S").to_string(),
        value.nanosecond(),
    )
}

fn format_timestamp(value: DateTime<FixedOffset>) -> String {
    format!(
        "{} {}",
        format_fraction(
            value.format("%Y-%m-%d %H:%M:%S").to_string(),
            value.nanosecond()
        ),
        value.format("%:z")
    )
}

fn format_fraction(mut base: String, nanoseconds: u32) -> String {
    if nanoseconds != 0 {
        base.push('.');
        base.push_str(format!("{nanoseconds:09}").trim_end_matches('0'));
    }
    base
}

fn days_in_month(year: i32, month: u32) -> u32 {
    (28..=31)
        .rev()
        .find(|day| NaiveDate::from_ymd_opt(year, month, *day).is_some())
        .unwrap_or(28)
}

#[cfg(test)]
mod tests {
    use crate::db::value::CellValue;
    use crate::model::text_input::TextInput;
    use crate::profile::DatabaseKind;
    use chrono::{DateTime, NaiveDate, NaiveTime};

    use super::{
        CellEditorBuffer, CellEditorContent, CellEditorKind, CellEditorPresence, JsonBuffer,
        JsonTokenKind, TemporalDraft, classify_column_type, format_json, validate_json,
    };

    #[test]
    fn json_buffer_supports_real_multiline_editing_and_cursor_movement() {
        let mut buffer = JsonBuffer::new("{\"name\": \"Ada\"}");
        buffer.move_home();
        buffer.insert('{');
        buffer.insert('\n');
        buffer.move_down();
        assert_eq!(buffer.line(), 1);
        assert!(buffer.value().contains('\n'));
    }

    #[test]
    fn json_buffer_selects_in_both_directions_and_replaces_atomically() {
        let mut buffer = JsonBuffer::new("one\ntwo\nthree");
        buffer.begin_selection(10);
        buffer.extend_selection(2);
        assert_eq!(buffer.selected_text(), Some("e\ntwo\nth"));
        buffer.insert('X');
        assert_eq!(buffer.value(), "onXree");
        buffer.undo();
        assert_eq!(buffer.value(), "one\ntwo\nthree");
        assert_eq!(buffer.selection(), Some((2, 10)));
    }

    #[test]
    fn json_buffer_deletes_multiline_selection_as_one_undo_step() {
        let mut buffer = JsonBuffer::new("a\nb\nc");
        buffer.begin_selection(0);
        buffer.extend_selection(2);
        buffer.backspace();
        assert_eq!(buffer.value(), "b\nc");
        buffer.undo();
        assert_eq!(buffer.value(), "a\nb\nc");
    }

    #[test]
    fn json_buffer_paste_replaces_multiline_selection_atomically() {
        let mut buffer = JsonBuffer::new("{\"name\": \"Ada\"}");
        buffer.begin_selection(1);
        buffer.extend_selection(5);
        buffer.paste("\"x\": 1,\n  \"y\": \"雪\"");
        assert_eq!(buffer.value(), "{\"x\": 1,\n  \"y\": \"雪\"e\": \"Ada\"}");
        buffer.undo();
        assert_eq!(buffer.value(), "{\"name\": \"Ada\"}");
        assert_eq!(buffer.selection(), Some((1, 5)));
        buffer.redo();
        assert!(buffer.value().contains("雪"));
    }

    #[test]
    fn json_validation_preserves_source_semantics_and_reports_line_and_column() {
        assert!(validate_json("{\"n\":90071992547409931234567890,\"x\":true,\"z\":null}").is_ok());
        let error = validate_json("{\n  \"name\": \"unterminated\n}").unwrap_err();
        assert_eq!((error.line, error.column), (2, 23));
    }

    #[test]
    fn json_formatting_does_not_change_invalid_source_and_handles_escaping_unicode_and_duplicates()
    {
        let source =
            r#"{"x":"\u263A","x":2,"big":90071992547409931234567890,"ok":false,"nil":null}"#;
        let formatted = format_json(source).unwrap();
        assert!(formatted.contains("90071992547409931234567890"));
        assert!(formatted.contains("\\u263A") || formatted.contains('☺'));
        assert!(format_json("{broken").is_err());
    }

    #[test]
    fn json_tokenizer_highlights_incomplete_strings_and_invalid_fragments() {
        let tokens = JsonBuffer::new("{\"key\": \"unfinished").tokens();
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == JsonTokenKind::String)
        );
        assert_eq!(
            tokens.first().map(|token| token.kind),
            Some(JsonTokenKind::Punctuation)
        );
    }

    #[test]
    fn temporal_draft_round_trips_fraction_precision_and_fixed_offset() {
        let timestamp = DateTime::parse_from_rfc3339("2026-08-28T10:20:31.120400+05:30").unwrap();
        let draft = TemporalDraft::from_timestamp(timestamp);
        assert_eq!(draft.render(), "2026-08-28 10:20:31.1204 +05:30");
        assert_eq!(draft.parse_timestamp().unwrap(), timestamp);

        let time = NaiveTime::parse_from_str("10:20:31.120400", "%H:%M:%S%.f").unwrap();
        assert_eq!(TemporalDraft::from_time(time).render(), "10:20:31.1204");
    }

    #[test]
    fn temporal_draft_rejects_impossible_segment_values() {
        let mut draft = TemporalDraft::date(NaiveDate::from_ymd_opt(2026, 8, 28).unwrap());
        draft.set_segment(1, "13");
        assert!(draft.parse_date().is_err());
        assert!(draft.error().is_some());
    }

    #[test]
    fn typed_temporal_buffer_edits_segments_instead_of_appending_to_formatted_text() {
        let value = CellValue::Date(NaiveDate::from_ymd_opt(2026, 8, 28).unwrap());
        let description = classify_column_type(DatabaseKind::Postgres, "date");
        let mut buffer = CellEditorBuffer::from_value(&value, description);
        buffer.temporal_move_to(1);
        buffer.temporal_insert('9');
        assert_eq!(buffer.value(), Some("2026-98-28"));
        assert!(matches!(buffer.content, CellEditorContent::Typed { .. }));
    }

    #[test]
    fn classifies_supported_postgres_types() {
        let cases = [
            ("bool", CellEditorKind::Boolean),
            ("BOOLEAN", CellEditorKind::Boolean),
            ("date", CellEditorKind::Date),
            ("time(6)", CellEditorKind::Time),
            ("timestamp (3)", CellEditorKind::DateTime),
            ("timestamptz", CellEditorKind::Timestamp),
            ("json", CellEditorKind::Json),
            ("JSONB", CellEditorKind::Json),
        ];

        for (type_name, expected) in cases {
            assert_eq!(
                classify_column_type(DatabaseKind::Postgres, type_name)
                    .map(|description| description.kind),
                Some(expected),
                "type_name={type_name}"
            );
        }
    }

    #[test]
    fn classifies_supported_sql_server_types_without_treating_timestamp_as_temporal() {
        let cases = [
            ("bit", CellEditorKind::Boolean),
            ("DATE", CellEditorKind::Date),
            ("time(7)", CellEditorKind::Time),
            ("datetime2 (3)", CellEditorKind::DateTime),
            ("datetimeoffset", CellEditorKind::Timestamp),
        ];

        for (type_name, expected) in cases {
            assert_eq!(
                classify_column_type(DatabaseKind::SqlServer, type_name)
                    .map(|description| description.kind),
                Some(expected),
                "type_name={type_name}"
            );
        }

        for type_name in ["timestamp", "rowversion", "TIMESTAMP (8)", "ROWVERSION"] {
            assert_eq!(
                classify_column_type(DatabaseKind::SqlServer, type_name),
                None,
                "type_name={type_name}"
            );
        }
    }

    #[test]
    fn keeps_mysql_and_sqlite_type_inference_conservative() {
        for type_name in ["tinyint(1)", "TINYINT (1)", "boolean", "date", "json"] {
            assert_eq!(
                classify_column_type(DatabaseKind::MySql, type_name),
                None,
                "mysql type_name={type_name}"
            );
        }

        for type_name in ["1", "true", "2024-01-01", "text", "json"] {
            assert_eq!(
                classify_column_type(DatabaseKind::Sqlite, type_name),
                None,
                "sqlite type_name={type_name}"
            );
        }
    }

    #[test]
    fn editor_buffer_uses_complete_values_and_distinguishes_null_states() {
        let description = classify_column_type(DatabaseKind::Postgres, "text");
        assert_eq!(
            CellEditorBuffer::from_value(&CellValue::Text("NULL".into()), description),
            CellEditorBuffer {
                presence: CellEditorPresence::Value,
                content: CellEditorContent::Text(TextInput::from("NULL")),
                ..CellEditorBuffer::default()
            }
        );
        assert_eq!(
            CellEditorBuffer::from_value(&CellValue::Null, description),
            CellEditorBuffer {
                presence: CellEditorPresence::Null,
                content: CellEditorContent::Text(TextInput::from("null")),
                ..CellEditorBuffer::default()
            }
        );
        assert_eq!(
            CellEditorBuffer::unprovided(),
            CellEditorBuffer {
                presence: CellEditorPresence::Unprovided,
                content: CellEditorContent::Text(TextInput::default()),
                ..CellEditorBuffer::default()
            }
        );
    }

    #[test]
    fn typing_into_null_or_unprovided_buffer_creates_an_empty_text_fallback() {
        let mut unprovided = CellEditorBuffer::unprovided();
        unprovided.input_mut().insert('x');
        assert_eq!(unprovided.value(), Some("x"));
    }

    #[test]
    fn semantic_text_edit_activates_presence_but_navigation_and_noop_do_not() {
        use crate::model::text_input::TextInputEdit;

        let mut input = CellEditorBuffer::unprovided();
        input.apply_text_edit(TextInputEdit::MoveRight);
        assert_eq!(input.presence(), CellEditorPresence::Unprovided);
        input.apply_text_edit(TextInputEdit::Backspace);
        assert_eq!(input.presence(), CellEditorPresence::Unprovided);
        input.apply_text_edit(TextInputEdit::Insert('x'));
        assert_eq!(input.presence(), CellEditorPresence::Value);
    }

    #[test]
    fn cell_undo_and_redo_restore_presence_and_content() {
        use crate::model::text_input::TextInputEdit;

        let mut input = CellEditorBuffer::typed_unprovided(CellEditorKind::Boolean);
        input.apply_text_edit(TextInputEdit::Insert('t'));
        assert_eq!(input.presence(), CellEditorPresence::Value);
        assert_eq!(input.value(), Some("t"));
        input.undo();
        assert_eq!(input.presence(), CellEditorPresence::Unprovided);
        assert_eq!(input.value(), Some(""));
        input.redo();
        assert_eq!(input.presence(), CellEditorPresence::Value);
    }

    #[test]
    fn cell_paste_is_atomic_and_restores_presence_and_content() {
        let mut input = CellEditorBuffer::unprovided();
        assert!(input.paste("hello 世界"));
        assert_eq!(input.presence(), CellEditorPresence::Value);
        assert_eq!(input.value(), Some("hello 世界"));
        assert!(input.undo());
        assert_eq!(input.presence(), CellEditorPresence::Unprovided);
        assert_eq!(input.value(), Some(""));
        assert!(input.redo());
        assert_eq!(input.value(), Some("hello 世界"));
        assert!(!input.paste(""));

        let mut json = CellEditorBuffer::typed_null(CellEditorKind::Json);
        assert!(json.paste("null"));
        assert_eq!(json.presence(), CellEditorPresence::Value);
        assert!(!json.is_null());
        assert_eq!(json.value(), Some("null"));
        assert!(json.undo());
        assert_eq!(json.presence(), CellEditorPresence::Null);
        assert!(json.is_null());
    }

    #[test]
    fn cell_paste_replaces_boolean_and_temporal_typed_drafts() {
        let mut boolean = CellEditorBuffer::typed_unprovided(CellEditorKind::Boolean);
        assert!(boolean.paste("  true \n"));
        assert_eq!(boolean.value(), Some("true"));
        assert!(boolean.undo());
        assert_eq!(boolean.presence(), CellEditorPresence::Unprovided);
        assert!(!boolean.paste(" \n\t"));

        let mut date = CellEditorBuffer::typed_null(CellEditorKind::Date);
        assert!(date.paste("2026-10-14"));
        assert_eq!(date.value(), Some("2026-10-14"));
        assert_eq!(
            date.typed_value().unwrap().unwrap(),
            CellValue::Date(NaiveDate::from_ymd_opt(2026, 10, 14).unwrap())
        );
        assert!(date.undo());
        assert_eq!(date.presence(), CellEditorPresence::Null);
        assert!(!date.paste(" \n"));

        let mut timestamp = CellEditorBuffer::typed_unprovided(CellEditorKind::Timestamp);
        assert!(timestamp.paste("2026-10-14 12:34:56.123456 +08:00"));
        assert_eq!(timestamp.value(), Some("2026-10-14 12:34:56.123456 +08:00"));
        assert!(timestamp.typed_value().unwrap().is_ok());
        timestamp.temporal_move_to(6);
        timestamp.temporal_insert('0');
        assert!(timestamp.value().unwrap().contains(".023456"));
        assert!(timestamp.undo());
        assert_eq!(timestamp.value(), Some("2026-10-14 12:34:56.123456 +08:00"));
        assert!(timestamp.undo());
        assert_eq!(timestamp.presence(), CellEditorPresence::Unprovided);
    }

    #[test]
    fn text_cell_paste_replaces_selection_and_undoes_atomically() {
        let mut input = CellEditorBuffer::default();
        input.apply_text_edit(crate::model::text_input::TextInputEdit::Insert('a'));
        input.apply_text_edit(crate::model::text_input::TextInputEdit::Insert('b'));
        input.apply_text_edit(crate::model::text_input::TextInputEdit::Insert('c'));
        if let CellEditorContent::Text(text) = &mut input.content {
            text.finish_edit_group();
        }
        let CellEditorContent::Text(text) = &mut input.content else {
            panic!("default cell buffer is text")
        };
        text.begin_selection(1);
        text.extend_selection(2);

        assert!(input.paste("雪"));
        assert_eq!(input.value(), Some("a雪c"));
        assert!(input.undo());
        assert_eq!(input.value(), Some("abc"));
        assert!(input.undo());
        assert_eq!(input.value(), Some("ab"));
        assert!(input.redo());
        assert_eq!(input.value(), Some("abc"));
        assert!(input.redo());
        assert_eq!(input.value(), Some("a雪c"));
    }

    #[test]
    fn cell_paste_clears_old_redo_history() {
        use crate::model::text_input::TextInputEdit;

        let mut input = CellEditorBuffer::default();
        input.apply_text_edit(TextInputEdit::Insert('a'));
        input.finish_edit_group();
        input.apply_text_edit(TextInputEdit::Insert('b'));
        input.finish_edit_group();
        assert!(input.undo());
        assert_eq!(input.value(), Some("a"));

        assert!(input.paste("x"));
        assert_eq!(input.value(), Some("ax"));
        assert!(!input.redo());
        assert!(input.undo());
        assert_eq!(input.value(), Some("a"));
    }

    #[test]
    fn null_stays_null_until_an_actual_mutation() {
        use crate::model::text_input::TextInputEdit;

        let mut input = CellEditorBuffer::typed_null(CellEditorKind::Date);
        input.temporal_move_to(1);
        input.apply_text_edit(TextInputEdit::MoveRight);
        assert_eq!(input.presence(), CellEditorPresence::Null);
        input.temporal_insert('9');
        assert_eq!(input.presence(), CellEditorPresence::Value);
    }

    #[test]
    fn temporal_month_navigation_only_activates_when_date_changes() {
        let mut input = CellEditorBuffer::typed_unprovided(CellEditorKind::Date);
        input.temporal_move_to(1);
        assert_eq!(input.presence(), CellEditorPresence::Unprovided);
        input.temporal_shift_month(1);
        assert_eq!(input.presence(), CellEditorPresence::Value);
    }

    #[test]
    fn boolean_buffer_exposes_selection_and_preserves_explicit_null_states() {
        let description = classify_column_type(DatabaseKind::Postgres, "boolean");
        let mut value = CellEditorBuffer::from_value(&CellValue::Boolean(true), description);
        assert_eq!(value.boolean_selection(), Some(true));
        value.set_boolean(false);
        assert_eq!(value.boolean_selection(), Some(false));

        assert_eq!(
            CellEditorBuffer::from_value(&CellValue::Null, description).boolean_selection(),
            None
        );
        assert!(CellEditorBuffer::unprovided().is_unprovided());
    }

    #[test]
    fn boolean_selection_cycles_left_and_right() {
        let description = classify_column_type(DatabaseKind::Postgres, "boolean");
        let mut value = CellEditorBuffer::from_value(&CellValue::Boolean(false), description);
        value.move_boolean(1);
        assert_eq!(value.boolean_selection(), Some(true));
        value.move_boolean(-1);
        assert_eq!(value.boolean_selection(), Some(false));
    }

    #[test]
    fn typed_unprovided_and_null_retain_editor_kind_without_becoming_values() {
        for kind in [
            CellEditorKind::Boolean,
            CellEditorKind::Date,
            CellEditorKind::Time,
            CellEditorKind::DateTime,
            CellEditorKind::Timestamp,
            CellEditorKind::Json,
        ] {
            let mut unprovided = CellEditorBuffer::typed_unprovided(kind);
            assert_eq!(unprovided.presence(), CellEditorPresence::Unprovided);
            assert!(!unprovided.is_boolean() || kind == CellEditorKind::Boolean);
            assert!(unprovided.input().is_some() || kind == CellEditorKind::Json);
            assert_eq!(unprovided.presence(), CellEditorPresence::Unprovided);
            if kind != CellEditorKind::Json {
                let _ = unprovided.input_mut();
                assert_eq!(unprovided.presence(), CellEditorPresence::Unprovided);
            }

            let null = CellEditorBuffer::typed_null(kind);
            assert_eq!(null.presence(), CellEditorPresence::Null);
            assert!(
                null.temporal_draft().is_some()
                    || !matches!(
                        kind,
                        CellEditorKind::Date
                            | CellEditorKind::Time
                            | CellEditorKind::DateTime
                            | CellEditorKind::Timestamp
                    )
            );
        }
    }

    #[test]
    fn populated_typed_values_keep_existing_drafts_and_presence() {
        let value = CellValue::Date(NaiveDate::from_ymd_opt(2026, 8, 28).unwrap());
        let buffer = CellEditorBuffer::from_value(
            &value,
            classify_column_type(DatabaseKind::Postgres, "date"),
        );
        assert_eq!(buffer.presence(), CellEditorPresence::Value);
        assert_eq!(buffer.value(), Some("2026-08-28"));
        assert!(buffer.temporal_draft().is_some());
    }

    #[test]
    fn presence_aware_extraction_parses_typed_values_without_an_old_cell() {
        let mut boolean = CellEditorBuffer::typed_unprovided(CellEditorKind::Boolean);
        boolean.set_boolean(false);
        assert_eq!(
            boolean.value_for_presence().unwrap(),
            Some(CellValue::Boolean(false))
        );

        let null = CellEditorBuffer::typed_null(CellEditorKind::Timestamp);
        assert_eq!(null.value_for_presence().unwrap(), Some(CellValue::Null));

        let json_null = CellEditorBuffer::typed_unprovided(CellEditorKind::Json);
        assert_eq!(json_null.value_for_presence().unwrap(), None);
    }

    #[test]
    fn typed_temporal_and_json_extraction_preserves_precision_and_null_literal() {
        let timestamp = DateTime::parse_from_rfc3339("2026-08-28T10:20:31.120400+05:30").unwrap();
        let timestamp = CellEditorBuffer::from_value(
            &CellValue::Timestamp(timestamp),
            classify_column_type(DatabaseKind::Postgres, "timestamptz"),
        );
        assert_eq!(
            timestamp.value_for_presence().unwrap(),
            Some(CellValue::Timestamp(
                DateTime::parse_from_rfc3339("2026-08-28T10:20:31.120400+05:30").unwrap()
            ))
        );

        let json = CellEditorBuffer::typed_unprovided(CellEditorKind::Json);
        assert_eq!(
            json.value_for_presence().unwrap(),
            None,
            "unprovided JSON must not become the JSON literal null"
        );
    }
}
