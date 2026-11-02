#![allow(dead_code)]

use std::sync::Arc;
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

fn full_line_width(text: &str) -> usize {
    text.split('\n')
        .map(project_editor_line)
        .map(|line| line.source_to_display_cells.last().copied().unwrap_or(0))
        .max()
        .unwrap_or(0)
}

mod indent;
mod jump_history;
mod preview;
mod prompt;
mod substitute;
use jump_history::{JumpHistory, remap_text_position};
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
    SaveRequested { console_id: Uuid, revision: u64 },
    Yanked(String),
    CopyStatement,
    CopyBuffer,
    RunCurrent,
    RunAll,
    FormatCurrent,
    CloseConsole,
    DeleteConsole,
    OpenSqlEditorList,
    OpenNotificationHistory,
    FocusPane(Focus),
    FocusNext,
    ResizePane(PaneResize),
    ResetPaneSizes,
    TogglePaneMaximized,
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
    OpenDatabaseSelector,
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
    Undo,
    Redo,
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
    PageUp,
    PageDown,
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
    pending_vim_character: bool,
    current_sequence: Vec<EditorKey>,
    last_sequence: Option<Vec<EditorKey>>,
    mode: EditorMode,
    position: EditorPosition,
    revision: u64,
    history: EditorHistory,
    jump_history: JumpHistory,
    capability: EditorSessionCapability,
    pending_tail_scroll: bool,
    preview_layout: std::cell::Cell<Option<(usize, usize, usize, usize)>>,
    preview_document: RefCell<Option<(u64, preview::SharedPreviewDocument)>>,
    preview_highlights: RefCell<PreviewHighlightCache>,
    preview_wrap_index: RefCell<Option<(u64, preview::PreviewWrapIndex)>>,
    preview_render_first_line: std::cell::Cell<Option<usize>>,
}

type PreviewHighlightCache = Option<(
    u64,
    crate::model::editor_language::EditorLanguage,
    Arc<Vec<Vec<EditorRenderSpan>>>,
)>;

const EDITOR_HISTORY_LIMIT: usize = 100;

#[derive(Clone, Debug, Eq, PartialEq)]
struct EditorSnapshot {
    text: String,
    position: EditorPosition,
}

// LazyDB owns history because programmatic edits reset modalkit's buffer history.
#[derive(Default)]
struct EditorHistory {
    undo: Vec<EditorSnapshot>,
    redo: Vec<EditorSnapshot>,
    transaction_start: Option<EditorSnapshot>,
}

impl EditorHistory {
    fn push_undo(&mut self, snapshot: EditorSnapshot) {
        if self.undo.last() == Some(&snapshot) {
            return;
        }
        if self.undo.len() == EDITOR_HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(snapshot);
        self.redo.clear();
    }

    fn push_redo(&mut self, snapshot: EditorSnapshot) {
        if self.redo.len() == EDITOR_HISTORY_LIMIT {
            self.redo.remove(0);
        }
        self.redo.push(snapshot);
    }

    fn push_undo_without_clearing_redo(&mut self, snapshot: EditorSnapshot) {
        if self.undo.len() == EDITOR_HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(snapshot);
    }
}

type VimKeyManager = modalkit::editing::key::KeyManager<
    modalkit::key::TerminalKey,
    modalkit::actions::Action<LazyDbApplicationInfo>,
    modalkit::prelude::RepeatType,
>;

fn editor_vim_keys()
-> modalkit::env::vim::keybindings::VimMachine<modalkit::key::TerminalKey, LazyDbApplicationInfo> {
    use modalkit::env::vim::{VimMode, keybindings::InputStep};
    use modalkit::keybindings::{EdgeEvent, EdgeRepeat};

    let mut keys = modalkit::env::vim::keybindings::default_vim_keys();
    let run = InputStep::new().actions(vec![modalkit::actions::Action::Application(
        ApplicationAction::Effect(EditorEffect::RunCurrent),
    )]);
    let path = [(EdgeRepeat::Once, EdgeEvent::Key("R".parse().unwrap()))];
    keys.add_mapping(VimMode::Normal, &path, &run);
    keys.add_mapping(VimMode::Visual, &path, &run);
    keys
}

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
            keys: VimKeyManager::new(editor_vim_keys()),
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

    /// Opens a value document with the same Vim editing engine as a SQL
    /// buffer, without enabling SQL-specific effects in the caller.
    pub(crate) fn open_value(&mut self, id: Uuid, text: &str) {
        self.open_session(id, text, EditorSessionCapability::Editable);
    }

    pub(crate) fn open_read_only(&mut self, id: Uuid, text: &str) {
        self.open_session(id, text, EditorSessionCapability::ReadOnly);
    }

    fn open_session(&mut self, id: Uuid, text: &str, capability: EditorSessionCapability) {
        self.analysis_cache
            .get_mut()
            .retain(|key, _| key.console_id != id);
        let mut buffer = modalkit::editing::buffer::EditBuffer::from_str(
            ContentId(id),
            &encode_editor_text(text),
        );
        let group_id = buffer.create_group();
        buffer.set_leader(group_id, modalkit::editing::cursor::Cursor::new(0, 0));
        let buffer = std::sync::Arc::new(std::sync::RwLock::new(buffer));
        let mut keys = VimKeyManager::new(editor_vim_keys());
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
                pending_vim_character: false,
                current_sequence: Vec::new(),
                last_sequence: None,
                mode,
                position: EditorPosition { line: 0, column: 0 },
                revision: 0,
                history: EditorHistory::default(),
                jump_history: JumpHistory::default(),
                capability,
                pending_tail_scroll: false,
                preview_layout: std::cell::Cell::new(None),
                preview_document: RefCell::new(None),
                preview_highlights: RefCell::new(None),
                preview_wrap_index: RefCell::new(None),
                preview_render_first_line: std::cell::Cell::new(None),
            },
        );
        if capability == EditorSessionCapability::Editable {
            let session = self
                .sessions
                .get_mut(&id)
                .expect("session was inserted above");
            session.history.transaction_start = Some(EditorSnapshot {
                text: text.to_owned(),
                position: EditorPosition { line: 0, column: 0 },
            });
        }
    }

    pub(crate) fn close_console(&mut self, id: Uuid) {
        if self.prompt_active(id) {
            self.prompt = None;
        }
        self.sessions.remove(&id);
        self.analysis_cache
            .borrow_mut()
            .retain(|key, _| key.console_id != id);
    }

    pub(crate) fn merge_sessions_from(&mut self, mut other: Self) {
        for (id, session) in other.sessions.drain() {
            self.sessions.entry(id).or_insert(session);
        }
        self.analysis_cache
            .get_mut()
            .extend(other.analysis_cache.into_inner());
    }

    pub(crate) fn has_session(&self, id: Uuid) -> bool {
        self.sessions.contains_key(&id)
    }

    pub(crate) fn is_editable(&self, id: Uuid) -> Result<bool, EditorError> {
        Ok(self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?
            .capability
            == EditorSessionCapability::Editable)
    }

    pub(crate) fn key(&mut self, id: Uuid, event: KeyEvent) -> Result<(), EditorError> {
        let key = if crate::input::is_text_redo(event) {
            EditorKey::Redo
        } else if crate::input::is_text_undo(event) {
            EditorKey::Undo
        } else if event.modifiers.contains(KeyModifiers::SUPER) {
            self.cancel_leader_bindings();
            return Ok(());
        } else if event.modifiers.contains(KeyModifiers::CONTROL) {
            match event.code {
                KeyCode::Char(c) => EditorKey::Control(c),
                _ => {
                    self.cancel_leader_bindings();
                    return Ok(());
                }
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
                KeyCode::PageUp => EditorKey::PageUp,
                KeyCode::PageDown => EditorKey::PageDown,
                _ => {
                    self.cancel_leader_bindings();
                    return Ok(());
                }
            }
        };
        self.press(id, key)
    }

    pub(crate) fn can_scroll_vertically(&self, id: Uuid, rows: isize) -> bool {
        let Some(session) = self.sessions.get(&id) else {
            return false;
        };
        let (offset, max_row) =
            if let Some((_, height, total, offset)) = session.preview_layout.get() {
                (offset, total.saturating_sub(height))
            } else {
                let Ok(lines) = self.line_count(id) else {
                    return false;
                };
                (
                    session.viewport.corner.get_y(),
                    lines.saturating_sub(session.viewport.get_height().max(1)),
                )
            };
        offset.saturating_add_signed(rows).min(max_row) != offset
    }

    pub(crate) fn scroll(
        &mut self,
        id: Uuid,
        rows: isize,
        columns: isize,
    ) -> Result<(), EditorError> {
        let has_preview_layout = self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?
            .preview_layout
            .get();
        if let Some((width, height, total, offset)) = has_preview_layout {
            self.sessions
                .get(&id)
                .expect("session checked above")
                .preview_layout
                .set(Some((
                    width,
                    height,
                    total,
                    offset
                        .saturating_add_signed(rows)
                        .min(total.saturating_sub(height)),
                )));
            return Ok(());
        }
        if columns == 0 {
            let line_count = self.line_count(id)?;
            let session = self.sessions.get_mut(&id).expect("session checked above");
            let max_row = line_count.saturating_sub(session.viewport.get_height().max(1));
            let corner = &mut session.viewport.corner;
            corner.set_y(corner.get_y().saturating_add_signed(rows).min(max_row));
            return Ok(());
        }
        let text = self.text(id)?;
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let max_line_width = text
            .split('\n')
            .map(project_editor_line)
            .map(|line| line.source_to_display_cells.last().copied().unwrap_or(0))
            .max()
            .unwrap_or(0);
        let max_row = text
            .split('\n')
            .count()
            .saturating_sub(session.viewport.get_height().max(1));
        let max_column = max_line_width.saturating_sub(session.viewport.get_width().max(1));
        let corner = &mut session.viewport.corner;
        if rows.is_negative() {
            corner.set_y(corner.get_y().saturating_sub(rows.unsigned_abs()));
        } else {
            corner.set_y(corner.get_y().saturating_add(rows as usize).min(max_row));
        }
        if columns.is_negative() {
            corner.set_x(corner.get_x().saturating_sub(columns.unsigned_abs()));
        } else {
            corner.set_x(
                corner
                    .get_x()
                    .saturating_add(columns as usize)
                    .min(max_column),
            );
        }
        corner.set_y(corner.get_y().min(max_row));
        corner.set_x(corner.get_x().min(max_column));
        Ok(())
    }

    pub(crate) fn set_scroll_offset(
        &mut self,
        id: Uuid,
        rows: usize,
        columns: usize,
    ) -> Result<(), EditorError> {
        let has_preview_layout = self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?
            .preview_layout
            .get();
        if let Some((width, height, total, _)) = has_preview_layout {
            self.sessions
                .get(&id)
                .expect("session checked above")
                .preview_layout
                .set(Some((
                    width,
                    height,
                    total,
                    rows.min(total.saturating_sub(height)),
                )));
            return Ok(());
        }
        let text = self.text(id)?;
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let max_row = text
            .split('\n')
            .count()
            .saturating_sub(session.viewport.get_height().max(1));
        let max_column = full_line_width(&text).saturating_sub(session.viewport.get_width().max(1));
        session.viewport.corner.set_y(rows.min(max_row));
        session.viewport.corner.set_x(columns.min(max_column));
        Ok(())
    }

    pub(crate) fn set_scroll_axis(
        &mut self,
        id: Uuid,
        vertical: bool,
        offset: usize,
    ) -> Result<(), EditorError> {
        let has_preview_layout = self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?
            .preview_layout
            .get();
        if let Some((width, height, total, _)) = has_preview_layout {
            self.sessions
                .get(&id)
                .expect("session checked above")
                .preview_layout
                .set(Some((
                    width,
                    height,
                    total,
                    offset.min(total.saturating_sub(height)),
                )));
            if vertical {
                return Ok(());
            }
        }
        let text = self.text(id)?;
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        if let Some((width, height, total, _)) = session.preview_layout.get() {
            if vertical {
                session.preview_layout.set(Some((
                    width,
                    height,
                    total,
                    offset.min(total.saturating_sub(height)),
                )));
            }
            return Ok(());
        }
        let max_row = text
            .split('\n')
            .count()
            .saturating_sub(session.viewport.get_height().max(1));
        let max_column = full_line_width(&text).saturating_sub(session.viewport.get_width().max(1));
        if vertical {
            session.viewport.corner.set_y(offset.min(max_row));
        } else {
            session.viewport.corner.set_x(offset.min(max_column));
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

    pub(crate) fn prompt_active(&self, id: Uuid) -> bool {
        self.prompt
            .as_ref()
            .is_some_and(|prompt| prompt.owner == id)
    }

    pub(crate) fn has_pending_interaction(&self, id: Uuid) -> bool {
        let Some(session) = self.sessions.get(&id) else {
            return false;
        };
        self.prompt_active(id)
            || !matches!(session.mode, EditorMode::Normal)
            || session.pending_binding.is_some()
            || session.pending_count.is_some()
            || !session.current_sequence.is_empty()
            || session.keys.get_cursor_indicator().is_some()
    }

    pub(crate) fn cancel_leader_bindings(&mut self) {
        for session in self.sessions.values_mut() {
            if matches!(
                session.pending_binding,
                Some(PendingBinding::Leader | PendingBinding::LeaderTransaction)
            ) {
                session.pending_binding = None;
                session.current_sequence.clear();
            }
        }
    }

    pub(crate) fn reset_interaction(&mut self, id: Uuid) -> Result<(), EditorError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        session.keys.reset_mode();
        session.pending_binding = None;
        session.pending_count = None;
        session.pending_vim_character = false;
        session.current_sequence.clear();
        session.last_sequence = None;
        session.mode = EditorMode::Normal;
        let prompt_owned = self
            .prompt
            .as_ref()
            .is_some_and(|prompt| prompt.owner == id);
        if prompt_owned {
            self.prompt = None;
        }
        Ok(())
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

    /// Clamp a source coordinate to a valid editor cursor without changing the session.
    pub(crate) fn mouse_position(
        &self,
        id: Uuid,
        position: EditorPosition,
    ) -> Result<EditorPosition, EditorError> {
        let text = self.text(id)?;
        let line_count = text.matches('\n').count();
        let line = position.line.min(line_count);
        let line_start = text
            .split_inclusive('\n')
            .take(line)
            .map(str::len)
            .sum::<usize>();
        let line_end = text[line_start..]
            .find('\n')
            .map_or(text.len(), |offset| line_start + offset);
        let column = text[line_start..line_end]
            .chars()
            .count()
            .min(position.column);
        Ok(EditorPosition { line, column })
    }

    /// Return the exact source text covered by a mouse range without changing
    /// the editor mode, cursor, selection, revision, or history.
    pub(crate) fn mouse_range_text(
        &self,
        id: Uuid,
        start: EditorPosition,
        end: EditorPosition,
    ) -> Result<String, EditorError> {
        let text = self.text(id)?;
        let start = self.mouse_position(id, start)?;
        let end = self.mouse_position(id, end)?;
        let start_offset = char_position_to_byte(&text, start);
        let end_offset = char_position_to_byte(&text, end);
        let (first, last) = if start_offset <= end_offset {
            (start, end)
        } else {
            (end, start)
        };
        let first_offset = char_position_to_byte(&text, first);
        let last_offset = char_position_to_byte(&text, last);
        let last_end = text[last_offset..]
            .chars()
            .next()
            .map_or(last_offset, |character| {
                if character == '\n' {
                    last_offset
                } else {
                    last_offset + character.len_utf8()
                }
            });
        let selected = text[first_offset..last_end].to_owned();

        Ok(selected)
    }

    /// Place the editor cursor from a rendered source coordinate without
    /// changing text, revision, or edit history.
    pub(crate) fn set_mouse_cursor(
        &mut self,
        id: Uuid,
        position: EditorPosition,
    ) -> Result<(), EditorError> {
        let position = self.mouse_position(id, position)?;
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        session.position = position;
        let mut buffer = session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        buffer.set_leader(
            session.group_id,
            modalkit::editing::cursor::Cursor::new(position.line, position.column),
        );
        Ok(())
    }

    fn set_keyboard_cursor(
        &mut self,
        id: Uuid,
        position: EditorPosition,
    ) -> Result<(), EditorError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        session.position = position;
        let mut buffer = session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        buffer.set_leader(
            session.group_id,
            modalkit::editing::cursor::Cursor::new(position.line, position.column),
        );
        Ok(())
    }

    pub(crate) fn mouse_selection(&self, id: Uuid) -> Result<Option<String>, EditorError> {
        let text = self.text(id)?;
        let session = self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let selection = session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?
            .get_leader_selection(session.group_id);
        let Some((start, end, _)) = selection else {
            return Ok(None);
        };
        let start = EditorPosition {
            line: start.get_y(),
            column: start.get_x(),
        };
        let end = EditorPosition {
            line: end.get_y(),
            column: end.get_x(),
        };
        let start_offset = char_position_to_byte(&text, start);
        let end_offset = char_position_to_byte(&text, end);
        let end_offset = text[end_offset..]
            .chars()
            .next()
            .map_or(end_offset, |character| {
                if character == '\n' {
                    end_offset
                } else {
                    end_offset + character.len_utf8()
                }
            });
        Ok(Some(
            text[start_offset.min(end_offset)..start_offset.max(end_offset)].to_owned(),
        ))
    }

    pub(crate) fn revision(&self, id: Uuid) -> Result<u64, EditorError> {
        self.sessions
            .get(&id)
            .map(|session| session.revision)
            .ok_or(EditorError::MissingSession(id))
    }

    fn snapshot(&self, id: Uuid) -> Result<EditorSnapshot, EditorError> {
        Ok(EditorSnapshot {
            text: self.text(id)?,
            position: self.position(id)?,
        })
    }

    fn write_text(
        &mut self,
        id: Uuid,
        text: &str,
        position: EditorPosition,
    ) -> Result<(), EditorError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let old_text = {
            let buffer = session
                .buffer
                .read()
                .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
            decode_editor_text(&buffer.get_text())?
        };
        if old_text != text {
            session
                .jump_history
                .remap(|position| remap_text_position(&old_text, text, position));
        }
        let mut buffer = session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        buffer.set_text(encode_editor_text(text));
        let line_count = text.matches('\n').count();
        let target_line = position.line.min(line_count);
        let line = text.split('\n').nth(target_line).unwrap_or_default();
        session.position = EditorPosition {
            line: target_line,
            column: position.column.min(line.chars().count()),
        };
        buffer.set_leader(
            session.group_id,
            modalkit::editing::cursor::Cursor::new(session.position.line, session.position.column),
        );
        Ok(())
    }

    fn restore_snapshot(&mut self, id: Uuid, snapshot: EditorSnapshot) -> Result<(), EditorError> {
        self.restore_snapshot_with_mode(id, snapshot, EditorMode::Normal)
    }

    fn restore_snapshot_with_mode(
        &mut self,
        id: Uuid,
        snapshot: EditorSnapshot,
        mode: EditorMode,
    ) -> Result<(), EditorError> {
        self.write_text(id, &snapshot.text, snapshot.position)?;
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
        } else if mode == EditorMode::Replace {
            session
                .keys
                .input_key(modalkit::key::TerminalKey::from(KeyEvent::new(
                    KeyCode::Char('R'),
                    KeyModifiers::NONE,
                )));
            while session.keys.pop().is_some() {}
        }
        session.mode = mode;
        session.pending_binding = None;
        session.pending_count = None;
        session.current_sequence.clear();
        session.last_sequence = None;
        Ok(())
    }

    fn record_edit_history(
        &mut self,
        id: Uuid,
        before: EditorSnapshot,
        after: &EditorSnapshot,
        mode_before: EditorMode,
        mode_after: EditorMode,
    ) -> Result<(), EditorError> {
        let in_insert = matches!(mode_before, EditorMode::Insert | EditorMode::Replace)
            || matches!(mode_after, EditorMode::Insert | EditorMode::Replace);
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        if in_insert {
            if session.history.transaction_start.is_none() {
                session.history.transaction_start = Some(before);
            }
            session.history.redo.clear();
        } else {
            session.history.push_undo(before);
        }
        if !matches!(mode_after, EditorMode::Insert | EditorMode::Replace)
            && matches!(mode_before, EditorMode::Insert | EditorMode::Replace)
            && let Some(start) = session.history.transaction_start.take()
            && start.text != after.text
        {
            session.history.push_undo(start);
        }
        Ok(())
    }

    fn commit_transaction(&mut self, id: Uuid) -> Result<(), EditorError> {
        let current = self.snapshot(id)?;
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        if let Some(start) = session.history.transaction_start.take()
            && start.text != current.text
        {
            session.history.push_undo(start);
        }
        Ok(())
    }

    fn record_changed(&mut self, id: Uuid) -> Result<(), EditorError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        session.revision = session.revision.saturating_add(1);
        self.effects.push(EditorEffect::Changed {
            console_id: id,
            revision: session.revision,
        });
        Ok(())
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

    pub(crate) fn line_count(&self, id: Uuid) -> Result<usize, EditorError> {
        let session = self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let buffer = session
            .buffer
            .read()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        Ok(buffer.get_lines().max(1))
    }

    pub(crate) fn render_snapshot(
        &self,
        id: Uuid,
        viewport: EditorViewport,
    ) -> Result<EditorRenderSnapshot, EditorError> {
        self.render_snapshot_with_dialect(id, viewport, SqlDialect::Generic)
    }

    /// Render a read-only document without invoking SQL analysis. This is the
    /// first shared viewport entry point for non-SQL value previews.
    pub(crate) fn render_plain_snapshot(
        &self,
        id: Uuid,
        viewport: EditorViewport,
    ) -> Result<EditorRenderSnapshot, EditorError> {
        self.render_preview_snapshot(
            id,
            viewport,
            crate::model::editor_language::EditorLanguage::Plain,
        )
    }

    pub(crate) fn render_preview_snapshot(
        &self,
        id: Uuid,
        viewport: EditorViewport,
        language: crate::model::editor_language::EditorLanguage,
    ) -> Result<EditorRenderSnapshot, EditorError> {
        let mut snapshot = self.render_snapshot_with_options(
            id,
            viewport,
            SqlDialect::Generic,
            None,
            None,
            false,
        )?;
        let cached_highlights = self.preview_highlights(id, language)?;
        for line in &mut snapshot.lines {
            line.spans = match language {
                crate::model::editor_language::EditorLanguage::Json
                | crate::model::editor_language::EditorLanguage::Yaml => cached_highlights
                    .get(line.line)
                    .cloned()
                    .unwrap_or_default(),
                crate::model::editor_language::EditorLanguage::Plain
                | crate::model::editor_language::EditorLanguage::Sql(_) => line
                    .spans
                    .iter()
                    .map(|span| {
                        let mut span = span.clone();
                        span.kind = EditorHighlightKind::Plain;
                        span
                    })
                    .collect(),
            };
        }
        snapshot.semantic_diagnostics.clear();
        Ok(snapshot)
    }

    fn preview_highlights(
        &self,
        id: Uuid,
        language: crate::model::editor_language::EditorLanguage,
    ) -> Result<Arc<Vec<Vec<EditorRenderSpan>>>, EditorError> {
        let session = self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?;
        if let Some((revision, cached_language, highlights)) =
            self.sessions.get(&id).and_then(|session| {
                session.preview_highlights.borrow().as_ref().map(
                    |(revision, cached_language, highlights)| {
                        (*revision, *cached_language, Arc::clone(highlights))
                    },
                )
            })
            && revision == session.revision
            && cached_language == language
        {
            return Ok(highlights);
        }
        let preview_document = session.preview_document.borrow();
        let Some((revision, document)) = preview_document.as_ref() else {
            return Ok(Arc::new(Vec::new()));
        };
        let highlights = Arc::new(
            document
                .projections
                .iter()
                .enumerate()
                .map(|(line, projection)| {
                    Self::preview_highlight_spans(
                        &projection.text,
                        document.line_starts[line],
                        language,
                    )
                })
                .collect::<Vec<_>>(),
        );
        *session.preview_highlights.borrow_mut() =
            Some((*revision, language, Arc::clone(&highlights)));
        Ok(highlights)
    }

    pub(crate) fn render_wrapped_preview_snapshot(
        &self,
        id: Uuid,
        viewport: EditorViewport,
        language: crate::model::editor_language::EditorLanguage,
        wrap: bool,
    ) -> Result<EditorRenderSnapshot, EditorError> {
        let session = self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?;
        if !wrap {
            session.preview_layout.set(None);
            return self.render_preview_snapshot(id, viewport, language);
        }
        let width = viewport.width.max(1);
        if session
            .preview_document
            .borrow()
            .as_ref()
            .is_none_or(|(revision, _)| *revision != session.revision)
        {
            let _ = self.render_preview_snapshot(id, viewport, language)?;
        }
        let document = session
            .preview_document
            .borrow()
            .as_ref()
            .map(|(_, document)| Arc::clone(document));
        let wrap_index = if let Some(document) = document.as_ref() {
            let mut cached = session.preview_wrap_index.borrow_mut();
            if cached.as_ref().is_none_or(|(revision, index)| {
                *revision != session.revision || index.width != width
            }) {
                *cached = Some((
                    session.revision,
                    preview::PreviewWrapIndex::from_document(document, width),
                ));
            }
            cached.as_ref().map(|(_, index)| index.clone())
        } else {
            None
        };
        let index = wrap_index
            .as_ref()
            .ok_or_else(|| EditorError::Operation("preview wrap index is unavailable".into()))?;
        let first = session
            .preview_layout
            .get()
            .map_or(0, |(_, _, _, offset)| offset)
            .min(index.total_rows.saturating_sub(viewport.height));
        let first_line = index
            .line_visual_starts
            .partition_point(|row| *row <= first)
            .saturating_sub(1);
        let last_line = index
            .line_visual_starts
            .partition_point(|row| *row < first.saturating_add(viewport.height.max(1)))
            .min(index.starts.len())
            .max(first_line + 1);
        session.preview_render_first_line.set(Some(first_line));
        let mut snapshot = self.render_preview_snapshot(
            id,
            EditorViewport {
                width: viewport.width,
                height: last_line.saturating_sub(first_line).saturating_add(2),
            },
            language,
        )?;
        session.preview_render_first_line.set(None);
        let mut visual = Vec::new();
        for (local_line, starts) in index.starts[first_line..last_line].iter().enumerate() {
            for offset in starts {
                visual.push((local_line, *offset));
            }
        }
        let local_first = first.saturating_sub(index.line_visual_starts[first_line]);
        if let Some(session) = self.sessions.get(&id) {
            session
                .preview_layout
                .set(Some((width, viewport.height, index.total_rows, first)));
        }
        snapshot.cursor_screen_cell = visual
            .iter()
            .enumerate()
            .skip(local_first)
            .take(viewport.height)
            .find_map(|(row, (index, offset))| {
                let line = &snapshot.lines[*index];
                if line.line != snapshot.cursor.line {
                    return None;
                }
                let cell = line
                    .source_to_display_cells
                    .get(snapshot.cursor.column)
                    .copied()
                    .unwrap_or(0);
                (cell >= *offset && cell < offset + width).then_some((
                    (cell.saturating_sub(*offset)) as u16,
                    (row - local_first) as u16,
                ))
            });
        snapshot.total_lines = index.total_rows;
        snapshot.first_line = first;
        snapshot.lines = visual
            .into_iter()
            .skip(local_first)
            .take(viewport.height)
            .map(|(index, offset)| {
                let mut line = snapshot.lines[index].clone();
                line.wrap_offset = offset;
                line
            })
            .collect();
        snapshot.viewport = viewport;
        snapshot.horizontal_offset = 0;
        snapshot.max_line_width = width;
        Ok(snapshot)
    }

    fn preview_highlight_spans(
        text: &str,
        source_start: usize,
        language: crate::model::editor_language::EditorLanguage,
    ) -> Vec<EditorRenderSpan> {
        let mut spans = Vec::new();
        let mut token_start = 0usize;
        let mut in_string = false;
        let mut quote = '"';
        let mut escaped = false;
        let mut index = 0usize;
        let chars = text.char_indices().collect::<Vec<_>>();
        let flush = |spans: &mut Vec<EditorRenderSpan>, start: usize, end: usize, kind| {
            if end > start {
                spans.push(EditorRenderSpan {
                    text: text[start..end].to_owned(),
                    source_start: source_start + start,
                    source_end: source_start + end,
                    kind,
                    current_statement: false,
                });
            }
        };
        while index < chars.len() {
            let (byte, character) = chars[index];
            if in_string {
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == quote {
                    in_string = false;
                    let end = byte + character.len_utf8();
                    flush(&mut spans, token_start, end, EditorHighlightKind::String);
                    token_start = end;
                }
                index += 1;
                continue;
            }
            if character == '"'
                || (character == '\''
                    && language == crate::model::editor_language::EditorLanguage::Yaml)
            {
                flush(&mut spans, token_start, byte, EditorHighlightKind::Plain);
                in_string = true;
                quote = character;
                token_start = byte;
            } else if matches!(
                language,
                crate::model::editor_language::EditorLanguage::Yaml
            ) && character == '#'
            {
                flush(&mut spans, token_start, byte, EditorHighlightKind::Plain);
                flush(&mut spans, byte, text.len(), EditorHighlightKind::Comment);
                token_start = text.len();
                break;
            } else if character.is_alphabetic() {
                let end = chars
                    .iter()
                    .skip(index)
                    .take_while(|(_, value)| value.is_alphanumeric() || *value == '_')
                    .last()
                    .map_or(byte + character.len_utf8(), |(offset, value)| {
                        offset + value.len_utf8()
                    });
                let token = &text[byte..end];
                let kind = if matches!(token, "true" | "false" | "null" | "True" | "False" | "Null")
                {
                    EditorHighlightKind::Keyword
                } else if language == crate::model::editor_language::EditorLanguage::Yaml
                    && text[end..].trim_start().starts_with(':')
                {
                    EditorHighlightKind::Identifier
                } else {
                    EditorHighlightKind::Plain
                };
                flush(&mut spans, token_start, byte, EditorHighlightKind::Plain);
                flush(&mut spans, byte, end, kind);
                token_start = end;
                index = chars
                    .iter()
                    .position(|(offset, _)| *offset >= end)
                    .unwrap_or(chars.len());
                continue;
            } else if character.is_ascii_digit() || (character == '-' && token_start == byte) {
                if token_start < byte {
                    flush(&mut spans, token_start, byte, EditorHighlightKind::Plain);
                }
                let end = chars
                    .iter()
                    .skip(index)
                    .take_while(|(_, value)| {
                        value.is_ascii_digit() || matches!(value, '.' | '-' | '+')
                    })
                    .last()
                    .map_or(byte + character.len_utf8(), |(end, value)| {
                        end + value.len_utf8()
                    });
                flush(&mut spans, byte, end, EditorHighlightKind::Number);
                token_start = end;
                index = chars
                    .iter()
                    .position(|(offset, _)| *offset >= end)
                    .unwrap_or(chars.len());
                continue;
            }
            index += 1;
        }
        if in_string {
            flush(
                &mut spans,
                token_start,
                text.len(),
                EditorHighlightKind::String,
            );
        } else {
            flush(
                &mut spans,
                token_start,
                text.len(),
                EditorHighlightKind::Plain,
            );
        }
        spans
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
        self.render_snapshot_with_options(id, viewport, dialect, statement, sql_ranges, true)
    }

    fn render_snapshot_with_options(
        &self,
        id: Uuid,
        viewport: EditorViewport,
        dialect: SqlDialect,
        statement: Option<sql::TextRange>,
        sql_ranges: Option<&[sql::TextRange]>,
        analyze_sql: bool,
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
        let preview_document = if !analyze_sql && statement.is_none() && sql_ranges.is_none() {
            let encoded = buffer.get_text();
            let mut cached = session.preview_document.borrow_mut();
            if cached
                .as_ref()
                .is_none_or(|(revision, _)| *revision != session.revision)
            {
                let text = decode_editor_text(&encoded)?;
                *cached = Some((
                    session.revision,
                    Arc::new(preview::PreviewDocument::from_text(text)),
                ));
            }
            cached.as_ref().map(|(_, document)| Arc::clone(document))
        } else {
            None
        };
        let full_text = preview_document.as_ref().map_or_else(
            || decode_editor_text(&buffer.get_text()),
            |document| Ok(document.text.clone()),
        )?;
        let max_line_width = preview_document.as_ref().map_or_else(
            || full_line_width(&full_text),
            |document| document.max_line_width,
        );
        let tail_requested =
            session.pending_tail_scroll && viewport.width > 0 && viewport.height > 0;
        let max_row = total_lines.saturating_sub(viewport.height.max(1));
        let first_line = if let Some(preview_first_line) = session.preview_render_first_line.get() {
            preview_first_line.min(total_lines.saturating_sub(1))
        } else if tail_requested {
            max_row
        } else {
            session.viewport.corner.get_y().min(max_row)
        };
        let horizontal_offset = if tail_requested {
            0
        } else {
            session.viewport.corner.get_x()
        };
        let overscan = 2;
        let highlights = if analyze_sql {
            let key = sql::AnalysisKey {
                console_id: id,
                document_revision: session.revision,
                dialect,
                highlight_ranges: sql_ranges.map(<[sql::TextRange]>::to_vec),
            };
            self.analysis_cache
                .borrow_mut()
                .retain(|cached, _| cached.console_id != id || *cached == key);
            self.analysis_cache
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
                .clone()
        } else {
            Vec::new()
        };
        let statements = statement.map(|_| sql::scan_statements(&full_text, dialect));
        let mut line_start = preview_document.as_ref().map_or_else(
            || {
                full_text
                    .split_inclusive('\n')
                    .take(first_line)
                    .map(str::len)
                    .sum::<usize>()
            },
            |document| *document.line_starts.get(first_line).unwrap_or(&0),
        );
        let mut lines = if let Some(document) = preview_document.as_ref() {
            document
                .lines
                .iter()
                .skip(first_line)
                .take(viewport.height.saturating_add(overscan))
                .enumerate()
                .map(|(offset, source)| {
                    let line_index = first_line + offset;
                    let source = source.clone();
                    let projection = &document.projections[line_index];
                    let current_line_start = document.line_starts[line_index];
                    let line_end = current_line_start + source.len();
                    let current_statement = statement.is_some_and(|range| {
                        range.start < line_end && range.end > current_line_start
                    });
                    let statement_background_cells = statement_background_cells(
                        &source,
                        current_line_start,
                        line_end,
                        statement,
                        statements.as_deref().unwrap_or_default(),
                        &projection.source_to_display_cells,
                    );
                    let mut spans = Vec::new();
                    let mut byte = 0;
                    for highlight in highlights.iter().filter(|item| {
                        item.range.start < line_end && item.range.end > current_line_start
                    }) {
                        let start =
                            highlight.range.start.max(current_line_start) - current_line_start;
                        let end = highlight.range.end.min(line_end) - current_line_start;
                        if start > byte {
                            spans.push(render_span(
                                byte,
                                start,
                                EditorHighlightKind::Plain,
                                statement,
                                current_line_start,
                                projection,
                            ));
                        }
                        if end > start {
                            spans.push(render_span(
                                start,
                                end,
                                map_highlight(highlight.kind),
                                statement,
                                current_line_start,
                                projection,
                            ));
                        }
                        byte = byte.max(end);
                    }
                    if byte < source.len() {
                        spans.push(render_span(
                            byte,
                            source.len(),
                            EditorHighlightKind::Plain,
                            statement,
                            current_line_start,
                            projection,
                        ));
                    }
                    EditorRenderLine {
                        wrap_offset: 0,
                        line: first_line + offset,
                        display_text: projection.text.clone(),
                        spans: if spans.is_empty() {
                            vec![EditorRenderSpan {
                                text: projection.text.clone(),
                                source_start: current_line_start,
                                source_end: line_end,
                                kind: EditorHighlightKind::Plain,
                                current_statement: statement.is_some_and(|range| {
                                    range.start < line_end && range.end > current_line_start
                                }),
                            }]
                        } else {
                            spans
                        },
                        source_start: current_line_start,
                        source_end: line_end,
                        source_byte_boundaries: projection.source_byte_boundaries.clone(),
                        source_to_display_bytes: projection.source_to_display_bytes.clone(),
                        source_to_display_cells: projection.source_to_display_cells.clone(),
                        current_statement,
                        statement_background_cells,
                        selection_newline: false,
                    }
                })
                .collect::<Vec<_>>()
        } else {
            buffer
                .lines(first_line)
                .take(viewport.height.saturating_add(overscan))
                .enumerate()
                .map(|(offset, line)| {
                    let source = line.to_string();
                    let projection = project_editor_line(&source);
                    let current_line_start = full_text
                        .split_inclusive('\n')
                        .take(first_line + offset)
                        .map(str::len)
                        .sum::<usize>();
                    let line_end = current_line_start + source.len();
                    let current_statement = statement.is_some_and(|range| {
                        range.start < line_end && range.end > current_line_start
                    });
                    let statement_background_cells = statement_background_cells(
                        &source,
                        current_line_start,
                        line_end,
                        statement,
                        statements.as_deref().unwrap_or_default(),
                        &projection.source_to_display_cells,
                    );
                    let mut spans = Vec::new();
                    let mut byte = 0;
                    for highlight in highlights.iter().filter(|item| {
                        item.range.start < line_end && item.range.end > current_line_start
                    }) {
                        let start =
                            highlight.range.start.max(current_line_start) - current_line_start;
                        let end = highlight.range.end.min(line_end) - current_line_start;
                        if start > byte {
                            spans.push(render_span(
                                byte,
                                start,
                                EditorHighlightKind::Plain,
                                statement,
                                current_line_start,
                                &projection,
                            ));
                        }
                        if end > start {
                            spans.push(render_span(
                                start,
                                end,
                                map_highlight(highlight.kind),
                                statement,
                                current_line_start,
                                &projection,
                            ));
                        }
                        byte = byte.max(end);
                    }
                    if byte < source.len() {
                        spans.push(render_span(
                            byte,
                            source.len(),
                            EditorHighlightKind::Plain,
                            statement,
                            current_line_start,
                            &projection,
                        ));
                    }
                    let rendered = EditorRenderLine {
                        wrap_offset: 0,
                        line: first_line + offset,
                        display_text: projection.text.clone(),
                        spans: if spans.is_empty() {
                            vec![EditorRenderSpan {
                                text: projection.text,
                                source_start: current_line_start,
                                source_end: line_end,
                                kind: EditorHighlightKind::Plain,
                                current_statement,
                            }]
                        } else {
                            spans
                        },
                        source_start: current_line_start,
                        source_end: line_end,
                        source_byte_boundaries: projection.source_byte_boundaries,
                        source_to_display_bytes: projection.source_to_display_bytes,
                        source_to_display_cells: projection.source_to_display_cells,
                        current_statement,
                        statement_background_cells,
                        selection_newline: false,
                    };
                    line_start = line_end + usize::from(line_end < full_text.len());
                    rendered
                })
                .collect::<Vec<_>>()
        };
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
        let selection_newlines = selections
            .iter()
            .filter(|selection| {
                selection.shape == EditorSelectionShape::Char
                    && selection.start.line != selection.end.line
            })
            .flat_map(|selection| {
                let first_line = selection.start.line.min(selection.end.line);
                let last_line = selection.start.line.max(selection.end.line);
                (first_line..last_line).collect::<Vec<_>>()
            })
            .collect::<std::collections::HashSet<_>>();
        for line in &mut lines {
            line.selection_newline = selection_newlines.contains(&line.line);
        }
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
                                let (first, last) =
                                    if (selection.start.line, selection.start.column)
                                        <= (selection.end.line, selection.end.column)
                                    {
                                        (selection.start, selection.end)
                                    } else {
                                        (selection.end, selection.start)
                                    };
                                let start = if line == first.line { first.column } else { 0 };
                                let end = if line == last.line {
                                    last.column.saturating_add(1)
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
                cell.checked_sub(horizontal_offset)
                    .filter(|cell| *cell < viewport.width && row < viewport.height)
                    .map(|cell| (cell as u16, row as u16))
            });

        Ok(EditorRenderSnapshot {
            revision: session.revision,
            mode: session.mode,
            first_line,
            logical_line_count: total_lines,
            total_lines,
            viewport,
            horizontal_offset,
            max_line_width,
            lines,
            cursor: session.position,
            cursor_screen_cell,
            selections,
            selection_cells,
            prompt: self
                .prompt
                .as_ref()
                .filter(|prompt| prompt.owner == id)
                .map(|prompt| EditorPromptSnapshot {
                    kind: prompt.kind,
                    prefix: match prompt.kind {
                        EditorPromptKind::SearchForward => "/".to_owned(),
                        EditorPromptKind::SearchBackward => "?".to_owned(),
                        EditorPromptKind::Command => ":".to_owned(),
                    },
                    text: project_editor_line(prompt.input.value()).text,
                    cursor: project_editor_line(prompt.input.value())
                        .source_to_display_cells
                        .get(prompt.input.cursor())
                        .copied()
                        .unwrap_or_default(),
                    error: prompt
                        .error
                        .as_deref()
                        .map(|error| project_editor_line(error).text),
                }),
            semantic_diagnostics: Vec::new(),
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
                    } else if kind == ScopeKind::VisualLine {
                        let first_row = cursor.get_y().min(anchor.get_y());
                        let last_row = cursor.get_y().max(anchor.get_y());
                        let start = line_bounds(&text, first_row).0;
                        let end = line_bounds(&text, last_row).1;
                        ScopeSelection::contiguous(kind, TextRange::new(start, end))
                    } else {
                        let (start, end) = if start <= end {
                            (start, inclusive_position_end(&text, end))
                        } else {
                            (end, inclusive_position_end(&text, start))
                        };
                        ScopeSelection::contiguous(kind, TextRange::new(start, end))
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
        let mode_before = self.mode(id)?;
        let before = self.snapshot(id)?;
        let old = before.text.clone();
        if range.get(&old).is_none() {
            return Err(EditorError::Operation(
                "format range is not on a UTF-8 boundary".into(),
            ));
        }
        let mut next = old.clone();
        next.replace_range(range.start..range.end, replacement);
        let cursor_offset = match cursor {
            ReplacementCursor::Start => range.start,
            ReplacementCursor::EndOfInsertion => range.start + replacement.len(),
            ReplacementCursor::PreserveRelative => range.start + replacement.len(),
        };
        let position = byte_to_char_position(&next, cursor_offset);
        if next == old {
            let session = self
                .sessions
                .get_mut(&id)
                .ok_or(EditorError::MissingSession(id))?;
            session.position = position;
            let mut buffer = session
                .buffer
                .write()
                .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
            buffer.set_leader(
                session.group_id,
                modalkit::editing::cursor::Cursor::new(position.line, position.column),
            );
            return Ok(());
        }
        self.write_text(id, &next, position)?;
        let after = self.snapshot(id)?;
        self.record_edit_history(id, before, &after, mode_before, mode_before)?;
        self.record_changed(id)?;
        Ok(())
    }

    pub(crate) fn replace_range_preserving_selection(
        &mut self,
        id: Uuid,
        range: TextRange,
        replacement: &str,
    ) -> Result<(), EditorError> {
        let mode_before = self.mode(id)?;
        let before = self.snapshot(id)?;
        let session = self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let mut buffer = session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        let selection = buffer.get_leader_selection(session.group_id);
        drop(buffer);
        let Some((selection_start, selection_end, shape)) = selection else {
            return self.replace_range(id, range, replacement, ReplacementCursor::Start);
        };
        let old_cursor = char_position_to_byte(&before.text, before.position);
        let selection_start = position_to_byte(
            &before.text,
            selection_start.get_y(),
            selection_start.get_x(),
        );
        let selection_end =
            position_to_byte(&before.text, selection_end.get_y(), selection_end.get_x());
        let old_anchor = if old_cursor == selection_start {
            selection_end
        } else {
            selection_start
        };
        let mut next = before.text.clone();
        if range.get(&next).is_none() {
            return Err(EditorError::Operation(
                "format range is not on a UTF-8 boundary".into(),
            ));
        }
        next.replace_range(range.start..range.end, replacement);
        if next == before.text {
            return Ok(());
        }
        let map_endpoint = |offset: usize, is_start: bool| {
            let mapped = if offset <= range.start {
                offset
            } else if offset >= range.end {
                range.start + replacement.len()
            } else if is_start {
                range.start
            } else {
                range.start + replacement.len()
            };
            byte_to_char_position(&next, mapped)
        };
        let cursor_is_start = old_cursor <= old_anchor;
        let cursor = map_endpoint(old_cursor, cursor_is_start);
        let anchor = map_endpoint(old_anchor, !cursor_is_start);
        self.write_text(id, &next, cursor)?;
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let mut buffer = session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        buffer.set_group(
            session.group_id,
            modalkit::editing::cursor::CursorGroup::new(
                modalkit::editing::cursor::CursorState::Selection(
                    modalkit::editing::cursor::Cursor::new(cursor.line, cursor.column),
                    modalkit::editing::cursor::Cursor::new(anchor.line, anchor.column),
                    shape,
                ),
                Vec::new(),
            ),
        );
        drop(buffer);
        let after = self.snapshot(id)?;
        self.record_edit_history(id, before, &after, mode_before, mode_before)?;
        self.record_changed(id)
    }

    pub(crate) fn set_viewport(
        &mut self,
        id: Uuid,
        viewport: EditorViewport,
    ) -> Result<(), EditorError> {
        let position = {
            let session = self
                .sessions
                .get_mut(&id)
                .ok_or(EditorError::MissingSession(id))?;
            session.viewport.dimensions = (viewport.width, viewport.height);
            session.position
        };
        self.ensure_cursor_visible_at(id, position, false)?;
        Ok(())
    }

    pub(crate) fn sync_output_viewport(
        &mut self,
        id: Uuid,
        viewport: EditorViewport,
    ) -> Result<(), EditorError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        if viewport.width == 0 || viewport.height == 0 {
            return Ok(());
        }
        let tail_pending = session.pending_tail_scroll;
        if !tail_pending
            && session.viewport.get_width() == viewport.width
            && session.viewport.get_height() == viewport.height
        {
            return Ok(());
        }
        let text = {
            let buffer = session
                .buffer
                .read()
                .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
            decode_editor_text(&buffer.get_text())?
        };
        let line_count = text.split('\n').count().max(1);
        let max_row = line_count.saturating_sub(viewport.height.max(1));
        let max_column = full_line_width(&text).saturating_sub(viewport.width.max(1));
        session.viewport.dimensions = (viewport.width, viewport.height);
        if tail_pending {
            session.viewport.corner.set_y(max_row);
            session.viewport.corner.set_x(0);
            session.pending_tail_scroll = false;
        } else {
            session
                .viewport
                .corner
                .set_y(session.viewport.corner.get_y().min(max_row));
            session
                .viewport
                .corner
                .set_x(session.viewport.corner.get_x().min(max_column));
        }
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
        session.jump_history.clear();
        session.keys.reset_mode();
        session.position = EditorPosition { line: 0, column: 0 };
        session.viewport = Default::default();
        session.mode = EditorMode::Normal;
        session.revision = session.revision.saturating_add(1);
        session.history = EditorHistory::default();
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
        let encoded = encode_editor_text(text);
        let old_text = {
            let buffer = session
                .buffer
                .read()
                .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
            decode_editor_text(&buffer.get_text())?
        };
        let mut buffer = session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        if buffer.get_text() != encoded {
            buffer.set_text(encoded);
            if old_text != text {
                session
                    .jump_history
                    .remap(|position| remap_text_position(&old_text, text, position));
            }
            session.revision = session.revision.saturating_add(1);
        }
        let line_count = text.matches('\n').count();
        let last_line_column = text
            .rsplit('\n')
            .next()
            .map_or(0, |line| line.chars().count());
        session.position = if follow_tail {
            EditorPosition {
                line: line_count,
                column: 0,
            }
        } else {
            EditorPosition {
                line: session.position.line.min(line_count),
                column: session.position.column.min(last_line_column),
            }
        };
        session.pending_tail_scroll = follow_tail;
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
        if self.prompt_active(id) {
            return self.press_prompt(id, key);
        }
        if self.prompt.is_some() {
            return Ok(());
        }
        if matches!(key, EditorKey::Escape | EditorKey::Control('c'))
            && self.sessions.get(&id).is_some_and(|session| {
                matches!(
                    session.pending_binding,
                    Some(PendingBinding::Leader | PendingBinding::LeaderTransaction)
                )
            })
        {
            self.cancel_leader_bindings();
            return Ok(());
        }
        if !matches!(key, EditorKey::Character(_)) {
            self.cancel_leader_bindings();
        }
        if read_only
            && matches!(
                key,
                EditorKey::Character('i' | 'a' | 'o' | 'O' | 'R' | 'Q')
                    | EditorKey::Undo
                    | EditorKey::Redo
                    | EditorKey::Control('r')
            )
        {
            return Ok(());
        }
        let mode = self.mode(id)?;
        if key == EditorKey::Control('w')
            && self.sessions.get(&id).is_some_and(|session| {
                matches!(session.pending_binding, Some(PendingBinding::Window(_)))
            })
        {
            self.sessions
                .get_mut(&id)
                .ok_or(EditorError::MissingSession(id))?
                .pending_binding = None;
            self.effects.push(EditorEffect::FocusNext);
            return Ok(());
        }
        if mode == EditorMode::Normal {
            if self
                .sessions
                .get(&id)
                .is_some_and(|session| session.pending_vim_character)
            {
                self.sessions
                    .get_mut(&id)
                    .ok_or(EditorError::MissingSession(id))?
                    .pending_vim_character = false;
                return self.input_vim_key(id, key);
            }
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
        if mode == EditorMode::Normal
            && let EditorKey::Character(character) = key
            && self
                .sessions
                .get(&id)
                .is_some_and(|session| session.pending_binding == Some(PendingBinding::Goto))
        {
            self.sessions
                .get_mut(&id)
                .ok_or(EditorError::MissingSession(id))?
                .pending_binding = None;
            match character {
                't' => self.effects.push(EditorEffect::NextTab),
                'T' => self.effects.push(EditorEffect::PreviousTab),
                _ => {
                    self.input_vim_key(id, EditorKey::Character('g'))?;
                    self.input_vim_key(id, key)?;
                }
            }
            return Ok(());
        }
        match (mode, key) {
            (
                EditorMode::Normal
                | EditorMode::VisualChar
                | EditorMode::VisualLine
                | EditorMode::VisualBlock,
                EditorKey::Control('b' | 'd' | 'f' | 'u'),
            ) => {
                let viewport = self.viewport(id)?;
                let amount = match key {
                    EditorKey::Control('d' | 'u') => (viewport.height / 2).max(1),
                    _ => viewport.height.saturating_sub(2).max(1),
                } as isize;
                let rows = match key {
                    EditorKey::Control('b' | 'u') => -amount,
                    _ => amount,
                };
                self.page_cursor(id, rows)
            }
            (
                EditorMode::Normal
                | EditorMode::VisualChar
                | EditorMode::VisualLine
                | EditorMode::VisualBlock,
                EditorKey::PageUp | EditorKey::PageDown,
            ) => {
                let viewport = self.viewport(id)?;
                let amount = viewport.height.saturating_sub(2).max(1) as isize;
                let rows = if key == EditorKey::PageUp {
                    -amount
                } else {
                    amount
                };
                self.page_cursor(id, rows)
            }
            (EditorMode::Insert | EditorMode::Replace, EditorKey::Undo) => {
                self.undo_preserving_mode(id, mode)
            }
            (EditorMode::Insert | EditorMode::Replace, EditorKey::Redo) => {
                self.redo_preserving_mode(id, mode)
            }
            (EditorMode::Normal, EditorKey::Character('Q')) => {
                self.effects.push(EditorEffect::Quit);
                Ok(())
            }
            (
                EditorMode::Normal,
                EditorKey::Character(character @ ('f' | 'F' | 'r' | 't' | 'T')),
            ) if self
                .sessions
                .get(&id)
                .is_none_or(|session| session.pending_binding.is_none()) =>
            {
                self.input_vim_key(id, EditorKey::Character(character))?;
                if self.mode(id)? == EditorMode::Normal {
                    self.sessions
                        .get_mut(&id)
                        .ok_or(EditorError::MissingSession(id))?
                        .pending_vim_character = true;
                }
                Ok(())
            }
            (EditorMode::Normal, EditorKey::Control('s'))
            | (EditorMode::Insert | EditorMode::Replace, EditorKey::Control('s'))
            | (
                EditorMode::VisualChar | EditorMode::VisualLine | EditorMode::VisualBlock,
                EditorKey::Control('s'),
            ) => {
                let revision = self
                    .sessions
                    .get(&id)
                    .map(|session| session.revision)
                    .unwrap_or_default();
                self.effects.push(EditorEffect::SaveRequested {
                    console_id: id,
                    revision,
                });
                Ok(())
            }
            (EditorMode::Normal, EditorKey::Character('?')) => {
                self.start_prompt(id, EditorPromptKind::SearchBackward)
            }
            (EditorMode::Normal, EditorKey::Character('/')) => {
                self.start_prompt(id, EditorPromptKind::SearchForward)
            }
            (EditorMode::Normal, EditorKey::Character(':')) => {
                self.start_prompt(id, EditorPromptKind::Command)
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
            (
                EditorMode::Normal
                | EditorMode::VisualChar
                | EditorMode::VisualLine
                | EditorMode::VisualBlock,
                EditorKey::Character(' ' | '\\'),
            ) => {
                self.sessions
                    .get_mut(&id)
                    .ok_or(EditorError::MissingSession(id))?
                    .pending_binding = Some(PendingBinding::Leader);
                Ok(())
            }
            (
                EditorMode::Normal
                | EditorMode::VisualChar
                | EditorMode::VisualLine
                | EditorMode::VisualBlock,
                EditorKey::Control('w'),
            ) => {
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
        self.commit_transaction(id)?;
        let current = self.snapshot(id)?;
        let previous = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?
            .history
            .undo
            .pop();
        let Some(previous) = previous else {
            return Ok(());
        };
        self.sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?
            .history
            .push_redo(current);
        self.restore_snapshot(id, previous)?;
        self.record_changed(id)
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
        self.commit_transaction(id)?;
        let current = self.snapshot(id)?;
        let next = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?
            .history
            .redo
            .pop();
        let Some(next) = next else { return Ok(()) };
        self.sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?
            .history
            .push_undo_without_clearing_redo(current);
        self.restore_snapshot(id, next)?;
        self.record_changed(id)
    }

    fn undo_preserving_mode(&mut self, id: Uuid, mode: EditorMode) -> Result<(), EditorError> {
        self.commit_transaction(id)?;
        let current = self.snapshot(id)?;
        let previous = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?
            .history
            .undo
            .pop();
        let Some(previous) = previous else {
            return Ok(());
        };
        self.sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?
            .history
            .push_redo(current);
        self.restore_snapshot_with_mode(id, previous, mode)?;
        self.record_changed(id)
    }

    fn redo_preserving_mode(&mut self, id: Uuid, mode: EditorMode) -> Result<(), EditorError> {
        self.commit_transaction(id)?;
        let current = self.snapshot(id)?;
        let next = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?
            .history
            .redo
            .pop();
        let Some(next) = next else {
            return Ok(());
        };
        self.sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?
            .history
            .push_undo_without_clearing_redo(current);
        self.restore_snapshot_with_mode(id, next, mode)?;
        self.record_changed(id)
    }

    pub(crate) fn set_mode(&mut self, id: Uuid, mode: EditorMode) -> Result<(), EditorError> {
        let old_mode = self.mode(id)?;
        let before = self.snapshot(id)?;
        {
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
            session.mode = mode;
        }
        if matches!(mode, EditorMode::Insert | EditorMode::Replace)
            && !matches!(old_mode, EditorMode::Insert | EditorMode::Replace)
        {
            self.sessions
                .get_mut(&id)
                .ok_or(EditorError::MissingSession(id))?
                .history
                .transaction_start = Some(before);
        } else if !matches!(mode, EditorMode::Insert | EditorMode::Replace)
            && matches!(old_mode, EditorMode::Insert | EditorMode::Replace)
        {
            self.commit_transaction(id)?;
        }
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
        let mode_before = self.mode(id)?;
        let before = self.snapshot(id)?;
        let offset = char_position_to_byte(&before.text, before.position);
        let mut next = before.text.clone();
        let new_offset = edit(&mut next, offset);
        if next == before.text {
            return Ok(());
        }
        let position = byte_to_char_position(&next, new_offset);
        self.write_text(id, &next, position)?;
        let after = self.snapshot(id)?;
        self.record_edit_history(id, before, &after, mode_before, mode_before)?;
        self.record_changed(id)
    }

    fn input_vim_key(&mut self, id: Uuid, key: EditorKey) -> Result<(), EditorError> {
        let mode_before = self.mode(id)?;
        let read_only = self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?
            .capability
            == EditorSessionCapability::ReadOnly;
        let before = (!read_only).then(|| self.snapshot(id)).transpose()?;
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
                (PendingBinding::Leader, 'Y') if mode_before == EditorMode::Normal => {
                    self.effects.push(EditorEffect::CopyBuffer)
                }
                (PendingBinding::Leader, 'f') => self.effects.push(EditorEffect::FormatCurrent),
                (PendingBinding::Leader, 's' | 'q' | 'x' | '?' | 'd' | 't')
                    if mode_before != EditorMode::Normal => {}
                (PendingBinding::Leader, 's') => self.effects.push(EditorEffect::OpenSqlEditorList),
                (PendingBinding::Leader, 'm') if mode_before == EditorMode::Normal => {
                    self.effects.push(EditorEffect::OpenNotificationHistory)
                }
                (PendingBinding::Leader, 'q') => self.effects.push(EditorEffect::CloseConsole),
                (PendingBinding::Leader, 'x') => self.effects.push(EditorEffect::DeleteConsole),
                (PendingBinding::Leader, '?') => self.effects.push(EditorEffect::ShowHelp),
                (PendingBinding::Leader, 't') => {
                    session.pending_binding = Some(PendingBinding::LeaderTransaction)
                }
                (PendingBinding::Leader, 'd') => {
                    self.effects.push(EditorEffect::OpenTargetSelector)
                }
                (PendingBinding::Leader, 'D') => {
                    self.effects.push(EditorEffect::OpenDatabaseSelector)
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
                (PendingBinding::Window(_), 'f') => {
                    self.effects.push(EditorEffect::TogglePaneMaximized)
                }
                (PendingBinding::Window(_), 'k' | 'l') => {}
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
        let visual_indent = matches!(
            mode_before,
            EditorMode::VisualChar | EditorMode::VisualLine | EditorMode::VisualBlock
        ) && matches!(key, EditorKey::Character('<' | '>'));
        if visual_indent {
            self.indent_target(
                id,
                &modalkit::prelude::EditTarget::Selection,
                matches!(key, EditorKey::Character('>')),
                1,
            )?;
        } else {
            let terminal_key = key.to_terminal_key()?;
            session.keys.input_key(terminal_key);
            let mut actions = Vec::new();
            while let Some((action, context)) = session.keys.pop() {
                actions.push((action, context));
            }
            for (action, context) in actions {
                self.apply_action(id, action, context)?;
            }
        }
        self.sync_session_from_buffer(id)?;
        self.sync_registers();
        if matches!(key, EditorKey::Character('y')) {
            let copied = self.register('"').unwrap_or_default().to_owned();
            if !copied.is_empty() && unnamed_before.as_deref() != Some(copied.as_str()) {
                self.effects.push(EditorEffect::Yanked(copied));
            }
        }
        let after = before.as_ref().map(|_| self.snapshot(id)).transpose()?;
        if let (Some(before), Some(after)) = (before.as_ref(), after.as_ref())
            && before.text != after.text
        {
            {
                let session = self
                    .sessions
                    .get_mut(&id)
                    .ok_or(EditorError::MissingSession(id))?;
                session.last_sequence = Some(std::mem::take(&mut session.current_sequence));
            }
            let mode_after = self.mode(id)?;
            self.record_edit_history(id, before.clone(), after, mode_before, mode_after)?;
            self.record_changed(id)?;
        } else {
            let mode_after = self.mode(id)?;
            if !matches!(mode_before, EditorMode::Insert | EditorMode::Replace)
                && matches!(mode_after, EditorMode::Insert | EditorMode::Replace)
            {
                let session = self
                    .sessions
                    .get_mut(&id)
                    .ok_or(EditorError::MissingSession(id))?;
                if session.history.transaction_start.is_none()
                    && let Some(before) = before.as_ref()
                {
                    session.history.transaction_start = Some(before.clone());
                }
            } else if matches!(mode_before, EditorMode::Insert | EditorMode::Replace)
                && !matches!(mode_after, EditorMode::Insert | EditorMode::Replace)
            {
                self.commit_transaction(id)?;
            }
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
        use modalkit::editing::context::Resolve;
        use modalkit::prelude::{
            Axis, MoveDir1D, MoveDir2D, PositionList, ScrollSize, ScrollStyle,
        };
        match action {
            Action::Jump(PositionList::JumpList, direction, count) => {
                let count = context.resolve(&count);
                let current = self.position(id)?;
                let target = {
                    let session = self
                        .sessions
                        .get_mut(&id)
                        .ok_or(EditorError::MissingSession(id))?;
                    match direction {
                        MoveDir1D::Previous => session.jump_history.backward(current, count),
                        MoveDir1D::Next => session.jump_history.forward(count),
                    }
                };
                if let Some(target) = target {
                    let target = self.mouse_position(id, target)?;
                    self.set_keyboard_cursor(id, target)?;
                    self.ensure_cursor_visible_at(id, target, false)?;
                }
            }
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
                if let modalkit::actions::EditorAction::Edit(operation, target) = &editor_action
                    && let modalkit::actions::EditAction::Indent(change) =
                        context.resolve(operation)
                    && let Some(increase) = match change {
                        modalkit::prelude::IndentChange::Increase(_) => Some(true),
                        modalkit::prelude::IndentChange::Decrease(_) => Some(false),
                        modalkit::prelude::IndentChange::Auto => None,
                    }
                {
                    let count = context.get_count().unwrap_or(1).max(1);
                    self.indent_target(id, target, increase, count)?;
                    return Ok(());
                }
                let record_jump = matches!(
                    &editor_action,
                    modalkit::actions::EditorAction::Edit(operation, target)
                        if matches!(target, modalkit::prelude::EditTarget::Motion(..))
                            && target.is_jumping()
                            && matches!(
                                context.resolve(operation),
                                modalkit::actions::EditAction::Motion
                            )
                            && mode_from_key_manager(&session.keys) == EditorMode::Normal
                );
                let before_position = if record_jump {
                    Some(self.position(id)?)
                } else {
                    None
                };
                let context = if matches!(
                    &editor_action,
                    modalkit::actions::EditorAction::Edit(operation, target)
                        if matches!(target, modalkit::prelude::EditTarget::Motion(..))
                            && matches!(
                                context.resolve(operation),
                                modalkit::actions::EditAction::Motion
                            )
                            && mode_from_key_manager(&session.keys) == EditorMode::Normal
                ) {
                    modalkit::editing::context::EditContextBuilder::from(context)
                        .target_shape(None)
                        .build()
                } else {
                    context
                };
                let mut buffer = session
                    .buffer
                    .write()
                    .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
                let ctx = (session.group_id, &session.viewport, &context);
                buffer
                    .editor_command(&editor_action, &ctx, &mut self.store)
                    .map_err(|error| EditorError::Operation(error.to_string()))?;
                drop(buffer);
                if let Some(before_position) = before_position {
                    let after_position = self.position(id)?;
                    if after_position != before_position {
                        self.sessions
                            .get_mut(&id)
                            .ok_or(EditorError::MissingSession(id))?
                            .jump_history
                            .record_jump(before_position);
                    }
                }
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
            Action::Scroll(style) => match style {
                ScrollStyle::Direction2D(direction, size, _) => {
                    let viewport = self.viewport(id)?;
                    let amount = match size {
                        ScrollSize::Cell => 1,
                        ScrollSize::HalfPage => (viewport.height / 2).max(1),
                        ScrollSize::Page => viewport.height.saturating_sub(2).max(1),
                    } as isize;
                    let (rows, columns) = match direction {
                        MoveDir2D::Up => (-amount, 0),
                        MoveDir2D::Down => (amount, 0),
                        MoveDir2D::Left => (0, -amount),
                        MoveDir2D::Right => (0, amount),
                    };
                    self.scroll(id, rows, columns)?;
                }
                ScrollStyle::CursorPos(_, Axis::Vertical) | ScrollStyle::LinePos(_, _) => {
                    let position = self.position(id)?;
                    self.ensure_cursor_visible_at(id, position, false)?;
                }
                ScrollStyle::CursorPos(_, Axis::Horizontal) => {}
            },
            Action::Repeat(repeat) => self.effects.push(EditorEffect::Message(format!(
                "repeat action deferred: {repeat:?}"
            ))),
            other => self.effects.push(EditorEffect::Message(format!(
                "editor action deferred: {other:?}"
            ))),
        }
        Ok(())
    }

    fn indent_target(
        &mut self,
        id: Uuid,
        target: &modalkit::prelude::EditTarget,
        increase: bool,
        count: usize,
    ) -> Result<(), EditorError> {
        const WIDTH: usize = 4;
        let before = self.snapshot(id)?;
        let session = self
            .sessions
            .get(&id)
            .ok_or(EditorError::MissingSession(id))?;
        let mode = mode_from_key_manager(&session.keys);
        let mut buffer = session
            .buffer
            .write()
            .map_err(|_| EditorError::Operation("buffer lock poisoned".into()))?;
        let selection = buffer.get_leader_selection(session.group_id);
        drop(buffer);
        let (first_line, last_line) =
            if matches!(
                mode,
                EditorMode::VisualChar | EditorMode::VisualLine | EditorMode::VisualBlock
            ) || matches!(target, modalkit::prelude::EditTarget::Selection)
            {
                let Some((start, end, _)) = selection else {
                    return Ok(());
                };
                (
                    start.get_y().min(end.get_y()),
                    start.get_y().max(end.get_y()),
                )
            } else {
                let last_line = before.position.line.saturating_add(count.saturating_sub(1));
                (
                    before.position.line,
                    last_line.min(before.text.matches('\n').count()),
                )
            };
        let direction = if increase {
            indent::IndentDirection::Increase
        } else {
            indent::IndentDirection::Decrease
        };
        let (next, _) = indent::indent_lines(&before.text, first_line, last_line, direction, WIDTH);
        if next == before.text {
            return Ok(());
        }
        let start = line_bounds(&before.text, first_line).0;
        let end = line_bounds(&before.text, last_line).1;
        let next_end = line_bounds(&next, last_line).1;
        self.replace_range_preserving_selection(
            id,
            TextRange::new(start, end),
            &next[start..next_end],
        )
    }

    fn sync_session_from_buffer(&mut self, id: Uuid) -> Result<(), EditorError> {
        let position = {
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
            session.position
        };
        self.ensure_cursor_visible_at(id, position, false)?;
        Ok(())
    }

    fn ensure_cursor_visible_at(
        &mut self,
        id: Uuid,
        position: EditorPosition,
        center: bool,
    ) -> Result<(), EditorError> {
        if let Some((width, height, total, offset, _cursor_row)) =
            self.sessions.get(&id).and_then(|session| {
                let document = session.preview_document.borrow();
                let index = session.preview_wrap_index.borrow();
                match (document.as_ref(), index.as_ref()) {
                    (Some((revision, document)), Some((index_revision, index)))
                        if *revision == session.revision && *index_revision == session.revision =>
                    {
                        let projection = document.projections.get(position.line)?;
                        let cell = projection
                            .source_to_display_cells
                            .get(position.column)
                            .copied()
                            .unwrap_or_else(|| {
                                projection
                                    .source_to_display_cells
                                    .last()
                                    .copied()
                                    .unwrap_or(0)
                            });
                        let cursor_row = index.cursor_row(position.line, cell);
                        let old = session.preview_layout.get();
                        let (width, height, total, offset) = old.unwrap_or((
                            index.width,
                            session.viewport.get_height(),
                            index.total_rows,
                            0,
                        ));
                        let next = if cursor_row < offset {
                            cursor_row
                        } else if cursor_row >= offset + height {
                            cursor_row.saturating_add(1).saturating_sub(height)
                        } else {
                            offset
                        };
                        Some((
                            width,
                            height,
                            total,
                            next.min(total.saturating_sub(height)),
                            cursor_row,
                        ))
                    }
                    _ => None,
                }
            })
        {
            if let Some(session) = self.sessions.get_mut(&id) {
                session
                    .preview_layout
                    .set(Some((width, height, total, offset)));
            }
            return Ok(());
        }
        let text = self.text(id)?;
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(EditorError::MissingSession(id))?;
        if let Some((width, height, total, offset)) = session.preview_layout.get() {
            let mut visual_row = 0;
            for (index, line) in text.split('\n').enumerate() {
                let projection = project_editor_line(line);
                let cells = projection
                    .source_to_display_cells
                    .last()
                    .copied()
                    .unwrap_or(0);
                let cursor_cell = projection
                    .source_to_display_cells
                    .get(position.column)
                    .copied()
                    .unwrap_or(cells);
                let mut start = 0;
                loop {
                    if index == position.line
                        && (cursor_cell < start + width || cells <= start + width)
                    {
                        let next = if visual_row < offset {
                            visual_row
                        } else if visual_row >= offset + height {
                            visual_row.saturating_add(1).saturating_sub(height)
                        } else {
                            offset
                        };
                        session.preview_layout.set(Some((
                            width,
                            height,
                            total,
                            next.min(total.saturating_sub(height)),
                        )));
                        return Ok(());
                    }
                    visual_row += 1;
                    if cells <= start + width {
                        break;
                    }
                    start = projection
                        .source_to_display_cells
                        .iter()
                        .copied()
                        .filter(|cell| *cell > start && *cell <= start + width)
                        .max()
                        .unwrap_or(start + width);
                }
            }
            return Ok(());
        }
        let height = session.viewport.get_height();
        let width = session.viewport.get_width();
        if height == 0 || width == 0 {
            return Ok(());
        }
        let line_count = text.split('\n').count().max(1);
        let max_row = line_count.saturating_sub(height);
        let row = position.line.min(line_count.saturating_sub(1));
        let scrolloff = 2.min(height.saturating_sub(1) / 2);
        let row_offset = if center {
            row.saturating_sub(height.saturating_sub(1) / 2)
        } else if row < session.viewport.corner.get_y().saturating_add(scrolloff) {
            row.saturating_sub(scrolloff)
        } else if row.saturating_add(scrolloff)
            >= session.viewport.corner.get_y().saturating_add(height)
        {
            row.saturating_add(scrolloff + 1).saturating_sub(height)
        } else {
            session.viewport.corner.get_y()
        };
        let row_offset = row_offset.min(max_row);
        let line = text.split('\n').nth(row).unwrap_or_default();
        let cell = project_editor_line(line)
            .source_to_display_cells
            .get(position.column)
            .copied()
            .unwrap_or_else(|| {
                project_editor_line(line)
                    .source_to_display_cells
                    .last()
                    .copied()
                    .unwrap_or(0)
            });
        let column_offset = if center {
            cell.saturating_sub(width.saturating_sub(1) / 2)
        } else if cell < session.viewport.corner.get_x() {
            cell
        } else if cell >= session.viewport.corner.get_x().saturating_add(width) {
            cell.saturating_add(1).saturating_sub(width)
        } else {
            session.viewport.corner.get_x()
        };
        let max_column = full_line_width(&text).saturating_sub(width);
        session.viewport.corner.set_y(row_offset);
        session.viewport.corner.set_x(column_offset.min(max_column));
        Ok(())
    }

    fn page_cursor(&mut self, id: Uuid, rows: isize) -> Result<(), EditorError> {
        let text = self.text(id)?;
        let position = self.position(id)?;
        let line_count = text.split('\n').count().max(1);
        let viewport_height = self.viewport(id)?.height.max(1);
        let target_line = position
            .line
            .saturating_add_signed(rows)
            .min(line_count.saturating_sub(1));
        let target_column = text
            .split('\n')
            .nth(target_line)
            .map(|line| position.column.min(line.chars().count()))
            .unwrap_or(0);
        self.set_keyboard_cursor(
            id,
            EditorPosition {
                line: target_line,
                column: target_column,
            },
        )?;
        let center = rows.unsigned_abs() >= viewport_height.saturating_sub(2).max(1);
        self.ensure_cursor_visible_at(id, self.position(id)?, center)
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

    fn start_prompt(&mut self, id: Uuid, kind: EditorPromptKind) -> Result<(), EditorError> {
        if !self.sessions.contains_key(&id) {
            return Err(EditorError::MissingSession(id));
        }
        self.prompt = Some(PromptSession::new(id, kind));
        Ok(())
    }

    fn press_prompt(&mut self, id: Uuid, key: EditorKey) -> Result<(), EditorError> {
        match key {
            EditorKey::Escape | EditorKey::Control('c') => {
                self.prompt = None;
            }
            EditorKey::Enter => self.submit_prompt(id)?,
            EditorKey::Undo => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.input.undo();
                    prompt.error = None;
                }
            }
            EditorKey::Redo => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.input.redo();
                    prompt.error = None;
                }
            }
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
                    prompt.input.clear();
                    prompt.error = None;
                }
            }
            EditorKey::Left => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.input.move_left();
                }
            }
            EditorKey::Right => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.input.move_right();
                }
            }
            EditorKey::Home => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.input.move_home();
                }
            }
            EditorKey::End => {
                if let Some(prompt) = self.prompt.as_mut() {
                    prompt.input.move_end();
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
        prompt.input.set(self.command_history[next].clone());
    }

    fn submit_prompt(&mut self, id: Uuid) -> Result<(), EditorError> {
        let Some(prompt) = self.prompt.take() else {
            return Ok(());
        };
        let raw = prompt.input.value().to_owned();
        match prompt.kind {
            EditorPromptKind::Command => {
                self.command_history.push(raw.clone());
                if raw.trim_start().chars().next().is_some_and(|character| {
                    matches!(character, '.' | '$' | '%' | 's' | '<' | '0'..='9')
                }) {
                    if self.sessions.get(&id).is_some_and(|session| {
                        session.capability == EditorSessionCapability::ReadOnly
                    }) {
                        self.prompt = Some(PromptSession {
                            owner: id,
                            kind: EditorPromptKind::Command,
                            input: raw.as_str().into(),
                            error: Some("cannot modify a read-only value".to_owned()),
                            history_index: None,
                        });
                        return Ok(());
                    }
                    match self.start_substitute(id, &raw) {
                        Ok(()) => {}
                        Err(error) => {
                            self.prompt = Some(PromptSession {
                                owner: id,
                                kind: EditorPromptKind::Command,
                                input: raw.as_str().into(),
                                error: Some(error.to_string()),
                                history_index: None,
                            })
                        }
                    }
                } else if let Some(effect) = parse_command(&raw) {
                    self.effects.push(effect);
                } else {
                    self.prompt = Some(PromptSession {
                        owner: id,
                        kind: EditorPromptKind::Command,
                        input: raw.as_str().into(),
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
                        owner: id,
                        kind,
                        input: raw.as_str().into(),
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
        let before_position = byte_to_char_position(&text, cursor);
        let found = if backward {
            text[..cursor]
                .rfind(pattern)
                .or_else(|| text.rfind(pattern))
        } else {
            let next = text[cursor..]
                .char_indices()
                .nth(1)
                .map_or(text.len(), |(offset, _)| cursor + offset);
            text[next..]
                .find(pattern)
                .map(|offset| offset + next)
                .or_else(|| text.find(pattern))
        };
        let Some(offset) = found else {
            return Ok(false);
        };
        let position = {
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
            session.position
        };
        if position != before_position {
            self.sessions
                .get_mut(&id)
                .ok_or(EditorError::MissingSession(id))?
                .jump_history
                .record_jump(before_position);
        }
        self.ensure_cursor_visible_at(id, position, true)?;
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
        let before = self.snapshot(id)?;
        self.write_text(id, &next, EditorPosition { line: 0, column: 0 })?;
        let after = self.snapshot(id)?;
        self.record_edit_history(id, before, &after, EditorMode::Normal, EditorMode::Normal)?;
        self.record_changed(id)
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
        sql::HighlightKind::Relation => EditorHighlightKind::Relation,
        sql::HighlightKind::RelationAlias => EditorHighlightKind::RelationAlias,
        sql::HighlightKind::Column => EditorHighlightKind::Column,
        sql::HighlightKind::Type => EditorHighlightKind::Type,
        sql::HighlightKind::Function => EditorHighlightKind::Function,
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
    start: usize,
    end: usize,
    kind: EditorHighlightKind,
    statement: Option<sql::TextRange>,
    line_start: usize,
    projection: &crate::security::DisplayLineProjection,
) -> EditorRenderSpan {
    let start_boundary = projection
        .source_byte_boundaries
        .binary_search(&start)
        .unwrap_or_else(|index| index.min(projection.source_to_display_bytes.len() - 1));
    let end_boundary = projection
        .source_byte_boundaries
        .binary_search(&end)
        .unwrap_or_else(|index| index.min(projection.source_to_display_bytes.len() - 1));
    let display_start = projection.source_to_display_bytes[start_boundary];
    let display_end = projection.source_to_display_bytes[end_boundary];
    let text = projection
        .text
        .get(display_start..display_end)
        .unwrap_or_default()
        .to_owned();
    EditorRenderSpan {
        text,
        source_start: line_start + start,
        source_end: line_start + end,
        kind,
        current_statement: statement
            .is_some_and(|range| range.start < line_start + end && range.end > line_start + start),
    }
}

fn statement_background_cells(
    source: &str,
    line_start: usize,
    line_end: usize,
    current: Option<sql::TextRange>,
    statements: &[sql::TextRange],
    source_to_display_cells: &[usize],
) -> Option<(usize, usize)> {
    let current = current?;
    let current_start = current.start.max(line_start);
    let current_end = current.end.min(line_end);
    if current_start >= current_end {
        return None;
    }
    let has_sibling = statements
        .iter()
        .any(|range| *range != current && range.start < line_end && range.end > line_start);
    if !has_sibling {
        return None;
    }

    let start = source[..current_start.saturating_sub(line_start)]
        .chars()
        .count();
    let end = source[..current_end.saturating_sub(line_start)]
        .chars()
        .count();
    let start = *source_to_display_cells.get(start)?;
    let end = *source_to_display_cells.get(end)?;
    (end > start).then_some((start, end))
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

fn inclusive_position_end(text: &str, position: usize) -> usize {
    position
        .checked_add(
            text.get(position..)
                .and_then(|tail| tail.chars().next())
                .map_or(0, char::len_utf8),
        )
        .unwrap_or(text.len())
        .min(text.len())
}

impl EditorKey {
    fn to_terminal_key(self) -> Result<modalkit::key::TerminalKey, EditorError> {
        let text = match self {
            Self::Character(c) => c.to_string(),
            Self::Undo | Self::Redo => {
                return Err(EditorError::Operation(
                    "history keys are handled before Vim key conversion".into(),
                ));
            }
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
            Self::PageUp => "<PageUp>".into(),
            Self::PageDown => "<PageDown>".into(),
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

#[cfg(test)]
mod preview_perf_tests;
