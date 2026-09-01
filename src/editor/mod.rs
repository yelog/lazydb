#![allow(dead_code)]

use std::{cell::RefCell, collections::HashMap};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use uuid::Uuid;

use crate::model::{
    editor::{
        EditorHighlightKind, EditorMode, EditorPosition, EditorPromptKind, EditorPromptSnapshot,
        EditorRenderLine, EditorRenderSnapshot, EditorRenderSpan, EditorSelection,
        EditorSelectionShape, EditorViewport,
    },
    workspace::{Focus, PaneResize, pane_resize},
};
use crate::security::project_editor_line;
use crate::sql::{self, ScopeKind, ScopeSelection, SqlDialect, TextRange};
use modalkit::{actions::Editable, keybindings::BindingMachine, prelude::Register};

mod prompt;
mod substitute;
use prompt::PromptSession;
use substitute::{LineRange, SubstitutionPlan};

#[derive(Clone, Debug, Eq, PartialEq)]
struct LazyDbApplicationInfo;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ContentId(Uuid);

impl modalkit::editing::application::ApplicationContentId for ContentId {}
impl modalkit::editing::application::ApplicationWindowId for ContentId {}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ApplicationAction {
    Effect(EditorEffect),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EditorEffect {
    Changed { console_id: Uuid, revision: u64 },
    Yanked(String),
    CopyStatement,
    CopyBuffer,
    RunCurrent,
    RunAll,
    FormatCurrent,
    NewConsole,
    GotoSqlConsole,
    CloseConsole,
    DeleteConsole,
    OpenSqlEditorList,
    FocusPane(Focus),
    ResizePane(PaneResize),
    ResetPaneSizes,
    NextTab,
    PreviousTab,
    ShowHelp,
    ToggleTransaction,
    SetTransactionModeRequested { manual: bool },
    TransactionControl,
    Commit,
    Rollback,
    ClearTransactionOutcome,
    SetConnectionTarget(String),
    SetDatabaseTarget(String),
    SetSchemaTarget(String),
    OpenTargetSelector,
    Quit,
    Message(String),
    BackwardSearch,
    SubstituteConfirmRequested { count: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EditorSessionCapability {
    Editable,
    ReadOnly,
}

impl modalkit::editing::application::ApplicationAction for ApplicationAction {
    fn is_edit_sequence(
        &self,
        _: &modalkit::editing::context::EditContext,
    ) -> modalkit::keybindings::SequenceStatus {
        modalkit::keybindings::SequenceStatus::Break
    }

    fn is_last_action(
        &self,
        _: &modalkit::editing::context::EditContext,
    ) -> modalkit::keybindings::SequenceStatus {
        modalkit::keybindings::SequenceStatus::Ignore
    }

    fn is_last_selection(
        &self,
        _: &modalkit::editing::context::EditContext,
    ) -> modalkit::keybindings::SequenceStatus {
        modalkit::keybindings::SequenceStatus::Ignore
    }

    fn is_switchable(&self, _: &modalkit::editing::context::EditContext) -> bool {
        false
    }
}

impl modalkit::editing::application::ApplicationInfo for LazyDbApplicationInfo {
    type Error = String;
    type Action = ApplicationAction;
    type Store = ();
    type WindowId = ContentId;
    type ContentId = ContentId;

    fn content_of_command(_: modalkit::prelude::CommandType) -> Self::ContentId {
        ContentId(Uuid::nil())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EditorError {
    #[error("editor text is missing its sentinel newline")]
    MissingSentinel,
    #[error("editor session does not exist: {0}")]
    MissingSession(Uuid),
    #[error("editor operation failed: {0}")]
    Operation(String),
}

fn encode_editor_text(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len() + 1);
    encoded.push_str(text);
    encoded.push('\n');
    encoded
}

fn decode_editor_text(text: &str) -> Result<String, EditorError> {
    text.strip_suffix('\n')
        .map(str::to_owned)
        .ok_or(EditorError::MissingSentinel)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EditorKey {
    Character(char),
    Escape,
    Enter,
    Backspace,
    Control(char),
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Tab,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReplacementCursor {
    Start,
    EndOfInsertion,
    PreserveRelative,
}

struct EditorSession {
    buffer: modalkit::editing::store::SharedBuffer<LazyDbApplicationInfo>,
    group_id: modalkit::editing::buffer::CursorGroupId,
    viewport: modalkit::prelude::ViewportContext<modalkit::editing::cursor::Cursor>,
    keys: VimKeyManager,
    pending_binding: Option<PendingBinding>,
    pending_count: Option<u32>,
    current_sequence: Vec<EditorKey>,
    last_sequence: Option<Vec<EditorKey>>,
    mode: EditorMode,
    position: EditorPosition,
    revision: u64,
    previous_text: Option<String>,
    redo_text: Option<String>,
    capability: EditorSessionCapability,
    interacted: bool,
}

type VimKeyManager = modalkit::editing::key::KeyManager<
    modalkit::key::TerminalKey,
    modalkit::actions::Action<LazyDbApplicationInfo>,
    modalkit::prelude::RepeatType,
>;

fn mode_from_key_manager(keys: &VimKeyManager) -> EditorMode {
    match keys.show_mode().as_deref() {
        Some("-- INSERT --") => EditorMode::Insert,
        Some("-- REPLACE --") => EditorMode::Replace,
        Some("-- VISUAL LINE --") => EditorMode::VisualLine,
        Some("-- VISUAL BLOCK --") => EditorMode::VisualBlock,
        Some("-- VISUAL --") => EditorMode::VisualChar,
        _ => EditorMode::Normal,
    }
}

pub(crate) struct EditorWorkspace {
    store: modalkit::editing::store::Store<LazyDbApplicationInfo>,
    sessions: HashMap<Uuid, EditorSession>,
    registers: HashMap<char, String>,
    effects: Vec<EditorEffect>,
    keys: VimKeyManager,
    pending_binding: Option<PendingBinding>,
    current_sequence: Vec<EditorKey>,
    last_sequence: Option<Vec<EditorKey>>,
    prompt: Option<PromptSession>,
    command_history: Vec<String>,
    last_search: Option<String>,
    last_search_backward: bool,
    previous_substitute_pattern: Option<String>,
    substitute: Option<PendingSubstitute>,
    analysis_cache: RefCell<HashMap<sql::AnalysisKey, Vec<sql::HighlightSpan>>>,
}

#[derive(Clone, Debug)]
struct PendingSubstitute {
    console_id: Uuid,
    plan: SubstitutionPlan,
    accepted: Vec<usize>,
    next: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingBinding {
    Leader,
    LeaderTransaction,
    Window(u32),
    Goto,
}

impl Default for EditorWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorWorkspace {
    pub(crate) fn new() -> Self {
        Self {
            store: Default::default(),
            sessions: HashMap::new(),
            registers: HashMap::new(),
            effects: Vec::new(),
            keys: VimKeyManager::new(modalkit::env::vim::keybindings::default_vim_keys()),
            pending_binding: None,
            current_sequence: Vec::new(),
            last_sequence: None,
            prompt: None,
            command_history: Vec::new(),
            last_search: None,
            last_search_backward: false,
            previous_substitute_pattern: None,
            substitute: None,
            analysis_cache: RefCell::new(HashMap::new()),
        }
    }

    pub(crate) fn open_console(&mut self, id: Uuid, text: &str) {
        self.open_session(id, text, EditorSessionCapability::Editable);
    }

    pub(crate) fn open_read_only(&mut self, id: Uuid, text: &str) {
        self.open_session(id, text, EditorSessionCapability::ReadOnly);
    }

    fn open_session(&mut self, id: Uuid, text: &str, capability: EditorSessionCapability) {
        let mut buffer = modalkit::editing::buffer::EditBuffer::from_str(
            ContentId(id),
            &encode_editor_text(text),
        );
        let group_id = buffer.create_group();
        buffer.set_leader(group_id, modalkit::editing::cursor::Cursor::new(0, 0));
        let buffer = std::sync::Arc::new(std::sync::RwLock::new(buffer));
        let mut keys = VimKeyManager::new(modalkit::env::vim::keybindings::default_vim_keys());
        let mode = if capability == EditorSessionCapability::Editable {
            keys.input_key(modalkit::key::TerminalKey::from(KeyEvent::new(
                KeyCode::Char('i'),
                KeyModifiers::NONE,
            )));
            while keys.pop().is_some() {}
            EditorMode::Insert
        } else {
            EditorMode::Normal
        };
        self.sessions.insert(
            id,
            EditorSession {
                buffer,
                group_id,
                viewport: Default::default(),
                keys,
                pending_binding: None,
                pending_count: None,
                current_sequence: Vec::new(),
                last_sequence: None,
                mode,
                position: EditorPosition { line: 0, column: 0 },
                revision: 0,
                previous_text: None,
                redo_text: None,
                capability,
                interacted: false,
            },
        );
    }

    pub(crate) fn close_console(&mut self, id: Uuid) {
        self.sessions.remove(&id);
    }

    pub(crate) fn has_session(&self, id: Uuid) -> bool {
        self.sessions.contains_key(&id)
    }

    pub(crate) fn key(&mut self, id: Uuid, event: KeyEvent) -> Result<(), EditorError> {
        let key = if event.modifiers.contains(KeyModifiers::CONTROL) {
            match event.code {
                KeyCode::Char(c) => EditorKey::Control(c),
                _ => return Ok(()),
            }
        } else {
            match event.code {
                KeyCode::Char(c) => EditorKey::Character(c),
                KeyCode::Esc => EditorKey::Escape,
                KeyCode::Enter => EditorKey::Enter,
                KeyCode::Backspace => EditorKey::Backspace,
                KeyCode::Delete => EditorKey::Delete,
                KeyCode::Left => EditorKey::Left,
                KeyCode::Right => EditorKey::Right,
                KeyCode::Up => EditorKey::Up,
                KeyCode::Down => EditorKey::Down,
                KeyCode::Home => EditorKey::Home,
                KeyCode::End => EditorKey::End,
                KeyCode::Tab => EditorKey::Tab,
                _ => return Ok(()),
            }
        };
        self.press(id, key)
    }

    pub(crate) fn scroll(
        &mut self,
        id: Uuid,
        rows: isize,
        columns: isize,
    ) -> Result<(), EditorError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let corner = &mut session.viewport.corner;
        if rows.is_negative() {
            corner.set_y(corner.get_y().saturating_sub(rows.unsigned_abs()));
        } else {
            corner.set_y(corner.get_y().saturating_add(rows as usize));
        }
        if columns.is_negative() {
            corner.set_x(corner.get_x().saturating_sub(columns.unsigned_abs()));
        } else {
            corner.set_x(corner.get_x().saturating_add(columns as usize));
        }
        Ok(())
    }

    pub(crate) fn text(&self, id: Uuid) -> Result<String, EditorError> {
        let session = self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let buffer = session
            .buffer
            .read()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        decode_editor_text(&buffer.get_text())
    }

    pub(crate) fn mode(&self, id: Uuid) -> Result<EditorMode, EditorError> {
        let session = self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?;
        Ok(mode_from_key_manager(&session.keys))
    }

    pub(crate) fn position(&self, id: Uuid) -> Result<EditorPosition, EditorError> {
        let session = self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let mut buffer = session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        let cursor = buffer.get_leader(session.group_id);
        Ok(EditorPosition {
            line: cursor.get_y(),
            column: cursor.get_x(),
        })
    }

    pub(crate) fn revision(&self, id: Uuid) -> Result<u64, EditorError> {
        self.sessions
            .get(&id)
            .map(|session| session.revision)
            .ok_or(EditorError::MissingSession(id))
    }

    pub(crate) fn viewport(&self, id: Uuid) -> Result<EditorViewport, EditorError> {
        let session = self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?;
        Ok(EditorViewport {
            width: session.viewport.get_width(),
            height: session.viewport.get_height(),
        })
    }

    pub(crate) fn render_snapshot(
        &self,
        id: Uuid,
        viewport: EditorViewport,
    ) -> Result<EditorRenderSnapshot, EditorError> {
        self.render_snapshot_with_dialect(id, viewport, SqlDialect::Generic)
    }

    pub(crate) fn render_snapshot_with_dialect(
        &self,
        id: Uuid,
        viewport: EditorViewport,
        dialect: SqlDialect,
    ) -> Result<EditorRenderSnapshot, EditorError> {
        self.render_snapshot_with_dialect_and_statement(id, viewport, dialect, None)
    }

    pub(crate) fn render_snapshot_with_dialect_and_statement(
        &self,
        id: Uuid,
        viewport: EditorViewport,
        dialect: SqlDialect,
        statement: Option<sql::TextRange>,
    ) -> Result<EditorRenderSnapshot, EditorError> {
        self.render_snapshot_with_sql_ranges(id, viewport, dialect, statement, None)
    }

    pub(crate) fn render_snapshot_with_dialect_and_ranges(
        &self,
        id: Uuid,
        viewport: EditorViewport,
        dialect: SqlDialect,
        sql_ranges: &[sql::TextRange],
    ) -> Result<EditorRenderSnapshot, EditorError> {
        self.render_snapshot_with_sql_ranges(id, viewport, dialect, None, Some(sql_ranges))
    }

    fn render_snapshot_with_sql_ranges(
        &self,
        id: Uuid,
        viewport: EditorViewport,
        dialect: SqlDialect,
        statement: Option<sql::TextRange>,
        sql_ranges: Option<&[sql::TextRange]>,
    ) -> Result<EditorRenderSnapshot, EditorError> {
        let session = self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let mut buffer = session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        let total_lines = buffer.get_lines().max(1);
        let first_line = session
            .viewport
            .corner
            .get_y()
            .min(total_lines.saturating_sub(1));
        let overscan = 2;
        let full_text = decode_editor_text(&buffer.get_text())?;
        let key = sql::AnalysisKey {
            console_id: id,
            document_revision: session.revision,
            dialect,
            highlight_ranges: sql_ranges.map(<[sql::TextRange]>::to_vec),
        };
        let highlights = self
            .analysis_cache
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                if full_text.chars().any(|character| {
                    character.is_control() && character != '\n' && character != '\t'
                }) {
                    Vec::new()
                } else {
                    sql_ranges.map_or_else(
                        || sql::highlight_sql(&full_text, dialect),
                        |ranges| sql::highlight_sql_ranges(&full_text, ranges, dialect),
                    )
                }
            })
            .clone();
        let lines = buffer
            .lines(first_line)
            .take(viewport.height.saturating_add(overscan))
            .enumerate()
            .map(|(offset, line)| {
                let source = line.to_string();
                let projection = project_editor_line(&source);
                let line_start = full_text
                    .split_inclusive('\n')
                    .take(first_line + offset)
                    .map(str::len)
                    .sum::<usize>();
                let line_end = line_start + source.len();
                let mut spans = Vec::new();
                let mut byte = 0;
                for highlight in highlights
                    .iter()
                    .filter(|item| item.range.start < line_end && item.range.end > line_start)
                {
                    let start = highlight.range.start.max(line_start) - line_start;
                    let end = highlight.range.end.min(line_end) - line_start;
                    if start > byte {
                        spans.push(render_span(
                            &source,
                            byte,
                            start,
                            EditorHighlightKind::Plain,
                            statement,
                            line_start,
                        ));
                    }
                    if end > start {
                        spans.push(render_span(
                            &source,
                            start,
                            end,
                            map_highlight(highlight.kind),
                            statement,
                            line_start,
                        ));
                    }
                    byte = byte.max(end);
                }
                if byte < source.len() {
                    spans.push(render_span(
                        &source,
                        byte,
                        source.len(),
                        EditorHighlightKind::Plain,
                        statement,
                        line_start,
                    ));
                }
                EditorRenderLine {
                    line: first_line + offset,
                    spans: if spans.is_empty() {
                        vec![EditorRenderSpan {
                            text: projection.text,
                            source_start: 0,
                            source_end: source.len(),
                            kind: EditorHighlightKind::Plain,
                            current_statement: statement.is_some_and(|range| {
                                range.start < line_end && range.end > line_start
                            }),
                        }]
                    } else {
                        spans
                    },
                    source_to_display_cells: projection.source_to_display_cells,
                }
            })
            .collect::<Vec<_>>();
        let selection =
            buffer
                .get_leader_selection(session.group_id)
                .map(|(cursor, anchor, shape)| EditorSelection {
                    start: EditorPosition {
                        line: cursor.get_y(),
                        column: cursor.get_x(),
                    },
                    end: EditorPosition {
                        line: anchor.get_y(),
                        column: anchor.get_x(),
                    },
                    shape: match shape {
                        modalkit::prelude::TargetShape::CharWise => EditorSelectionShape::Char,
                        modalkit::prelude::TargetShape::LineWise => EditorSelectionShape::Line,
                        modalkit::prelude::TargetShape::BlockWise => EditorSelectionShape::Block,
                    },
                });
        let selections = selection.into_iter().collect::<Vec<_>>();
        let selection_cells = selections
            .iter()
            .flat_map(|selection| {
                let first_line = selection.start.line.min(selection.end.line);
                let last_line = selection.start.line.max(selection.end.line);
                (first_line..=last_line).filter_map(|line| {
                    let line_data = lines.iter().find(|item| item.line == line)?;
                    let (start, end) =
                        match selection.shape {
                            EditorSelectionShape::Line => (
                                0,
                                line_data
                                    .source_to_display_cells
                                    .last()
                                    .copied()
                                    .unwrap_or(0),
                            ),
                            EditorSelectionShape::Block => {
                                let start = selection.start.column.min(selection.end.column);
                                let end = selection
                                    .start
                                    .column
                                    .max(selection.end.column)
                                    .saturating_add(1);
                                (
                                    *line_data.source_to_display_cells.get(start).unwrap_or(&0),
                                    *line_data.source_to_display_cells.get(end).unwrap_or_else(
                                        || line_data.source_to_display_cells.last().unwrap_or(&0),
                                    ),
                                )
                            }
                            EditorSelectionShape::Char => {
                                let start = if line == selection.start.line {
                                    selection.start.column
                                } else {
                                    0
                                };
                                let end = if line == selection.end.line {
                                    selection.end.column.saturating_add(1)
                                } else {
                                    line_data.source_to_display_cells.len().saturating_sub(1)
                                };
                                (
                                    *line_data
                                        .source_to_display_cells
                                        .get(start.min(end))
                                        .unwrap_or(&0),
                                    *line_data.source_to_display_cells.get(end).unwrap_or_else(
                                        || line_data.source_to_display_cells.last().unwrap_or(&0),
                                    ),
                                )
                            }
                        };
                    (end > start).then_some((line, start, end))
                })
            })
            .collect();
        let cursor_screen_cell = lines
            .iter()
            .position(|line| line.line == session.position.line)
            .and_then(|row| {
                let line = &lines[row];
                let cell = *line
                    .source_to_display_cells
                    .get(session.position.column)
                    .unwrap_or_else(|| line.source_to_display_cells.last().unwrap_or(&0));
                cell.checked_sub(session.viewport.corner.get_x())
                    .filter(|cell| *cell < viewport.width)
                    .map(|cell| (cell as u16, row as u16))
            });

        Ok(EditorRenderSnapshot {
            revision: session.revision,
            mode: session.mode,
            first_line,
            total_lines,
            viewport,
            horizontal_offset: session.viewport.corner.get_x(),
            lines,
            cursor: session.position,
            cursor_screen_cell,
            selections,
            selection_cells,
            prompt: self.prompt.as_ref().map(|prompt| EditorPromptSnapshot {
                kind: prompt.kind,
                prefix: match prompt.kind {
                    EditorPromptKind::SearchForward => "/".to_owned(),
                    EditorPromptKind::SearchBackward => "?".to_owned(),
                    EditorPromptKind::Command => ":".to_owned(),
                },
                text: project_editor_line(&prompt.text).text,
                cursor: project_editor_line(&prompt.text)
                    .source_to_display_cells
                    .get(prompt.cursor)
                    .copied()
                    .unwrap_or_default(),
                error: prompt
                    .error
                    .as_deref()
                    .map(|error| project_editor_line(error).text),
            }),
        })
    }

    pub(crate) fn current_scope(
        &self,
        id: Uuid,
        dialect: SqlDialect,
    ) -> Result<Option<sql::ResolvedScope>, EditorError> {
        let text = self.text(id)?;
        let session = self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let cursor = char_position_to_byte(&text, session.position);
        let selection = session.buffer.write().ok().and_then(|mut buffer| {
            buffer
                .get_leader_selection(session.group_id)
                .map(|(cursor, anchor, shape)| {
                    let start = position_to_byte(&text, cursor.get_y(), cursor.get_x());
                    let end = position_to_byte(&text, anchor.get_y(), anchor.get_x());
                    let kind = match shape {
                        modalkit::prelude::TargetShape::CharWise => ScopeKind::VisualChar,
                        modalkit::prelude::TargetShape::LineWise => ScopeKind::VisualLine,
                        modalkit::prelude::TargetShape::BlockWise => ScopeKind::VisualBlock,
                    };
                    if kind == ScopeKind::VisualBlock {
                        let first_row = cursor.get_y().min(anchor.get_y());
                        let last_row = cursor.get_y().max(anchor.get_y());
                        let first_column = cursor.get_x().min(anchor.get_x());
                        let last_column = cursor.get_x().max(anchor.get_x());
                        let ranges = (first_row..=last_row)
                            .map(|row| {
                                let (line_start, line_end) = line_bounds(&text, row);
                                let line = &text[line_start..line_end];
                                let start = line_start
                                    + line
                                        .char_indices()
                                        .nth(first_column.min(line.chars().count()))
                                        .map_or(line.len(), |(offset, _)| offset);
                                let end_column =
                                    last_column.saturating_add(1).min(line.chars().count());
                                let end = line_start
                                    + line
                                        .char_indices()
                                        .nth(end_column)
                                        .map_or(line.len(), |(offset, _)| offset);
                                TextRange::new(start, end)
                            })
                            .collect();
                        ScopeSelection::block(ranges)
                    } else {
                        ScopeSelection::contiguous(
                            kind,
                            TextRange::new(start.min(end), start.max(end)),
                        )
                    }
                })
        });
        Ok(sql::resolve_scope(
            &text,
            cursor,
            selection.as_ref(),
            dialect,
        ))
    }

    pub(crate) fn replace_range(
        &mut self,
        id: Uuid,
        range: TextRange,
        replacement: &str,
        cursor: ReplacementCursor,
    ) -> Result<(), EditorError> {
        let old = self.text(id)?;
        if range.get(&old).is_none() {
            return Err(EditorError::Operation(
                "format range is not on a UTF-8 boundary".into(),
            ));
        }
        let mut next = old.clone();
        next.replace_range(range.start..range.end, replacement);
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        session.previous_text = Some(old);
        session.redo_text = None;
        let cursor_offset = match cursor {
            ReplacementCursor::Start => range.start,
            ReplacementCursor::EndOfInsertion => range.start + replacement.len(),
            ReplacementCursor::PreserveRelative => range.start + replacement.len(),
        };
        session.position = byte_to_char_position(&next, cursor_offset);
        let mut buffer = session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        buffer.set_text(encode_editor_text(&next));
        buffer.set_leader(
            session.group_id,
            modalkit::editing::cursor::Cursor::new(session.position.line, session.position.column),
        );
        session.revision = session.revision.saturating_add(1);
        self.effects.push(EditorEffect::Changed {
            console_id: id,
            revision: session.revision,
        });
        Ok(())
    }

    pub(crate) fn set_viewport(
        &mut self,
        id: Uuid,
        viewport: EditorViewport,
    ) -> Result<(), EditorError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        session.viewport.dimensions = (viewport.width, viewport.height);
        Ok(())
    }

    pub(crate) fn set_text(&mut self, id: Uuid, text: &str) -> Result<(), EditorError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let mut buffer = session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        buffer.set_text(encode_editor_text(text));
        session.keys.reset_mode();
        session.position = EditorPosition { line: 0, column: 0 };
        session.viewport = Default::default();
        session.mode = EditorMode::Normal;
        session.revision = session.revision.saturating_add(1);
        session.previous_text = None;
        session.redo_text = None;
        Ok(())
    }

    pub(crate) fn set_read_only_text(
        &mut self,
        id: Uuid,
        text: &str,
        follow_tail: bool,
    ) -> Result<(), EditorError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        if session.capability != EditorSessionCapability::ReadOnly {
            return Err(EditorError::Operation("session is editable".into()));
        }
        let position = if follow_tail && !session.interacted {
            let line = text.rsplit('\n').next().unwrap_or_default();
            EditorPosition {
                line: text.matches('\n').count(),
                column: line.chars().count(),
            }
        } else {
            session.position
        };
        let encoded = encode_editor_text(text);
        let mut buffer = session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        if buffer.get_text() != encoded {
            buffer.set_text(encoded);
            session.revision = session.revision.saturating_add(1);
        }
        let line_count = text.matches('\n').count();
        let line = text.rsplit('\n').next().unwrap_or_default();
        session.position = EditorPosition {
            line: position.line.min(line_count),
            column: position.column.min(line.chars().count()),
        };
        buffer.set_leader(
            session.group_id,
            modalkit::editing::cursor::Cursor::new(session.position.line, session.position.column),
        );
        session.keys.reset_mode();
        session.mode = EditorMode::Normal;
        Ok(())
    }

    pub(crate) fn move_cursor_to_end(&mut self, id: Uuid) -> Result<(), EditorError> {
        let text = self.text(id)?;
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let value = text.rsplit('\n').next().unwrap_or_default();
        let position = EditorPosition {
            line: text.matches('\n').count(),
            column: value.chars().count(),
        };
        session.position = position;
        session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?
            .set_leader(
                session.group_id,
                modalkit::editing::cursor::Cursor::new(position.line, position.column),
            );
        Ok(())
    }

    pub(crate) fn press(&mut self, id: Uuid, key: EditorKey) -> Result<(), EditorError> {
        let read_only = self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?
            .capability
            == EditorSessionCapability::ReadOnly;
        if read_only {
            self.sessions
                .get_mut(&id)
                .expect("session was checked above")
                .interacted = true;
            if matches!(
                key,
                EditorKey::Character('i' | 'a' | 'o' | 'O' | 'R' | 'Q' | ':')
                    | EditorKey::Control('r')
            ) {
                return Ok(());
            }
        }
        if self.prompt.is_some() {
            return self.press_prompt(id, key);
        }
        let mode = self.mode(id)?;
        if mode == EditorMode::Normal {
            if self
                .sessions
                .get(&id)
                .is_some_and(|session| session.pending_count.is_none())
                && let EditorKey::Character(character @ '1'..='9') = key
            {
                let session = self
                    .sessions
                    .get_mut(&id)
                    .ok_or(EditorError::MissingSession(id))?;
                session.pending_count = Some(character.to_digit(10).unwrap_or(1));
                return Ok(());
            }
            if let Some(count) = self
                .sessions
                .get(&id)
                .and_then(|session| session.pending_count)
            {
                if let EditorKey::Character(character @ '0'..='9') = key {
                    let session = self
                        .sessions
                        .get_mut(&id)
                        .ok_or(EditorError::MissingSession(id))?;
                    session.pending_count = Some(
                        count
                            .saturating_mul(10)
                            .saturating_add(character.to_digit(10).unwrap_or(0)),
                    );
                    return Ok(());
                }
                if key == EditorKey::Control('w') {
                    self.sessions
                        .get_mut(&id)
                        .ok_or(EditorError::MissingSession(id))?
                        .pending_binding = Some(PendingBinding::Window(count));
                    self.sessions
                        .get_mut(&id)
                        .ok_or(EditorError::MissingSession(id))?
                        .pending_count = None;
                    return Ok(());
                }
                self.sessions
                    .get_mut(&id)
                    .ok_or(EditorError::MissingSession(id))?
                    .pending_count = None;
                for digit in count.to_string().chars() {
                    self.input_vim_key(id, EditorKey::Character(digit))?;
                }
            }
        }
        match (mode, key) {
            (EditorMode::Normal, EditorKey::Character('Q')) => {
                self.effects.push(EditorEffect::Quit);
                Ok(())
            }
            (EditorMode::Normal, EditorKey::Character('?')) => {
                self.start_prompt(EditorPromptKind::SearchBackward)
            }
            (EditorMode::Normal, EditorKey::Character('/')) => {
                self.start_prompt(EditorPromptKind::SearchForward)
            }
            (EditorMode::Normal, EditorKey::Character(':')) => {
                self.start_prompt(EditorPromptKind::Command)
            }
            (EditorMode::Normal, EditorKey::Character('n')) => self.repeat_search(id, false),
            (EditorMode::Normal, EditorKey::Character('N')) => self.repeat_search(id, true),
            (EditorMode::Normal, EditorKey::Character('u')) => self.undo(id),
            (EditorMode::Normal, EditorKey::Control('r')) => self.redo(id),
            (EditorMode::Normal, EditorKey::Character('.')) => {
                let sequence = self
                    .sessions
                    .get(&id)
                    .and_then(|session| session.last_sequence.clone());
                if let Some(sequence) = sequence {
                    for key in sequence {
                        self.press(id, key)?;
                    }
                }
                Ok(())
            }
            (EditorMode::Normal, EditorKey::Character(' ' | '\\')) => {
                self.sessions
                    .get_mut(&id)
                    .ok_or(EditorError::MissingSession(id))?
                    .pending_binding = Some(PendingBinding::Leader);
                Ok(())
            }
            (EditorMode::Normal, EditorKey::Control('w')) => {
                self.sessions
                    .get_mut(&id)
                    .ok_or(EditorError::MissingSession(id))?
                    .pending_binding = Some(PendingBinding::Window(1));
                Ok(())
            }
            (EditorMode::Normal, EditorKey::Character('g')) => {
                self.sessions
                    .get_mut(&id)
                    .ok_or(EditorError::MissingSession(id))?
                    .pending_binding = Some(PendingBinding::Goto);
                Ok(())
            }
            (EditorMode::Insert | EditorMode::Replace, EditorKey::Control('w')) => {
                self.delete_previous_word(id)
            }
            (EditorMode::Insert | EditorMode::Replace, EditorKey::Control('u')) => {
                self.delete_to_line_start(id)
            }
            (_, key) => self.input_vim_key(id, key),
        }
    }

    pub(crate) fn drain_effects(&mut self) -> Vec<EditorEffect> {
        std::mem::take(&mut self.effects)
    }

    pub(crate) fn paste(&mut self, id: Uuid, text: &str) -> Result<(), EditorError> {
        if let Some(prompt) = self.prompt.as_mut() {
            prompt.insert(text);
            return Ok(());
        }
        self.insert(id, text)
    }

    pub(crate) fn undo(&mut self, id: Uuid) -> Result<(), EditorError> {
        let current = self.text(id)?;
        if let Some(previous) = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?
            .previous_text
            .take()
        {
            let session = self
                .sessions
                .get_mut(&id)
                .ok_or(EditorError::MissingSession(id))?;
            session.redo_text = Some(current);
            session
                .buffer
                .write()
                .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?
                .set_text(encode_editor_text(&previous));
            session.position = EditorPosition { line: 0, column: 0 };
            session.revision = session.revision.saturating_add(1);
            return Ok(());
        }
        if self.mode(id)? == EditorMode::Insert {
            self.press(id, EditorKey::Escape)?;
        }
        self.input_vim_key(id, EditorKey::Character('u'))
    }

    pub(crate) fn substitute_confirm(
        &mut self,
        accept: bool,
        all: bool,
        last: bool,
    ) -> Result<(), EditorError> {
        let Some(mut pending) = self.substitute.take() else {
            return Ok(());
        };
        if all {
            pending
                .accepted
                .extend(pending.next..pending.plan.matches.len());
            pending.next = pending.plan.matches.len();
        } else if accept {
            pending.accepted.push(pending.next);
            pending.next += 1;
        }
        if last {
            pending.next = pending.plan.matches.len();
        }
        if pending.next < pending.plan.matches.len() {
            let count = pending.plan.matches.len() - pending.next;
            self.effects
                .push(EditorEffect::SubstituteConfirmRequested { count });
            self.substitute = Some(pending);
        } else {
            self.apply_substitution(pending.console_id, &pending.plan, &pending.accepted)?;
        }
        Ok(())
    }

    pub(crate) fn cancel_substitute(&mut self) {
        self.substitute = None;
    }

    pub(crate) fn redo(&mut self, id: Uuid) -> Result<(), EditorError> {
        let current = self.text(id)?;
        if let Some(next) = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?
            .redo_text
            .take()
        {
            let session = self
                .sessions
                .get_mut(&id)
                .ok_or(EditorError::MissingSession(id))?;
            session.previous_text = Some(current);
            session
                .buffer
                .write()
                .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?
                .set_text(encode_editor_text(&next));
            session.position = byte_to_char_position(&next, next.len());
            session.revision = session.revision.saturating_add(1);
            return Ok(());
        }
        self.input_vim_key(id, EditorKey::Control('r'))
    }

    pub(crate) fn set_mode(&mut self, id: Uuid, mode: EditorMode) -> Result<(), EditorError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        session.keys.reset_mode();
        if mode == EditorMode::Insert {
            session
                .keys
                .input_key(modalkit::key::TerminalKey::from(KeyEvent::new(
                    KeyCode::Char('i'),
                    KeyModifiers::NONE,
                )));
            while session.keys.pop().is_some() {}
        }
        if mode == EditorMode::Insert && session.mode != EditorMode::Insert {
            session.previous_text = None;
        }
        session.mode = mode;
        Ok(())
    }

    pub(crate) fn set_register(&mut self, name: char, text: impl Into<String>) {
        self.registers.insert(name, text.into());
        if name == '+' || name == '*' {
            self.registers.insert('+', self.registers[&name].clone());
            self.registers.insert('*', self.registers[&name].clone());
        }
    }

    pub(crate) fn register(&self, name: char) -> Option<&str> {
        self.registers.get(&name).map(String::as_str)
    }

    fn insert(&mut self, id: Uuid, value: &str) -> Result<(), EditorError> {
        self.replace_at_cursor(id, |text, offset| {
            text.insert_str(offset, value);
            offset + value.len()
        })
    }

    fn backspace(&mut self, id: Uuid) -> Result<(), EditorError> {
        self.replace_at_cursor(id, |text, offset| {
            let start = text[..offset].char_indices().last().map_or(0, |(i, _)| i);
            text.replace_range(start..offset, "");
            start
        })
    }

    fn delete_previous_word(&mut self, id: Uuid) -> Result<(), EditorError> {
        self.replace_at_cursor(id, |text, offset| {
            let prefix = &text[..offset];
            let end = prefix.len();
            let start = prefix
                .char_indices()
                .rev()
                .find(|(_, c)| !c.is_whitespace())
                .map(|(i, _)| i)
                .and_then(|i| {
                    prefix[..i]
                        .char_indices()
                        .rev()
                        .find(|(_, c)| c.is_whitespace())
                        .map(|(j, _)| j + 1)
                })
                .unwrap_or(0);
            text.replace_range(start..end, "");
            start
        })
    }

    fn delete_to_line_start(&mut self, id: Uuid) -> Result<(), EditorError> {
        self.replace_at_cursor(id, |text, offset| {
            let start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
            text.replace_range(start..offset, "");
            start
        })
    }

    fn replace_at_cursor<F>(&mut self, id: Uuid, edit: F) -> Result<(), EditorError>
    where
        F: FnOnce(&mut String, usize) -> usize,
    {
        let old = self.text(id)?;
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let position = {
            let mut buffer = session
                .buffer
                .write()
                .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
            let cursor = buffer.get_leader(session.group_id);
            EditorPosition {
                line: cursor.get_y(),
                column: cursor.get_x(),
            }
        };
        let offset = char_position_to_byte(&old, position);
        let mut next = old.clone();
        let new_offset = edit(&mut next, offset);
        if next == old {
            return Ok(());
        }
        if session.previous_text.is_none() {
            session.previous_text = Some(old);
        }
        session.position = byte_to_char_position(&next, new_offset);
        let mut buffer = session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        buffer.set_text(encode_editor_text(&next));
        buffer.set_leader(
            session.group_id,
            modalkit::editing::cursor::Cursor::new(session.position.line, session.position.column),
        );
        session.revision = session.revision.saturating_add(1);
        session.redo_text = None;
        self.effects.push(EditorEffect::Changed {
            console_id: id,
            revision: session.revision,
        });
        Ok(())
    }

    fn input_vim_key(&mut self, id: Uuid, key: EditorKey) -> Result<(), EditorError> {
        let before = self.text(id)?;
        let unnamed_before = self.register('"').map(str::to_owned);
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        session.current_sequence.push(key);
        let pending_binding = if let EditorKey::Character(character) = key {
            session
                .pending_binding
                .take()
                .map(|binding| (binding, character))
        } else {
            None
        };
        if let Some((binding, character)) = pending_binding {
            match (binding, character) {
                (PendingBinding::Leader, 'r') => self.effects.push(EditorEffect::RunCurrent),
                (PendingBinding::Leader, 'R') => self.effects.push(EditorEffect::RunAll),
                (PendingBinding::Leader, 'y') => self.effects.push(EditorEffect::CopyStatement),
                (PendingBinding::Leader, 'Y') => self.effects.push(EditorEffect::CopyBuffer),
                (PendingBinding::Leader, 'f') => self.effects.push(EditorEffect::FormatCurrent),
                (PendingBinding::Leader, 'n') => self.effects.push(EditorEffect::NewConsole),
                (PendingBinding::Leader, 's') => self.effects.push(EditorEffect::GotoSqlConsole),
                (PendingBinding::Leader, 'q') => self.effects.push(EditorEffect::CloseConsole),
                (PendingBinding::Leader, 'x') => self.effects.push(EditorEffect::DeleteConsole),
                (PendingBinding::Leader, 'e') => self.effects.push(EditorEffect::OpenSqlEditorList),
                (PendingBinding::Leader, '?') => self.effects.push(EditorEffect::ShowHelp),
                (PendingBinding::Leader, 't') => {
                    session.pending_binding = Some(PendingBinding::LeaderTransaction)
                }
                (PendingBinding::Leader, 'd') => {
                    self.effects.push(EditorEffect::OpenTargetSelector)
                }
                (PendingBinding::LeaderTransaction, 't') => {
                    self.effects.push(EditorEffect::ToggleTransaction)
                }
                (PendingBinding::LeaderTransaction, 'c') => {
                    self.effects.push(EditorEffect::TransactionControl)
                }
                (PendingBinding::Window(_), 'h') => {
                    self.effects.push(EditorEffect::FocusPane(Focus::Explorer))
                }
                (PendingBinding::Window(_), 'j') => {
                    self.effects.push(EditorEffect::FocusPane(Focus::Results))
                }
                (PendingBinding::Window(count), operator @ ('+' | '-' | '>' | '<')) => {
                    if let Some(resize) = pane_resize(Focus::Editor, operator, count) {
                        self.effects.push(EditorEffect::ResizePane(resize));
                    }
                }
                (PendingBinding::Window(_), '=') => self.effects.push(EditorEffect::ResetPaneSizes),
                (PendingBinding::Window(_), 'k' | 'l') => {}
                (PendingBinding::Goto, 'g') => self.input_vim_key(id, EditorKey::Character('g'))?,
                (PendingBinding::Goto, 't') => self.effects.push(EditorEffect::NextTab),
                (PendingBinding::Goto, 'T') => self.effects.push(EditorEffect::PreviousTab),
                (_, _) => {}
            }
            if self
                .sessions
                .get(&id)
                .is_some_and(|session| session.pending_binding.is_some())
            {
                return Ok(());
            }
            return Ok(());
        }
        let terminal_key = key.to_terminal_key()?;
        session.keys.input_key(terminal_key);
        let mut actions = Vec::new();
        while let Some((action, context)) = session.keys.pop() {
            actions.push((action, context));
        }
        for (action, context) in actions {
            self.apply_action(id, action, context)?;
        }
        self.sync_session_from_buffer(id)?;
        self.sync_registers();
        if matches!(key, EditorKey::Character('y')) {
            let copied = self.register('"').unwrap_or_default().to_owned();
            if !copied.is_empty() && unnamed_before.as_deref() != Some(copied.as_str()) {
                self.effects.push(EditorEffect::Yanked(copied));
            }
        }
        if before != self.text(id)? {
            let session = self
                .sessions
                .get_mut(&id)
                .ok_or(EditorError::MissingSession(id))?;
            session.revision = session.revision.saturating_add(1);
            session.last_sequence = Some(std::mem::take(&mut session.current_sequence));
            self.effects.push(EditorEffect::Changed {
                console_id: id,
                revision: session.revision,
            });
        }
        Ok(())
    }

    fn apply_action(
        &mut self,
        id: Uuid,
        action: modalkit::actions::Action<LazyDbApplicationInfo>,
        context: modalkit::editing::context::EditContext,
    ) -> Result<(), EditorError> {
        use modalkit::actions::Action;
        match action {
            Action::Editor(editor_action) => {
                let session = self
                    .sessions
                    .get(&id)
                    .ok_or(EditorError::MissingSession(id))?;
                if session.capability == EditorSessionCapability::ReadOnly
                    && !editor_action.is_readonly(&context)
                {
                    return Ok(());
                }
                let mut buffer = session
                    .buffer
                    .write()
                    .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
                let ctx = (session.group_id, &session.viewport, &context);
                buffer
                    .editor_command(&editor_action, &ctx, &mut self.store)
                    .map_err(|error| EditorError::Operation(error.to_string()))?;
            }
            Action::Application(ApplicationAction::Effect(effect)) => {
                if self
                    .sessions
                    .get(&id)
                    .is_some_and(|session| session.capability == EditorSessionCapability::Editable)
                {
                    self.effects.push(effect);
                }
            }
            Action::NoOp | Action::RedrawScreen => {}
            Action::Repeat(repeat) => self.effects.push(EditorEffect::Message(format!(
                "repeat action deferred: {repeat:?}"
            ))),
            other => self.effects.push(EditorEffect::Message(format!(
                "editor action deferred: {other:?}"
            ))),
        }
        Ok(())
    }

    fn sync_session_from_buffer(&mut self, id: Uuid) -> Result<(), EditorError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let mut buffer = session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        let cursor = buffer.get_leader(session.group_id);
        session.position = EditorPosition {
            line: cursor.get_y(),
            column: cursor.get_x(),
        };
        session.mode = mode_from_key_manager(&session.keys);
        Ok(())
    }

    fn sync_registers(&mut self) {
        for (name, register) in std::iter::once(('"', Register::Unnamed))
            .chain(('a'..='z').map(|name| (name, Register::Named(name))))
        {
            if let Ok(cell) = self.store.registers.get(&register) {
                self.registers.insert(name, cell.value.to_string());
            }
        }
    }

    fn start_prompt(&mut self, kind: EditorPromptKind) -> Result<(), EditorError> {
        self.prompt = Some(PromptSession::new(kind));
        Ok(())
    }

    fn press_prompt(&mut self, id: Uuid, key: EditorKey) -> Result<(), EditorError> {
        match key {
            EditorKey::Escape | EditorKey::Control('c') => {
                self.prompt = None;
            }
            EditorKey::Enter => self.submit_prompt(id)?,
            EditorKey::Backspace | EditorKey::Control('h') => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.backspace();
                }
            }
            EditorKey::Control('w') => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.delete_previous_word();
                }
            }
            EditorKey::Control('u') => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.text.clear();
                    prompt.cursor = 0;
                    prompt.error = None;
                }
            }
            EditorKey::Left => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.cursor = prompt.cursor.saturating_sub(1);
                }
            }
            EditorKey::Right => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.cursor = (prompt.cursor + 1).min(prompt.text.chars().count());
                }
            }
            EditorKey::Home => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.cursor = 0;
                }
            }
            EditorKey::End => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.cursor = prompt.text.chars().count();
                }
            }
            EditorKey::Up | EditorKey::Down => self.history_move(matches!(key, EditorKey::Down)),
            EditorKey::Tab => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.insert("\t");
                }
            }
            EditorKey::Character(character) | EditorKey::Control(character) => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.insert(&character.to_string());
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn history_move(&mut self, forward: bool) {
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        if self.command_history.is_empty() {
            return;
        }
        let next = match (prompt.history_index, forward) {
            (None, false) => self.command_history.len().saturating_sub(1),
            (Some(index), false) => index.saturating_sub(1),
            (Some(index), true) if index + 1 < self.command_history.len() => index + 1,
            (_, true) => return,
        };
        prompt.history_index = Some(next);
        prompt.text = self.command_history[next].clone();
        prompt.cursor = prompt.text.chars().count();
    }

    fn submit_prompt(&mut self, id: Uuid) -> Result<(), EditorError> {
        let Some(prompt) = self.prompt.take() else {
            return Ok(());
        };
        let raw = prompt.text;
        match prompt.kind {
            EditorPromptKind::Command => {
                self.command_history.push(raw.clone());
                if raw.trim_start().chars().next().is_some_and(|character| {
                    matches!(character, '.' | '$' | '%' | 's' | '<' | '0'..='9')
                }) {
                    match self.start_substitute(id, &raw) {
                        Ok(()) => {}
                        Err(error) => {
                            self.prompt = Some(PromptSession {
                                kind: EditorPromptKind::Command,
                                text: raw,
                                cursor: 0,
                                error: Some(error.to_string()),
                                history_index: None,
                            })
                        }
                    }
                } else if let Some(effect) = parse_command(&raw) {
                    self.effects.push(effect);
                } else {
                    self.prompt = Some(PromptSession {
                        kind: EditorPromptKind::Command,
                        text: raw,
                        cursor: 0,
                        error: Some("unknown or invalid Ex command".to_owned()),
                        history_index: None,
                    });
                }
            }
            kind => {
                if raw.is_empty() {
                    return Ok(());
                }
                self.last_search = Some(raw.clone());
                self.last_search_backward = kind == EditorPromptKind::SearchBackward;
                if !self.search(id, &raw, self.last_search_backward)? {
                    self.prompt = Some(PromptSession {
                        kind,
                        text: raw.clone(),
                        cursor: raw.chars().count(),
                        error: Some("pattern not found".to_owned()),
                        history_index: None,
                    });
                }
            }
        }
        Ok(())
    }

    fn repeat_search(&mut self, id: Uuid, reverse: bool) -> Result<(), EditorError> {
        let Some(pattern) = self.last_search.clone() else {
            return Ok(());
        };
        let _ = self.search(id, &pattern, self.last_search_backward ^ reverse)?;
        Ok(())
    }

    fn search(&mut self, id: Uuid, pattern: &str, backward: bool) -> Result<bool, EditorError> {
        let text = self.text(id)?;
        let session = self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let cursor = {
            let mut buffer = session
                .buffer
                .write()
                .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
            let cursor = buffer.get_leader(session.group_id);
            char_position_to_byte(
                &text,
                EditorPosition {
                    line: cursor.get_y(),
                    column: cursor.get_x(),
                },
            )
        };
        let found = if backward {
            text[..cursor]
                .rfind(pattern)
                .or_else(|| text.rfind(pattern))
        } else {
            text[cursor.saturating_add((cursor < text.len()) as usize)..]
                .find(pattern)
                .map(|offset| offset + cursor + (cursor < text.len()) as usize)
                .or_else(|| text.find(pattern))
        };
        let Some(offset) = found else {
            return Ok(false);
        };
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        session.position = byte_to_char_position(&text, offset);
        session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?
            .set_leader(
                session.group_id,
                modalkit::editing::cursor::Cursor::new(
                    session.position.line,
                    session.position.column,
                ),
            );
        Ok(true)
    }

    fn start_substitute(&mut self, id: Uuid, raw: &str) -> Result<(), substitute::SubstituteError> {
        let source = self
            .text(id)
            .map_err(|_| substitute::SubstituteError::Syntax)?;
        let session = self
            .sessions
            .get(&id)
            .ok_or(substitute::SubstituteError::Syntax)?;
        let selection = session
            .buffer
            .write()
            .ok()
            .and_then(|mut buffer| buffer.get_leader_selection(session.group_id))
            .map(|(cursor, anchor, _)| LineRange {
                start: cursor.get_y().min(anchor.get_y()),
                end: cursor.get_y().max(anchor.get_y()),
            });
        let cursor_line = {
            let mut buffer = session
                .buffer
                .write()
                .map_err(|_| substitute::SubstituteError::Syntax)?;
            buffer.get_leader(session.group_id).get_y()
        };
        let (plan, flags) = substitute::plan(
            &source,
            raw,
            cursor_line,
            selection,
            self.previous_substitute_pattern.as_deref(),
        )?;
        self.previous_substitute_pattern = Some(plan.pattern.clone());
        if flags.confirm {
            let count = plan.matches.len();
            self.substitute = Some(PendingSubstitute {
                console_id: id,
                plan,
                accepted: Vec::new(),
                next: 0,
            });
            self.effects
                .push(EditorEffect::SubstituteConfirmRequested { count });
        } else {
            let accepted = (0..plan.matches.len()).collect::<Vec<_>>();
            self.apply_substitution(id, &plan, &accepted)
                .map_err(|_| substitute::SubstituteError::Syntax)?;
        }
        Ok(())
    }

    fn apply_substitution(
        &mut self,
        id: Uuid,
        plan: &SubstitutionPlan,
        accepted: &[usize],
    ) -> Result<(), EditorError> {
        let next = substitute::apply(plan, accepted);
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        session.previous_text = Some(plan.source.clone());
        session.redo_text = None;
        session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?
            .set_text(encode_editor_text(&next));
        session.position = byte_to_char_position(&next, 0);
        session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?
            .set_leader(
                session.group_id,
                modalkit::editing::cursor::Cursor::new(0, 0),
            );
        session.revision = session.revision.saturating_add(1);
        self.effects.push(EditorEffect::Changed {
            console_id: id,
            revision: session.revision,
        });
        Ok(())
    }
}

fn parse_command(command: &str) -> Option<EditorEffect> {
    let command = command.trim();
    if let Some(value) = command.strip_prefix("connection ") {
        return Some(EditorEffect::SetConnectionTarget(value.trim().to_owned()));
    }
    if let Some(value) = command.strip_prefix("database ") {
        return Some(EditorEffect::SetDatabaseTarget(value.trim().to_owned()));
    }
    if let Some(value) = command.strip_prefix("schema ") {
        return Some(EditorEffect::SetSchemaTarget(value.trim().to_owned()));
    }
    match command {
        "run" => Some(EditorEffect::RunCurrent),
        "runall" => Some(EditorEffect::RunAll),
        "format" => Some(EditorEffect::FormatCurrent),
        "tx auto" => Some(EditorEffect::SetTransactionModeRequested { manual: false }),
        "tx manual" => Some(EditorEffect::SetTransactionModeRequested { manual: true }),
        "tx clear" => Some(EditorEffect::ClearTransactionOutcome),
        "commit" => Some(EditorEffect::Commit),
        "rollback" => Some(EditorEffect::Rollback),
        "q" => Some(EditorEffect::Quit),
        _ => None,
    }
}

fn map_highlight(kind: sql::HighlightKind) -> EditorHighlightKind {
    match kind {
        sql::HighlightKind::Keyword => EditorHighlightKind::Keyword,
        sql::HighlightKind::Identifier => EditorHighlightKind::Identifier,
        sql::HighlightKind::String => EditorHighlightKind::String,
        sql::HighlightKind::Number => EditorHighlightKind::Number,
        sql::HighlightKind::Comment => EditorHighlightKind::Comment,
        sql::HighlightKind::Operator => EditorHighlightKind::Operator,
        sql::HighlightKind::Punctuation => EditorHighlightKind::Punctuation,
        sql::HighlightKind::Parameter => EditorHighlightKind::Parameter,
        sql::HighlightKind::Plain => EditorHighlightKind::Plain,
    }
}

fn render_span(
    source: &str,
    start: usize,
    end: usize,
    kind: EditorHighlightKind,
    statement: Option<sql::TextRange>,
    line_start: usize,
) -> EditorRenderSpan {
    let text = source.get(start..end).unwrap_or_default();
    EditorRenderSpan {
        text: project_editor_line(text).text,
        source_start: start,
        source_end: end,
        kind,
        current_statement: statement
            .is_some_and(|range| range.start < line_start + end && range.end > line_start + start),
    }
}

fn position_to_byte(text: &str, line: usize, column: usize) -> usize {
    let start = text
        .split_inclusive('\n')
        .take(line)
        .map(str::len)
        .sum::<usize>();
    start
        + text[start..]
            .char_indices()
            .nth(column)
            .map_or(text[start..].len(), |(offset, _)| offset)
}

fn line_bounds(text: &str, line: usize) -> (usize, usize) {
    let start = text
        .split_inclusive('\n')
        .take(line)
        .map(str::len)
        .sum::<usize>();
    let end = text[start..]
        .find('\n')
        .map_or(text.len(), |offset| start + offset);
    (start.min(text.len()), end.min(text.len()))
}

impl EditorKey {
    fn to_terminal_key(self) -> Result<modalkit::key::TerminalKey, EditorError> {
        let text = match self {
            Self::Character(c) => c.to_string(),
            Self::Escape => "<Esc>".into(),
            Self::Enter => "<Enter>".into(),
            Self::Backspace => "<BS>".into(),
            Self::Control(c) => format!("<C-{c}>"),
            Self::Delete => "<Del>".into(),
            Self::Left => "<Left>".into(),
            Self::Right => "<Right>".into(),
            Self::Up => "<Up>".into(),
            Self::Down => "<Down>".into(),
            Self::Home => "<Home>".into(),
            Self::End => "<End>".into(),
            Self::Tab => "<Tab>".into(),
        };
        text.parse()
            .map_err(|error| EditorError::Operation(format!("invalid key: {error}")))
    }
}

fn char_position_to_byte(text: &str, position: EditorPosition) -> usize {
    let line_start = text
        .split_inclusive('\n')
        .take(position.line)
        .map(str::len)
        .sum::<usize>();
    line_start
        + text[line_start..]
            .char_indices()
            .nth(position.column)
            .map_or(text[line_start..].len(), |(i, _)| i)
}

fn byte_to_char_position(text: &str, offset: usize) -> EditorPosition {
    let prefix = &text[..offset.min(text.len())];
    let line = prefix.matches('\n').count();
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, value)| value)
        .chars()
        .count();
    EditorPosition { line, column }
}

#[cfg(test)]
mod tests;
