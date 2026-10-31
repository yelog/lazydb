use std::collections::{BTreeSet, VecDeque};

use mongodb::bson::spec::ElementType;
use mongodb::bson::{Bson, Document};

use crate::db::mongodb::types::{MongoDocument, MongoNamespace};
use crate::db::mutation::RelationMutationRequest;
use crate::db::mutation::RowVersion;
use crate::db::value::CellValue;
use crate::model::cell_editor::{CellEditorBuffer, JsonBuffer};
use crate::model::text_input::{TextInput, TextInputEdit};

/// Shown in the modal when the connection profile forbids every write. The
/// profile is checked again on save, so a read-only profile can never emit a
/// write even if the modal was opened before the profile changed.
pub const MONGODB_READ_ONLY_MESSAGE: &str = "This profile is read-only; documents cannot be edited";

/// The field that identifies a document. It is shown in the field list but it
/// is never editable: the replacement this modal plans stays bounded to the
/// `_id` value the document was loaded with.
pub const DOCUMENT_ID_FIELD: &str = "_id";

/// Shown when a field edit targets `_id`, which the write path can never move.
pub const DOCUMENT_ID_IMMUTABLE_MESSAGE: &str =
    "the _id field is immutable: it identifies this document";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EditableRowId(pub u64);

#[derive(Clone, Debug, Default, PartialEq)]
pub enum RelationGridMode {
    #[default]
    Browse,
    EditCell(Box<CellEditorState>),
    /// A MongoDB collection's whole document, edited as Canonical EJSON.
    EditDocument(Box<DocumentEditorState>),
    VisualLine {
        anchor: usize,
    },
    Busy,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CellEditorState {
    pub row: usize,
    pub column: usize,
    pub input: CellEditorBuffer,
    pub error: Option<String>,
}

/// The two views the document modal offers.
///
/// `Ejson` is the default: the whole document as one Canonical Extended JSON
/// text. `Fields` lists one row per top-level field and edits them one by one.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DocumentEditorView {
    #[default]
    Ejson,
    Fields,
}

/// How one field's text is parsed back into BSON.
///
/// The kind comes from the value the field currently holds, so the same text
/// means different things in different fields: `true` is a bool for a boolean
/// field and the four-character string for a string field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentFieldKind {
    Boolean,
    Integer32,
    Integer64,
    Double,
    Null,
    String,
    ObjectId,
    /// A BSON date reads and edits as ISO-8601 UTC: the canonical EJSON form is
    /// unreadable and easy to get wrong by hand.
    DateTime,
    /// Documents, arrays and every other BSON type (timestamps, regexes,
    /// decimals) are edited as Canonical Extended JSON, so they keep their
    /// types when they round-trip.
    Ejson,
}

impl DocumentFieldKind {
    pub fn of(value: &Bson) -> Self {
        match value {
            Bson::Boolean(_) => Self::Boolean,
            Bson::Int32(_) => Self::Integer32,
            Bson::Int64(_) => Self::Integer64,
            Bson::Double(_) => Self::Double,
            Bson::Null => Self::Null,
            Bson::String(_) => Self::String,
            Bson::ObjectId(_) => Self::ObjectId,
            Bson::DateTime(_) => Self::DateTime,
            _ => Self::Ejson,
        }
    }

    /// Parses one field's text into the BSON value of this field's kind.
    pub fn parse(self, text: &str) -> Result<Bson, String> {
        let trimmed = text.trim();
        match self {
            Self::String => Ok(Bson::String(text.to_owned())),
            Self::Boolean => match trimmed {
                "true" => Ok(Bson::Boolean(true)),
                "false" => Ok(Bson::Boolean(false)),
                _ => Err(format!("expected true or false, got {trimmed:?}")),
            },
            Self::Integer32 => trimmed
                .parse::<i32>()
                .map(Bson::Int32)
                .map_err(|_| format!("expected a 32-bit integer, got {trimmed:?}")),
            Self::Integer64 => trimmed
                .parse::<i64>()
                .map(Bson::Int64)
                .map_err(|_| format!("expected a 64-bit integer, got {trimmed:?}")),
            Self::Double => trimmed
                .parse::<f64>()
                .map(Bson::Double)
                .map_err(|_| format!("expected a double, got {trimmed:?}")),
            Self::Null => {
                if trimmed.eq_ignore_ascii_case("null") {
                    Ok(Bson::Null)
                } else {
                    Err(format!("expected null, got {trimmed:?}"))
                }
            }
            Self::ObjectId => mongodb::bson::oid::ObjectId::parse_str(trimmed)
                .map(Bson::ObjectId)
                .map_err(|_| format!("expected a 24-character objectId, got {trimmed:?}")),
            Self::DateTime => parse_bson_date(trimmed),
            Self::Ejson => {
                let value: serde_json::Value = serde_json::from_str(trimmed)
                    .map_err(|error| format!("invalid Canonical EJSON: {error}"))?;
                Bson::try_from(value).map_err(|error| format!("invalid Canonical EJSON: {error}"))
            }
        }
    }
}

/// How a container field is summarized in the field list.
///
/// A container is edited in its own multi-line modal, so the row shows how much
/// the field holds instead of a canonical EJSON line clipped at the pane edge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentFieldShape {
    Array(usize),
    Document(usize),
}

/// One top-level field of the document, as the field list shows it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentField {
    pub name: String,
    pub kind: DocumentFieldKind,
    /// The observed BSON type of the value the field holds right now.
    pub type_name: &'static str,
    /// The compact value the field editor starts from.
    pub value: String,
    /// `Some` for the BSON containers, which are edited in the multi-line modal.
    pub shape: Option<DocumentFieldShape>,
}

impl DocumentField {
    fn new(name: &str, value: &Bson) -> Self {
        let shape = match value {
            Bson::Array(items) => Some(DocumentFieldShape::Array(items.len())),
            Bson::Document(fields) => Some(DocumentFieldShape::Document(fields.len())),
            _ => None,
        };
        Self {
            name: name.to_owned(),
            kind: DocumentFieldKind::of(value),
            type_name: bson_type_name(value),
            value: compact_bson_value(value),
            shape,
        }
    }

    /// True for an array or an embedded document.
    pub fn is_container(&self) -> bool {
        self.shape.is_some()
    }

    /// What the field row shows: the compact value of a scalar, or a shape
    /// marker for a container (the same shape the data grid shows for them).
    pub fn list_value(&self) -> String {
        match self.shape {
            Some(DocumentFieldShape::Array(entries)) => format!("[{entries} items]"),
            Some(DocumentFieldShape::Document(entries)) => format!("{{{entries} fields}}"),
            None => self.value.clone(),
        }
    }
}

/// The text one field is being edited as.
///
/// Every field opens in the modal, so both kinds are modal values: a scalar is
/// edited on one line, and a container gets a multi-line JSON buffer, because a
/// canonical EJSON array on one line is unreadable and wider than the pane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DocumentFieldInput {
    Line(TextInput),
    Block(JsonBuffer),
}

impl DocumentFieldInput {
    pub fn value(&self) -> &str {
        match self {
            Self::Line(input) => input.value(),
            Self::Block(buffer) => buffer.value(),
        }
    }

    /// True for the multi-line editor, which owns `Enter` and the cursor keys.
    pub fn is_multiline(&self) -> bool {
        matches!(self, Self::Block(_))
    }

    /// How many lines the value spans, which is one half of the modal size.
    pub fn line_count(&self) -> usize {
        self.value().split('\n').count()
    }

    /// The terminal cells of the widest projected line.
    pub fn longest_line_columns(&self) -> usize {
        self.value()
            .split('\n')
            .map(|line| {
                unicode_width::UnicodeWidthStr::width(
                    crate::security::project_editor_line(line).text.as_str(),
                )
            })
            .max()
            .unwrap_or_default()
    }

    /// The lines the value itself takes in the field modal: its own lines, plus
    /// one for the refusal when the last apply was rejected. The modal adds the
    /// chrome around it, including the margins above and below.
    pub fn modal_value_rows(&self, error: bool) -> usize {
        self.line_count() + usize::from(error)
    }

    fn apply(&mut self, edit: TextInputEdit) -> bool {
        match self {
            Self::Line(input) => input.apply(edit),
            Self::Block(buffer) => buffer.apply(edit),
        }
    }

    fn paste(&mut self, text: &str) {
        match self {
            Self::Line(input) => input.paste(text),
            Self::Block(buffer) => buffer.paste(text),
        }
    }

    fn move_line(&mut self, delta: isize) -> bool {
        match self {
            Self::Line(_) => false,
            Self::Block(buffer) => {
                if delta < 0 {
                    buffer.move_up();
                } else {
                    buffer.move_down();
                }
                true
            }
        }
    }

    /// Re-indents a multi-line field, and reports why a single-line field has
    /// nothing to format.
    fn format(&mut self) -> Result<(), String> {
        match self {
            Self::Line(_) => Err("this field is edited on one line".to_owned()),
            Self::Block(buffer) => {
                let formatted = buffer.format().map_err(|error| {
                    format!("{} at {}:{}", error.message, error.line, error.column)
                })?;
                buffer.replace_value(formatted);
                Ok(())
            }
        }
    }
}

/// The single field being edited, with the input it edits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentFieldEdit {
    pub name: String,
    pub type_name: &'static str,
    pub input: DocumentFieldInput,
}

fn bson_type_name(value: &Bson) -> &'static str {
    match value.element_type() {
        ElementType::Double => "double",
        ElementType::String => "string",
        ElementType::EmbeddedDocument => "document",
        ElementType::Array => "array",
        ElementType::Binary => "binary",
        ElementType::Undefined => "undefined",
        ElementType::ObjectId => "objectId",
        ElementType::Boolean => "bool",
        ElementType::DateTime => "date",
        ElementType::Null => "null",
        ElementType::RegularExpression => "regex",
        ElementType::DbPointer => "dbPointer",
        ElementType::JavaScriptCode => "javascript",
        ElementType::Symbol => "symbol",
        ElementType::JavaScriptCodeWithScope => "javascriptWithScope",
        ElementType::Int32 => "int32",
        ElementType::Timestamp => "timestamp",
        ElementType::Int64 => "int64",
        ElementType::Decimal128 => "decimal128",
        ElementType::MaxKey => "maxKey",
        ElementType::MinKey => "minKey",
    }
}

/// The compact one-line rendering a field row shows.
///
/// Scalars lose the Extended JSON wrapper the modal uses for the whole
/// document, so an integral field reads `1200` instead of `{"$numberLong":
/// "1200"}`. Documents, arrays and the remaining BSON types keep Canonical
/// Extended JSON, which is exactly the text their field editor parses.
fn compact_bson_value(value: &Bson) -> String {
    match value {
        Bson::String(text) => text.clone(),
        Bson::ObjectId(id) => id.to_hex(),
        Bson::Boolean(flag) => flag.to_string(),
        Bson::Int32(number) => number.to_string(),
        Bson::Int64(number) => number.to_string(),
        Bson::Double(number) => number.to_string(),
        Bson::Null => "null".to_owned(),
        Bson::DateTime(date) => iso_utc_date(date),
        other => other.clone().into_canonical_extjson().to_string(),
    }
}

/// One BSON date as ISO-8601 UTC, with milliseconds only when it has them.
///
/// The canonical EJSON form (`{"$date":{"$numberLong":"…"}}`) is what the write
/// path stores, but it is not what a person can read or safely edit, so the
/// document editor shows and accepts this instead.
fn iso_utc_date(value: &mongodb::bson::DateTime) -> String {
    let Some(timestamp) = chrono::DateTime::from_timestamp_millis(value.timestamp_millis()) else {
        // Outside chrono's range: keep the canonical form instead of inventing
        // a date the value does not hold.
        return Bson::DateTime(*value).into_canonical_extjson().to_string();
    };
    if !(0..=9999).contains(&chrono::Datelike::year(&timestamp)) {
        return Bson::DateTime(*value).into_canonical_extjson().to_string();
    }
    if timestamp.timestamp_subsec_millis() == 0 {
        timestamp.format("%Y-%m-%dT%H:%M:%SZ").to_string()
    } else {
        timestamp.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
    }
}

/// Reads a date the way a person writes one.
///
/// `2026-11-08T00:00:00Z`, `2026-11-08 15:30`, `2026-11-08 15:30:15.250` and
/// `2026-11-08` are all accepted, the forms without an offset as UTC, and so is
/// canonical or relaxed EJSON, so a value copied from the EJSON view still
/// round-trips.
fn parse_bson_date(text: &str) -> Result<Bson, String> {
    let invalid =
        || format!("expected an ISO-8601 timestamp like 2026-11-08T00:00:00Z, got {text:?}");
    if let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(text) {
        return bson_date_from_timestamp(timestamp.with_timezone(&chrono::Utc));
    }
    for format in [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(text, format) {
            return bson_date_from_timestamp(naive.and_utc());
        }
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d")
        && let Some(naive) = date.and_hms_opt(0, 0, 0)
    {
        return Ok(Bson::DateTime(mongodb::bson::DateTime::from_millis(
            naive.and_utc().timestamp_millis(),
        )));
    }
    if text.starts_with('{') {
        let value: serde_json::Value = serde_json::from_str(text)
            .map_err(|error| format!("invalid Canonical EJSON: {error}"))?;
        if let Some(object) = value.as_object()
            && object.len() == 1
            && let Some(date) = object.get("$date").and_then(serde_json::Value::as_str)
        {
            let timestamp = chrono::DateTime::parse_from_rfc3339(date)
                .map_err(|error| format!("invalid date: {error}"))?;
            return bson_date_from_timestamp(timestamp.with_timezone(&chrono::Utc));
        }
        return match Bson::try_from(value) {
            Ok(Bson::DateTime(date)) => Ok(Bson::DateTime(date)),
            Ok(other) => Err(format!("expected a date, got {}", bson_type_name(&other))),
            Err(error) => Err(format!("invalid Canonical EJSON: {error}")),
        };
    }
    Err(invalid())
}

fn bson_date_from_timestamp(timestamp: chrono::DateTime<chrono::Utc>) -> Result<Bson, String> {
    // BSON stores whole milliseconds. Refuse submillisecond precision and leap
    // seconds instead of silently changing the supplied instant.
    let nanos = timestamp.timestamp_subsec_nanos();
    if nanos >= 1_000_000_000 || !nanos.is_multiple_of(1_000_000) {
        return Err(
            "BSON dates require millisecond precision and do not support leap seconds".to_owned(),
        );
    }
    Ok(Bson::DateTime(mongodb::bson::DateTime::from_millis(
        timestamp.timestamp_millis(),
    )))
}

/// The state of the modal that edits one MongoDB document as a whole.
///
/// The write this editor plans is bounded to `id`, which is the `_id` value the
/// document was loaded with. The editor text can change every other field, but
/// it can never move the write to another document: only a replacement whose
/// `_id` still equals `id` is accepted.
#[derive(Clone, Debug, PartialEq)]
pub struct DocumentEditorState {
    /// Grid row the modal was opened from.
    pub row: usize,
    /// `database.collection` the document belongs to.
    pub collection: String,
    /// The original BSON `_id` value, kept typed and never re-serialized.
    pub id: Bson,
    /// The exact BSON document loaded when the modal opened, used for
    /// compare-and-replace so edits from another client cannot be overwritten.
    pub expected: Document,
    /// The whole document as Canonical Extended JSON.
    ///
    /// This text is the single source of truth of the modal: the field list is
    /// derived from it and applying a field writes back into it.
    pub input: JsonBuffer,
    pub error: Option<String>,
    /// True while one replacement is in flight, so one save emits one write.
    pub saving: bool,
    /// Which of the two views the modal shows.
    pub view: DocumentEditorView,
    /// Index of the field the list highlights.
    pub selected_field: usize,
    /// The field being edited, if any.
    pub field_edit: Option<DocumentFieldEdit>,
    /// The refusal of the last field edit, shown next to that field.
    pub field_error: Option<String>,
}

impl DocumentEditorState {
    pub fn new(
        row: usize,
        namespace: &MongoNamespace,
        document: &MongoDocument,
    ) -> Result<Self, String> {
        let id = document.bson.get("_id").cloned().ok_or_else(|| {
            "this document has no _id, so it cannot be replaced unambiguously".to_owned()
        })?;
        let ejson = canonical_document_ejson(&document.bson);
        Ok(Self {
            row,
            collection: namespace.full_name(),
            id,
            expected: document.bson.clone(),
            input: JsonBuffer::new(ejson),
            error: None,
            saving: false,
            view: DocumentEditorView::Fields,
            selected_field: 0,
            field_edit: None,
            field_error: None,
        })
    }

    /// The header the modal shows: the collection and the `_id` it will write.
    pub fn title(&self) -> String {
        format!(
            "{} · _id {}",
            self.collection,
            self.id.clone().into_canonical_extjson()
        )
    }

    /// The top-level fields of the document as the editor text holds it now.
    ///
    /// The list is derived from the text instead of being stored beside it, so
    /// the field view can never drift from the document that saving writes.
    pub fn fields(&self) -> Result<Vec<DocumentField>, String> {
        self.document_and_fields().map(|(_, fields)| fields)
    }

    /// The parsed document and its field rows, from one parse of the text.
    fn document_and_fields(&self) -> Result<(Document, Vec<DocumentField>), String> {
        let document = parse_document_ejson(self.input.value())?;
        let fields = document
            .iter()
            .map(|(name, value)| DocumentField::new(name, value))
            .collect();
        Ok((document, fields))
    }

    /// The row the field list highlights, clamped to the fields that exist.
    pub fn selected_field_index(&self, field_count: usize) -> usize {
        self.selected_field.min(field_count.saturating_sub(1))
    }

    /// Applies a scalar draft locally before opening an adjacent editable field.
    pub fn navigate_field_edit(&mut self, delta: isize) -> Result<(), String> {
        let Some(field) = self.field_edit.as_ref() else {
            self.move_field(delta);
            return Ok(());
        };
        if field.input.is_multiline() || delta == 0 {
            return Ok(());
        }
        let name = field.name.clone();
        let text = field.input.value().to_owned();
        self.apply_field(&name, &text)?;
        self.field_error = None;
        let fields = self.fields()?;
        let current = self.selected_field_index(fields.len());
        let next = if delta < 0 {
            (0..current)
                .rev()
                .find(|&index| fields[index].name != DOCUMENT_ID_FIELD)
        } else {
            (current + 1..fields.len()).find(|&index| fields[index].name != DOCUMENT_ID_FIELD)
        };
        if let Some(next) = next {
            self.selected_field = next;
            self.begin_field_edit();
        }
        Ok(())
    }

    /// Moves the field selection by `delta` rows, stopping at both ends.
    pub fn move_field(&mut self, delta: isize) {
        let Ok(fields) = self.fields() else {
            return;
        };
        let Some(last) = fields.len().checked_sub(1) else {
            self.selected_field = 0;
            return;
        };
        let current = self.selected_field_index(fields.len());
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta as usize).min(last)
        };
        if next != self.selected_field {
            self.selected_field = next;
            self.field_error = None;
        }
    }

    /// Opens the modal on the selected field.
    ///
    /// A container opens it multi-line, so its value is shown indented instead
    /// of as one canonical EJSON line. `_id` reports why it never opens: the
    /// identifier is immutable and the list must not offer an edit the write
    /// path would refuse.
    pub fn begin_field_edit(&mut self) {
        let (document, fields) = match self.document_and_fields() {
            Ok(parsed) => parsed,
            Err(message) => {
                self.field_edit = None;
                self.field_error = Some(message);
                return;
            }
        };
        let Some(field) = fields.get(self.selected_field_index(fields.len())) else {
            self.field_edit = None;
            self.field_error = Some("this document has no fields to edit".to_owned());
            return;
        };
        if field.name == DOCUMENT_ID_FIELD {
            self.field_edit = None;
            self.field_error = Some(DOCUMENT_ID_IMMUTABLE_MESSAGE.to_owned());
            return;
        }
        let input = match (field.is_container(), document.get(&field.name)) {
            (true, Some(value)) => {
                DocumentFieldInput::Block(JsonBuffer::new(indented_bson_value(value)))
            }
            _ => DocumentFieldInput::Line(TextInput::from(field.value.clone())),
        };
        self.field_error = None;
        self.field_edit = Some(DocumentFieldEdit {
            name: field.name.clone(),
            type_name: field.type_name,
            input,
        });
    }

    /// Writes one field's text back into the document text.
    ///
    /// The value is parsed with the field's own BSON type, written into the
    /// document the text holds, and the text is rendered again as Canonical
    /// EJSON. A refusal returns the reason and leaves the text untouched, so an
    /// invalid field edit can never reach the save path.
    pub fn apply_field(&mut self, name: &str, text: &str) -> Result<(), String> {
        if name == DOCUMENT_ID_FIELD {
            return Err(DOCUMENT_ID_IMMUTABLE_MESSAGE.to_owned());
        }
        let mut document = parse_document_ejson(self.input.value())?;
        let current = document
            .get(name)
            .ok_or_else(|| format!("the document has no field {name:?}"))?;
        let value = DocumentFieldKind::of(current).parse(text)?;
        document.insert(name, value);
        self.input = JsonBuffer::new(canonical_document_ejson(&document));
        Ok(())
    }

    /// Feeds one text edit to the field editor, if one is open.
    pub fn edit_field_text(&mut self, edit: TextInputEdit) -> bool {
        let Some(field) = self.field_edit.as_mut() else {
            return false;
        };
        field.input.apply(edit);
        self.field_error = None;
        true
    }

    /// Feeds pasted text to the field editor, if one is open.
    pub fn paste_field_text(&mut self, text: &str) -> bool {
        let Some(field) = self.field_edit.as_mut() else {
            return false;
        };
        field.input.paste(text);
        self.field_error = None;
        true
    }

    /// Moves the multi-line field cursor by `delta` lines.
    pub fn move_field_edit_line(&mut self, delta: isize) -> bool {
        let Some(field) = self.field_edit.as_mut() else {
            return false;
        };
        field.input.move_line(delta)
    }

    /// Re-indents the open multi-line field value.
    pub fn format_field_edit(&mut self) -> Result<(), String> {
        let Some(field) = self.field_edit.as_mut() else {
            return Err("no field editor is open".to_owned());
        };
        field.input.format()
    }

    /// True while the open field editor is the multi-line one, which owns
    /// `Enter` and the cursor keys instead of applying on `Enter`.
    pub fn field_edit_is_multiline(&self) -> bool {
        self.field_edit
            .as_ref()
            .is_some_and(|field| field.input.is_multiline())
    }

    /// The document to write, or the reason it cannot be written.
    ///
    /// The editor text is parsed back into BSON and must still carry the
    /// original `_id`, so a replacement can never retarget the write.
    pub fn replacement_document(&self) -> Result<Document, String> {
        let replacement = parse_document_ejson(self.input.value())?;
        if replacement.get("_id") != Some(&self.id) {
            return Err("the replacement document must keep the original _id".to_owned());
        }
        Ok(replacement)
    }
}

/// Renders one document as Canonical Extended JSON.
pub fn canonical_document_ejson(document: &Document) -> String {
    indented_bson_value(&Bson::Document(document.clone()))
}

/// Renders one BSON value as Canonical Extended JSON across lines.
///
/// An array or a nested document on one compact line is unreadable and gets
/// clipped at the pane edge, so every value the editors show is indented.
/// Scalars render as the same single token canonical EJSON would produce.
pub fn indented_bson_value(value: &Bson) -> String {
    let value = value.clone().into_canonical_extjson();
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
}

/// Parses one document from Canonical Extended JSON.
///
/// The value is parsed as JSON first and then converted to BSON, which keeps
/// Extended JSON types (`$oid`, `$numberLong`, `$date`, ...) intact.
pub fn parse_document_ejson(text: &str) -> Result<Document, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("invalid Canonical EJSON: {error}"))?;
    let bson =
        Bson::try_from(value).map_err(|error| format!("invalid Canonical EJSON: {error}"))?;
    match bson {
        Bson::Document(document) => Ok(document),
        other => Err(format!(
            "the document editor requires a BSON document, got {:?}",
            other.element_type()
        )),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditableRowState {
    Clean,
    Updated { changed_columns: BTreeSet<usize> },
    InsertDraft,
    Inserted,
    Deleted,
    Conflict { message: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditableRow {
    pub id: EditableRowId,
    pub original: Vec<CellValue>,
    pub current: Vec<CellValue>,
    pub state: EditableRowState,
    pub supplied_columns: BTreeSet<usize>,
    pub version: Option<RowVersion>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RelationMutationHistory {
    pub forward: RelationMutationRequest,
    pub inverse: RelationMutationRequest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PendingMutationHistory {
    Undo,
    Redo,
}

impl EditableRow {
    pub fn new(id: EditableRowId, values: Vec<CellValue>) -> Self {
        Self {
            id,
            original: values.clone(),
            current: values,
            state: EditableRowState::Clean,
            supplied_columns: BTreeSet::new(),
            version: None,
        }
    }

    pub fn update_cell(&mut self, column: usize, value: CellValue) -> bool {
        if matches!(self.state, EditableRowState::Deleted) {
            return false;
        }
        let Some(current) = self.current.get_mut(column) else {
            return false;
        };
        *current = value;
        if matches!(
            self.state,
            EditableRowState::InsertDraft | EditableRowState::Inserted
        ) {
            self.supplied_columns.insert(column);
            return true;
        }
        let changed_columns = self
            .current
            .iter()
            .enumerate()
            .filter_map(|(index, value)| (self.original.get(index) != Some(value)).then_some(index))
            .collect::<BTreeSet<_>>();
        self.state = if changed_columns.is_empty() {
            EditableRowState::Clean
        } else {
            EditableRowState::Updated { changed_columns }
        };
        true
    }

    pub fn restore_unprovided(&mut self, column: usize) -> bool {
        if !matches!(self.state, EditableRowState::InsertDraft) {
            return false;
        }
        if self.current.get(column).is_none() || !self.supplied_columns.remove(&column) {
            return false;
        }
        self.current[column] = CellValue::Null;
        true
    }

    pub fn mark_deleted(&mut self) -> bool {
        if matches!(self.state, EditableRowState::Deleted) {
            return false;
        }
        self.state = EditableRowState::Deleted;
        true
    }

    pub fn mark_inserted(&mut self, values: Vec<CellValue>, version: Option<RowVersion>) {
        self.current = values.clone();
        self.original = values;
        self.state = EditableRowState::Inserted;
        self.supplied_columns.clear();
        self.version = version;
    }

    pub fn mark_conflict(&mut self, message: impl Into<String>) {
        self.state = EditableRowState::Conflict {
            message: message.into(),
        };
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RelationEditSession {
    pub mode: RelationGridMode,
    pub rows: Vec<EditableRow>,
    pub yank: Option<Vec<CellValue>>,
    pub undo_depth: usize,
    pub redo_depth: usize,
    next_row_id: u64,
    undo: Vec<Vec<EditableRow>>,
    redo: Vec<Vec<EditableRow>>,
    pub mutation_undo: Vec<RelationMutationHistory>,
    pub mutation_redo: Vec<RelationMutationHistory>,
    pub pending_mutation_history: Option<PendingMutationHistory>,
    pub pending_save: VecDeque<RelationMutationRequest>,
    pub save_after_metadata_load: bool,
    pub(crate) display_revision: u64,
}

impl RelationEditSession {
    pub fn from_rows(rows: Vec<Vec<CellValue>>) -> Self {
        Self::from_rows_with_versions(rows, None).expect("unversioned rows have matching metadata")
    }

    pub fn from_rows_with_versions(
        rows: Vec<Vec<CellValue>>,
        versions: Option<Vec<RowVersion>>,
    ) -> Result<Self, String> {
        if let Some(versions) = &versions
            && versions.len() != rows.len()
        {
            return Err(format!(
                "row version count {} does not match row count {}",
                versions.len(),
                rows.len()
            ));
        }
        let mut session = Self::default();
        session.rows = rows
            .into_iter()
            .enumerate()
            .map(|(index, values)| {
                let id = session.allocate_id();
                let mut row = EditableRow::new(id, values);
                row.version = versions
                    .as_ref()
                    .and_then(|values| values.get(index).copied());
                row
            })
            .collect();
        Ok(session)
    }

    pub fn allocate_id(&mut self) -> EditableRowId {
        self.next_row_id = self.next_row_id.saturating_add(1);
        EditableRowId(self.next_row_id)
    }

    pub fn visual_range(&self, cursor: usize) -> Option<(usize, usize)> {
        let RelationGridMode::VisualLine { anchor } = self.mode else {
            return None;
        };
        Some((anchor.min(cursor), anchor.max(cursor)))
    }

    pub fn insert_row(&mut self, position: usize, values: Vec<CellValue>) -> EditableRowId {
        self.record_change();
        let id = self.allocate_id();
        let position = position.min(self.rows.len());
        let mut row = EditableRow::new(id, values);
        row.state = EditableRowState::InsertDraft;
        self.rows.insert(position, row);
        id
    }

    pub fn yank_row(&mut self, row: usize) -> bool {
        let Some(row) = self.rows.get(row) else {
            return false;
        };
        self.yank = Some(row.current.clone());
        true
    }

    pub fn update_cell(&mut self, row: usize, column: usize, value: CellValue) -> bool {
        if self.rows.get(row).is_none() {
            return false;
        }
        self.record_change();
        self.rows[row].update_cell(column, value)
    }

    pub fn restore_unprovided(&mut self, row: usize, column: usize) -> bool {
        let Some(editable_row) = self.rows.get(row) else {
            return false;
        };
        if !matches!(editable_row.state, EditableRowState::InsertDraft)
            || editable_row.current.get(column).is_none()
            || !editable_row.supplied_columns.contains(&column)
        {
            return false;
        }
        self.record_change();
        self.rows[row].restore_unprovided(column)
    }

    pub fn delete_rows(&mut self, range: std::ops::RangeInclusive<usize>) -> bool {
        let mut changed = false;
        self.record_change();
        for index in range.rev() {
            if let Some(row) = self.rows.get_mut(index) {
                if matches!(row.state, EditableRowState::InsertDraft) {
                    self.rows.remove(index);
                    changed = true;
                } else {
                    changed |= row.mark_deleted();
                }
            }
        }
        if !changed {
            self.undo.pop();
        }
        changed
    }

    pub fn paste_row(&mut self, position: usize) -> bool {
        let Some(values) = self.yank.clone() else {
            return false;
        };
        let id = self.insert_row(position, values);
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            row.supplied_columns = (0..row.current.len()).collect();
        }
        true
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.rows.clone());
        self.rows = previous;
        self.touch_display_revision();
        self.sync_history_depth();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.rows.clone());
        self.rows = next;
        self.touch_display_revision();
        self.sync_history_depth();
        true
    }

    pub fn discard_changes(&mut self) {
        self.rows
            .retain(|row| !matches!(row.state, EditableRowState::InsertDraft));
        for row in &mut self.rows {
            row.current = row.original.clone();
            row.state = EditableRowState::Clean;
            row.supplied_columns.clear();
        }
        self.clear_edit_bookkeeping();
        self.touch_display_revision();
    }

    pub fn commit_changes(&mut self) {
        self.rows
            .retain(|row| !matches!(row.state, EditableRowState::Deleted));
        for row in &mut self.rows {
            row.original = row.current.clone();
            row.state = EditableRowState::Clean;
            row.supplied_columns.clear();
        }
        self.clear_edit_bookkeeping();
        self.touch_display_revision();
    }

    pub fn sync_history_depth(&mut self) {
        self.undo_depth = self.undo.len();
        self.redo_depth = self.redo.len();
    }

    pub fn has_dirty_rows(&self) -> bool {
        self.rows
            .iter()
            .any(|row| !matches!(row.state, EditableRowState::Clean))
    }

    pub fn has_pending_work(&self) -> bool {
        matches!(self.mode, RelationGridMode::Busy)
            || !self.pending_save.is_empty()
            || self.save_after_metadata_load
            || self.pending_mutation_history.is_some()
    }

    pub fn has_unfinished_cell_edit(&self) -> bool {
        matches!(self.mode, RelationGridMode::EditCell(_))
    }

    /// True while the document modal is open on this collection.
    pub fn has_unfinished_document_edit(&self) -> bool {
        matches!(self.mode, RelationGridMode::EditDocument(_))
    }

    pub fn record_mutation(&mut self, history: RelationMutationHistory) {
        self.mutation_undo.push(history);
        self.mutation_redo.clear();
    }

    pub fn pending_mutation(
        &mut self,
        direction: PendingMutationHistory,
    ) -> Option<RelationMutationRequest> {
        let request = match direction {
            PendingMutationHistory::Undo => self.mutation_undo.last()?.inverse.clone(),
            PendingMutationHistory::Redo => self.mutation_redo.last()?.forward.clone(),
        };
        self.pending_mutation_history = Some(direction);
        Some(request)
    }

    pub fn complete_mutation(&mut self) -> bool {
        let Some(direction) = self.pending_mutation_history.take() else {
            return false;
        };
        match direction {
            PendingMutationHistory::Undo => {
                let Some(history) = self.mutation_undo.pop() else {
                    return false;
                };
                self.mutation_redo.push(history);
            }
            PendingMutationHistory::Redo => {
                let Some(history) = self.mutation_redo.pop() else {
                    return false;
                };
                self.mutation_undo.push(history);
            }
        }
        true
    }

    fn record_change(&mut self) {
        self.undo.push(self.rows.clone());
        self.redo.clear();
        self.touch_display_revision();
        self.sync_history_depth();
    }

    fn touch_display_revision(&mut self) {
        self.display_revision = self.display_revision.wrapping_add(1);
    }

    fn clear_edit_bookkeeping(&mut self) {
        self.mode = RelationGridMode::Browse;
        self.undo.clear();
        self.redo.clear();
        self.mutation_undo.clear();
        self.mutation_redo.clear();
        self.pending_mutation_history = None;
        self.pending_save.clear();
        self.save_after_metadata_load = false;
        self.sync_history_depth();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CellEditorState, EditableRow, EditableRowId, EditableRowState, PendingMutationHistory,
        RelationEditSession, RelationGridMode, RelationMutationHistory, RowVersion,
    };
    use crate::db::value::CellValue;
    use crate::model::cell_editor::CellEditorBuffer;

    fn row() -> EditableRow {
        EditableRow::new(
            EditableRowId(1),
            vec![CellValue::Integer(1), CellValue::Text("old".into())],
        )
    }

    #[test]
    fn versioned_rows_preserve_order_and_reject_mismatched_versions() {
        let rows = vec![vec![CellValue::Integer(1)], vec![CellValue::Integer(2)]];
        let versions = vec![RowVersion::PostgresXmin(11), RowVersion::PostgresXmin(22)];
        let session = RelationEditSession::from_rows_with_versions(rows, Some(versions)).unwrap();
        assert_eq!(session.rows[0].id, EditableRowId(1));
        assert_eq!(session.rows[1].id, EditableRowId(2));
        assert_eq!(session.rows[0].version, Some(RowVersion::PostgresXmin(11)));
        assert_eq!(session.rows[1].version, Some(RowVersion::PostgresXmin(22)));

        let error = RelationEditSession::from_rows_with_versions(
            vec![vec![CellValue::Integer(1)]],
            Some(Vec::new()),
        )
        .unwrap_err();
        assert!(error.contains("version count 0 does not match row count 1"));
    }

    #[test]
    fn row_versions_survive_history_discard_and_snapshot_clone() {
        let mut session = RelationEditSession::from_rows_with_versions(
            vec![vec![CellValue::Integer(1)]],
            Some(vec![RowVersion::PostgresXmin(7)]),
        )
        .unwrap();
        let snapshot = session.clone();
        session.update_cell(0, 0, CellValue::Integer(2));
        assert!(session.undo());
        assert_eq!(session.rows[0].version, snapshot.rows[0].version);
        assert!(session.redo());
        assert_eq!(session.rows[0].version, snapshot.rows[0].version);
        session.discard_changes();
        assert_eq!(session.rows[0].version, Some(RowVersion::PostgresXmin(7)));
        assert_eq!(snapshot.rows[0].version, Some(RowVersion::PostgresXmin(7)));

        let inserted = session.insert_row(1, vec![CellValue::Integer(3)]);
        let inserted_row = session.rows.iter().find(|row| row.id == inserted).unwrap();
        assert_eq!(inserted_row.version, None);
        assert!(session.yank_row(0));
        assert_eq!(session.yank, Some(vec![CellValue::Integer(1)]));
    }

    #[test]
    fn changing_a_cell_marks_only_changed_columns() {
        let mut row = row();
        assert!(row.update_cell(1, CellValue::Text("new".into())));
        assert_eq!(
            row.state,
            EditableRowState::Updated {
                changed_columns: [1].into_iter().collect()
            }
        );
        assert!(row.update_cell(1, CellValue::Text("old".into())));
        assert_eq!(row.state, EditableRowState::Clean);
    }

    #[test]
    fn visual_line_range_is_inclusive_and_direction_independent() {
        let mut session = RelationEditSession::from_rows(vec![vec![CellValue::Integer(1)]; 5]);
        session.mode = RelationGridMode::VisualLine { anchor: 3 };
        assert_eq!(session.visual_range(1), Some((1, 3)));
        assert_eq!(session.visual_range(4), Some((3, 4)));
    }

    #[test]
    fn inserted_rows_receive_stable_ids_and_yank_is_structured() {
        let mut session = RelationEditSession::from_rows(vec![vec![CellValue::Integer(1)]]);
        let first = session.rows[0].id;
        let inserted = session.insert_row(0, vec![CellValue::Integer(2)]);
        assert_ne!(first, inserted);
        assert!(session.yank_row(0));
        assert_eq!(session.yank, Some(vec![CellValue::Integer(2)]));
        assert_ne!(session.rows[1].id, session.rows[0].id);
    }

    #[test]
    fn deleted_rows_cannot_be_updated_or_deleted_twice() {
        let mut row = row();
        assert!(row.mark_deleted());
        assert!(!row.mark_deleted());
        assert!(!row.update_cell(1, CellValue::Text("no".into())));
    }

    #[test]
    fn delete_marks_existing_rows_but_removes_uncommitted_inserts() {
        let mut session = RelationEditSession::from_rows(vec![vec![CellValue::Integer(1)]]);
        session.insert_row(1, vec![CellValue::Null]);

        assert!(session.delete_rows(0..=0));
        assert_eq!(session.rows[0].state, EditableRowState::Deleted);
        assert!(session.delete_rows(1..=1));
        assert_eq!(session.rows.len(), 1);
    }

    #[test]
    fn insert_draft_tracks_only_explicitly_edited_columns() {
        let mut session = RelationEditSession::default();
        session.insert_row(0, vec![CellValue::Null, CellValue::Null]);

        assert!(session.update_cell(0, 1, CellValue::Integer(7)));
        assert_eq!(session.rows[0].supplied_columns, [1].into_iter().collect());
    }

    #[test]
    fn insert_draft_tracks_explicit_false_and_null() {
        let mut session = RelationEditSession::default();
        session.insert_row(0, vec![CellValue::Null, CellValue::Null]);

        assert!(session.update_cell(0, 0, CellValue::Boolean(false)));
        assert!(session.update_cell(0, 1, CellValue::Null));
        assert_eq!(
            session.rows[0].supplied_columns,
            [0, 1].into_iter().collect()
        );
    }

    #[test]
    fn restore_unprovided_omits_only_the_selected_insert_column() {
        let mut session = RelationEditSession::default();
        session.insert_row(
            0,
            vec![
                CellValue::Integer(1),
                CellValue::Text("value".into()),
                CellValue::Null,
            ],
        );
        session.update_cell(0, 0, CellValue::Integer(7));
        session.update_cell(0, 1, CellValue::Text("changed".into()));
        let undo_depth = session.undo_depth;

        assert!(session.restore_unprovided(0, 0));
        assert_eq!(session.rows[0].current[0], CellValue::Null);
        assert_eq!(session.rows[0].supplied_columns, [1].into_iter().collect());
        assert_eq!(session.undo_depth, undo_depth + 1);

        assert!(session.undo());
        assert_eq!(session.rows[0].current[0], CellValue::Integer(7));
        assert_eq!(
            session.rows[0].supplied_columns,
            [0, 1].into_iter().collect()
        );
        assert!(session.redo());
        assert_eq!(session.rows[0].current[0], CellValue::Null);
        assert_eq!(session.rows[0].supplied_columns, [1].into_iter().collect());
    }

    #[test]
    fn restore_unprovided_rejects_existing_rows_and_invalid_columns_without_history() {
        let mut session = RelationEditSession::from_rows(vec![vec![CellValue::Null]]);
        let undo_depth = session.undo_depth;
        assert!(!session.restore_unprovided(0, 0));
        assert!(!session.restore_unprovided(0, 1));
        assert_eq!(session.undo_depth, undo_depth);

        session.insert_row(1, vec![CellValue::Null]);
        let undo_depth = session.undo_depth;
        assert!(!session.restore_unprovided(1, 0));
        assert_eq!(session.undo_depth, undo_depth);
        assert!(session.rows[1].supplied_columns.is_empty());
    }

    #[test]
    fn typed_mutation_history_moves_only_after_success() {
        let request = relation_request();
        let mut session = RelationEditSession::default();
        session.record_mutation(RelationMutationHistory {
            forward: request.clone(),
            inverse: request,
        });
        assert_eq!(session.mutation_undo.len(), 1);
        assert!(
            session
                .pending_mutation(PendingMutationHistory::Undo)
                .is_some()
        );
        assert_eq!(session.mutation_undo.len(), 1);
        assert!(session.complete_mutation());
        assert!(session.mutation_undo.is_empty());
        assert_eq!(session.mutation_redo.len(), 1);
    }

    fn relation_request() -> crate::db::mutation::RelationMutationRequest {
        crate::db::mutation::RelationMutationRequest {
            tab_id: uuid::Uuid::nil(),
            tab_generation: 1,
            edit_generation: 1,
            row_id: EditableRowId(1),
            connection: crate::identity::ConnectionIdentity {
                profile_id: uuid::Uuid::nil(),
                generation: 1,
            },
            target: crate::model::execution_target::ExecutionTarget {
                profile_id: uuid::Uuid::nil(),
                database: "db".into(),
                schema: None,
            },
            relation: crate::db::catalog::CatalogId::new(
                uuid::Uuid::nil(),
                crate::db::catalog::CatalogKind::Table,
                ["db", "items"],
            ),
            relation_key: crate::model::relation::RelationKey {
                profile_id: uuid::Uuid::nil(),
                object_id: crate::db::catalog::CatalogId::new(
                    uuid::Uuid::nil(),
                    crate::db::catalog::CatalogKind::Table,
                    ["db", "items"],
                ),
            },
            scope: crate::profile::CatalogScope::for_profile(
                crate::profile::DatabaseKind::Sqlite,
                "db",
                None,
            ),
            metadata: crate::db::mutation::MetadataFingerprint {
                relation: "items".into(),
                columns: vec![("id".into(), "INTEGER".into(), false)],
                primary_key: vec!["id".into()],
            },
            operation: crate::db::mutation::RelationMutation::DeleteRows(Vec::new()),
        }
    }

    #[test]
    fn commit_and_discard_clear_all_edit_bookkeeping() {
        let mut session = RelationEditSession::from_rows(vec![vec![CellValue::Integer(1)]]);
        session.update_cell(0, 0, CellValue::Integer(2));
        session.mode = RelationGridMode::EditCell(Box::new(CellEditorState {
            row: 0,
            column: 0,
            input: CellEditorBuffer::default(),
            error: None,
        }));
        session.pending_save.push_back(relation_request());
        session.mutation_undo.push(RelationMutationHistory {
            forward: relation_request(),
            inverse: relation_request(),
        });
        session.mutation_redo.push(RelationMutationHistory {
            forward: relation_request(),
            inverse: relation_request(),
        });
        session.pending_mutation_history = Some(PendingMutationHistory::Undo);

        session.commit_changes();
        assert!(session.mutation_undo.is_empty());
        assert!(session.mutation_redo.is_empty());
        assert!(session.pending_mutation_history.is_none());
        assert!(session.pending_save.is_empty());
        assert!(matches!(session.mode, RelationGridMode::Browse));
        assert!(session.rows.iter().all(|row| {
            matches!(row.state, EditableRowState::Clean) && row.current == row.original
        }));

        session.update_cell(0, 0, CellValue::Integer(3));
        session.pending_save.push_back(relation_request());
        session.mutation_undo.push(RelationMutationHistory {
            forward: relation_request(),
            inverse: relation_request(),
        });
        session.discard_changes();
        assert!(session.mutation_undo.is_empty());
        assert!(session.pending_save.is_empty());
        assert_eq!(session.rows[0].current, vec![CellValue::Integer(2)]);
    }

    #[test]
    fn cell_editor_state_can_hold_a_typed_boolean_without_changing_relation_mode() {
        let mut session = RelationEditSession::from_rows(vec![vec![CellValue::Boolean(true)]]);
        session.mode = RelationGridMode::EditCell(Box::new(CellEditorState {
            row: 0,
            column: 0,
            input: CellEditorBuffer {
                presence: crate::model::cell_editor::CellEditorPresence::Value,
                content: crate::model::cell_editor::CellEditorContent::Typed {
                    kind: crate::model::cell_editor::CellEditorKind::Boolean,
                    draft: crate::model::cell_editor::TypedDraft::Boolean(
                        crate::model::text_input::TextInput::from("true"),
                    ),
                },
                ..CellEditorBuffer::default()
            },
            error: None,
        }));
        assert!(matches!(session.mode, RelationGridMode::EditCell(_)));
    }
}
