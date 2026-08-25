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
    workspace::Focus,
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
    RunCurrent,
    RunAll,
    FormatCurrent,
    NewConsole,
    CloseConsole,
    FocusPane(Focus),
    NextTab,
    PreviousTab,
    ShowHelp,
    ToggleTransaction,
    SetTransactionModeRequested { manual: bool },
    Commit,
    Rollback,
    ClearTransactionOutcome,
    Quit,
    Message(String),
    BackwardSearch,
    SubstituteConfirmRequested { count: usize },
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

struct EditorSession {
    buffer: modalkit::editing::store::SharedBuffer<LazyDbApplicationInfo>,
    group_id: modalkit::editing::buffer::CursorGroupId,
    viewport: modalkit::prelude::ViewportContext<modalkit::editing::cursor::Cursor>,
    mode: EditorMode,
    position: EditorPosition,
    revision: u64,
    previous_text: Option<String>,
    redo_text: Option<String>,
}

pub(crate) struct EditorWorkspace {
    store: modalkit::editing::store::Store<LazyDbApplicationInfo>,
    sessions: HashMap<Uuid, EditorSession>,
    registers: HashMap<char, String>,
    keys: modalkit::editing::key::KeyManager<
        modalkit::key::TerminalKey,
        modalkit::actions::Action<LazyDbApplicationInfo>,
        modalkit::prelude::RepeatType,
    >,
    effects: Vec<EditorEffect>,
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
    Window,
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
            keys: modalkit::editing::key::KeyManager::new(
                modalkit::env::vim::keybindings::default_vim_keys(),
            ),
            effects: Vec::new(),
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
        let mut buffer = modalkit::editing::buffer::EditBuffer::from_str(
            ContentId(id),
            &encode_editor_text(text),
        );
        let group_id = buffer.create_group();
        buffer.set_leader(group_id, modalkit::editing::cursor::Cursor::new(0, 0));
        let buffer = std::sync::Arc::new(std::sync::RwLock::new(buffer));
        self.sessions.insert(
            id,
            EditorSession {
                buffer,
                group_id,
                viewport: Default::default(),
                mode: EditorMode::Insert,
                position: EditorPosition { line: 0, column: 0 },
                revision: 0,
                previous_text: None,
                redo_text: None,
            },
        );
    }

    pub(crate) fn close_console(&mut self, id: Uuid) {
        self.sessions.remove(&id);
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
        self.sessions
            .get(&id)
            .map(|session| session.mode)
            .ok_or(EditorError::MissingSession(id))
    }

    pub(crate) fn position(&self, id: Uuid) -> Result<EditorPosition, EditorError> {
        self.sessions
            .get(&id)
            .map(|session| session.position)
            .ok_or(EditorError::MissingSession(id))
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
                    sql::highlight_sql(&full_text, dialect)
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
                        ));
                    }
                    if end > start {
                        spans.push(render_span(
                            &source,
                            start,
                            end,
                            map_highlight(highlight.kind),
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
        session.position = byte_to_char_position(&next, range.start);
        session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?
            .set_text(encode_editor_text(&next));
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
        session.position = EditorPosition { line: 0, column: 0 };
        session.viewport = Default::default();
        session.mode = EditorMode::Normal;
        session.revision = session.revision.saturating_add(1);
        session.previous_text = None;
        session.redo_text = None;
        Ok(())
    }

    pub(crate) fn move_cursor_to_end(&mut self, id: Uuid) -> Result<(), EditorError> {
        let text = self.text(id)?;
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let value = text.rsplit('\n').next().unwrap_or_default();
        session.position = EditorPosition {
            line: text.matches('\n').count(),
            column: value.chars().count(),
        };
        Ok(())
    }

    pub(crate) fn press(&mut self, id: Uuid, key: EditorKey) -> Result<(), EditorError> {
        if self.prompt.is_some() {
            return self.press_prompt(id, key);
        }
        let mode = self.mode(id)?;
        match (mode, key) {
            (_, EditorKey::Escape) => self.set_mode(id, EditorMode::Normal),
            (EditorMode::Normal, EditorKey::Character('i')) => {
                self.set_mode(id, EditorMode::Insert)
            }
            (EditorMode::Insert, EditorKey::Enter) => self.insert(id, "\n"),
            (EditorMode::Insert, EditorKey::Backspace) => self.backspace(id),
            (EditorMode::Insert, EditorKey::Control('h')) => self.backspace(id),
            (EditorMode::Insert, EditorKey::Control('w')) => self.delete_previous_word(id),
            (EditorMode::Insert, EditorKey::Control('u')) => self.delete_to_line_start(id),
            (EditorMode::Insert, EditorKey::Control('c')) => self.set_mode(id, EditorMode::Normal),
            (EditorMode::Insert, EditorKey::Character(c)) => self.insert(id, &c.to_string()),
            (EditorMode::Insert, EditorKey::Control(c)) => self.insert(id, &c.to_string()),
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
            (EditorMode::Normal, EditorKey::Character('.')) => {
                if let Some(sequence) = self.last_sequence.clone() {
                    for key in sequence {
                        self.press(id, key)?;
                    }
                }
                Ok(())
            }
            (EditorMode::Normal, EditorKey::Character(' ' | '\\')) => {
                self.pending_binding = Some(PendingBinding::Leader);
                Ok(())
            }
            (EditorMode::Normal, EditorKey::Control('w')) => {
                self.pending_binding = Some(PendingBinding::Window);
                Ok(())
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
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let Some(previous) = session.previous_text.take() else {
            return Ok(());
        };
        session.redo_text = Some(current);
        let mut buffer = session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        buffer.set_text(encode_editor_text(&previous));
        session.revision = session.revision.saturating_add(1);
        session.position = EditorPosition { line: 0, column: 0 };
        Ok(())
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
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let Some(next) = session.redo_text.take() else {
            return Ok(());
        };
        session.previous_text = Some(current);
        let mut buffer = session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        buffer.set_text(encode_editor_text(&next));
        session.revision = session.revision.saturating_add(1);
        session.position = byte_to_char_position(&next, next.len());
        Ok(())
    }

    pub(crate) fn set_mode(&mut self, id: Uuid, mode: EditorMode) -> Result<(), EditorError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
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
        let offset = char_position_to_byte(&old, session.position);
        let mut next = old.clone();
        let new_offset = edit(&mut next, offset);
        if session.previous_text.is_none() {
            session.previous_text = Some(old);
        }
        session.position = byte_to_char_position(&next, new_offset);
        let mut buffer = session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        buffer.set_text(encode_editor_text(&next));
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
        self.current_sequence.push(key);
        if let EditorKey::Character(character) = key
            && let Some(binding) = self.pending_binding.take()
        {
            match (binding, character) {
                (PendingBinding::Leader, 'r') => self.effects.push(EditorEffect::RunCurrent),
                (PendingBinding::Leader, 'R') => self.effects.push(EditorEffect::RunAll),
                (PendingBinding::Leader, 'f') => self.effects.push(EditorEffect::FormatCurrent),
                (PendingBinding::Leader, 'n') => self.effects.push(EditorEffect::NewConsole),
                (PendingBinding::Leader, '?') => self.effects.push(EditorEffect::ShowHelp),
                (PendingBinding::Leader, 't') => {
                    self.pending_binding = Some(PendingBinding::LeaderTransaction)
                }
                (PendingBinding::LeaderTransaction, 't') => {
                    self.effects.push(EditorEffect::ToggleTransaction)
                }
                (PendingBinding::LeaderTransaction, 'c') => self.effects.push(EditorEffect::Commit),
                (PendingBinding::LeaderTransaction, 'r') => {
                    self.effects.push(EditorEffect::Rollback)
                }
                (PendingBinding::Window, 'h') => {
                    self.effects.push(EditorEffect::FocusPane(Focus::Explorer))
                }
                (PendingBinding::Window, 'j') => {
                    self.effects.push(EditorEffect::FocusPane(Focus::Results))
                }
                (PendingBinding::Window, 'k') => {
                    self.effects.push(EditorEffect::FocusPane(Focus::Explorer))
                }
                (PendingBinding::Window, 'l') => {
                    self.effects.push(EditorEffect::FocusPane(Focus::Results))
                }
                (_, _) => {}
            }
            if self.pending_binding.is_some() {
                return Ok(());
            }
            return Ok(());
        }
        let terminal_key = key.to_terminal_key()?;
        self.keys.input_key(terminal_key);
        while let Some((action, context)) = self.keys.pop() {
            self.apply_action(id, action, context)?;
        }
        self.sync_session_from_buffer(id)?;
        self.sync_registers();
        if before != self.text(id)? {
            self.last_sequence = Some(std::mem::take(&mut self.current_sequence));
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
                let mut buffer = session
                    .buffer
                    .write()
                    .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
                let ctx = (session.group_id, &session.viewport, &context);
                buffer
                    .editor_command(&editor_action, &ctx, &mut self.store)
                    .map_err(|error| EditorError::Operation(error.to_string()))?;
            }
            Action::Application(ApplicationAction::Effect(effect)) => self.effects.push(effect),
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
        session.mode = match self.keys.show_mode().as_deref() {
            Some("-- INSERT --") => EditorMode::Insert,
            Some("-- REPLACE --") => EditorMode::Replace,
            Some("-- VISUAL LINE --") => EditorMode::VisualLine,
            Some("-- VISUAL BLOCK --") => EditorMode::VisualBlock,
            Some("-- VISUAL --") => EditorMode::VisualChar,
            _ => EditorMode::Normal,
        };
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
                    prompt.text.truncate(0);
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
        let cursor = char_position_to_byte(&text, session.position);
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
        let (plan, flags) = substitute::plan(
            &source,
            raw,
            session.position.line,
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
        session.revision = session.revision.saturating_add(1);
        self.effects.push(EditorEffect::Changed {
            console_id: id,
            revision: session.revision,
        });
        Ok(())
    }
}

fn parse_command(command: &str) -> Option<EditorEffect> {
    match command.trim() {
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
) -> EditorRenderSpan {
    let text = source.get(start..end).unwrap_or_default();
    EditorRenderSpan {
        text: project_editor_line(text).text,
        source_start: start,
        source_end: end,
        kind,
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
