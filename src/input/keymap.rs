use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use uuid::Uuid;

use crate::{
    action::Action,
    app::App,
    input::{is_text_redo, is_text_undo},
    model::{
        editor::EditorMode,
        explorer::ExplorerNodeId,
        profile_manager::{ProfileField, ProfileInput, ProfileManagerPage},
        text_input::TextInputEdit,
        workspace::{Focus, Overlay},
    },
};

#[derive(Clone, Debug, Eq, PartialEq)]
enum Pending {
    Leader,
    EditorLeader,
    Window { count: u32 },
    PaneSequence { events: Vec<KeyEvent>, count: u32 },
    WindowCount { count: u32 },
    Previous,
    Next,
    RelationDelete,
    GridAlign,
    RecordViewGoto,
    ExplorerAlign,
    Goto,
    LeaderTransaction,
    RedisPreviewLeader,
    CatalogColumnDelete { selected_column: usize },
}

impl Pending {
    fn is_leader(&self) -> bool {
        matches!(
            self,
            Self::Leader | Self::EditorLeader | Self::LeaderTransaction | Self::RedisPreviewLeader
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct KeymapContext {
    focus: Focus,
    editor_mode: EditorMode,
    tab_id: Uuid,
    overlay: Option<std::mem::Discriminant<Overlay>>,
    omni: bool,
}

impl KeymapContext {
    fn from_app(app: &App) -> Self {
        Self {
            focus: app.focus,
            editor_mode: app.active_editor_mode(),
            tab_id: app
                .tabs
                .get(app.active_tab)
                .map_or(Uuid::nil(), |tab| tab.id()),
            overlay: app.overlay.as_ref().map(std::mem::discriminant),
            omni: app.omni.is_some(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeySequenceState {
    pub prefix: crate::help::ShortcutPrefix,
    pub display: String,
    pub selected: usize,
    pub candidates: Vec<(String, String)>,
}

#[derive(Debug)]
struct PendingState {
    pending: Pending,
    focus: Focus,
    editor_mode: EditorMode,
    tab_id: Uuid,
    generation: u64,
    record_view_active: bool,
    started_at: Instant,
}

#[derive(Debug)]
pub struct Keymap {
    pending: Option<PendingState>,
    observed: Option<KeymapContext>,
    generation: u64,
    sequence_selected: usize,
    sequence_timeout: Duration,
    sequence_width: u16,
    bindings: crate::config::KeyBindings,
}

/// Map a shortcut advertised by an interactive UI hint through the same
/// context-sensitive keymap used for terminal keyboard input.
pub(crate) fn map_shortcut(sequence: &[KeyEvent], app: &App) -> Option<Action> {
    if sequence.is_empty() {
        return None;
    }
    let mut keymap = Keymap::with_sequence_timeout_and_bindings(
        std::time::Duration::from_millis(
            crate::config::AppConfig::default()
                .keybindings
                .sequence_timeout_ms,
        ),
        app.key_bindings.clone(),
    );
    let mut action = None;
    for (index, event) in sequence.iter().copied().enumerate() {
        let mapped = keymap.map(event, app);
        if index + 1 < sequence.len() {
            if mapped.is_some() {
                return None;
            }
        } else {
            action = mapped;
        }
    }
    action
}

impl Default for Keymap {
    fn default() -> Self {
        Self::with_sequence_timeout(Duration::from_millis(
            crate::config::AppConfig::default()
                .keybindings
                .sequence_timeout_ms,
        ))
    }
}

impl Keymap {
    pub fn with_sequence_timeout(sequence_timeout: Duration) -> Self {
        Self::with_sequence_timeout_and_bindings(
            sequence_timeout,
            crate::config::AppConfig::default()
                .keybindings
                .key_bindings()
                .unwrap(),
        )
    }

    pub fn with_sequence_timeout_and_bindings(
        sequence_timeout: Duration,
        bindings: crate::config::KeyBindings,
    ) -> Self {
        Self {
            pending: None,
            observed: None,
            generation: 0,
            sequence_selected: 0,
            sequence_timeout,
            sequence_width: 0,
            bindings,
        }
    }

    pub fn map(&mut self, event: KeyEvent, app: &App) -> Option<Action> {
        self.observe_state(app);
        let event = normalize_shift_tab(event);
        if matches!(event.kind, KeyEventKind::Release) {
            return None;
        }
        if matches!(
            app.tabs.get(app.active_tab),
            Some(crate::model::tab::WorkspaceTab::MongoConsole(_))
        ) && event.modifiers.contains(KeyModifiers::ALT)
        {
            self.clear_pending();
            return match event.code {
                KeyCode::Char('b') => Some(Action::MongoTransactionBegin),
                KeyCode::Char('c') => Some(Action::MongoTransactionCommit),
                KeyCode::Char('a') => Some(Action::MongoTransactionAbort),
                _ => None,
            };
        }
        if matches!(
            app.tabs.get(app.active_tab),
            Some(crate::model::tab::WorkspaceTab::MongoDashboard(_))
        ) && event.modifiers.is_empty()
        {
            match event.code {
                KeyCode::Char('r') => return Some(Action::MongoMonitorRefresh),
                KeyCode::Char('u') => return Some(Action::MongoPrincipalsRefresh),
                KeyCode::Char('n') => return Some(Action::MongoPrincipalNewUser),
                KeyCode::Char('N') => return Some(Action::MongoPrincipalNewRole),
                _ => {}
            }
        }
        if let Some(crate::model::tab::WorkspaceTab::MongoConsole(_)) = app.tabs.get(app.active_tab)
            && event.modifiers.contains(KeyModifiers::CONTROL)
            && event.code == KeyCode::Enter
        {
            return Some(Action::MongoConsoleRun);
        }
        if app.focus == Focus::Results
            && let Some(crate::model::tab::WorkspaceTab::MongoBrowser(tab)) =
                app.tabs.get(app.active_tab)
        {
            if tab.editing {
                self.clear_pending();
                return match event.code {
                    KeyCode::Esc => Some(Action::MongoDocumentCancelEdit),
                    KeyCode::Enter if event.modifiers.contains(KeyModifiers::CONTROL) => {
                        Some(Action::MongoDocumentSave)
                    }
                    KeyCode::Char('s') if event.modifiers == KeyModifiers::CONTROL => None,
                    KeyCode::Backspace => Some(Action::MongoDocumentEditText(
                        crate::model::text_input::TextInputEdit::Backspace,
                    )),
                    KeyCode::Char(ch) if event.modifiers.is_empty() => {
                        Some(Action::MongoDocumentEditText(
                            crate::model::text_input::TextInputEdit::Insert(ch),
                        ))
                    }
                    _ => None,
                };
            }
            if tab.query_field.is_some() {
                return match event.code {
                    KeyCode::Esc => Some(Action::MongoQueryCancel),
                    KeyCode::Enter => Some(Action::MongoQuerySubmit),
                    KeyCode::Left => Some(Action::MongoQueryEdit(
                        crate::model::text_input::TextInputEdit::MoveLeft,
                    )),
                    KeyCode::Right => Some(Action::MongoQueryEdit(
                        crate::model::text_input::TextInputEdit::MoveRight,
                    )),
                    KeyCode::Home => Some(Action::MongoQueryEdit(
                        crate::model::text_input::TextInputEdit::MoveHome,
                    )),
                    KeyCode::End => Some(Action::MongoQueryEdit(
                        crate::model::text_input::TextInputEdit::MoveEnd,
                    )),
                    KeyCode::Backspace => Some(Action::MongoQueryEdit(
                        crate::model::text_input::TextInputEdit::Backspace,
                    )),
                    KeyCode::Delete => Some(Action::MongoQueryEdit(
                        crate::model::text_input::TextInputEdit::Delete,
                    )),
                    KeyCode::Char(ch) if event.modifiers.is_empty() => Some(
                        Action::MongoQueryEdit(crate::model::text_input::TextInputEdit::Insert(ch)),
                    ),
                    _ => None,
                };
            }
            if event.modifiers.is_empty() {
                let action = match event.code {
                    KeyCode::Char('j') | KeyCode::Down => Some(Action::MongoDocumentNext),
                    KeyCode::Char('k') | KeyCode::Up => Some(Action::MongoDocumentPrevious),
                    KeyCode::Char('e') => Some(Action::MongoDocumentEdit),
                    KeyCode::Char('f') => Some(Action::MongoQueryField(
                        crate::model::mongodb_browser::MongoQueryField::Filter,
                    )),
                    KeyCode::Char('s') => Some(Action::MongoQueryField(
                        crate::model::mongodb_browser::MongoQueryField::Sort,
                    )),
                    KeyCode::Char('p') => Some(Action::MongoQueryField(
                        crate::model::mongodb_browser::MongoQueryField::Projection,
                    )),
                    KeyCode::Char('y') => Some(Action::MongoCopyDocument),
                    KeyCode::Char('E') => Some(Action::MongoExportEjson),
                    KeyCode::Char('i') if tab.import_draft.is_some() => {
                        Some(Action::MongoImportAccept)
                    }
                    KeyCode::Char('t') => Some(Action::MongoToggleInspector),
                    KeyCode::Char('d') => Some(Action::MongoDeleteSelected),
                    KeyCode::Char('l') => Some(Action::MongoPageNext),
                    KeyCode::Char('r')
                        if matches!(
                            tab.page,
                            crate::model::mongodb_browser::MongoPageState::Failed(_)
                        ) =>
                    {
                        Some(Action::MongoPageRetry)
                    }
                    _ => None,
                };
                if action.is_some() {
                    self.clear_pending();
                    return action;
                }
            }
        }
        if self.bindings.matches(
            "smart-toggle-pane-maximized",
            KeyEvent::new(event.code, event.modifiers),
        ) {
            self.clear_pending();
            return (event.kind == KeyEventKind::Press
                && app.overlay.is_none()
                && app.omni.is_none())
            .then_some(Action::SmartTogglePaneMaximized);
        }
        if app.omni.is_some() {
            self.pending = None;
            if event.kind == KeyEventKind::Repeat && self.bindings.matches("omni", event) {
                return None;
            }
            return map_omni(event, app);
        }
        if matches!(app.overlay, Some(Overlay::Help(_)))
            && event.modifiers == KeyModifiers::NONE
            && event.code == KeyCode::Tab
        {
            self.pending = None;
            return Some(Action::ToggleHelpPanel);
        }
        if self.bindings.matches("omni", event) {
            self.pending = None;
            return (event.kind != KeyEventKind::Repeat).then_some(Action::OpenOmni);
        }
        if self.bindings.matches("open-consoles", event) {
            self.pending = None;
            return (event.kind != KeyEventKind::Repeat).then_some(Action::OpenSqlEditorList);
        }
        if app.overlay == Some(Overlay::CatalogEditor)
            && app
                .catalog_editor
                .as_ref()
                .is_some_and(|editor| editor.is_busy())
        {
            return (event.code == KeyCode::Esc).then_some(Action::CatalogEditorCancel);
        }
        if let Some(Overlay::RedisObjectEditor(editor)) = app.overlay.as_ref() {
            return map_redis_object_editor(event, editor);
        }
        if let Some(Overlay::RedisTableEditor(_editor)) = app.overlay.as_ref() {
            if event.code == KeyCode::Esc {
                return Some(Action::RedisObjectEditorCancel);
            }
            if matches!(event.code, KeyCode::Tab | KeyCode::Down) {
                return Some(Action::RedisObjectEditorFocusNext);
            }
            if matches!(event.code, KeyCode::BackTab | KeyCode::Up) {
                return Some(Action::RedisObjectEditorFocusPrevious);
            }
            if event.code == KeyCode::Enter {
                return Some(Action::RedisObjectEditorApply);
            }
            return match event.code {
                KeyCode::Backspace => Some(Action::RedisObjectEditorBackspace),
                KeyCode::Delete => Some(Action::RedisObjectEditorDelete),
                KeyCode::Left => Some(Action::RedisObjectEditorMoveLeft),
                KeyCode::Right => Some(Action::RedisObjectEditorMoveRight),
                KeyCode::Home => Some(Action::RedisObjectEditorMoveHome),
                KeyCode::End => Some(Action::RedisObjectEditorMoveEnd),
                KeyCode::Char(c) if event.modifiers.is_empty() => {
                    Some(Action::RedisObjectEditorInsert(c))
                }
                _ => None,
            };
        }
        if let Some(Overlay::RedisTableDeleteConfirm(confirm)) = app.overlay.as_ref() {
            return match event.code {
                KeyCode::Esc => Some(Action::RedisTableDeleteCancel),
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                    Some(Action::RedisTableDeleteToggleFocus)
                }
                KeyCode::Enter
                    if confirm.focus
                        == crate::model::redis_table_editor::RedisTableDeleteFocus::Delete =>
                {
                    Some(Action::RedisTableDeleteConfirm)
                }
                KeyCode::Enter => Some(Action::RedisTableDeleteCancel),
                _ => None,
            };
        }
        if let Some(Overlay::RedisValueSaveConfirm { focus, .. }) = app.overlay.as_ref() {
            return match event.code {
                KeyCode::Esc => Some(Action::RedisValueSaveCancel),
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                    Some(Action::RedisValueSaveToggleFocus)
                }
                KeyCode::Enter => Some(Action::RedisValueSaveActivate(*focus)),
                _ => None,
            };
        }
        if let Some(Overlay::RedisUnsavedValueConfirm { focus, .. }) = app.overlay.as_ref() {
            return match event.code {
                KeyCode::Char('s') => Some(Action::RedisUnsavedValueSave),
                KeyCode::Char('d') => Some(Action::RedisUnsavedValueDiscard),
                KeyCode::Esc => Some(Action::RedisUnsavedValueCancel),
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                    Some(Action::RedisUnsavedValueToggleFocus)
                }
                KeyCode::Enter => Some(Action::RedisUnsavedValueActivate(*focus)),
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::Update(_))) {
            self.pending = None;
            return match event.code {
                KeyCode::Esc | KeyCode::Char('q') => Some(Action::DismissOverlay),
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                    Some(Action::UpdateOverlayToggleFocus)
                }
                KeyCode::Enter => Some(Action::UpdateOverlayConfirm),
                KeyCode::Char('r') => Some(Action::StartUpdateCheck { automatic: false }),
                _ => None,
            };
        }
        if app.active_redis_preview_prompt_active()
            && let Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab)) =
                app.tabs.get(app.active_tab)
        {
            return Some(if app.editor_is_editable(tab.preview_editor_id) {
                Action::EditorKey(event)
            } else {
                Action::ReadOnlyEditorKey {
                    session_id: tab.preview_editor_id,
                    event,
                }
            });
        }
        if matches!(app.overlay, Some(Overlay::SqlHistory(_))) {
            self.pending = None;
            if let Some(crate::model::workspace::Overlay::SqlHistory(view)) = app.overlay.as_ref()
                && view.mode == crate::model::sql_history_view::SqlHistoryMode::Sql
            {
                return match event.code {
                    KeyCode::Esc => Some(Action::SqlHistoryBackToBrowse),
                    KeyCode::Tab => Some(Action::SqlHistoryBackToBrowse),
                    _ => Some(Action::ReadOnlyEditorKey {
                        session_id: view.editor_session_id,
                        event,
                    }),
                };
            }
            if let Some(crate::model::workspace::Overlay::SqlHistory(view)) = app.overlay.as_ref()
                && view.mode == crate::model::sql_history_view::SqlHistoryMode::Search
            {
                return match event.code {
                    KeyCode::Esc => Some(Action::SqlHistorySearchCancel),
                    KeyCode::Enter => Some(Action::SqlHistorySearchConfirm),
                    KeyCode::Backspace => Some(Action::SqlHistorySearchBackspace),
                    _ => map_single_line_text_input_edit(event).map(Action::SqlHistorySearchEdit),
                };
            }
            return match event.code {
                KeyCode::Esc => Some(Action::DismissOverlay),
                KeyCode::Char('y') => Some(Action::SqlHistoryCopy),
                KeyCode::Enter => Some(Action::SqlHistoryOpenDetail),
                KeyCode::Char('f') => Some(Action::SqlHistoryCycleStatus),
                KeyCode::Char('t') => Some(Action::SqlHistoryCycleTransaction),
                KeyCode::Char('r') => Some(Action::SqlHistoryRefresh),
                KeyCode::PageDown => Some(Action::SqlHistoryLoadNext),
                KeyCode::Char('/') => Some(Action::SqlHistorySearchOpen),
                KeyCode::Char('j') | KeyCode::Down => Some(Action::SqlHistoryMove(1)),
                KeyCode::Char('k') | KeyCode::Up => Some(Action::SqlHistoryMove(-1)),
                KeyCode::Backspace => Some(Action::SqlHistorySearchClear),
                KeyCode::Char(character) if (event.modifiers & !KeyModifiers::SHIFT).is_empty() => {
                    Some(Action::SqlHistorySearchInsert(character))
                }
                _ => None,
            };
        }
        if self.pending.is_some()
            && (event.code == KeyCode::Esc
                || event.modifiers == KeyModifiers::CONTROL && event.code == KeyCode::Char('c'))
        {
            let editor_leader = self.pending.as_ref().is_some_and(|pending| {
                matches!(
                    pending.pending,
                    Pending::EditorLeader | Pending::LeaderTransaction
                )
            });
            self.clear_pending();
            return editor_leader.then_some(Action::EditorKey(event));
        }
        if self.bindings.matches("quit", event) {
            return Some(Action::Quit);
        }
        if self.bindings.matches("terminal-selection", event) {
            self.pending = None;
            return Some(Action::ToggleTerminalSelection);
        }
        if self.bindings.matches("update", event) && app.overlay.is_none() {
            self.pending = None;
            return Some(Action::OpenUpdateCenter);
        }
        if self.bindings.matches("notification-history", event) && app.overlay.is_none() {
            self.pending = None;
            return Some(Action::OpenNotificationHistory);
        }
        if self.bindings.matches("open-sql-history", event) && app.overlay.is_none() {
            self.pending = None;
            return Some(Action::OpenSqlHistory);
        }
        if app
            .overlay
            .as_ref()
            .is_some_and(|overlay| matches!(overlay, Overlay::Help(_)))
        {
            self.pending = None;
            if let Some(edit) = map_single_line_text_input_edit(event) {
                return Some(Action::HelpEdit(edit));
            }
            if event.modifiers == KeyModifiers::NONE {
                return match event.code {
                    KeyCode::Up => Some(Action::HelpMove(-1)),
                    KeyCode::Down => Some(Action::HelpMove(1)),
                    KeyCode::Enter => app.help_selected_id().map(Action::ExecuteHelpShortcut),
                    KeyCode::Esc => Some(Action::DismissOverlay),
                    _ => None,
                };
            }
            return None;
        }
        if matches!(
            app.overlay,
            Some(Overlay::WorkspaceSaveFailed {
                retryable: true,
                ..
            })
        ) {
            self.pending = None;
            return match event.code {
                KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                    Some(Action::WorkspaceSaveFocusNext)
                }
                KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                    Some(Action::WorkspaceSaveFocusPrevious)
                }
                KeyCode::Enter => Some(Action::WorkspaceSaveActivate),
                KeyCode::PageUp => Some(Action::WorkspaceSaveScroll(-1)),
                KeyCode::PageDown => Some(Action::WorkspaceSaveScroll(1)),
                KeyCode::Char('r') if event.modifiers == KeyModifiers::NONE => {
                    Some(Action::RetryWorkspaceQuitSave)
                }
                KeyCode::Char('d') => Some(Action::DiscardWorkspaceQuitSave),
                KeyCode::Esc => Some(Action::DismissOverlay),
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::WorkspaceSaveFailed { .. })) {
            self.pending = None;
            return match event.code {
                KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                    Some(Action::WorkspaceSaveFocusNext)
                }
                KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                    Some(Action::WorkspaceSaveFocusPrevious)
                }
                KeyCode::Enter => Some(Action::WorkspaceSaveActivate),
                KeyCode::PageUp => Some(Action::WorkspaceSaveScroll(-1)),
                KeyCode::PageDown => Some(Action::WorkspaceSaveScroll(1)),
                KeyCode::Char('d') => Some(Action::DiscardWorkspaceQuitSave),
                KeyCode::Esc => Some(Action::DismissOverlay),
                _ => None,
            };
        }
        if app.overlay == Some(Overlay::ProfileManager) {
            self.pending = None;
            return map_profile_manager(event, app);
        }
        if matches!(app.overlay, Some(Overlay::CatalogEditor)) {
            return map_catalog_editor_with_pending(self, event, app);
        }
        if is_relation_cell_editor(app) {
            self.pending = None;
            return map_relation_data(event, app);
        }
        if matches!(app.overlay, Some(Overlay::ProfileAccess { .. })) {
            self.pending = None;
            return match event.code {
                KeyCode::Enter => Some(Action::ProfileAccessConfirm),
                KeyCode::Esc | KeyCode::Char('q') => Some(Action::ProfileAccessCancel),
                KeyCode::Up | KeyCode::Char('k') => Some(Action::ProfileAccessMove(-1)),
                KeyCode::Down | KeyCode::Char('j') => Some(Action::ProfileAccessMove(1)),
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::ExplorerAdd(_))) {
            self.pending = None;
            return match event.code {
                KeyCode::Up | KeyCode::Char('k') => Some(Action::ExplorerAddMove(-1)),
                KeyCode::Down | KeyCode::Char('j') => Some(Action::ExplorerAddMove(1)),
                KeyCode::Enter => Some(Action::ExplorerAddConfirm),
                KeyCode::Esc | KeyCode::Char('q') => Some(Action::ExplorerAddCancel),
                _ => None,
            };
        }
        if let Some(Overlay::ProfileGroup(group)) = app.overlay.as_ref() {
            self.pending = None;
            return match group {
                crate::model::profile_group::ProfileGroupOverlay::Picker { .. } => match event.code
                {
                    KeyCode::Enter => Some(Action::ProfileGroupConfirm),
                    KeyCode::Esc | KeyCode::Char('q') => Some(Action::ProfileGroupCancel),
                    KeyCode::Up | KeyCode::Char('k') => Some(Action::ProfileGroupMove(-1)),
                    KeyCode::Down | KeyCode::Char('j') => Some(Action::ProfileGroupMove(1)),
                    _ => None,
                },
                crate::model::profile_group::ProfileGroupOverlay::Edit { .. } => {
                    match (event.modifiers, event.code) {
                        (KeyModifiers::NONE, KeyCode::Enter) => Some(Action::ProfileGroupConfirm),
                        (KeyModifiers::NONE, KeyCode::Esc) => Some(Action::ProfileGroupCancel),
                        _ => map_single_line_text_input_edit(event).map(Action::ProfileGroupEdit),
                    }
                }
                crate::model::profile_group::ProfileGroupOverlay::DeleteConfirm {
                    cancel_selected,
                    ..
                } => match event.code {
                    KeyCode::Enter if *cancel_selected => Some(Action::ProfileGroupCancel),
                    KeyCode::Enter | KeyCode::Char('y') => Some(Action::ProfileGroupConfirm),
                    KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                        Some(Action::ToggleProfileGroupDeleteFocus)
                    }
                    KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => {
                        Some(Action::ProfileGroupCancel)
                    }
                    _ => None,
                },
            };
        }
        if matches!(app.overlay, Some(Overlay::RecordView(_))) {
            let pending = self.pending.take();
            return match event.code {
                KeyCode::Char('g') => {
                    if let Some(pending) = pending
                        && pending_is_valid(
                            &pending,
                            app,
                            Instant::now(),
                            self.generation,
                            self.sequence_timeout,
                        )
                        && matches!(pending.pending, Pending::RecordViewGoto)
                    {
                        return Some(Action::RecordViewJumpFirstField);
                    }
                    self.set_pending(Pending::RecordViewGoto, app);
                    None
                }
                KeyCode::Char('j') | KeyCode::Down => Some(Action::RecordViewMoveFields(1)),
                KeyCode::Char('k') | KeyCode::Up => Some(Action::RecordViewMoveFields(-1)),
                KeyCode::Char('h') | KeyCode::Left => Some(Action::RecordViewMoveRow(-1)),
                KeyCode::Char('l') | KeyCode::Right => Some(Action::RecordViewMoveRow(1)),
                KeyCode::Char('G') | KeyCode::End => Some(Action::RecordViewJumpLastField),
                KeyCode::Home => Some(Action::RecordViewJumpFirstField),
                KeyCode::Char('y') => Some(Action::CopyRecordViewCell),
                KeyCode::Char('Y') => Some(Action::CopyRecordViewRow {
                    include_headers: false,
                }),
                KeyCode::Enter => Some(Action::ViewRecordViewValue),
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('v') => {
                    Some(Action::CloseRecordView)
                }
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::TextDetail(_))) {
            self.pending = None;
            return match event.code {
                KeyCode::Esc | KeyCode::Char('q') => Some(Action::CloseTextDetail),
                KeyCode::Up | KeyCode::Char('k') => {
                    app.overlay.as_ref().and_then(|overlay| match overlay {
                        Overlay::TextDetail(view) => Some(Action::ReadOnlyEditorScroll {
                            session_id: view.session_id,
                            rows: -1,
                            columns: 0,
                        }),
                        _ => None,
                    })
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    app.overlay.as_ref().and_then(|overlay| match overlay {
                        Overlay::TextDetail(view) => Some(Action::ReadOnlyEditorScroll {
                            session_id: view.session_id,
                            rows: 1,
                            columns: 0,
                        }),
                        _ => None,
                    })
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    app.overlay.as_ref().and_then(|overlay| match overlay {
                        Overlay::TextDetail(view) => Some(Action::ReadOnlyEditorScroll {
                            session_id: view.session_id,
                            rows: 0,
                            columns: -1,
                        }),
                        _ => None,
                    })
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    app.overlay.as_ref().and_then(|overlay| match overlay {
                        Overlay::TextDetail(view) => Some(Action::ReadOnlyEditorScroll {
                            session_id: view.session_id,
                            rows: 0,
                            columns: 1,
                        }),
                        _ => None,
                    })
                }
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::SubstituteConfirm { .. })) {
            self.pending = None;
            return match event.code {
                KeyCode::Char('y') => Some(Action::SubstituteYes),
                KeyCode::Char('n') => Some(Action::SubstituteNo),
                KeyCode::Char('a') => Some(Action::SubstituteAll),
                KeyCode::Char('l') => Some(Action::SubstituteLast),
                KeyCode::Char('q') | KeyCode::Esc => Some(Action::SubstituteQuit),
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::ExecutionConfirm { .. })) {
            self.pending = None;
            return match event.code {
                KeyCode::Enter => Some(Action::ConfirmExecution),
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => {
                    Some(Action::CancelExecution)
                }
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                    Some(Action::ToggleExecutionConfirmationFocus)
                }
                KeyCode::Up => Some(Action::ScrollExecutionConfirmation { rows: -1 }),
                KeyCode::Down => Some(Action::ScrollExecutionConfirmation { rows: 1 }),
                KeyCode::PageUp => Some(Action::ScrollExecutionConfirmation { rows: -10 }),
                KeyCode::PageDown => Some(Action::ScrollExecutionConfirmation { rows: 10 }),
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::PrincipalMutationConfirm { .. })) {
            self.pending = None;
            return match event.code {
                KeyCode::Enter => match app.overlay {
                    Some(Overlay::PrincipalMutationConfirm {
                        focus: crate::model::workspace::PrincipalMutationConfirmFocus::Apply,
                        ..
                    }) => Some(Action::ConfirmPrincipalMutation),
                    _ => Some(Action::CancelPrincipalMutation),
                },
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => {
                    Some(Action::CancelPrincipalMutation)
                }
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                    Some(Action::TogglePrincipalMutationFocus)
                }
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::MongoPrincipalMutation(_))) {
            self.pending = None;
            let Some(Overlay::MongoPrincipalMutation(draft)) = app.overlay.as_ref() else {
                return None;
            };
            let password = draft.selected_field
                == crate::model::mongodb_principal::MongoPrincipalField::Password;
            return match event.code {
                KeyCode::Esc => Some(Action::MongoPrincipalMutationCancel),
                KeyCode::Tab => Some(Action::MongoPrincipalMutationMoveField(1)),
                KeyCode::BackTab => Some(Action::MongoPrincipalMutationMoveField(-1)),
                KeyCode::Enter
                    if draft.selected_field
                        == crate::model::mongodb_principal::MongoPrincipalField::Operation =>
                {
                    Some(Action::MongoPrincipalMutationOperation("next".into()))
                }
                KeyCode::Enter => Some(Action::MongoPrincipalMutationConfirm),
                KeyCode::Backspace if password => {
                    Some(Action::MongoPrincipalMutationPasswordBackspace)
                }
                KeyCode::Delete if password => Some(Action::MongoPrincipalMutationPasswordDelete),
                KeyCode::Left if password => Some(Action::MongoPrincipalMutationPasswordMove(-1)),
                KeyCode::Right if password => Some(Action::MongoPrincipalMutationPasswordMove(1)),
                KeyCode::Home if password => {
                    Some(Action::MongoPrincipalMutationPasswordMove(isize::MIN))
                }
                KeyCode::End if password => {
                    Some(Action::MongoPrincipalMutationPasswordMove(isize::MAX))
                }
                KeyCode::Backspace => Some(Action::MongoPrincipalMutationEdit(
                    crate::model::text_input::TextInputEdit::Backspace,
                )),
                KeyCode::Delete => Some(Action::MongoPrincipalMutationEdit(
                    crate::model::text_input::TextInputEdit::Delete,
                )),
                KeyCode::Left => Some(Action::MongoPrincipalMutationEdit(
                    crate::model::text_input::TextInputEdit::MoveLeft,
                )),
                KeyCode::Right => Some(Action::MongoPrincipalMutationEdit(
                    crate::model::text_input::TextInputEdit::MoveRight,
                )),
                KeyCode::Home => Some(Action::MongoPrincipalMutationEdit(
                    crate::model::text_input::TextInputEdit::MoveHome,
                )),
                KeyCode::End => Some(Action::MongoPrincipalMutationEdit(
                    crate::model::text_input::TextInputEdit::MoveEnd,
                )),
                KeyCode::Char('v') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    Some(Action::MongoPrincipalMutationPaste(String::new()))
                }
                KeyCode::Char('v')
                    if cfg!(target_os = "macos")
                        && event.modifiers.contains(KeyModifiers::SUPER) =>
                {
                    Some(Action::MongoPrincipalMutationPaste(String::new()))
                }
                KeyCode::Char(ch) if event.modifiers.is_empty() && password => {
                    Some(Action::MongoPrincipalMutationPasswordInsert(ch))
                }
                KeyCode::Char(ch)
                    if event.modifiers.is_empty()
                        && draft.selected_field
                            != crate::model::mongodb_principal::MongoPrincipalField::Operation =>
                {
                    Some(Action::MongoPrincipalMutationEdit(
                        crate::model::text_input::TextInputEdit::Insert(ch),
                    ))
                }
                KeyCode::Char(ch)
                    if event.modifiers.is_empty()
                        && draft.selected_field
                            == crate::model::mongodb_principal::MongoPrincipalField::Operation =>
                {
                    Some(Action::MongoPrincipalMutationOperation(ch.to_string()))
                }
                _ => None,
            };
        }
        if matches!(
            app.overlay,
            Some(Overlay::MongoPrincipalConfirmation { .. })
        ) {
            self.pending = None;
            return match event.code {
                KeyCode::Esc => Some(Action::MongoPrincipalConfirmationCancel),
                KeyCode::Enter => Some(Action::MongoPrincipalConfirmationConfirm),
                KeyCode::Backspace => Some(Action::MongoPrincipalConfirmationBackspace),
                KeyCode::Delete => Some(Action::MongoPrincipalConfirmationEdit(
                    crate::model::text_input::TextInputEdit::Delete,
                )),
                KeyCode::Left => Some(Action::MongoPrincipalConfirmationEdit(
                    crate::model::text_input::TextInputEdit::MoveLeft,
                )),
                KeyCode::Right => Some(Action::MongoPrincipalConfirmationEdit(
                    crate::model::text_input::TextInputEdit::MoveRight,
                )),
                KeyCode::Home => Some(Action::MongoPrincipalConfirmationEdit(
                    crate::model::text_input::TextInputEdit::MoveHome,
                )),
                KeyCode::End => Some(Action::MongoPrincipalConfirmationEdit(
                    crate::model::text_input::TextInputEdit::MoveEnd,
                )),
                KeyCode::Char(ch) if event.modifiers.is_empty() => {
                    Some(Action::MongoPrincipalConfirmationInsert(ch))
                }
                KeyCode::Char('v')
                    if event.modifiers.contains(KeyModifiers::CONTROL)
                        || (cfg!(target_os = "macos")
                            && event.modifiers.contains(KeyModifiers::SUPER)) =>
                {
                    Some(Action::MongoPrincipalConfirmationPaste(String::new()))
                }
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::PrincipalMutationForm(_))) {
            self.pending = None;
            return match (event.code, app.overlay.as_ref()) {
                (KeyCode::Tab | KeyCode::BackTab, _) => {
                    Some(Action::TogglePrincipalMutationFormField)
                }
                (KeyCode::Char(' '), Some(Overlay::PrincipalMutationForm(form)))
                    if form.selected_field
                        == crate::model::principal::PrincipalMutationField::GrantOption =>
                {
                    Some(Action::TogglePrincipalMutationFormOption)
                }
                (KeyCode::Char('o'), _) => Some(Action::TogglePrincipalMutationFormOperation),
                (KeyCode::Enter, _) => Some(Action::ConfirmPrincipalMutationForm),
                (KeyCode::Esc | KeyCode::Char('q'), _) => Some(Action::CancelPrincipalMutationForm),
                _ => None,
            };
        }
        if matches!(
            app.overlay,
            Some(Overlay::MongoPrincipalConfirmation { .. })
        ) {
            self.pending = None;
            return match event.code {
                KeyCode::Esc => Some(Action::MongoPrincipalConfirmationCancel),
                KeyCode::Enter => Some(Action::MongoPrincipalConfirmationConfirm),
                KeyCode::Backspace => Some(Action::MongoPrincipalConfirmationBackspace),
                KeyCode::Char(ch) if event.modifiers.is_empty() => {
                    Some(Action::MongoPrincipalConfirmationInsert(ch))
                }
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::ManualCancelConfirm { .. })) {
            self.pending = None;
            return match event.code {
                KeyCode::Enter => match app.overlay {
                    Some(Overlay::ManualCancelConfirm {
                        focus: crate::model::workspace::ManualCancelFocus::KeepRunning,
                        ..
                    }) => Some(Action::CancelManualCancellation),
                    _ => Some(Action::ConfirmManualCancellation),
                },
                KeyCode::Esc | KeyCode::Char('k') => Some(Action::CancelManualCancellation),
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                    Some(Action::ToggleManualCancellationFocus)
                }
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::TransactionExitConfirm { .. })) {
            self.pending = None;
            return match event.code {
                KeyCode::Enter => Some(Action::ConfirmTransactionExit),
                KeyCode::Char('a') => Some(Action::ConfirmTransactionExitChoice(
                    crate::model::transaction::TransactionExitChoice::Abandon,
                )),
                KeyCode::Char('r') => Some(Action::ConfirmTransactionExitChoice(
                    crate::model::transaction::TransactionExitChoice::Rollback,
                )),
                KeyCode::Char('c') => Some(Action::ConfirmTransactionExitChoice(
                    crate::model::transaction::TransactionExitChoice::Commit,
                )),
                KeyCode::Esc | KeyCode::Char('n') => Some(Action::CancelTransactionExit),
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                    Some(Action::ToggleTransactionExitChoice)
                }
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::RelationTransactionConfirm(_))) {
            self.pending = None;
            return map_transaction_review(event, app);
        }
        if matches!(app.overlay, Some(Overlay::ClearTransactionOutcome { .. })) {
            self.pending = None;
            return match event.code {
                KeyCode::Enter => Some(Action::ConfirmClearTransactionOutcome),
                KeyCode::Esc => Some(Action::CancelClearTransactionOutcome),
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                    Some(Action::ToggleClearTransactionOutcomeFocus)
                }
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::TransactionMenu { .. })) {
            self.pending = None;
            return match event.code {
                KeyCode::Enter => Some(Action::ConfirmTransactionMenu),
                KeyCode::Esc | KeyCode::Char('q') => Some(Action::CancelTransactionMenu),
                KeyCode::Down | KeyCode::Char('j') => Some(Action::MoveTransactionMenu(1)),
                KeyCode::Up | KeyCode::Char('k') => Some(Action::MoveTransactionMenu(-1)),
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::CatalogDropConfirm { .. })) {
            self.pending = None;
            return match event.code {
                KeyCode::Enter => Some(Action::CatalogDropConfirm),
                KeyCode::Esc => Some(Action::CatalogDropCancel),
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                    Some(Action::ToggleCatalogDropFocus)
                }
                KeyCode::Backspace
                    if matches!(
                        app.overlay,
                        Some(Overlay::CatalogDropConfirm {
                            maintenance_database: Some(_),
                            ..
                        })
                    ) =>
                {
                    Some(Action::CatalogDropMaintenanceBackspace)
                }
                KeyCode::Char(character)
                    if matches!(
                        app.overlay,
                        Some(Overlay::CatalogDropConfirm {
                            maintenance_database: Some(_),
                            ..
                        })
                    ) =>
                {
                    Some(Action::CatalogDropMaintenanceInsert(character))
                }
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::PrincipalDropConfirm { .. })) {
            self.pending = None;
            return match event.code {
                KeyCode::Enter => Some(Action::PrincipalDropConfirm),
                KeyCode::Esc => Some(Action::PrincipalDropCancel),
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                    Some(Action::TogglePrincipalDropFocus)
                }
                _ => None,
            };
        }
        if matches!(
            app.overlay,
            Some(Overlay::CatalogEditorDestructiveConfirm { .. })
        ) {
            if is_text_redo(event) {
                return Some(Action::CatalogEditorConfirmRedo);
            }
            if is_text_undo(event) {
                return Some(Action::CatalogEditorConfirmUndo);
            }
            return match event.code {
                KeyCode::Enter => Some(Action::CatalogEditorApply),
                KeyCode::Esc => Some(Action::CatalogEditorCancel),
                KeyCode::Char(character) if event.modifiers.is_empty() => {
                    Some(Action::CatalogEditorConfirmInsert(character))
                }
                KeyCode::Backspace => Some(Action::CatalogEditorConfirmBackspace),
                _ => None,
            };
        }
        if let Some(Overlay::CatalogEditorDiscardConfirm { focus }) = app.overlay.as_ref() {
            return match event.code {
                KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                    Some(Action::CatalogEditorDiscardMove(1))
                }
                KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                    Some(Action::CatalogEditorDiscardMove(-1))
                }
                KeyCode::Enter => Some(match focus {
                    crate::model::workspace::CatalogEditorDiscardFocus::KeepEditing => {
                        Action::CatalogEditorDiscardKeepEditing
                    }
                    crate::model::workspace::CatalogEditorDiscardFocus::DiscardChanges => {
                        Action::CatalogEditorDiscardChanges
                    }
                }),
                KeyCode::Esc => Some(Action::CatalogEditorCancel),
                _ => None,
            };
        }
        if let Some(Overlay::NotificationHistory(history)) = app.overlay.as_ref() {
            self.pending = None;
            if history.clear_confirm {
                return match event.code {
                    KeyCode::Char('y') => Some(Action::NotificationHistoryClearConfirm),
                    KeyCode::Char('n') | KeyCode::Esc => {
                        Some(Action::NotificationHistoryClearCancel)
                    }
                    _ => None,
                };
            }
            if history.phase == crate::model::notification::HistorySearchPhase::Editing {
                if is_text_redo(event) {
                    return Some(Action::NotificationHistorySearchRedo);
                }
                if is_text_undo(event) {
                    return Some(Action::NotificationHistorySearchUndo);
                }
                return match event.code {
                    KeyCode::Esc => Some(Action::NotificationHistorySearchCancel),
                    KeyCode::Enter => Some(Action::NotificationHistorySearchConfirm),
                    KeyCode::Backspace => Some(Action::NotificationHistorySearchBackspace),
                    KeyCode::Char('u') if event.modifiers == KeyModifiers::CONTROL => {
                        Some(Action::NotificationHistorySearchClear)
                    }
                    KeyCode::Char(character) if event.modifiers.is_empty() => {
                        Some(Action::NotificationHistorySearchInsert(character))
                    }
                    _ => None,
                };
            }
            return match event.code {
                KeyCode::Esc | KeyCode::Char('q') => Some(Action::DismissOverlay),
                KeyCode::Enter => Some(Action::NotificationHistoryOpenDetail),
                KeyCode::Char('y') => Some(Action::NotificationHistoryCopy),
                KeyCode::Char('/') => Some(Action::NotificationHistorySearchOpen),
                KeyCode::Char('n') => Some(Action::NotificationHistoryNext),
                KeyCode::Char('N') => Some(Action::NotificationHistoryPrevious),
                KeyCode::Char('c') => Some(Action::NotificationHistoryClear),
                KeyCode::Up | KeyCode::Char('k') => Some(Action::NotificationHistoryMove(-1)),
                KeyCode::Down | KeyCode::Char('j') => Some(Action::NotificationHistoryMove(1)),
                KeyCode::PageUp => Some(Action::NotificationHistoryPage(-1)),
                KeyCode::PageDown => Some(Action::NotificationHistoryPage(1)),
                KeyCode::Home => Some(Action::NotificationHistoryJump(false)),
                KeyCode::End => Some(Action::NotificationHistoryJump(true)),
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::NotificationDetail(_))) {
            self.pending = None;
            return match event.code {
                KeyCode::Esc | KeyCode::Char('q') => Some(Action::NotificationDetailClose),
                KeyCode::Char('y') => Some(Action::NotificationDetailCopy),
                KeyCode::Up | KeyCode::Char('k') => Some(Action::NotificationDetailMove(-1)),
                KeyCode::Down | KeyCode::Char('j') => Some(Action::NotificationDetailMove(1)),
                KeyCode::PageUp => Some(Action::NotificationDetailMove(-10)),
                KeyCode::PageDown => Some(Action::NotificationDetailMove(10)),
                KeyCode::Home => Some(Action::NotificationDetailMove(isize::MIN)),
                KeyCode::End => Some(Action::NotificationDetailMove(isize::MAX)),
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::TargetSelector { .. })) {
            self.pending = None;
            return match event.code {
                KeyCode::Enter => Some(Action::ConfirmTargetSelector),
                KeyCode::Esc => Some(Action::CancelTargetSelector),
                KeyCode::Down | KeyCode::Char('j') => Some(Action::MoveTargetSelector(1)),
                KeyCode::Up | KeyCode::Char('k') => Some(Action::MoveTargetSelector(-1)),
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::DatabaseSelector(_))) {
            self.pending = None;
            return match event.code {
                KeyCode::Enter => Some(Action::ConfirmDatabaseSelector),
                KeyCode::Esc => Some(Action::CancelDatabaseSelector),
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                    Some(Action::MoveDatabaseSelector(1))
                }
                KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                    Some(Action::MoveDatabaseSelector(-1))
                }
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::PageSizeSelector { .. })) {
            self.pending = None;
            return match event.code {
                KeyCode::Enter => Some(Action::ConfirmPageSizeSelector),
                KeyCode::Esc => Some(Action::CancelPageSizeSelector),
                KeyCode::Down | KeyCode::Char('j') => Some(Action::MovePageSizeSelector(1)),
                KeyCode::Up | KeyCode::Char('k') => Some(Action::MovePageSizeSelector(-1)),
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::RedisPreviewFormat { .. })) {
            return match event.code {
                KeyCode::Enter => Some(Action::RedisPreviewFormatAccept),
                KeyCode::Esc => Some(Action::RedisPreviewFormatCancel),
                KeyCode::Down | KeyCode::Char('j') => Some(Action::RedisPreviewFormatMove(1)),
                KeyCode::Up | KeyCode::Char('k') => Some(Action::RedisPreviewFormatMove(-1)),
                _ => None,
            };
        }
        if app.focus == Focus::Explorer && app.explorer.find.is_some() {
            let confirmed = app.explorer.find.as_ref().is_some_and(|find| {
                find.phase == crate::model::workspace::ExplorerSearchPhase::Confirmed
            });
            if !confirmed {
                if !matches!(
                    self.pending.as_ref().map(|pending| &pending.pending),
                    Some(Pending::EditorLeader)
                ) {
                    self.pending = None;
                }
                if is_text_redo(event) {
                    return Some(Action::ExplorerFindRedo);
                }
                if is_text_undo(event) {
                    return Some(Action::ExplorerFindUndo);
                }
                if event.modifiers == KeyModifiers::CONTROL {
                    match event.code {
                        KeyCode::Char('w') => {
                            return Some(Action::ExplorerFindDeletePreviousWord);
                        }
                        KeyCode::Char('u') => return Some(Action::ExplorerFindClear),
                        _ => {}
                    }
                }
                if !event.modifiers.is_empty() && event.modifiers != KeyModifiers::SHIFT {
                    return None;
                }
                return match event.code {
                    KeyCode::Esc => Some(Action::ExplorerFindClose),
                    KeyCode::Enter => Some(Action::ExplorerFindConfirm),
                    KeyCode::Backspace => Some(Action::ExplorerFindBackspace),
                    KeyCode::Char(character) => Some(Action::ExplorerFindInsert(character)),
                    _ => None,
                };
            }
            match event.code {
                KeyCode::Esc => {
                    self.pending = None;
                    return Some(Action::ExplorerFindClose);
                }
                KeyCode::Char('n') => {
                    self.pending = None;
                    return Some(Action::ExplorerFindNext);
                }
                KeyCode::Char('N') => {
                    self.pending = None;
                    return Some(Action::ExplorerFindPrevious);
                }
                _ => {}
            }
        }
        if app.focus == Focus::Explorer && app.explorer.search.is_some() {
            let confirmed = app.explorer.search.as_ref().is_some_and(|search| {
                search.phase == crate::model::workspace::ExplorerSearchPhase::Confirmed
            });
            if confirmed {
                match event.code {
                    KeyCode::Esc => {
                        self.pending = None;
                        return Some(Action::ExplorerSearchClose);
                    }
                    KeyCode::Char('n') => {
                        self.pending = None;
                        return Some(Action::ExplorerSearchNext);
                    }
                    KeyCode::Char('N') => {
                        self.pending = None;
                        return Some(Action::ExplorerSearchPrevious);
                    }
                    _ => {}
                }
            } else {
                self.pending = None;
                if is_text_redo(event) {
                    return Some(Action::ExplorerSearchRedo);
                }
                if is_text_undo(event) {
                    return Some(Action::ExplorerSearchUndo);
                }
                if event.modifiers == KeyModifiers::CONTROL && event.code == KeyCode::Char('w') {
                    return Some(Action::ExplorerSearchDeletePreviousWord);
                }
                if event.modifiers == KeyModifiers::CONTROL && event.code == KeyCode::Char('u') {
                    return Some(Action::ExplorerSearchClear);
                }
                if !event.modifiers.is_empty() && event.modifiers != KeyModifiers::SHIFT {
                    return None;
                }
                return match event.code {
                    KeyCode::Esc => Some(Action::ExplorerSearchClose),
                    KeyCode::Enter => Some(Action::ExplorerSearchLocate),
                    KeyCode::Backspace => Some(Action::ExplorerSearchBackspace),
                    KeyCode::Down => Some(Action::ExplorerSearchMove(1)),
                    KeyCode::Up => Some(Action::ExplorerSearchMove(-1)),
                    KeyCode::Home => Some(Action::ExplorerSearchMove(isize::MIN)),
                    KeyCode::End => Some(Action::ExplorerSearchMove(isize::MAX)),
                    KeyCode::Char(character) => Some(Action::ExplorerSearchInsert(character)),
                    _ => None,
                };
            }
        }
        if let Some(Overlay::SqlEditorList(list)) = app.overlay.as_ref() {
            self.pending = None;
            use crate::model::sql_editor_list::SqlEditorListMode;
            return match list.mode {
                SqlEditorListMode::Browse => match event.code {
                    KeyCode::Enter => Some(Action::SqlEditorListActivate),
                    KeyCode::Up | KeyCode::Char('k') => Some(Action::SqlEditorListMove(-1)),
                    KeyCode::Down | KeyCode::Char('j') => Some(Action::SqlEditorListMove(1)),
                    KeyCode::Char('a') => Some(Action::SqlEditorListCreate),
                    KeyCode::Char('d') => Some(Action::SqlEditorListDeleteRequest),
                    KeyCode::Char('r') => Some(Action::SqlEditorListRenameStart),
                    KeyCode::Char('/') => Some(Action::SqlEditorListSearchStart),
                    KeyCode::Esc => Some(Action::SqlEditorListCancel),
                    _ => None,
                },
                SqlEditorListMode::Search => {
                    if is_text_redo(event) {
                        return Some(Action::SqlEditorListInputRedo);
                    }
                    if is_text_undo(event) {
                        return Some(Action::SqlEditorListInputUndo);
                    }
                    if event.modifiers == KeyModifiers::CONTROL {
                        return match event.code {
                            KeyCode::Char('w') => {
                                Some(Action::SqlEditorListInputDeletePreviousWord)
                            }
                            KeyCode::Char('u') => Some(Action::SqlEditorListInputDeleteToStart),
                            _ => None,
                        };
                    }
                    match event.code {
                        KeyCode::Enter => Some(Action::SqlEditorListActivate),
                        KeyCode::Esc => Some(Action::SqlEditorListCancel),
                        KeyCode::Up => Some(Action::SqlEditorListMove(-1)),
                        KeyCode::Down => Some(Action::SqlEditorListMove(1)),
                        KeyCode::Backspace => Some(Action::SqlEditorListInputBackspace),
                        KeyCode::Delete => Some(Action::SqlEditorListInputDelete),
                        KeyCode::Left => Some(Action::SqlEditorListInputMoveLeft),
                        KeyCode::Right => Some(Action::SqlEditorListInputMoveRight),
                        KeyCode::Home => Some(Action::SqlEditorListInputMoveHome),
                        KeyCode::End => Some(Action::SqlEditorListInputMoveEnd),
                        KeyCode::Char(value) if event.modifiers.is_empty() => {
                            Some(Action::SqlEditorListInputInsert(value))
                        }
                        _ => None,
                    }
                }
                SqlEditorListMode::Rename { .. } => {
                    if is_text_redo(event) {
                        return Some(Action::SqlEditorListInputRedo);
                    }
                    if is_text_undo(event) {
                        return Some(Action::SqlEditorListInputUndo);
                    }
                    if event.modifiers == KeyModifiers::CONTROL {
                        return match event.code {
                            KeyCode::Char('w') => {
                                Some(Action::SqlEditorListInputDeletePreviousWord)
                            }
                            KeyCode::Char('u') => Some(Action::SqlEditorListInputDeleteToStart),
                            _ => None,
                        };
                    }
                    match event.code {
                        KeyCode::Enter if event.modifiers.is_empty() => {
                            Some(Action::SqlEditorListRenameCommit)
                        }
                        KeyCode::Esc => Some(Action::SqlEditorListCancel),
                        KeyCode::Backspace => Some(Action::SqlEditorListInputBackspace),
                        KeyCode::Delete => Some(Action::SqlEditorListInputDelete),
                        KeyCode::Left => Some(Action::SqlEditorListInputMoveLeft),
                        KeyCode::Right => Some(Action::SqlEditorListInputMoveRight),
                        KeyCode::Home => Some(Action::SqlEditorListInputMoveHome),
                        KeyCode::End => Some(Action::SqlEditorListInputMoveEnd),
                        KeyCode::Char(value) if event.modifiers.is_empty() => {
                            Some(Action::SqlEditorListInputInsert(value))
                        }
                        _ => None,
                    }
                }
                SqlEditorListMode::DeleteConfirm { .. } => match event.code {
                    KeyCode::Enter => Some(match list.delete_focus {
                        crate::model::sql_editor_list::DeleteFocus::Cancel => {
                            Action::SqlEditorListDeleteCancel
                        }
                        crate::model::sql_editor_list::DeleteFocus::Delete => {
                            Action::SqlEditorListDeleteConfirm
                        }
                    }),
                    KeyCode::Esc => Some(Action::SqlEditorListDeleteCancel),
                    KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                        Some(Action::SqlEditorListDeleteFocusNext)
                    }
                    KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                        Some(Action::SqlEditorListDeleteFocusPrevious)
                    }
                    _ => None,
                },
            };
        }
        if let Some(Overlay::DeleteConsole { focus, .. }) = app.overlay.as_ref() {
            self.pending = None;
            return match event.code {
                KeyCode::Enter => Some(match focus {
                    crate::model::workspace::DeleteConsoleFocus::Cancel => {
                        Action::CancelDeleteConsole
                    }
                    crate::model::workspace::DeleteConsoleFocus::Delete => {
                        Action::ConfirmDeleteConsole
                    }
                }),
                KeyCode::Esc => Some(Action::CancelDeleteConsole),
                KeyCode::Tab | KeyCode::Right | KeyCode::Down => {
                    Some(Action::ToggleDeleteConsoleFocus)
                }
                KeyCode::BackTab | KeyCode::Left | KeyCode::Up => {
                    Some(Action::ToggleDeleteConsoleFocus)
                }
                _ => None,
            };
        }
        if let Some(Overlay::RedisDeleteConfirm { focus, .. }) = app.overlay.as_ref() {
            self.pending = None;
            return match event.code {
                KeyCode::Enter => Some(match focus {
                    crate::model::workspace::DeleteConsoleFocus::Cancel => {
                        Action::RedisDeleteCancel
                    }
                    crate::model::workspace::DeleteConsoleFocus::Delete => {
                        Action::RedisDeleteConfirm
                    }
                }),
                KeyCode::Esc => Some(Action::RedisDeleteCancel),
                KeyCode::Tab
                | KeyCode::Right
                | KeyCode::Down
                | KeyCode::BackTab
                | KeyCode::Left
                | KeyCode::Up => Some(Action::RedisDeleteToggleFocus),
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::RedisDeletePreparing { .. })) {
            self.pending = None;
            return match event.code {
                KeyCode::Esc => Some(Action::RedisDeleteCancel),
                _ => None,
            };
        }
        if let Some(Overlay::MongoManagement(draft)) = app.overlay.as_ref() {
            if draft.confirmation_text.is_none() {
                return match event.code {
                    KeyCode::Esc => Some(Action::MongoManagementCancel),
                    KeyCode::Enter if event.modifiers.contains(KeyModifiers::CONTROL) => {
                        Some(Action::MongoManagementConfirm)
                    }
                    KeyCode::Backspace => Some(Action::MongoManagementEditText(
                        crate::model::text_input::TextInputEdit::Backspace,
                    )),
                    KeyCode::Delete => Some(Action::MongoManagementEditText(
                        crate::model::text_input::TextInputEdit::Delete,
                    )),
                    KeyCode::Left => Some(Action::MongoManagementEditText(
                        crate::model::text_input::TextInputEdit::MoveLeft,
                    )),
                    KeyCode::Right => Some(Action::MongoManagementEditText(
                        crate::model::text_input::TextInputEdit::MoveRight,
                    )),
                    KeyCode::Home => Some(Action::MongoManagementEditText(
                        crate::model::text_input::TextInputEdit::MoveHome,
                    )),
                    KeyCode::End => Some(Action::MongoManagementEditText(
                        crate::model::text_input::TextInputEdit::MoveEnd,
                    )),
                    KeyCode::Char('v') if event.modifiers == KeyModifiers::CONTROL => None,
                    KeyCode::Char('c') if event.modifiers == KeyModifiers::CONTROL => None,
                    KeyCode::Char(ch) if event.modifiers.is_empty() => {
                        Some(Action::MongoManagementEditText(
                            crate::model::text_input::TextInputEdit::Insert(ch),
                        ))
                    }
                    _ => None,
                };
            }
            self.pending = None;
            return match event.code {
                KeyCode::Enter if draft.confirmation_valid() => {
                    Some(Action::MongoManagementConfirm)
                }
                KeyCode::Esc => Some(Action::MongoManagementCancel),
                KeyCode::Backspace => Some(Action::MongoManagementBackspace),
                KeyCode::Char(ch)
                    if !event.modifiers.intersects(
                        KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                    ) =>
                {
                    Some(Action::MongoManagementChar(ch))
                }
                _ => None,
            };
        }
        if app.overlay.is_some() {
            self.pending = None;
            return match event.code {
                KeyCode::Esc | KeyCode::Char('q') => Some(Action::DismissOverlay),
                _ => None,
            };
        }
        if event.kind == KeyEventKind::Press
            && let Some(command) = crate::input::panes::SmartPaneCommand::ALL
                .into_iter()
                .find(|command| self.bindings.matches(command.name(), event))
        {
            return Some(Action::SmartFocusPane(command.direction()));
        }
        if matches!(event.kind, KeyEventKind::Press | KeyEventKind::Repeat)
            && let Some(command) = crate::input::panes::SmartResizePaneCommand::ALL
                .into_iter()
                .find(|command| {
                    self.bindings
                        .matches(command.name(), KeyEvent::new(event.code, event.modifiers))
                })
        {
            self.clear_pending();
            return Some(Action::SmartResizePane(command.direction()));
        }
        if app.overlay.is_none()
            && event.kind == KeyEventKind::Press
            && (app.focus != Focus::Editor
                || (matches!(
                    app.active_editor_mode(),
                    EditorMode::Normal
                        | EditorMode::VisualChar
                        | EditorMode::VisualLine
                        | EditorMode::VisualBlock
                ) && event.modifiers != KeyModifiers::NONE))
            && !(event.modifiers == KeyModifiers::CONTROL && event.code == KeyCode::Char('w'))
            && let Some((command, sequences)) = crate::input::panes::PaneCommand::ALL
                .into_iter()
                .find_map(|command| {
                    let sequences = self.bindings.configured_sequences(command.name());
                    sequences
                        .iter()
                        .any(|sequence| sequence.first() == Some(&event))
                        .then_some((command, sequences))
                })
        {
            let matching = sequences
                .iter()
                .filter(|sequence| sequence.first() == Some(&event))
                .collect::<Vec<_>>();
            if matching.iter().any(|sequence| sequence.len() == 1) {
                return match crate::input::panes::dispatch(command, app) {
                    crate::input::panes::PaneDispatch::Action(action) => Some(*action),
                    _ => None,
                };
            }
            self.set_pending(
                Pending::PaneSequence {
                    events: vec![event],
                    count: 1,
                },
                app,
            );
            return None;
        }
        if active_data_query_has_focus(app)
            && let Some(action) = map_data_query(event, app)
        {
            return Some(action);
        }
        if app.overlay.is_none()
            && app.focus == Focus::Editor
            && matches!(
                app.active_editor_mode(),
                EditorMode::Insert | EditorMode::Replace
            )
            && event.kind == KeyEventKind::Press
            && event.modifiers.intersects(
                KeyModifiers::CONTROL
                    | KeyModifiers::ALT
                    | KeyModifiers::SUPER
                    | KeyModifiers::META
                    | KeyModifiers::HYPER,
            )
            && let Some(command) = crate::input::panes::PaneCommand::ALL
                .into_iter()
                .find(|command| self.bindings.matches(command.name(), event))
        {
            return match crate::input::panes::dispatch(command, app) {
                crate::input::panes::PaneDispatch::Action(action) => Some(*action),
                crate::input::panes::PaneDispatch::Consumed => None,
            };
        }

        // Redis panes have input handlers that intentionally run before the
        // generic bindings below. Keep the configured help binding ahead of
        // those handlers, while allowing find/editor insert modes to consume
        // printable characters as text.
        if redis_help_binding_is_active(app, event, &self.bindings) {
            self.pending = None;
            return Some(Action::ShowHelp);
        }

        if app.focus == Focus::Editor
            && app.active_editor_mode() != EditorMode::Normal
            && self.bindings.matches("focus-previous-pane", event)
        {
            return Some(Action::FocusPrevious);
        }

        if app.focus == Focus::Editor
            && matches!(
                app.active_editor_mode(),
                EditorMode::Normal
                    | EditorMode::VisualChar
                    | EditorMode::VisualLine
                    | EditorMode::VisualBlock
            )
        {
            match event.code {
                _ if self.bindings.matches("help", event) => return Some(Action::ShowHelp),
                _ if self.bindings.matches("focus-next-pane", event) => {
                    return Some(Action::FocusNext);
                }
                _ if self.bindings.matches("focus-previous-pane", event) => {
                    return Some(Action::FocusPrevious);
                }
                _ => {}
            }
        }
        if app.focus == Focus::Editor
            && app.active_editor_mode() == EditorMode::Insert
            && event.code == KeyCode::Esc
        {
            return Some(Action::EditorKey(event));
        }
        if app.focus == Focus::Editor
            && matches!(
                app.active_editor_mode(),
                EditorMode::Insert | EditorMode::Replace
            )
            && (is_text_undo(event) || is_text_redo(event))
        {
            return Some(Action::EditorKey(event));
        }
        if app.focus == Focus::Editor && (is_text_undo(event) || is_text_redo(event)) {
            if self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.pending.is_leader())
            {
                self.clear_pending();
            }
            return Some(Action::EditorKey(event));
        }
        if app.active_console_opt().is_some_and(|tab| {
            tab.completion.is_some() && app.active_editor_mode() == EditorMode::Insert
        }) {
            let completion_action = match event.code {
                KeyCode::Down if event.modifiers.is_empty() => Some(Action::CompletionNext),
                KeyCode::Char('n') if event.modifiers == KeyModifiers::CONTROL => {
                    Some(Action::CompletionNext)
                }
                KeyCode::Up if event.modifiers.is_empty() => Some(Action::CompletionPrevious),
                KeyCode::Char('p') if event.modifiers == KeyModifiers::CONTROL => {
                    Some(Action::CompletionPrevious)
                }
                KeyCode::Enter => Some(Action::CompletionAccept),
                KeyCode::Esc => Some(Action::CompletionDismiss),
                _ => None,
            };
            if completion_action.is_some() {
                return completion_action;
            }
        }

        if app.focus != Focus::Editor && self.bindings.matches("focus-next-pane", event) {
            return Some(Action::FocusNext);
        }
        if app.focus != Focus::Editor && self.bindings.matches("focus-previous-pane", event) {
            return Some(Action::FocusPrevious);
        }

        if tab_shortcuts_have_focus(app) {
            if matches_tab_binding(&self.bindings, "move-tab-right", event) {
                return Some(Action::MoveTabRight);
            }
            if matches_tab_binding(&self.bindings, "move-tab-left", event) {
                return Some(Action::MoveTabLeft);
            }
            if self.bindings.matches("next-tab", event) {
                return Some(Action::NextTab);
            }
            if self.bindings.matches("previous-tab", event) {
                return Some(Action::PreviousTab);
            }
        }
        if self.bindings.matches("close-tab", event) {
            return Some(Action::CloseActiveTab);
        }
        match (event.modifiers, event.code) {
            (modifiers, KeyCode::Char('q' | 'Q'))
                if modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT) =>
            {
                return Some(Action::CloseOtherTabs);
            }
            _ => {}
        }

        if self.pending.as_ref().is_some_and(|pending| {
            pending_is_valid(
                pending,
                app,
                Instant::now(),
                self.generation,
                self.sequence_timeout,
            )
        }) {
            let prefix = self
                .pending
                .as_ref()
                .and_then(|pending| pending_display(pending.pending.clone()))
                .map(|(prefix, _)| prefix);
            if let Some(prefix) = prefix {
                let pane_sequence = matches!(
                    self.pending.as_ref().map(|pending| &pending.pending),
                    Some(Pending::PaneSequence { .. })
                );
                let shortcuts = if pane_sequence {
                    Vec::new()
                } else {
                    crate::help::prefix_shortcuts_with_bindings(
                        crate::help::shortcut_context(app),
                        crate::help::shortcut_capabilities(app),
                        prefix,
                        Some(&self.bindings),
                    )
                };
                let chooser_available = app.focus != Focus::Editor;
                if pane_sequence
                    && let Some(Pending::PaneSequence { events, count }) =
                        self.pending.as_ref().map(|pending| &pending.pending)
                {
                    let mut next = events.clone();
                    next.push(event);
                    if let Some(command) =
                        crate::input::panes::PaneCommand::ALL
                            .into_iter()
                            .find(|command| {
                                self.bindings
                                    .configured_sequences(command.name())
                                    .iter()
                                    .any(|sequence| sequence.as_slice() == next.as_slice())
                            })
                        && self.bindings.matches_sequence(command.name(), &next)
                    {
                        self.pending = None;
                        return match crate::input::panes::dispatch(command, app) {
                            crate::input::panes::PaneDispatch::Action(action) => Some(*action),
                            _ => None,
                        };
                    }
                    let continues = crate::input::panes::PaneCommand::ALL
                        .into_iter()
                        .any(|command| self.bindings.has_sequence_prefix(command.name(), &next));
                    if continues {
                        self.continue_pending(
                            Pending::PaneSequence {
                                events: next,
                                count: *count + 1,
                            },
                            app.focus,
                            app.active_editor_mode(),
                            app.tabs
                                .get(app.active_tab)
                                .map_or(Uuid::nil(), |tab| tab.id()),
                            matches!(app.overlay, Some(Overlay::RecordView(_))),
                        );
                        return None;
                    }
                    if let Some(Pending::PaneSequence { .. }) =
                        self.pending.as_ref().map(|pending| &pending.pending)
                    {
                        return None;
                    }
                }
                match event.code {
                    KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down
                        if chooser_available
                            && event.modifiers.is_empty()
                            && !shortcuts.is_empty() =>
                    {
                        let layout = crate::input::sequence_navigation::Layout::new(
                            self.sequence_width,
                            shortcuts.len(),
                        );
                        let direction = match event.code {
                            KeyCode::Left => crate::input::sequence_navigation::Direction::Left,
                            KeyCode::Right => crate::input::sequence_navigation::Direction::Right,
                            KeyCode::Up => crate::input::sequence_navigation::Direction::Up,
                            KeyCode::Down => crate::input::sequence_navigation::Direction::Down,
                            _ => unreachable!(),
                        };
                        self.sequence_selected = layout
                            .move_selection(self.sequence_selected, direction)
                            .unwrap_or(self.sequence_selected);
                        return None;
                    }
                    KeyCode::Enter => {
                        let action = shortcuts
                            .get(
                                self.sequence_selected
                                    .min(shortcuts.len().saturating_sub(1)),
                            )
                            .map(|shortcut| Action::ExecuteHelpShortcut(shortcut.id));
                        if action.is_some() {
                            self.pending = None;
                            self.sequence_selected = 0;
                        }
                        return action;
                    }
                    _ => {}
                }
            }
            let PendingState {
                pending,
                focus,
                editor_mode,
                tab_id,
                ..
            } = self.pending.take().unwrap();
            if let Pending::PaneSequence { events, count } = &pending {
                if event.code == KeyCode::Esc
                    || event.modifiers == KeyModifiers::CONTROL && event.code == KeyCode::Char('c')
                {
                    return None;
                }
                let mut next = events.clone();
                next.push(event);
                if let Some(command) = crate::input::panes::PaneCommand::ALL
                    .into_iter()
                    .find(|command| self.bindings.matches_sequence(command.name(), &next))
                {
                    return match crate::input::panes::dispatch(command, app) {
                        crate::input::panes::PaneDispatch::Action(action) => Some(*action),
                        _ => None,
                    };
                }
                if crate::input::panes::PaneCommand::ALL
                    .into_iter()
                    .any(|command| self.bindings.has_sequence_prefix(command.name(), &next))
                {
                    self.continue_pending(
                        Pending::PaneSequence {
                            events: next,
                            count: *count + 1,
                        },
                        focus,
                        editor_mode,
                        tab_id,
                        matches!(app.overlay, Some(Overlay::RecordView(_))),
                    );
                }
                return None;
            }
            if matches!(pending, Pending::CatalogColumnDelete { .. }) {
                if event.kind == KeyEventKind::Press
                    && event.modifiers.is_empty()
                    && event.code == KeyCode::Char('d')
                {
                    return Some(Action::CatalogEditorRemoveTableColumn);
                }
                return map_catalog_editor(event, app);
            }
            if matches!(pending, Pending::EditorLeader | Pending::LeaderTransaction) {
                if event.modifiers.is_empty() && matches!(event.code, KeyCode::Char(' ' | '\\')) {
                    self.set_pending(Pending::EditorLeader, app);
                } else if pending == Pending::EditorLeader
                    && editor_mode == EditorMode::Normal
                    && event.modifiers.is_empty()
                    && event.code == KeyCode::Char('t')
                {
                    self.continue_pending(
                        Pending::LeaderTransaction,
                        focus,
                        editor_mode,
                        tab_id,
                        matches!(app.overlay, Some(Overlay::RecordView(_))),
                    );
                }
                return Some(Action::EditorKey(event));
            }
            if let Pending::WindowCount { count } = pending {
                if event.modifiers.is_empty()
                    && let KeyCode::Char(character @ '0'..='9') = event.code
                {
                    self.continue_pending(
                        Pending::WindowCount {
                            count: count
                                .saturating_mul(10)
                                .saturating_add(character.to_digit(10).unwrap_or(0)),
                        },
                        focus,
                        editor_mode,
                        tab_id,
                        matches!(app.overlay, Some(Overlay::RecordView(_))),
                    );
                    return None;
                }
                if event.modifiers == KeyModifiers::CONTROL && event.code == KeyCode::Char('w') {
                    self.continue_pending(
                        Pending::Window { count },
                        focus,
                        editor_mode,
                        tab_id,
                        matches!(app.overlay, Some(Overlay::RecordView(_))),
                    );
                    return None;
                }
                self.continue_pending(
                    pending,
                    focus,
                    editor_mode,
                    tab_id,
                    matches!(app.overlay, Some(Overlay::RecordView(_))),
                );
                return None;
            }
            if matches!(pending, Pending::Window { .. }) {
                if event.kind == KeyEventKind::Press
                    && let Some(command) =
                        crate::input::panes::PaneCommand::ALL
                            .into_iter()
                            .find(|command| {
                                self.bindings.matches_sequence(
                                    command.name(),
                                    &[
                                        KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
                                        event,
                                    ],
                                )
                            })
                {
                    return match crate::input::panes::dispatch(command, app) {
                        crate::input::panes::PaneDispatch::Action(action) => Some(*action),
                        _ => None,
                    };
                }
                let sequence = [
                    KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
                    event,
                ];
                let redis_direction =
                    if self.bindings.matches_sequence("focus-pane-left", &sequence) {
                        Some(true)
                    } else if self
                        .bindings
                        .matches_sequence("focus-pane-right", &sequence)
                    {
                        Some(false)
                    } else {
                        None
                    };
                if let Some(is_left) = redis_direction
                    && let Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab)) =
                        app.tabs.get(app.active_tab)
                    && matches!(app.focus, Focus::Explorer | Focus::Results)
                {
                    let action = match (app.focus, tab.focus, is_left) {
                        (Focus::Explorer, _, true) | (Focus::Results, _, false)
                            if app.focus == Focus::Explorer && is_left
                                || app.focus == Focus::Results
                                    && tab.focus
                                        == crate::model::redis_browser::RedisBrowserFocus::Preview
                                    && !is_left =>
                        {
                            None
                        }
                        (Focus::Explorer, _, false) => Some(Action::RedisFocusPane(
                            crate::model::redis_browser::RedisBrowserFocus::Keys,
                        )),
                        (
                            Focus::Results,
                            crate::model::redis_browser::RedisBrowserFocus::Keys,
                            true,
                        ) => Some(Action::Focus(Focus::Explorer)),
                        (
                            Focus::Results,
                            crate::model::redis_browser::RedisBrowserFocus::Keys,
                            false,
                        ) => Some(Action::RedisFocusPane(
                            crate::model::redis_browser::RedisBrowserFocus::Preview,
                        )),
                        (
                            Focus::Results,
                            crate::model::redis_browser::RedisBrowserFocus::Preview,
                            true,
                        ) => Some(Action::RedisFocusPane(
                            crate::model::redis_browser::RedisBrowserFocus::Keys,
                        )),
                        _ => None,
                    };
                    return action;
                }
                let command = if self.bindings.matches_sequence("focus-pane-left", &sequence)
                    && matches!(app.focus, Focus::Editor | Focus::Results)
                {
                    Some(Action::Focus(Focus::Explorer))
                } else if self.bindings.matches_sequence("focus-pane-down", &sequence)
                    && app.focus == Focus::Editor
                {
                    Some(Action::Focus(Focus::Results))
                } else if self.bindings.matches_sequence("focus-pane-up", &sequence)
                    && app.focus == Focus::Results
                    && !app.is_active_relation_tab()
                {
                    Some(Action::Focus(Focus::Editor))
                } else if self
                    .bindings
                    .matches_sequence("focus-pane-right", &sequence)
                    && app.focus == Focus::Explorer
                {
                    Some(Action::Focus(if app.active_console_opt().is_none() {
                        Focus::Results
                    } else {
                        Focus::Editor
                    }))
                } else if self
                    .bindings
                    .matches_sequence("toggle-pane-maximized", &sequence)
                {
                    Some(Action::TogglePaneMaximized)
                } else if self
                    .bindings
                    .matches_sequence("reset-pane-sizes", &sequence)
                {
                    Some(Action::ResetPaneSizes)
                } else {
                    None
                };
                if command.is_some() {
                    return command;
                }
            }
            if pending == Pending::GridAlign {
                let sequence = [KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE), event];
                if self
                    .bindings
                    .matches_sequence("results-align-middle", &sequence)
                {
                    return Some(Action::GridAlignSelectedRow(
                        crate::model::tab::GridRowAlignment::Middle,
                    ));
                }
                if self
                    .bindings
                    .matches_sequence("results-align-top", &sequence)
                {
                    return Some(Action::GridAlignSelectedRow(
                        crate::model::tab::GridRowAlignment::Top,
                    ));
                }
                if self
                    .bindings
                    .matches_sequence("results-align-bottom", &sequence)
                {
                    return Some(Action::GridAlignSelectedRow(
                        crate::model::tab::GridRowAlignment::Bottom,
                    ));
                }
            }
            if let Some(action) = map_pending(pending.clone(), event, app, &self.bindings) {
                return Some(action);
            }
            self.continue_pending(
                pending,
                focus,
                editor_mode,
                tab_id,
                matches!(app.overlay, Some(Overlay::RecordView(_))),
            );
            return None;
        }

        // Redis Preview is a read-only Vim buffer. Route its complete input
        // stream before generic window counts and Redis tree navigation; the
        // editor must own counts, operators, searches, and motions.
        if app.focus == Focus::Results
            && event.modifiers == KeyModifiers::CONTROL
            && event.code == KeyCode::Char('w')
            && matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
                    if (tab.focus == crate::model::redis_browser::RedisBrowserFocus::Preview
                        || (tab.focus == crate::model::redis_browser::RedisBrowserFocus::Keys
                            && tab.find.as_ref().is_none_or(|find| {
                                find.phase != crate::model::redis_browser::RedisFindPhase::Editing
                            })))
            )
            && !matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
                    if app.editor_prompt_active(tab.preview_editor_id)
            )
        {
            self.set_pending(Pending::Window { count: 1 }, app);
            return None;
        }
        if app.focus == Focus::Results
            && event.modifiers == KeyModifiers::CONTROL
            && event.code == KeyCode::Char('w')
            && matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::MongoBrowser(_))
            )
        {
            self.set_pending(Pending::Window { count: 1 }, app);
            return None;
        }
        if app.focus == Focus::Results
            && matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::MongoBrowser(_))
            )
        {
            let Some(crate::model::tab::WorkspaceTab::MongoBrowser(tab)) =
                app.tabs.get(app.active_tab)
            else {
                return None;
            };
            if tab.editing {
                return match event.code {
                    KeyCode::Esc => Some(Action::MongoDocumentCancelEdit),
                    KeyCode::Enter if event.modifiers.contains(KeyModifiers::CONTROL) => {
                        Some(Action::MongoDocumentSave)
                    }
                    KeyCode::Backspace => Some(Action::MongoDocumentEditText(
                        crate::model::text_input::TextInputEdit::Backspace,
                    )),
                    KeyCode::Char(ch) if event.modifiers.is_empty() => {
                        Some(Action::MongoDocumentEditText(
                            crate::model::text_input::TextInputEdit::Insert(ch),
                        ))
                    }
                    _ => None,
                };
            }
            if matches!(
                tab.page,
                crate::model::mongodb_browser::MongoPageState::Failed(_)
            ) && event.modifiers.is_empty()
                && event.code == KeyCode::Char('r')
            {
                return Some(Action::MongoPageRetry);
            }
            return match event.code {
                KeyCode::Char('j') | KeyCode::Down => Some(Action::MongoDocumentNext),
                KeyCode::Char('k') | KeyCode::Up => Some(Action::MongoDocumentPrevious),
                KeyCode::Char('e') => Some(Action::MongoDocumentEdit),
                KeyCode::Char('l') => Some(Action::MongoPageNext),
                _ => None,
            };
        }

        if app.focus == Focus::Results
            && matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
                    if tab.focus == crate::model::redis_browser::RedisBrowserFocus::Preview
                        && tab.format.view() != crate::value_preview::ValueView::Table
            )
            && let Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab)) =
                app.tabs.get(app.active_tab)
        {
            if event.modifiers.is_empty()
                && event.code == KeyCode::Char(' ')
                && app
                    .active_read_only_editor_mode()
                    .is_none_or(|mode| mode == EditorMode::Normal)
                && !app.active_redis_preview_prompt_active()
            {
                self.set_pending(Pending::RedisPreviewLeader, app);
                return None;
            }
            return if app.editor_is_editable(tab.preview_editor_id) {
                Some(Action::EditorKey(event))
            } else {
                Some(Action::ReadOnlyEditorKey {
                    session_id: tab.preview_editor_id,
                    event,
                })
            };
        }

        if app.focus != Focus::Editor
            && event.modifiers.is_empty()
            && !matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::Dashboard(_))
            )
            && let KeyCode::Char(character @ '1'..='9') = event.code
        {
            self.set_pending(
                Pending::WindowCount {
                    count: character.to_digit(10).unwrap_or(1),
                },
                app,
            );
            return None;
        }

        if is_principal_overview(app)
            && let Some(crate::model::tab::WorkspaceTab::PrincipalDdl(tab)) =
                app.tabs.get(app.active_tab)
            && let Some(find) = tab.access_find.as_ref()
        {
            match find.phase {
                crate::model::principal::PrincipalFindPhase::Editing => {
                    if event.modifiers.is_empty() || event.modifiers == KeyModifiers::SHIFT {
                        self.pending = None;
                    }
                    if is_text_undo(event) {
                        return Some(Action::PrincipalAccessFindEdit(
                            crate::model::text_input::TextInputEdit::Undo,
                        ));
                    }
                    if is_text_redo(event) {
                        return Some(Action::PrincipalAccessFindEdit(
                            crate::model::text_input::TextInputEdit::Redo,
                        ));
                    }
                    return match event.code {
                        KeyCode::Esc => Some(Action::PrincipalAccessFindClose),
                        KeyCode::Enter => Some(Action::PrincipalAccessFindConfirm),
                        _ => map_text_input_edit(event).map(Action::PrincipalAccessFindEdit),
                    };
                }
                crate::model::principal::PrincipalFindPhase::Confirmed => match event.code {
                    KeyCode::Esc => return Some(Action::PrincipalAccessFindClose),
                    KeyCode::Char('n') if event.modifiers.is_empty() => {
                        return Some(Action::PrincipalAccessFindNext);
                    }
                    KeyCode::Char('N')
                        if event.modifiers.is_empty() || event.modifiers == KeyModifiers::SHIFT =>
                    {
                        return Some(Action::PrincipalAccessFindPrevious);
                    }
                    _ => {}
                },
            }
        }
        if is_ddl_only_focus(app)
            && event.modifiers.is_empty()
            && event.code == KeyCode::Char(' ')
            && app
                .active_read_only_editor_mode()
                .is_none_or(|mode| mode == EditorMode::Normal)
        {
            self.set_pending(Pending::Leader, app);
            return None;
        }
        if is_ddl_only_focus(app)
            && is_read_only_editor_key(event)
            && let Some(session_id) = ddl_only_session_id(app)
        {
            return Some(Action::ReadOnlyEditorKey { session_id, event });
        }
        if app.focus == Focus::Results
            && event.modifiers.is_empty()
            && event.code == KeyCode::Char('o')
            && matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::Sql(tab))
                    if matches!(tab.result_view, crate::model::tab::ResultView::Output | crate::model::tab::ResultView::Plan)
            )
        {
            return Some(Action::ToggleResultView);
        }
        if let Some(crate::model::tab::WorkspaceTab::Sql(tab)) = app.tabs.get(app.active_tab)
            && app.focus == Focus::Results
            && matches!(
                tab.result_view,
                crate::model::tab::ResultView::Output | crate::model::tab::ResultView::Plan
            )
            && event.modifiers.is_empty()
            && event.code == KeyCode::Char(' ')
            && app
                .active_read_only_editor_mode()
                .is_none_or(|mode| mode == EditorMode::Normal)
        {
            self.set_pending(Pending::Leader, app);
            return None;
        }
        if let Some(crate::model::tab::WorkspaceTab::Sql(tab)) = app.tabs.get(app.active_tab)
            && app.focus == Focus::Results
            && matches!(
                tab.result_view,
                crate::model::tab::ResultView::Output | crate::model::tab::ResultView::Plan
            )
            && is_read_only_editor_key(event)
        {
            return Some(Action::ReadOnlyEditorKey {
                session_id: tab.output_editor_id,
                event,
            });
        }

        if event.modifiers.is_empty()
            && event.code == KeyCode::Char('g')
            && (app.focus == Focus::Explorer
                || (is_grid_navigation_focus(app) && relation_grid_is_browse(app)))
        {
            self.set_pending(Pending::Goto, app);
            return None;
        }

        if app.focus == Focus::Results
            && matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
                    if tab.focus == crate::model::redis_browser::RedisBrowserFocus::Keys
            )
        {
            if let Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab)) =
                app.tabs.get(app.active_tab)
                && let Some(find) = tab.find.as_ref()
            {
                if find.phase == crate::model::redis_browser::RedisFindPhase::Editing {
                    match event.code {
                        KeyCode::Esc => return Some(Action::RedisFindCancel),
                        KeyCode::Enter => return Some(Action::RedisFindConfirm),
                        KeyCode::Backspace => return Some(Action::RedisFindBackspace),
                        KeyCode::Delete => return Some(Action::RedisFindDelete),
                        KeyCode::Char(character) if event.modifiers.is_empty() => {
                            return Some(Action::RedisFindInsert(character));
                        }
                        _ => return None,
                    }
                }
                if find.phase == crate::model::redis_browser::RedisFindPhase::Confirmed {
                    match event.code {
                        KeyCode::Esc => return Some(Action::RedisFindCancel),
                        KeyCode::Char('n') => return Some(Action::RedisFindNext),
                        KeyCode::Char('N') => return Some(Action::RedisFindPrevious),
                        _ => {}
                    }
                }
            }
            if event.modifiers.is_empty() && event.code == KeyCode::Char('/') {
                return Some(Action::RedisFindOpen);
            }
            if event.modifiers.is_empty() && event.code == KeyCode::Char('a') {
                let can_create = matches!(
                    app.tabs.get(app.active_tab),
                    Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
                        if tab.tree.selected.is_none()
                            || matches!(
                                tab.tree.selected,
                                Some(crate::model::redis_key_tree::KeyTreeNodeId::Prefix(_))
                                    | Some(crate::model::redis_key_tree::KeyTreeNodeId::Key(_))
                            )
                );
                if can_create {
                    return Some(Action::OpenRedisObjectCreate);
                }
            }
            if event.modifiers.is_empty() && event.code == KeyCode::Char('e') {
                let can_edit = matches!(
                    app.tabs.get(app.active_tab),
                    Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
                        if matches!(
                            tab.tree.selected,
                            Some(crate::model::redis_key_tree::KeyTreeNodeId::Key(_))
                        )
                );
                if can_edit {
                    return Some(Action::OpenRedisObjectEdit);
                }
            }
            if matches!(app.tabs.get(app.active_tab), Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab)) if tab.focus == crate::model::redis_browser::RedisBrowserFocus::Keys)
                && event.modifiers.is_empty()
                && event.code == KeyCode::Char('r')
            {
                return Some(Action::RedisRetryScan);
            }
            if event.code == KeyCode::PageDown {
                return Some(Action::RedisKeysScroll(10));
            }
            if event.code == KeyCode::PageUp {
                return Some(Action::RedisKeysScroll(-10));
            }
            if matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
                    if tab.focus == crate::model::redis_browser::RedisBrowserFocus::Keys
            ) && event.modifiers.is_empty()
            {
                match event.code {
                    KeyCode::Char('y') => return Some(Action::RedisCopyKey),
                    KeyCode::Char('d') => return Some(Action::RedisDeleteKey),
                    _ => {}
                }
            }
            if event.modifiers.is_empty() && event.code == KeyCode::Char('o') {
                return match app.tabs.get(app.active_tab) {
                    Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab)) => {
                        match tab.tree.selected.as_ref() {
                            Some(node @ crate::model::redis_key_tree::KeyTreeNodeId::Prefix(_)) => {
                                Some(Action::RedisToggleNode {
                                    tab_id: tab.id,
                                    node: node.clone(),
                                })
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
            }
            if let Some(action) = map_configured_navigation(event, app, &self.bindings) {
                return Some(match action {
                    Action::GridMove { rows, columns: 0 } if rows != 0 => {
                        Action::RedisMoveSelection(rows)
                    }
                    Action::GridMove {
                        rows: 0,
                        columns: -1,
                    } if matches!(
                        app.tabs.get(app.active_tab),
                        Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
                            if tab.focus
                                == crate::model::redis_browser::RedisBrowserFocus::Keys
                    ) =>
                    {
                        Action::RedisCollapseSelection
                    }
                    Action::GridMove {
                        rows: 0,
                        columns: 1,
                    } if matches!(
                        app.tabs.get(app.active_tab),
                        Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
                            if tab.focus
                                == crate::model::redis_browser::RedisBrowserFocus::Keys
                    ) =>
                    {
                        Action::RedisExpandSelection
                    }
                    Action::ExplorerFindOpen => Action::RedisFindOpen,
                    other => other,
                });
            }
            let focus = match app.tabs.get(app.active_tab) {
                Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab)) => tab.focus,
                _ => return None,
            };
            return match (focus, event.code) {
                (
                    crate::model::redis_browser::RedisBrowserFocus::Keys,
                    KeyCode::Char('j') | KeyCode::Down,
                ) => Some(Action::RedisMoveSelection(1)),
                (
                    crate::model::redis_browser::RedisBrowserFocus::Keys,
                    KeyCode::Char('k') | KeyCode::Up,
                ) => Some(Action::RedisMoveSelection(-1)),
                (
                    crate::model::redis_browser::RedisBrowserFocus::Preview,
                    KeyCode::Char('j') | KeyCode::Down,
                ) => Some(Action::RedisPreviewScroll(1)),
                (
                    crate::model::redis_browser::RedisBrowserFocus::Preview,
                    KeyCode::Char('k') | KeyCode::Up,
                ) => Some(Action::RedisPreviewScroll(-1)),
                (
                    crate::model::redis_browser::RedisBrowserFocus::Keys,
                    KeyCode::Right | KeyCode::Char('l'),
                ) => Some(Action::RedisExpandSelection),
                (
                    crate::model::redis_browser::RedisBrowserFocus::Keys,
                    KeyCode::Left | KeyCode::Char('h'),
                ) => Some(Action::RedisCollapseSelection),
                (crate::model::redis_browser::RedisBrowserFocus::Keys, KeyCode::Enter) => {
                    Some(Action::RedisPrimarySelection)
                }
                (crate::model::redis_browser::RedisBrowserFocus::Keys, KeyCode::Char('y')) => {
                    Some(Action::RedisCopyKey)
                }
                (crate::model::redis_browser::RedisBrowserFocus::Keys, KeyCode::Char('d')) => {
                    Some(Action::RedisDeleteKey)
                }
                _ => None,
            };
        }

        if app.focus == Focus::Explorer
            && app.explorer.search.is_none()
            && event.modifiers.is_empty()
            && event.code == KeyCode::Char('z')
        {
            self.set_pending(Pending::ExplorerAlign, app);
            return None;
        }

        if is_grid_navigation_focus(app)
            && event.modifiers.is_empty()
            && event.code == KeyCode::Char('z')
        {
            self.set_pending(Pending::GridAlign, app);
            return None;
        }

        if (is_sql_grid_focus(app)
            || is_read_only_grid_focus(app)
            || is_redis_table_grid_focus(app))
            && (event.modifiers.is_empty() || event.modifiers == KeyModifiers::SHIFT)
        {
            match event.code {
                KeyCode::Char('y') => return Some(Action::CopyGridCell),
                KeyCode::Char('Y') => {
                    return Some(Action::CopyGridRow {
                        include_headers: false,
                    });
                }
                _ => {}
            }
        }

        if is_relation_data_focus(app) {
            if (event.modifiers.is_empty() || event.modifiers == KeyModifiers::SHIFT)
                && relation_grid_is_browse(app)
            {
                match event.code {
                    KeyCode::Char('d') => {
                        self.set_pending(Pending::RelationDelete, app);
                        return None;
                    }
                    KeyCode::Char('y') => {
                        return Some(Action::CopyGridCell);
                    }
                    KeyCode::Char('Y') => {
                        return Some(Action::CopyGridRow {
                            include_headers: false,
                        });
                    }
                    _ => {}
                }
            }
            if let Some(action) = map_relation_data(event, app) {
                return Some(action);
            }
        }

        if app.overlay.is_none()
            && event.kind == KeyEventKind::Press
            && app.focus != Focus::Editor
            && event.modifiers != KeyModifiers::NONE
            && !(event.modifiers == KeyModifiers::CONTROL && event.code == KeyCode::Char('w'))
            && let Some((command, sequences)) = crate::input::panes::PaneCommand::ALL
                .into_iter()
                .find_map(|command| {
                    let sequences = self.bindings.configured_sequences(command.name());
                    sequences
                        .iter()
                        .any(|sequence| sequence.first() == Some(&event))
                        .then_some((command, sequences))
                })
        {
            let matching = sequences
                .iter()
                .filter(|sequence| sequence.first() == Some(&event))
                .collect::<Vec<_>>();
            if matching.iter().any(|sequence| sequence.len() == 1) {
                return match crate::input::panes::dispatch(command, app) {
                    crate::input::panes::PaneDispatch::Action(action) => Some(*action),
                    _ => None,
                };
            }
            self.set_pending(
                Pending::PaneSequence {
                    events: vec![event],
                    count: 1,
                },
                app,
            );
            return None;
        }
        if let Some(action) = map_data_query(event, app) {
            return Some(action);
        }

        if app.focus == Focus::Editor
            && matches!(
                app.active_editor_mode(),
                EditorMode::Normal
                    | EditorMode::VisualChar
                    | EditorMode::VisualLine
                    | EditorMode::VisualBlock
            )
            && matches!(event.code, KeyCode::Char(' ' | '\\'))
            && event.modifiers.is_empty()
        {
            self.set_pending(Pending::EditorLeader, app);
            return Some(Action::EditorKey(event));
        }
        if app.focus == Focus::Editor && app.active_editor_mode() == EditorMode::Normal {
            match event.code {
                KeyCode::Char('[') => {
                    self.set_pending(Pending::Previous, app);
                    return None;
                }
                KeyCode::Char(']') => {
                    self.set_pending(Pending::Next, app);
                    return None;
                }
                _ => {}
            }
        }

        if event.modifiers.contains(KeyModifiers::CONTROL) {
            if event.modifiers == KeyModifiers::CONTROL
                && is_principal_overview(app)
                && let Some(action) = map_principal_access_page(event.code)
            {
                return Some(action);
            }
            if let Some(action) = map_configured_navigation(event, app, &self.bindings) {
                return Some(action);
            }
            if event.modifiers == KeyModifiers::CONTROL && tab_shortcuts_have_focus(app) {
                match event.code {
                    KeyCode::PageDown => return Some(Action::NextTab),
                    KeyCode::PageUp => return Some(Action::PreviousTab),
                    _ => {}
                }
            }
            if event.modifiers == KeyModifiers::CONTROL
                && app.focus == Focus::Explorer
                && app.explorer.search.is_none()
            {
                let action = match event.code {
                    KeyCode::Char('d') => Some(Action::ExplorerScrollNodes {
                        direction: 1,
                        amount: crate::model::explorer::ExplorerScrollAmount::HalfPage,
                    }),
                    KeyCode::Char('u') => Some(Action::ExplorerScrollNodes {
                        direction: -1,
                        amount: crate::model::explorer::ExplorerScrollAmount::HalfPage,
                    }),
                    KeyCode::Char('f') => Some(Action::ExplorerScrollNodes {
                        direction: 1,
                        amount: crate::model::explorer::ExplorerScrollAmount::Page,
                    }),
                    KeyCode::Char('b') => Some(Action::ExplorerScrollNodes {
                        direction: -1,
                        amount: crate::model::explorer::ExplorerScrollAmount::Page,
                    }),
                    _ => None,
                };
                if action.is_some() {
                    return action;
                }
            }
            if event.modifiers == KeyModifiers::CONTROL && is_grid_navigation_focus(app) {
                use crate::model::tab::GridScrollAmount;

                let action = match event.code {
                    KeyCode::Char('d') => Some(Action::GridScrollRows {
                        direction: 1,
                        amount: GridScrollAmount::HalfPage,
                    }),
                    KeyCode::Char('u') => Some(Action::GridScrollRows {
                        direction: -1,
                        amount: GridScrollAmount::HalfPage,
                    }),
                    KeyCode::Char('f') => Some(Action::GridScrollRows {
                        direction: 1,
                        amount: GridScrollAmount::Page,
                    }),
                    KeyCode::Char('b') => Some(Action::GridScrollRows {
                        direction: -1,
                        amount: GridScrollAmount::Page,
                    }),
                    _ => None,
                };
                if action.is_some() {
                    return action;
                }
            }
            if event.modifiers == KeyModifiers::CONTROL
                && app.focus == Focus::Editor
                && matches!(
                    app.active_editor_mode(),
                    EditorMode::Normal
                        | EditorMode::VisualChar
                        | EditorMode::VisualLine
                        | EditorMode::VisualBlock
                )
                && matches!(event.code, KeyCode::Char('b' | 'd' | 'f' | 'u'))
            {
                return Some(Action::EditorKey(event));
            }
            if event.modifiers == KeyModifiers::CONTROL
                && app.focus == Focus::Editor
                && app.active_editor_mode() == EditorMode::Normal
                && matches!(event.code, KeyCode::Char('o' | 'i'))
            {
                return Some(Action::EditorKey(event));
            }
            return match event.code {
                KeyCode::Char('u')
                    if app.focus == Focus::Editor
                        && matches!(
                            app.active_editor_mode(),
                            EditorMode::Insert | EditorMode::Replace
                        ) =>
                {
                    Some(Action::EditorKey(event))
                }
                KeyCode::Char('r')
                    if app.focus == Focus::Editor
                        && app.active_editor_mode() == EditorMode::Normal =>
                {
                    Some(Action::EditorKey(event))
                }
                KeyCode::Char('w') if app.focus == Focus::Editor => Some(Action::EditorKey(event)),
                KeyCode::Char('w') => {
                    self.set_pending(Pending::Window { count: 1 }, app);
                    None
                }
                KeyCode::Char('c') => {
                    if matches!(
                        app.tabs.get(app.active_tab),
                        Some(crate::model::tab::WorkspaceTab::Relation(_))
                    ) {
                        Some(Action::CancelActiveRelationRequest)
                    } else if app.active_console_opt().is_some_and(|tab| {
                        tab.query_status == crate::model::workspace::QueryStatus::Running
                    }) {
                        Some(Action::CancelActiveQuery)
                    } else if app.focus == Focus::Editor
                        && app.active_editor_mode() == EditorMode::Insert
                    {
                        Some(Action::EditorKey(event))
                    } else {
                        None
                    }
                }
                KeyCode::Char('h') if app.focus == Focus::Editor => Some(Action::EditorKey(event)),
                KeyCode::Char(' ')
                    if app.focus == Focus::Editor
                        && app.active_editor_mode() == EditorMode::Insert =>
                {
                    Some(Action::CompletionExplicit)
                }
                _ => None,
            };
        }
        if self.bindings.matches("run-buffer", event) {
            return Some(Action::RunAllSql);
        }
        if self.bindings.matches("run-statement", event) {
            return Some(Action::RunActiveSql);
        }
        if self.bindings.matches("help", event)
            && !(app.focus == Focus::Editor
                && matches!(
                    app.active_editor_mode(),
                    EditorMode::Insert | EditorMode::Replace
                )
                && matches!(event.code, KeyCode::Char(_)))
        {
            return Some(Action::ShowHelp);
        }
        if matches!(
            app.tabs.get(app.active_tab),
            Some(crate::model::tab::WorkspaceTab::MongoConsole(_))
        ) && event.modifiers.contains(KeyModifiers::ALT)
        {
            self.clear_pending();
            return match event.code {
                KeyCode::Char('b') => Some(Action::MongoTransactionBegin),
                KeyCode::Char('c') => Some(Action::MongoTransactionCommit),
                KeyCode::Char('a') => Some(Action::MongoTransactionAbort),
                _ => None,
            };
        }
        if app.focus == Focus::Editor {
            return Some(Action::EditorKey(event));
        }

        let redis_table_preview = matches!(
            app.tabs.get(app.active_tab),
            Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
                if app.focus == Focus::Results
                    && tab.focus == crate::model::redis_browser::RedisBrowserFocus::Preview
                    && tab.format.view() == crate::value_preview::ValueView::Table
        );
        match event.code {
            KeyCode::Char('?') => return Some(Action::ShowHelp),
            KeyCode::Char(' ') => {
                self.set_pending(Pending::Leader, app);
                return None;
            }
            KeyCode::Char('[') if !redis_table_preview => {
                self.set_pending(Pending::Previous, app);
                return None;
            }
            KeyCode::Char('g')
                if app.focus != Focus::Editor
                    && !is_ddl_only_focus(app)
                    && !is_relation_data_focus(app) =>
            {
                if matches!(
                    app.tabs.get(app.active_tab),
                    Some(crate::model::tab::WorkspaceTab::Dashboard(tab))
                        if tab.page == crate::model::dashboard::DashboardPage::Processes
                            && tab.process_filter_active
                ) {
                    return None;
                }
                self.set_pending(Pending::Goto, app);
                return None;
            }
            KeyCode::Char(']') if !redis_table_preview => {
                self.set_pending(Pending::Next, app);
                return None;
            }
            _ => {}
        }

        let relation_tab = matches!(
            app.tabs.get(app.active_tab),
            Some(crate::model::tab::WorkspaceTab::Relation(_))
        );
        let dashboard_tab = matches!(
            app.tabs.get(app.active_tab),
            Some(crate::model::tab::WorkspaceTab::Dashboard(_))
        );
        if dashboard_tab && app.focus == Focus::Results {
            let process_page = matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::Dashboard(tab))
                    if tab.page == crate::model::dashboard::DashboardPage::Processes
            );
            let filter_active = matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::Dashboard(tab))
                    if tab.process_filter_active
            );
            if process_page
                && !filter_active
                && event.modifiers.is_empty()
                && event.code == KeyCode::Char('/')
            {
                return Some(Action::DashboardProcessFilterStart);
            }
            if process_page && filter_active {
                if let Some(edit) = map_text_input_edit(event) {
                    return Some(match edit {
                        TextInputEdit::Insert(value) => Action::DashboardProcessFilterInsert(value),
                        TextInputEdit::Backspace => Action::DashboardProcessFilterBackspace,
                        TextInputEdit::DeletePreviousWord => {
                            Action::DashboardProcessFilterDeletePreviousWord
                        }
                        TextInputEdit::DeleteToStart => Action::DashboardProcessFilterDeleteToStart,
                        TextInputEdit::Clear => Action::DashboardProcessFilterDeleteToStart,
                        TextInputEdit::Delete => Action::DashboardProcessFilterDelete,
                        TextInputEdit::MoveLeft => Action::DashboardProcessFilterMoveLeft,
                        TextInputEdit::MoveRight => Action::DashboardProcessFilterMoveRight,
                        TextInputEdit::MoveHome => Action::DashboardProcessFilterMoveHome,
                        TextInputEdit::MoveEnd => Action::DashboardProcessFilterMoveEnd,
                        TextInputEdit::Undo => Action::DashboardProcessFilterUndo,
                        TextInputEdit::Redo => Action::DashboardProcessFilterRedo,
                    });
                }
                return match event.code {
                    KeyCode::Enter if event.modifiers.is_empty() => {
                        Some(Action::DashboardProcessFilterCommit)
                    }
                    KeyCode::Esc if event.modifiers.is_empty() => {
                        Some(Action::DashboardProcessFilterCancel)
                    }
                    _ => None,
                };
            }
            let dashboard_action = match event.code {
                KeyCode::Char('o') => Some(Action::DashboardSetPage(
                    match app.tabs.get(app.active_tab) {
                        Some(crate::model::tab::WorkspaceTab::Dashboard(tab))
                            if tab.redis_details.is_some()
                                && tab.page == crate::model::dashboard::DashboardPage::Info =>
                        {
                            crate::model::dashboard::DashboardPage::Overview
                        }
                        Some(crate::model::tab::WorkspaceTab::Dashboard(tab))
                            if tab.redis_details.is_some() =>
                        {
                            crate::model::dashboard::DashboardPage::Info
                        }
                        Some(crate::model::tab::WorkspaceTab::Dashboard(tab))
                            if tab.page == crate::model::dashboard::DashboardPage::Processes =>
                        {
                            crate::model::dashboard::DashboardPage::Overview
                        }
                        _ => crate::model::dashboard::DashboardPage::Processes,
                    },
                )),
                KeyCode::Char('r') => Some(Action::DashboardRefresh),
                KeyCode::Char('p') => Some(Action::DashboardTogglePolling),
                _ => None,
            };
            if dashboard_action.is_some() {
                return dashboard_action;
            }
            if matches!(
                event.code,
                KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown
            ) && matches!(app.tabs.get(app.active_tab), Some(crate::model::tab::WorkspaceTab::Dashboard(tab)) if tab.page == crate::model::dashboard::DashboardPage::Info)
            {
                return Some(Action::DashboardInfoScroll(match event.code {
                    KeyCode::Up => -1,
                    KeyCode::Down => 1,
                    KeyCode::PageUp => -10,
                    KeyCode::PageDown => 10,
                    _ => 0,
                }));
            }
        }
        if relation_tab
            && app.focus == Focus::Results
            && matches!(event.code, KeyCode::Char('o' | 'p' | 'D' | 'r'))
        {
            return map_relation(event.code, app);
        }
        if relation_tab && app.focus == Focus::Results {
            if let Some(session_id) = ddl_only_session_id(app)
                && is_read_only_editor_key(event)
            {
                return Some(Action::ReadOnlyEditorKey { session_id, event });
            }
            if let Some(action) = map_configured_navigation(event, app, &self.bindings) {
                return Some(action);
            }
            return map_relation(event.code, app);
        }
        if app.focus == Focus::Results
            && matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::PrincipalDdl(_))
            )
        {
            if matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::PrincipalDdl(tab))
                    if tab.view == crate::model::principal::PrincipalView::Overview
            ) && self.bindings.matches("results-copy-cell", event)
            {
                return Some(Action::CopyPrincipalAccess);
            }
            if matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::PrincipalDdl(tab))
                    if tab.view == crate::model::principal::PrincipalView::Overview
            ) && event.code == KeyCode::Char('G')
                && (event.modifiers.is_empty() || event.modifiers == KeyModifiers::SHIFT)
            {
                return Some(Action::PrincipalAccessSelectLast);
            }
            if event.modifiers.is_empty()
                && matches!(
                        app.tabs.get(app.active_tab),
                        Some(crate::model::tab::WorkspaceTab::PrincipalDdl(tab))
                            if tab.view == crate::model::principal::PrincipalView::Overview
                )
            {
                if event.code == KeyCode::Char('/') {
                    return Some(Action::PrincipalAccessFindOpen);
                }
                if event.code == KeyCode::Char('?') {
                    return Some(Action::ShowHelp);
                }
                if let Some(action) = map_configured_navigation(event, app, &self.bindings) {
                    return Some(action);
                }
                match event.code {
                    KeyCode::Char('j') => return Some(Action::MovePrincipalAccess(1)),
                    KeyCode::Char('k') => return Some(Action::MovePrincipalAccess(-1)),
                    KeyCode::Down => return Some(Action::MovePrincipalAccess(1)),
                    KeyCode::Up => return Some(Action::MovePrincipalAccess(-1)),
                    _ => {}
                }
                match event.code {
                    KeyCode::Char('v') | KeyCode::Enter => {
                        return Some(Action::OpenPrincipalAccessDetails);
                    }
                    KeyCode::Char('y') => return Some(Action::CopyPrincipalAccess),
                    KeyCode::Char('e') => {
                        return Some(Action::OpenPrincipalPermissionMutation { grant: true });
                    }
                    _ => {}
                }
            }
            if event.modifiers.is_empty() && event.code == KeyCode::Char('o') {
                let Some(crate::model::tab::WorkspaceTab::PrincipalDdl(tab)) =
                    app.tabs.get(app.active_tab)
                else {
                    return None;
                };
                return Some(Action::SetPrincipalView(match tab.view {
                    crate::model::principal::PrincipalView::Overview => {
                        crate::model::principal::PrincipalView::Ddl
                    }
                    crate::model::principal::PrincipalView::Ddl => {
                        crate::model::principal::PrincipalView::Overview
                    }
                }));
            }
            if matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::PrincipalDdl(tab))
                    if tab.view == crate::model::principal::PrincipalView::Ddl
            ) && let Some(session_id) = ddl_only_session_id(app)
                && is_read_only_editor_key(event)
            {
                return Some(Action::ReadOnlyEditorKey { session_id, event });
            }
            if let Some(action) = map_configured_navigation(event, app, &self.bindings) {
                return Some(action);
            }
            if event.modifiers.is_empty() && event.code == KeyCode::Char('r') {
                return Some(Action::RefreshActivePrincipal);
            }
            return None;
        }
        if app.focus == Focus::Results
            && matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::RedisBrowser(_))
            )
        {
            if event.modifiers.is_empty()
                && event.code == KeyCode::Char('/')
                && is_redis_table_grid_focus(app)
            {
                let tab_id = app.tabs[app.active_tab].id();
                return Some(Action::RedisValueFilterFocus { tab_id });
            }
            if matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
                    if tab.focus == crate::model::redis_browser::RedisBrowserFocus::Preview
                        && tab.format.view() == crate::value_preview::ValueView::Table
                        && tab.value_filter.editing
            ) {
                if let Some(edit) = map_text_input_edit(event) {
                    return Some(Action::RedisValueFilterEdit(edit));
                }
                return match event.code {
                    KeyCode::Enter if event.modifiers.is_empty() => {
                        Some(Action::RedisValueFilterSubmit)
                    }
                    KeyCode::Esc if event.modifiers.is_empty() => {
                        Some(Action::RedisValueFilterCancel)
                    }
                    _ => None,
                };
            }
            if let Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab)) =
                app.tabs.get(app.active_tab)
                && let Some(find) = tab.find.as_ref()
            {
                if find.phase == crate::model::redis_browser::RedisFindPhase::Editing {
                    match event.code {
                        KeyCode::Esc => return Some(Action::RedisFindCancel),
                        KeyCode::Enter => return Some(Action::RedisFindConfirm),
                        KeyCode::Backspace => return Some(Action::RedisFindBackspace),
                        KeyCode::Delete => return Some(Action::RedisFindDelete),
                        KeyCode::Char(character) if event.modifiers.is_empty() => {
                            return Some(Action::RedisFindInsert(character));
                        }
                        _ => return None,
                    }
                }
                if find.phase == crate::model::redis_browser::RedisFindPhase::Confirmed {
                    match event.code {
                        KeyCode::Esc => return Some(Action::RedisFindCancel),
                        KeyCode::Char('n') => return Some(Action::RedisFindNext),
                        KeyCode::Char('N') => return Some(Action::RedisFindPrevious),
                        _ => {}
                    }
                }
            }
            if event.modifiers.is_empty() && event.code == KeyCode::Char('/') {
                return Some(Action::RedisFindOpen);
            }
            if event.modifiers.is_empty() && event.code == KeyCode::Char('f') {
                return Some(Action::RedisPreviewCycleFormat);
            }
            if event.modifiers.is_empty()
                && event.code == KeyCode::Char('W')
                && !matches!(
                    app.tabs.get(app.active_tab),
                    Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
                        if tab.format.view() == crate::value_preview::ValueView::Table
                )
            {
                return Some(Action::RedisPreviewToggleWrap);
            }
            if let Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab)) =
                app.tabs.get(app.active_tab)
                && tab.focus == crate::model::redis_browser::RedisBrowserFocus::Preview
                && tab.format.view() == crate::value_preview::ValueView::Table
            {
                if event.modifiers.is_empty() {
                    match event.code {
                        KeyCode::Char('e') => return Some(Action::RedisPreviewEdit),
                        KeyCode::Char('a') => return Some(Action::RedisPreviewAdd),
                        KeyCode::Char('d') => {
                            self.set_pending(Pending::RedisPreviewLeader, app);
                            return None;
                        }
                        _ => {}
                    }
                }
                return map_results(event.code, app);
            }
            if let Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab)) =
                app.tabs.get(app.active_tab)
                && tab.focus == crate::model::redis_browser::RedisBrowserFocus::Preview
                && is_read_only_editor_key(event)
            {
                return Some(Action::ReadOnlyEditorKey {
                    session_id: tab.preview_editor_id,
                    event,
                });
            }
            if event.modifiers.is_empty() && event.code == KeyCode::Char(']') {
                return Some(Action::RedisPreviewLoadNext);
            }
            if event.code == KeyCode::Char('h') || event.code == KeyCode::Left {
                return Some(Action::RedisFocusPane(
                    crate::model::redis_browser::RedisBrowserFocus::Keys,
                ));
            }
            if event.code == KeyCode::Char('l') || event.code == KeyCode::Right {
                return Some(Action::RedisFocusPane(
                    crate::model::redis_browser::RedisBrowserFocus::Preview,
                ));
            }
        }
        if let Some(crate::model::tab::WorkspaceTab::Sql(tab)) = app.tabs.get(app.active_tab)
            && app.focus == Focus::Results
            && matches!(
                tab.result_view,
                crate::model::tab::ResultView::Output | crate::model::tab::ResultView::Plan
            )
        {
            return Some(Action::ReadOnlyEditorKey {
                session_id: tab.output_editor_id,
                event,
            });
        }
        if let Some(action) = map_configured_navigation(event, app, &self.bindings) {
            return Some(action);
        }
        match app.focus {
            Focus::Explorer => map_explorer(event.code, app),
            Focus::Editor => None,
            Focus::Results => map_results(event.code, app),
        }
    }

    fn set_pending(&mut self, pending: Pending, app: &App) {
        self.sequence_selected = 0;
        self.continue_pending(
            pending,
            app.focus,
            app.active_editor_mode(),
            app.tabs
                .get(app.active_tab)
                .map_or(Uuid::nil(), |tab| tab.id()),
            matches!(app.overlay, Some(Overlay::RecordView(_))),
        );
    }

    fn continue_pending(
        &mut self,
        pending: Pending,
        focus: Focus,
        editor_mode: EditorMode,
        tab_id: Uuid,
        record_view_active: bool,
    ) {
        self.pending = Some(PendingState {
            pending,
            focus,
            editor_mode,
            tab_id,
            generation: self.generation,
            record_view_active,
            started_at: Instant::now(),
        });
    }

    pub fn clear_pending(&mut self) {
        self.pending = None;
        self.sequence_selected = 0;
    }

    pub(crate) fn set_sequence_width(&mut self, width: u16) {
        self.sequence_width = width;
    }

    fn observe_state(&mut self, app: &App) {
        let state = KeymapContext::from_app(app);
        if self.observed != Some(state) {
            if self.observed.is_some() {
                self.generation = self.generation.wrapping_add(1);
                self.clear_pending();
            }
            self.observed = Some(state);
        }
    }

    pub fn sequence_state(&mut self, app: &App, now: Instant) -> Option<KeySequenceState> {
        self.observe_state(app);
        let pending = self.pending.as_ref()?;
        if !pending_is_valid(pending, app, now, self.generation, self.sequence_timeout) {
            self.pending = None;
            return None;
        }
        let pending = &pending.pending;
        let (prefix, display) = pending_display(pending.clone())?;
        let candidates = if matches!(
            pending,
            Pending::Window { .. } | Pending::PaneSequence { .. }
        ) {
            crate::input::panes::PaneCommand::ALL
                .into_iter()
                .flat_map(|command| {
                    self.bindings
                        .configured_sequences(command.name())
                        .iter()
                        .filter(|sequence| sequence.len() > 1)
                        .filter(|sequence| {
                            if let Pending::PaneSequence { events, .. } = pending {
                                sequence.starts_with(events)
                            } else {
                                true
                            }
                        })
                        .map(|sequence| {
                            let offset = if let Pending::PaneSequence { events, .. } = pending {
                                events.len()
                            } else {
                                1
                            };
                            let suffix = sequence[offset..]
                                .iter()
                                .map(crate::config::keybinding_display)
                                .collect::<Vec<_>>()
                                .join(" ");
                            (command.name().to_owned(), suffix)
                        })
                        .collect::<Vec<_>>()
                })
                .collect()
        } else {
            Vec::new()
        };
        Some(KeySequenceState {
            prefix,
            display,
            selected: self.sequence_selected,
            candidates,
        })
    }

    pub fn expire_pending(&mut self, app: &App, now: Instant) -> bool {
        self.observe_state(app);
        let Some(pending) = self.pending.as_ref() else {
            return false;
        };
        if pending_is_valid(pending, app, now, self.generation, self.sequence_timeout) {
            return false;
        }
        let was_visible = pending_display(pending.pending.clone()).is_some();
        self.pending = None;
        was_visible
    }
}

fn tab_shortcuts_have_focus(app: &App) -> bool {
    matches!(app.focus, Focus::Editor | Focus::Results) && app.tabs.get(app.active_tab).is_some()
}

fn matches_tab_binding(
    bindings: &crate::config::KeyBindings,
    command: &str,
    event: KeyEvent,
) -> bool {
    if bindings.matches(command, event) {
        return true;
    }
    if event.modifiers != (KeyModifiers::CONTROL | KeyModifiers::SHIFT) {
        return false;
    }
    let KeyCode::Char(character) = event.code else {
        return false;
    };
    let alternate = if character.is_ascii_lowercase() {
        character.to_ascii_uppercase()
    } else if character.is_ascii_uppercase() {
        character.to_ascii_lowercase()
    } else {
        return false;
    };
    bindings.matches(
        command,
        KeyEvent::new(KeyCode::Char(alternate), event.modifiers),
    )
}

fn map_configured_navigation(
    event: KeyEvent,
    app: &App,
    bindings: &crate::config::KeyBindings,
) -> Option<Action> {
    if app.focus == Focus::Explorer {
        if bindings.matches("explorer-create", event)
            && let Some(crate::model::explorer::ExplorerNodeId::Mongo {
                profile_id: _,
                node,
            }) = app.explorer.selected_id()
        {
            let operation = match node {
                crate::db::mongodb::types::MongoExplorerNodeId::Database(database) => Some(
                    crate::model::mongodb_management::MongoManagementOperation::CreateCollection {
                        database: database.clone(),
                    },
                ),
                crate::db::mongodb::types::MongoExplorerNodeId::Collection(namespace) => Some(
                    crate::model::mongodb_management::MongoManagementOperation::CreateIndex {
                        namespace: namespace.clone(),
                    },
                ),
                _ => None,
            };
            if let Some(operation) = operation {
                return Some(Action::MongoManagementOpen(operation));
            }
        }
        if bindings.matches("explorer-edit", event) {
            if let Some(crate::model::explorer::ExplorerNodeId::Mongo {
                node: crate::db::mongodb::types::MongoExplorerNodeId::Database(database),
                ..
            }) = app.explorer.selected_id()
            {
                return Some(Action::MongoManagementOpen(
                    crate::model::mongodb_management::MongoManagementOperation::CreateView {
                        database: database.clone(),
                    },
                ));
            }
            if let Some(crate::model::explorer::ExplorerNodeId::Mongo {
                profile_id: _,
                node: crate::db::mongodb::types::MongoExplorerNodeId::Collection(namespace),
            }) = app.explorer.selected_id()
            {
                return Some(Action::MongoManagementOpen(
                    crate::model::mongodb_management::MongoManagementOperation::CollMod {
                        namespace: namespace.clone(),
                    },
                ));
            }
            if let Some(crate::model::explorer::ExplorerNodeId::Mongo {
                node: crate::db::mongodb::types::MongoExplorerNodeId::Index { namespace, name },
                ..
            }) = app.explorer.selected_id()
            {
                return Some(Action::MongoManagementOpen(
                    crate::model::mongodb_management::MongoManagementOperation::DropIndex {
                        namespace: namespace.clone(),
                        name: name.clone(),
                    },
                ));
            }
        }
        if bindings.matches("explorer-rename", event)
            && let Some(crate::model::explorer::ExplorerNodeId::Mongo {
                node: crate::db::mongodb::types::MongoExplorerNodeId::Collection(namespace),
                ..
            }) = app.explorer.selected_id()
        {
            return Some(Action::MongoManagementOpen(
                crate::model::mongodb_management::MongoManagementOperation::RenameCollection {
                    namespace: namespace.clone(),
                },
            ));
        }
        if bindings.matches("explorer-delete", event) {
            if let Some(crate::model::explorer::ExplorerNodeId::Mongo {
                profile_id,
                node: crate::db::mongodb::types::MongoExplorerNodeId::Collection(namespace),
            }) = app.explorer.selected_id()
            {
                let kind = app
                    .explorer
                    .normalized
                    .profiles
                    .get(profile_id)
                    .and_then(|p| p.mongodb_collections.get(&namespace.database))
                    .and_then(|items| items.iter().find(|item| item.namespace == *namespace))
                    .map(|item| item.collection_type)
                    .unwrap_or(crate::db::mongodb::types::MongoCollectionType::Collection);
                return Some(Action::MongoManagementOpen(
                    crate::model::mongodb_management::MongoManagementOperation::DropCollection {
                        namespace: namespace.clone(),
                        kind,
                    },
                ));
            }
            if let Some(crate::model::explorer::ExplorerNodeId::Mongo {
                node: crate::db::mongodb::types::MongoExplorerNodeId::Index { namespace, name },
                ..
            }) = app.explorer.selected_id()
            {
                return Some(Action::MongoManagementOpen(
                    crate::model::mongodb_management::MongoManagementOperation::DropIndex {
                        namespace: namespace.clone(),
                        name: name.clone(),
                    },
                ));
            }
        }
        if bindings.matches("explorer-copy-selection", event) {
            return Some(Action::CopyExplorerSelection);
        }
        if bindings.matches("explorer-find", event) {
            return Some(Action::ExplorerFindOpen);
        }
        if bindings.matches("explorer-search", event) {
            return Some(Action::ExplorerSearchOpen);
        }
        if bindings.matches("explorer-new-profile", event) {
            return Some(Action::ProfileStartNew);
        }
        if bindings.matches("explorer-refresh", event) {
            return Some(Action::ExplorerRefresh);
        }
        if bindings.matches("explorer-toggle", event) {
            return Some(Action::ExplorerToggle);
        }
        if bindings.matches("explorer-move-down", event) {
            return Some(Action::ExplorerMove(1));
        }
        if bindings.matches("explorer-move-up", event) {
            return Some(Action::ExplorerMove(-1));
        }
        if let Some(crate::model::explorer::ExplorerNodeId::Mongo {
            node: crate::db::mongodb::types::MongoExplorerNodeId::Database(database),
            ..
        }) = app.explorer.selected_id()
        {
            let database = database.clone();
            match event.code {
                KeyCode::Char('m') => {
                    return Some(Action::OpenMongoNode(
                        crate::db::mongodb::types::MongoExplorerNodeId::Monitor(database),
                    ));
                }
                KeyCode::Char('u') => {
                    return Some(Action::OpenMongoNode(
                        crate::db::mongodb::types::MongoExplorerNodeId::Users(database),
                    ));
                }
                _ => {}
            }
        }
        if bindings.matches("explorer-expand", event) {
            return Some(Action::ExplorerExpand);
        }
        if bindings.matches("explorer-collapse", event) {
            return Some(Action::ExplorerCollapse);
        }
    }
    if app.focus == Focus::Results && !app.is_active_relation_tab() {
        if matches!(
            app.tabs.get(app.active_tab),
            Some(crate::model::tab::WorkspaceTab::PrincipalDdl(tab))
                if tab.view == crate::model::principal::PrincipalView::Overview
        ) {
            if bindings.matches("results-copy-cell", event) {
                return Some(Action::CopyPrincipalAccess);
            }
            if bindings.matches("results-move-left", event) {
                return Some(Action::SelectPrincipalAccess(
                    app.active_principal_access_section().next(-1),
                ));
            }
            if bindings.matches("results-move-right", event) {
                return Some(Action::SelectPrincipalAccess(
                    app.active_principal_access_section().next(1),
                ));
            }
            if bindings.matches("results-move-down", event) {
                return Some(Action::MovePrincipalAccess(1));
            }
            if bindings.matches("results-move-up", event) {
                return Some(Action::MovePrincipalAccess(-1));
            }
        }
        if let Some(action) = map_configured_pagination(event, false, app, bindings) {
            return Some(action);
        }
        if bindings.matches("results-open-record", event)
            && app.active_grid_dimensions_for_input().0 > 0
            && app.active_grid_dimensions_for_input().1 > 0
        {
            return Some(Action::OpenRecordView);
        }
        if bindings.matches("results-copy-cell", event) {
            return Some(Action::CopyGridCell);
        }
        if bindings.matches("results-copy-row", event) {
            return Some(Action::CopyGridRow {
                include_headers: false,
            });
        }
        if bindings.matches("results-toggle-view", event) {
            return Some(Action::ToggleResultView);
        }
        if bindings.matches("results-first-column", event) {
            return Some(Action::GridSelectColumn(
                crate::model::tab::GridColumnTarget::First,
            ));
        }
        if bindings.matches("results-last-column", event) {
            return Some(Action::GridSelectColumn(
                crate::model::tab::GridColumnTarget::Last,
            ));
        }
        if bindings.matches("results-move-left", event) {
            return Some(Action::GridMove {
                rows: 0,
                columns: -1,
            });
        }
        if bindings.matches("results-move-down", event) {
            return Some(Action::GridMove {
                rows: 1,
                columns: 0,
            });
        }
        if bindings.matches("results-move-up", event) {
            return Some(Action::GridMove {
                rows: -1,
                columns: 0,
            });
        }
        if bindings.matches("results-move-right", event) {
            return Some(Action::GridMove {
                rows: 0,
                columns: 1,
            });
        }
    }
    if app.focus == Focus::Results
        && app.is_active_relation_tab()
        && let Some(action) = map_configured_pagination(event, true, app, bindings)
    {
        return Some(action);
    }
    None
}

fn is_principal_overview(app: &App) -> bool {
    matches!(
        app.tabs.get(app.active_tab),
        Some(crate::model::tab::WorkspaceTab::PrincipalDdl(tab))
            if tab.view == crate::model::principal::PrincipalView::Overview
    ) && app.focus == Focus::Results
}

fn map_principal_access_page(code: KeyCode) -> Option<Action> {
    let (direction, half_page) = match code {
        KeyCode::Char('f') => (1, false),
        KeyCode::Char('b') => (-1, false),
        KeyCode::Char('d') => (1, true),
        KeyCode::Char('u') => (-1, true),
        _ => return None,
    };
    Some(Action::PagePrincipalAccess {
        direction,
        half_page,
    })
}

fn map_configured_pagination(
    event: KeyEvent,
    relation: bool,
    app: &App,
    bindings: &crate::config::KeyBindings,
) -> Option<Action> {
    let action = |result: Action, relation_action: Action| {
        if relation { relation_action } else { result }
    };
    [
        (
            "results-first-page",
            Action::ResultFirstPage,
            Action::RelationFirstPage,
        ),
        (
            "results-previous-page",
            Action::ResultPreviousPage,
            Action::RelationPreviousPage,
        ),
        (
            "results-next-page",
            Action::ResultNextPage,
            Action::RelationNextPage,
        ),
        (
            "results-last-page",
            Action::ResultLastPage,
            Action::RelationLastPage,
        ),
    ]
    .into_iter()
    .find_map(|(command, result, relation_action)| {
        bindings
            .matches(command, event)
            .then(|| action(result, relation_action))
    })
    .or_else(|| {
        bindings
            .matches("results-page-size", event)
            .then_some(Action::OpenPageSizeSelector { relation })
    })
    .filter(|_| {
        if relation {
            matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::Relation(tab))
                    if tab.view == crate::model::relation::RelationView::Data
                        && tab.query.focus.is_none()
            )
        } else {
            matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::Sql(tab))
                    if tab.result_view == crate::model::tab::ResultView::Data
            )
        }
    })
}

fn map_catalog_editor(event: KeyEvent, app: &App) -> Option<Action> {
    let editor = app.catalog_editor.as_ref()?;
    if editor.is_busy() {
        let applying = matches!(
            editor.operation,
            Some(crate::model::catalog_editor::CatalogEditorOperation::Applying { .. })
        );
        return match event.code {
            KeyCode::Esc if !applying => Some(Action::CatalogEditorCancel),
            _ => None,
        };
    }
    match editor.page {
        crate::model::catalog_editor::CatalogEditorPage::ObjectPicker => match event.code {
            KeyCode::Up | KeyCode::Char('k') => Some(Action::CatalogEditorMove(-1)),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::CatalogEditorMove(1)),
            KeyCode::Enter => Some(Action::CatalogEditorSelect),
            KeyCode::Esc | KeyCode::Char('q') => Some(Action::CatalogEditorCancel),
            _ => None,
        },
        crate::model::catalog_editor::CatalogEditorPage::Form if editor.owner_picker_active() => {
            if is_text_redo(event) {
                return Some(Action::CatalogOwnerPickerRedo);
            }
            if is_text_undo(event) {
                return Some(Action::CatalogOwnerPickerUndo);
            }
            match event.code {
                KeyCode::Esc => Some(Action::CatalogOwnerPickerClose),
                KeyCode::Tab => Some(Action::CatalogEditorFieldNext),
                KeyCode::BackTab => Some(Action::CatalogEditorFieldPrevious),
                KeyCode::Up => Some(Action::CatalogOwnerPickerMove(-1)),
                KeyCode::Down => Some(Action::CatalogOwnerPickerMove(1)),
                KeyCode::Enter => Some(Action::CatalogOwnerPickerAccept),
                KeyCode::Backspace => Some(Action::CatalogOwnerPickerBackspace),
                KeyCode::Char('w') if event.modifiers == KeyModifiers::CONTROL => {
                    Some(Action::CatalogOwnerPickerDeletePreviousWord)
                }
                KeyCode::Char('u') if event.modifiers == KeyModifiers::CONTROL => {
                    Some(Action::CatalogOwnerPickerDeleteToStart)
                }
                KeyCode::Char(character) if event.modifiers.is_empty() => {
                    Some(Action::CatalogOwnerPickerInsert(character))
                }
                _ => None,
            }
        }
        crate::model::catalog_editor::CatalogEditorPage::Form => {
            // The owner field only claims the keys that open or filter the role list; every
            // other key falls through to the shared form keymap so focus never gets stuck.
            if editor.owner_field_focused() && app.catalog_owner_choices().is_some() {
                match event.code {
                    KeyCode::Enter => return Some(Action::CatalogOwnerPickerOpen),
                    KeyCode::Char(character) if event.modifiers.is_empty() => {
                        return Some(Action::CatalogOwnerPickerInsert(character));
                    }
                    _ => {}
                }
            }
            map_catalog_editor_form(event, editor)
        }
        crate::model::catalog_editor::CatalogEditorPage::SqlPreview => match event.code {
            KeyCode::Up => Some(Action::CatalogEditorPreviewScroll(-1)),
            KeyCode::Down => Some(Action::CatalogEditorPreviewScroll(1)),
            KeyCode::PageUp => Some(Action::CatalogEditorPreviewScroll(-10)),
            KeyCode::PageDown => Some(Action::CatalogEditorPreviewScroll(10)),
            KeyCode::Home => Some(Action::CatalogEditorPreviewHome),
            KeyCode::End => Some(Action::CatalogEditorPreviewEnd),
            KeyCode::Enter => Some(Action::CatalogEditorApply),
            KeyCode::Esc => Some(Action::CatalogEditorBack),
            _ => None,
        },
        crate::model::catalog_editor::CatalogEditorPage::Loading => {
            (event.code == KeyCode::Esc).then_some(Action::CatalogEditorCancel)
        }
    }
}

fn map_catalog_editor_with_pending(
    keymap: &mut Keymap,
    event: KeyEvent,
    app: &App,
) -> Option<Action> {
    let valid_column_delete = keymap.pending.as_ref().is_some_and(|pending| {
        matches!(pending.pending, Pending::CatalogColumnDelete { .. })
            && pending_is_valid(
                pending,
                app,
                Instant::now(),
                keymap.generation,
                keymap.sequence_timeout,
            )
    });
    if valid_column_delete {
        if event.kind == KeyEventKind::Press
            && event.modifiers.is_empty()
            && event.code == KeyCode::Char('d')
        {
            keymap.pending = None;
            return Some(Action::CatalogEditorRemoveTableColumn);
        }
        keymap.pending = None;
    } else {
        keymap.pending = None;
    }

    if event.kind == KeyEventKind::Press
        && event.modifiers.is_empty()
        && event.code == KeyCode::Char('d')
        && is_catalog_columns_focus(app)
    {
        let selected_column = catalog_selected_column(app).unwrap_or_default();
        keymap.continue_pending(
            Pending::CatalogColumnDelete { selected_column },
            app.focus,
            app.active_editor_mode(),
            app.tabs
                .get(app.active_tab)
                .map_or(Uuid::nil(), |tab| tab.id()),
            false,
        );
        return None;
    }

    map_catalog_editor(event, app)
}

fn is_catalog_columns_focus(app: &App) -> bool {
    matches!(
        app.catalog_editor.as_ref().and_then(|editor| editor.draft.as_ref()),
        Some(crate::model::catalog_editor::CatalogDraft::Table(draft))
            if draft.focus == crate::model::catalog_editor::TableEditorFocus::Columns
                && draft.column_editor.is_none()
    )
}

fn catalog_selected_column(app: &App) -> Option<usize> {
    match app.catalog_editor.as_ref()?.draft.as_ref()? {
        crate::model::catalog_editor::CatalogDraft::Table(draft) => Some(draft.selected_column),
        _ => None,
    }
}

fn map_redis_object_editor(
    event: KeyEvent,
    editor: &crate::model::redis_object_editor::RedisObjectEditorState,
) -> Option<Action> {
    if editor.busy {
        return (event.code == KeyCode::Esc).then_some(Action::RedisObjectEditorCancel);
    }
    if is_text_redo(event) {
        return Some(Action::RedisObjectEditorRedo);
    }
    if is_text_undo(event) {
        return Some(Action::RedisObjectEditorUndo);
    }
    use crate::model::redis_object_editor::RedisObjectEditorFocus;
    let text_focus = matches!(
        editor.focus,
        RedisObjectEditorFocus::Value | RedisObjectEditorFocus::Ttl
    ) || (editor.mode == crate::db::redis::mutation::RedisMutationMode::Create
        && editor.focus == RedisObjectEditorFocus::Key);
    match event.code {
        KeyCode::Esc => Some(Action::RedisObjectEditorCancel),
        KeyCode::Tab | KeyCode::Down => Some(Action::RedisObjectEditorFocusNext),
        KeyCode::BackTab | KeyCode::Up => Some(Action::RedisObjectEditorFocusPrevious),
        KeyCode::Enter => Some(Action::RedisObjectEditorApply),
        KeyCode::Left
            if editor.focus == RedisObjectEditorFocus::Type
                || editor.focus == RedisObjectEditorFocus::Ttl =>
        {
            Some(if editor.focus == RedisObjectEditorFocus::Type {
                Action::RedisObjectEditorCycleType(-1)
            } else {
                Action::RedisObjectEditorCycleTtl(-1)
            })
        }
        KeyCode::Right
            if editor.focus == RedisObjectEditorFocus::Type
                || editor.focus == RedisObjectEditorFocus::Ttl =>
        {
            Some(if editor.focus == RedisObjectEditorFocus::Type {
                Action::RedisObjectEditorCycleType(1)
            } else {
                Action::RedisObjectEditorCycleTtl(1)
            })
        }
        KeyCode::Char(' ') if editor.focus == RedisObjectEditorFocus::Type => {
            Some(Action::RedisObjectEditorCycleType(1))
        }
        KeyCode::Char(' ') if editor.focus == RedisObjectEditorFocus::Ttl => {
            Some(Action::RedisObjectEditorCycleTtl(1))
        }
        KeyCode::Char(character) if event.modifiers.is_empty() && text_focus => {
            Some(Action::RedisObjectEditorInsert(character))
        }
        KeyCode::Backspace if text_focus => Some(Action::RedisObjectEditorBackspace),
        KeyCode::Delete if text_focus => Some(Action::RedisObjectEditorDelete),
        KeyCode::Left if text_focus => Some(Action::RedisObjectEditorMoveLeft),
        KeyCode::Right if text_focus => Some(Action::RedisObjectEditorMoveRight),
        KeyCode::Home if text_focus => Some(Action::RedisObjectEditorMoveHome),
        KeyCode::End if text_focus => Some(Action::RedisObjectEditorMoveEnd),
        KeyCode::Char('w') if event.modifiers == KeyModifiers::CONTROL && text_focus => {
            Some(Action::RedisObjectEditorDeletePreviousWord)
        }
        KeyCode::Char('u') if event.modifiers == KeyModifiers::CONTROL && text_focus => {
            Some(Action::RedisObjectEditorDeleteToStart)
        }
        _ => None,
    }
}

fn map_catalog_editor_form(
    event: KeyEvent,
    editor: &crate::model::catalog_editor::CatalogEditorState,
) -> Option<Action> {
    if matches!(
        editor.draft.as_ref(),
        Some(crate::model::catalog_editor::CatalogDraft::Table(_))
    ) {
        return map_table_editor(event, editor);
    }
    if is_text_redo(event) {
        return Some(Action::CatalogEditorRedo);
    }
    if is_text_undo(event) {
        return Some(Action::CatalogEditorUndo);
    }
    let draft = editor.draft.as_ref()?;
    let create_mode = editor.mode == crate::db::catalog_mutation::CatalogMutationMode::Create;
    let text_focus = draft.focus_accepts_text();
    let choice_focus = draft.focus_is_choice();
    let toggle_focus = draft.focus_is_toggle(create_mode);
    match event.code {
        KeyCode::Esc => Some(Action::CatalogEditorCancel),
        KeyCode::Tab | KeyCode::Down => Some(Action::CatalogEditorFieldNext),
        KeyCode::BackTab | KeyCode::Up => Some(Action::CatalogEditorFieldPrevious),
        KeyCode::Enter if toggle_focus => Some(Action::CatalogEditorToggleFocused),
        KeyCode::Enter
            if text_focus
                && matches!(
                    draft,
                    crate::model::catalog_editor::CatalogDraft::Database(_)
                        | crate::model::catalog_editor::CatalogDraft::Role(_)
                ) =>
        {
            Some(Action::CatalogEditorPreview)
        }
        KeyCode::Enter => match draft.focused_action() {
            Some(crate::model::catalog_editor::CatalogFormFocus::Review) => {
                Some(Action::CatalogEditorPreview)
            }
            Some(crate::model::catalog_editor::CatalogFormFocus::Cancel) => {
                Some(Action::CatalogEditorCancel)
            }
            _ => None,
        },
        KeyCode::Char(' ') if event.modifiers.is_empty() && toggle_focus => {
            Some(Action::CatalogEditorToggleFocused)
        }
        KeyCode::Char(' ') if event.modifiers.is_empty() && choice_focus => {
            Some(Action::CatalogEditorCycleChoice(1))
        }
        KeyCode::Left if choice_focus && !text_focus => Some(Action::CatalogEditorCycleChoice(-1)),
        KeyCode::Right if choice_focus && !text_focus => Some(Action::CatalogEditorCycleChoice(1)),
        KeyCode::Char(character) if event.modifiers.is_empty() && text_focus => Some(match draft {
            crate::model::catalog_editor::CatalogDraft::Role(role)
                if role.focus == crate::model::catalog_editor::CatalogFormFocus::Password =>
            {
                Action::CatalogEditorPasswordInsert(character.into())
            }
            _ => Action::CatalogEditorInsert(character),
        }),
        KeyCode::Backspace if text_focus => Some(Action::CatalogEditorBackspace),
        KeyCode::Delete if text_focus => Some(Action::CatalogEditorDelete),
        KeyCode::Left if text_focus => Some(Action::CatalogEditorMoveLeft),
        KeyCode::Right if text_focus => Some(Action::CatalogEditorMoveRight),
        KeyCode::Home if text_focus => Some(Action::CatalogEditorMoveHome),
        KeyCode::End if text_focus => Some(Action::CatalogEditorMoveEnd),
        KeyCode::Char('w') if event.modifiers == KeyModifiers::CONTROL && text_focus => {
            Some(Action::CatalogEditorDeletePreviousWord)
        }
        KeyCode::Char('u') if event.modifiers == KeyModifiers::CONTROL && text_focus => {
            Some(Action::CatalogEditorDeleteToStart)
        }
        _ => None,
    }
}

fn map_table_editor(
    event: KeyEvent,
    editor: &crate::model::catalog_editor::CatalogEditorState,
) -> Option<Action> {
    use crate::model::catalog_editor::{TableActionField, TableColumnField, TableEditorFocus};
    let field = match editor.draft.as_ref() {
        Some(crate::model::catalog_editor::CatalogDraft::Table(draft)) => draft.focus,
        _ => return None,
    };
    if editor_table_details_open(editor) {
        return map_table_column_details(event, field);
    }
    if matches!(
        field,
        TableEditorFocus::Action(TableActionField::Constraint(_))
    ) {
        return match event.code {
            KeyCode::Esc => Some(Action::CatalogEditorPreview),
            KeyCode::Tab => Some(Action::CatalogEditorFieldNext),
            KeyCode::BackTab => Some(Action::CatalogEditorFieldPrevious),
            KeyCode::Up => Some(Action::CatalogEditorMove(-1)),
            KeyCode::Down => Some(Action::CatalogEditorMove(1)),
            KeyCode::Char(character) => Some(Action::CatalogEditorInsert(character)),
            KeyCode::Backspace => Some(Action::CatalogEditorBackspace),
            KeyCode::Delete => Some(Action::CatalogEditorDelete),
            KeyCode::Left => Some(Action::CatalogEditorMoveLeft),
            KeyCode::Right => Some(Action::CatalogEditorMoveRight),
            KeyCode::Home => Some(Action::CatalogEditorMoveHome),
            KeyCode::End => Some(Action::CatalogEditorMoveEnd),
            _ => None,
        };
    }
    let text_focus = matches!(
        field,
        TableEditorFocus::General(_)
            | TableEditorFocus::ColumnDetails(TableColumnField::Name)
            | TableEditorFocus::ColumnDetails(TableColumnField::Type)
            | TableEditorFocus::ColumnDetails(TableColumnField::Default)
            | TableEditorFocus::ColumnDetails(TableColumnField::Comment)
    );
    if text_focus && is_text_redo(event) {
        return Some(Action::CatalogEditorRedo);
    }
    if text_focus && is_text_undo(event) {
        return Some(Action::CatalogEditorUndo);
    }
    if text_focus && event.modifiers == KeyModifiers::CONTROL {
        return match event.code {
            KeyCode::Char('w') => Some(Action::CatalogEditorDeletePreviousWord),
            KeyCode::Char('u') => Some(Action::CatalogEditorDeleteToStart),
            KeyCode::Char('a') => Some(Action::CatalogEditorMoveHome),
            KeyCode::Char('e') => Some(Action::CatalogEditorMoveEnd),
            _ => None,
        };
    }
    if event.modifiers != KeyModifiers::NONE
        && !matches!(event.code, KeyCode::Tab | KeyCode::BackTab)
        && !(field == TableEditorFocus::Columns
            && event.modifiers == KeyModifiers::SHIFT
            && matches!(event.code, KeyCode::Char('A' | 'J' | 'K' | 'a' | 'j' | 'k')))
    {
        return None;
    }
    match (field, event.code) {
        (TableEditorFocus::General(_), KeyCode::Esc)
        | (TableEditorFocus::Columns, KeyCode::Esc)
        | (TableEditorFocus::Action(TableActionField::AddColumn), KeyCode::Esc)
        | (TableEditorFocus::Action(TableActionField::Review), KeyCode::Esc)
        | (TableEditorFocus::Action(TableActionField::Cancel), KeyCode::Esc) => {
            return Some(Action::CatalogEditorCancel);
        }
        _ => {}
    }
    if matches!(field, TableEditorFocus::General(_)) {
        return match event.code {
            KeyCode::Tab if event.modifiers.contains(KeyModifiers::SHIFT) => {
                Some(Action::CatalogEditorFieldPrevious)
            }
            KeyCode::Tab => Some(Action::CatalogEditorFieldNext),
            KeyCode::BackTab => Some(Action::CatalogEditorFieldPrevious),
            KeyCode::Up => Some(Action::CatalogEditorFieldPrevious),
            KeyCode::Down => Some(Action::CatalogEditorFieldNext),
            KeyCode::Enter => Some(Action::CatalogEditorPreview),
            KeyCode::Char(c) => Some(Action::CatalogEditorInsert(c)),
            KeyCode::Backspace => Some(Action::CatalogEditorBackspace),
            KeyCode::Delete => Some(Action::CatalogEditorDelete),
            KeyCode::Left => Some(Action::CatalogEditorMoveLeft),
            KeyCode::Right => Some(Action::CatalogEditorMoveRight),
            KeyCode::Home => Some(Action::CatalogEditorMoveHome),
            KeyCode::End => Some(Action::CatalogEditorMoveEnd),
            _ => None,
        };
    }
    if field == TableEditorFocus::Columns {
        if event.modifiers == KeyModifiers::SHIFT {
            return match event.code {
                KeyCode::Char('a') => Some(Action::CatalogEditorAddTableColumnAbove),
                KeyCode::Char('j') => Some(Action::CatalogEditorReorderTableColumn(1)),
                KeyCode::Char('k') => Some(Action::CatalogEditorReorderTableColumn(-1)),
                _ => None,
            };
        }
        return match event.code {
            KeyCode::Enter => Some(Action::CatalogEditorPreview),
            KeyCode::Char('a') => Some(Action::CatalogEditorAddTableColumn),
            KeyCode::Char('A') => Some(Action::CatalogEditorAddTableColumnAbove),
            KeyCode::Char('j') => Some(Action::CatalogEditorFieldNext),
            KeyCode::Char('k') => Some(Action::CatalogEditorFieldPrevious),
            KeyCode::Char('J') => Some(Action::CatalogEditorReorderTableColumn(1)),
            KeyCode::Char('K') => Some(Action::CatalogEditorReorderTableColumn(-1)),
            KeyCode::Char('e') if editor_table_has_selected_column(editor) => {
                Some(Action::CatalogEditorOpenTableColumnDetails)
            }
            KeyCode::Char('r') if editor_table_selected_column_is_removed(editor) => {
                Some(Action::CatalogEditorRestoreTableColumn)
            }
            KeyCode::Tab if event.modifiers.contains(KeyModifiers::SHIFT) => {
                Some(Action::CatalogEditorFieldPrevious)
            }
            KeyCode::Tab => Some(Action::CatalogEditorFieldNext),
            KeyCode::BackTab => Some(Action::CatalogEditorFieldPrevious),
            KeyCode::Up => Some(Action::CatalogEditorFieldPrevious),
            KeyCode::Down => Some(Action::CatalogEditorFieldNext),
            _ => None,
        };
    }
    match (field, event.code) {
        (_, KeyCode::Tab) if event.modifiers.contains(KeyModifiers::SHIFT) => {
            Some(Action::CatalogEditorFieldPrevious)
        }
        (_, KeyCode::Tab) => Some(Action::CatalogEditorFieldNext),
        (_, KeyCode::BackTab) => Some(Action::CatalogEditorFieldPrevious),
        (_, KeyCode::Up) => Some(Action::CatalogEditorFieldPrevious),
        (_, KeyCode::Down) => Some(Action::CatalogEditorFieldNext),
        (
            TableEditorFocus::Action(TableActionField::AddColumn),
            KeyCode::Enter | KeyCode::Char(' '),
        ) => Some(Action::CatalogEditorAddTableColumn),
        (
            TableEditorFocus::Action(TableActionField::AddConstraintPrimaryKey),
            KeyCode::Enter | KeyCode::Char(' '),
        ) => Some(Action::CatalogEditorAddTableConstraintPrimaryKey),
        (
            TableEditorFocus::Action(TableActionField::AddConstraintUnique),
            KeyCode::Enter | KeyCode::Char(' '),
        ) => Some(Action::CatalogEditorAddTableConstraintUnique),
        (
            TableEditorFocus::Action(TableActionField::AddConstraintCheck),
            KeyCode::Enter | KeyCode::Char(' '),
        ) => Some(Action::CatalogEditorAddTableConstraintCheck),
        (
            TableEditorFocus::Action(TableActionField::AddConstraintForeignKey),
            KeyCode::Enter | KeyCode::Char(' '),
        ) => Some(Action::CatalogEditorAddTableConstraintForeignKey),
        (
            TableEditorFocus::Action(TableActionField::RemoveConstraint(index)),
            KeyCode::Enter | KeyCode::Char(' '),
        ) => Some(Action::CatalogEditorRemoveTableConstraint(index)),
        (
            TableEditorFocus::Action(TableActionField::Review),
            KeyCode::Enter | KeyCode::Char(' '),
        ) => Some(Action::CatalogEditorPreview),
        (
            TableEditorFocus::Action(TableActionField::Cancel),
            KeyCode::Enter | KeyCode::Char(' '),
        ) => Some(Action::CatalogEditorCancel),
        _ => None,
    }
}

fn editor_table_details_open(editor: &crate::model::catalog_editor::CatalogEditorState) -> bool {
    matches!(
        editor.draft.as_ref(),
        Some(crate::model::catalog_editor::CatalogDraft::Table(draft))
            if draft.column_editor.is_some()
    )
}

fn map_table_column_details(
    event: KeyEvent,
    field: crate::model::catalog_editor::TableEditorFocus,
) -> Option<Action> {
    use crate::model::catalog_editor::{TableColumnField, TableEditorFocus};
    if is_text_redo(event) {
        return Some(Action::CatalogEditorRedo);
    }
    if is_text_undo(event) {
        return Some(Action::CatalogEditorUndo);
    }
    if event.modifiers == KeyModifiers::CONTROL {
        return match event.code {
            KeyCode::Char('w') => Some(Action::CatalogEditorDeletePreviousWord),
            KeyCode::Char('u') => Some(Action::CatalogEditorDeleteToStart),
            KeyCode::Char('a') => Some(Action::CatalogEditorMoveHome),
            KeyCode::Char('e') => Some(Action::CatalogEditorMoveEnd),
            _ => None,
        };
    }
    if event.modifiers != KeyModifiers::NONE
        && !matches!(event.code, KeyCode::Tab | KeyCode::BackTab)
    {
        return None;
    }
    match event.code {
        KeyCode::Esc => Some(Action::CatalogEditorCancelTableColumnDetails),
        KeyCode::Enter => Some(Action::CatalogEditorConfirmTableColumnDetails),
        KeyCode::Tab if event.modifiers.contains(KeyModifiers::SHIFT) => {
            Some(Action::CatalogEditorFieldPrevious)
        }
        KeyCode::Tab => Some(Action::CatalogEditorFieldNext),
        KeyCode::BackTab | KeyCode::Up => Some(Action::CatalogEditorFieldPrevious),
        KeyCode::Down => Some(Action::CatalogEditorFieldNext),
        KeyCode::Char(' ')
            if matches!(
                field,
                TableEditorFocus::ColumnDetails(
                    TableColumnField::Nullable | TableColumnField::Identity
                )
            ) =>
        {
            Some(
                if field == TableEditorFocus::ColumnDetails(TableColumnField::Nullable) {
                    Action::CatalogEditorToggleTableColumnNullable
                } else {
                    Action::CatalogEditorToggleTableColumnIdentity
                },
            )
        }
        KeyCode::Char(character) => Some(Action::CatalogEditorInsert(character)),
        KeyCode::Backspace => Some(Action::CatalogEditorBackspace),
        KeyCode::Delete => Some(Action::CatalogEditorDelete),
        KeyCode::Left => Some(Action::CatalogEditorMoveLeft),
        KeyCode::Right => Some(Action::CatalogEditorMoveRight),
        KeyCode::Home => Some(Action::CatalogEditorMoveHome),
        KeyCode::End => Some(Action::CatalogEditorMoveEnd),
        _ => None,
    }
}

fn editor_table_has_selected_column(
    editor: &crate::model::catalog_editor::CatalogEditorState,
) -> bool {
    matches!(
        editor.draft.as_ref(),
        Some(crate::model::catalog_editor::CatalogDraft::Table(draft))
            if draft.selected_column().is_some() && !draft.selected_column_is_removed()
    )
}

fn editor_table_selected_column_is_removed(
    editor: &crate::model::catalog_editor::CatalogEditorState,
) -> bool {
    matches!(
        editor.draft.as_ref(),
        Some(crate::model::catalog_editor::CatalogDraft::Table(draft))
            if draft.selected_column_is_removed()
    )
}

fn pending_is_valid(
    pending: &PendingState,
    app: &App,
    now: Instant,
    generation: u64,
    sequence_timeout: Duration,
) -> bool {
    let base_valid = (pending.pending.is_leader()
        || now.saturating_duration_since(pending.started_at) < sequence_timeout)
        && pending.focus == app.focus
        && pending.editor_mode == app.active_editor_mode()
        && pending.generation == generation
        && pending.record_view_active == matches!(app.overlay, Some(Overlay::RecordView(_)))
        && app
            .tabs
            .get(app.active_tab)
            .map_or(pending.tab_id == Uuid::nil(), |tab| {
                pending.tab_id == tab.id()
            });
    base_valid
        && match pending.pending {
            Pending::CatalogColumnDelete { selected_column } => {
                is_catalog_columns_focus(app)
                    && catalog_selected_column(app) == Some(selected_column)
            }
            _ => true,
        }
}

fn pending_display(pending: Pending) -> Option<(crate::help::ShortcutPrefix, String)> {
    use crate::help::ShortcutPrefix;
    match pending {
        Pending::Leader => Some((ShortcutPrefix::Leader, "Space".into())),
        Pending::EditorLeader => Some((ShortcutPrefix::EditorLeader, "Space".into())),
        Pending::Window { count } => Some((
            ShortcutPrefix::Window,
            if count == 1 {
                "Ctrl-w".into()
            } else {
                format!("{count} Ctrl-w")
            },
        )),
        Pending::PaneSequence { count, .. } => {
            Some((ShortcutPrefix::Window, format!("pane sequence ({count})")))
        }
        Pending::WindowCount { count } => {
            Some((ShortcutPrefix::WindowCount(count), count.to_string()))
        }
        Pending::Previous => Some((ShortcutPrefix::Previous, "[".into())),
        Pending::Next => Some((ShortcutPrefix::Next, "]".into())),
        Pending::RelationDelete => Some((ShortcutPrefix::RelationDelete, "d".into())),
        Pending::GridAlign => Some((ShortcutPrefix::GridAlign, "z".into())),
        Pending::RecordViewGoto => Some((ShortcutPrefix::RecordViewGoto, "g".into())),
        Pending::ExplorerAlign => Some((ShortcutPrefix::ExplorerAlign, "z".into())),
        Pending::Goto => Some((ShortcutPrefix::Goto, "g".into())),
        Pending::LeaderTransaction => {
            Some((ShortcutPrefix::EditorLeaderTransaction, "Space t".into()))
        }
        Pending::RedisPreviewLeader => Some((ShortcutPrefix::Leader, "Space".into())),
        Pending::CatalogColumnDelete { .. } => None,
    }
}

fn map_pending(
    pending: Pending,
    event: KeyEvent,
    app: &App,
    bindings: &crate::config::KeyBindings,
) -> Option<Action> {
    let valid_modifiers = event.modifiers.is_empty()
        || (pending == Pending::Leader
            && event.modifiers == KeyModifiers::SHIFT
            && matches!(event.code, KeyCode::Char('R' | 'Y')))
        || (pending == Pending::Goto
            && event.modifiers == KeyModifiers::SHIFT
            && event.code == KeyCode::Char('T'))
        || matches!(pending, Pending::Window { .. });
    if pending == Pending::Leader {
        let sequence = [KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE), event];
        if let Some(command) = bindings.matching_command(&sequence) {
            return configured_command_action(command, app);
        }
        if bindings.has_any_prefix(&sequence) {
            return None;
        }
    }
    if !valid_modifiers {
        return None;
    }
    match (pending, event.code) {
        (Pending::RedisPreviewLeader, KeyCode::Char('f')) => Some(Action::RedisPreviewCycleFormat),
        (Pending::RedisPreviewLeader, KeyCode::Char('w')) => Some(Action::RedisPreviewToggleWrap),
        (Pending::RedisPreviewLeader, KeyCode::Char('l')) => Some(Action::RedisPreviewLoadNext),
        (Pending::RedisPreviewLeader, KeyCode::Char('d')) => Some(Action::RedisPreviewDelete),
        (Pending::Leader, KeyCode::Char('B')) if app.active_console_opt().is_some() => {
            Some(Action::OpenConsoleTargetSelector {
                console_id: app.active_console_opt().expect("checked above").id,
            })
        }
        (Pending::Leader, KeyCode::Char('q')) => Some(Action::CloseActiveTab),
        (Pending::Leader, KeyCode::Char('x')) => Some(Action::RequestDeleteActiveConsole),
        (Pending::Leader, KeyCode::Char('m')) => Some(Action::OpenNotificationHistory),
        (Pending::Leader, KeyCode::Char('Y')) if is_grid_navigation_focus(app) => {
            Some(Action::CopyGridRow {
                include_headers: true,
            })
        }
        (Pending::Window { .. }, KeyCode::Char('w'))
            if event.modifiers == KeyModifiers::CONTROL =>
        {
            Some(Action::FocusNext)
        }
        (Pending::Window { .. }, KeyCode::Char('j')) if app.focus == Focus::Explorer => None,
        (Pending::Window { count }, KeyCode::Char(operator @ ('+' | '-' | '>' | '<'))) => {
            let keys_focused = matches!(
                app.tabs.get(app.active_tab),
                Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
                    if tab.focus == crate::model::redis_browser::RedisBrowserFocus::Keys
            );
            crate::model::workspace::redis_pane_resize(app.focus, keys_focused, operator, count)
                .map(Action::ResizePane)
        }
        (Pending::Goto, KeyCode::Char('g')) if app.focus == Focus::Explorer => Some(
            Action::ExplorerSelectTarget(crate::model::explorer::ExplorerNodeTarget::First),
        ),
        (Pending::Goto, KeyCode::Char('g'))
            if matches!(app.tabs.get(app.active_tab), Some(crate::model::tab::WorkspaceTab::PrincipalDdl(tab))
                if app.focus == Focus::Results && tab.view == crate::model::principal::PrincipalView::Overview) =>
        {
            Some(Action::PrincipalAccessSelectFirst)
        }
        (Pending::Goto, KeyCode::Char('g')) if is_grid_navigation_focus(app) => Some(
            Action::GridSelectRow(crate::model::tab::GridRowTarget::First),
        ),
        (Pending::Goto, KeyCode::Char('m')) if app.focus == Focus::Explorer => {
            Some(Action::ProfileGroupOpen)
        }
        (Pending::Goto, code) => {
            let prefix = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE);
            let event = KeyEvent::new(code, KeyModifiers::NONE);
            if !tab_shortcuts_have_focus(app) {
                None
            } else if bindings.matches_sequence("next-tab", &[prefix, event]) {
                Some(Action::NextTab)
            } else if bindings.matches_sequence("previous-tab", &[prefix, event]) {
                Some(Action::PreviousTab)
            } else {
                None
            }
        }
        (Pending::Previous, code) => {
            let prefix = KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE);
            let event = KeyEvent::new(code, KeyModifiers::NONE);
            (tab_shortcuts_have_focus(app)
                && bindings.matches_sequence("previous-tab", &[prefix, event]))
            .then_some(Action::PreviousTab)
        }
        (Pending::Next, code) => {
            let prefix = KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE);
            let event = KeyEvent::new(code, KeyModifiers::NONE);
            (tab_shortcuts_have_focus(app)
                && bindings.matches_sequence("next-tab", &[prefix, event]))
            .then_some(Action::NextTab)
        }
        (Pending::Leader, KeyCode::Char('t')) => None,
        (Pending::RelationDelete, KeyCode::Char('d')) => Some(Action::RelationDeleteCurrent),
        (Pending::RecordViewGoto, KeyCode::Char('g')) => Some(Action::RecordViewJumpFirstField),
        (Pending::ExplorerAlign, KeyCode::Char('z')) => Some(Action::ExplorerAlignSelected(
            crate::model::explorer::ExplorerNodeAlignment::Middle,
        )),
        (Pending::ExplorerAlign, KeyCode::Char('t')) => Some(Action::ExplorerAlignSelected(
            crate::model::explorer::ExplorerNodeAlignment::Top,
        )),
        (Pending::ExplorerAlign, KeyCode::Char('b')) => Some(Action::ExplorerAlignSelected(
            crate::model::explorer::ExplorerNodeAlignment::Bottom,
        )),
        _ => None,
    }
}

fn configured_command_action(command: &str, app: &App) -> Option<Action> {
    match command {
        "open-dashboard" if app.dashboard_supported() => Some(Action::OpenDashboard),
        "open-sql-history" => Some(Action::OpenSqlHistory),
        "open-explorer" => Some(Action::Focus(Focus::Explorer)),
        "open-editors" => Some(Action::OpenSqlEditorList),
        "open-consoles" => Some(Action::OpenSqlEditorList),
        "run-leader-statement" => Some(Action::RunActiveSql),
        "run-leader-buffer" => Some(Action::RunAllSql),
        "open-target-selector" => Some(Action::OpenTargetSelector),
        "open-database-selector" => Some(Action::OpenDatabaseSelector),
        "results-copy-row-headers" if is_grid_navigation_focus(app) => Some(Action::CopyGridRow {
            include_headers: true,
        }),
        _ => None,
    }
}

pub fn map_paste(value: String, app: &App) -> Vec<Action> {
    if let Some(Overlay::MongoPrincipalMutation(draft)) = app.overlay.as_ref() {
        if draft.selected_field == crate::model::mongodb_principal::MongoPrincipalField::Password {
            return vec![Action::MongoPrincipalMutationPasswordPaste(value)];
        }
        return vec![Action::MongoPrincipalMutationPaste(value)];
    }
    if matches!(
        app.overlay,
        Some(Overlay::MongoPrincipalConfirmation { .. })
    ) {
        return vec![Action::MongoPrincipalConfirmationPaste(value)];
    }
    if app.omni.is_some() {
        return vec![Action::OmniPaste(value)];
    }
    if app
        .overlay
        .as_ref()
        .is_some_and(|overlay| matches!(overlay, Overlay::Help(_)))
    {
        return vec![Action::HelpPaste(value)];
    }
    if app.overlay == Some(Overlay::ProfileManager) {
        let Some(manager) = app.profile_manager.as_ref().filter(|manager| {
            manager.page == ProfileManagerPage::Form && manager.operation.is_none()
        }) else {
            return Vec::new();
        };
        return is_text_field(manager.selected_field)
            .then(|| Action::ProfilePaste(ProfileInput::from(value)))
            .into_iter()
            .collect();
    }
    if app.overlay.as_ref().is_some_and(|overlay| {
        matches!(
            overlay,
            Overlay::SqlHistory(view)
                if view.mode == crate::model::sql_history_view::SqlHistoryMode::Search
        )
    }) {
        return vec![Action::SqlHistorySearchPaste(value)];
    }
    if let Some(Overlay::RelationTransactionConfirm(review)) = app.overlay.as_ref()
        && review.focus == crate::model::transaction_review::TransactionReviewFocus::SqlPreview
        && app.editor_prompt_active(review.editor_session_id)
    {
        return vec![Action::TransactionReviewPromptPaste(value)];
    }
    if app.overlay == Some(Overlay::CatalogEditor) {
        let editor = app.catalog_editor.as_ref();
        let editable_table = editor.is_some_and(|editor| {
            editor.page == crate::model::catalog_editor::CatalogEditorPage::Form
                && !editor.is_busy()
                && matches!(
                    editor.draft,
                    Some(crate::model::catalog_editor::CatalogDraft::Table(_))
                )
        });
        if editable_table {
            return vec![Action::CatalogEditorPaste(value)];
        } else if let Some(editor) = editor.filter(|editor| {
            editor.page == crate::model::catalog_editor::CatalogEditorPage::Form
                && !editor.is_busy()
        }) && let Some(draft) = editor.draft.as_ref()
            && matches!(
                draft,
                crate::model::catalog_editor::CatalogDraft::Database(_)
                    | crate::model::catalog_editor::CatalogDraft::Role(_)
            )
            && draft.focus_accepts_text()
        {
            return match draft {
                crate::model::catalog_editor::CatalogDraft::Role(role)
                    if role.focus == crate::model::catalog_editor::CatalogFormFocus::Password =>
                {
                    vec![Action::CatalogEditorPasswordPaste(value.into())]
                }
                _ => vec![Action::CatalogEditorPaste(value)],
            };
        } else if editor.is_some_and(|editor| {
            editor.page == crate::model::catalog_editor::CatalogEditorPage::Form
                && !editor.is_busy()
                && matches!(
                    editor.draft,
                    Some(
                        crate::model::catalog_editor::CatalogDraft::Schema(_)
                            | crate::model::catalog_editor::CatalogDraft::Sequence(_)
                    )
                )
        }) {
            return value.chars().map(Action::CatalogEditorInsert).collect();
        } else {
            return Vec::new();
        }
    }
    if let Some(Overlay::RedisObjectEditor(editor)) = app.overlay.as_ref() {
        if !editor.busy
            && matches!(
                editor.focus,
                crate::model::redis_object_editor::RedisObjectEditorFocus::Value
                    | crate::model::redis_object_editor::RedisObjectEditorFocus::Ttl
            )
            || (!editor.busy
                && editor.mode == crate::db::redis::mutation::RedisMutationMode::Create
                && editor.focus == crate::model::redis_object_editor::RedisObjectEditorFocus::Key)
        {
            return vec![Action::RedisObjectEditorPaste(value)];
        }
        return Vec::new();
    }
    if app.overlay.is_some() {
        return Vec::new();
    }
    if let Some(crate::model::tab::WorkspaceTab::MongoBrowser(tab)) = app.tabs.get(app.active_tab)
        && app.focus == Focus::Results
    {
        if tab.query_field.is_some() {
            return vec![Action::MongoQueryPaste(value)];
        }
        if tab.editing {
            return vec![Action::MongoDocumentEditPaste(value)];
        }
        return vec![Action::MongoImportDraft(value)];
    }
    if matches!(
        app.tabs.get(app.active_tab),
        Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
            if app.focus == Focus::Results
                && tab.focus == crate::model::redis_browser::RedisBrowserFocus::Preview
                && tab.format.view() == crate::value_preview::ValueView::Table
                && tab.value_filter.editing
    ) {
        return vec![Action::RedisValueFilterPaste(value)];
    }
    if let Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab)) = app.tabs.get(app.active_tab)
        && app.focus == Focus::Results
        && tab.focus == crate::model::redis_browser::RedisBrowserFocus::Preview
        && app.editor_is_editable(tab.preview_editor_id)
    {
        return vec![Action::EditorPaste(value)];
    }
    if app.focus == Focus::Results
        && let Some(query) = app.tabs.get(app.active_tab).and_then(|tab| match tab {
            crate::model::tab::WorkspaceTab::Relation(tab)
                if tab.view == crate::model::relation::RelationView::Data =>
            {
                Some(&tab.query)
            }
            crate::model::tab::WorkspaceTab::Sql(tab)
                if tab.result_view == crate::model::tab::ResultView::Data =>
            {
                Some(&tab.query)
            }
            _ => None,
        })
        && query.focus.is_some()
        && matches!(
            query.capability,
            crate::model::data_query::DataQueryCapability::Relation
                | crate::model::data_query::DataQueryCapability::Sql
        )
    {
        return vec![Action::DataQueryPaste(value)];
    }
    if app.focus == Focus::Results
        && let Some(crate::model::tab::WorkspaceTab::Relation(tab)) = app.tabs.get(app.active_tab)
        && tab.view == crate::model::relation::RelationView::Data
    {
        use crate::model::relation_edit::RelationGridMode;
        match tab.edit.as_ref().map(|edit| &edit.mode) {
            Some(RelationGridMode::EditCell(_)) => {
                return vec![Action::RelationEditPaste(value)];
            }
            Some(RelationGridMode::EditDocument(_)) => {
                return vec![Action::RelationEditPaste(value)];
            }
            Some(RelationGridMode::Busy) => return Vec::new(),
            _ => {}
        }
    }
    if is_relation_data_focus(app) {
        return vec![Action::RelationPaste];
    }
    if app.focus != Focus::Editor {
        return Vec::new();
    }
    match app.active_editor_mode() {
        EditorMode::Insert | EditorMode::Replace => vec![Action::EditorPaste(value)],
        EditorMode::Normal
        | EditorMode::VisualChar
        | EditorMode::VisualLine
        | EditorMode::VisualBlock => {
            vec![Action::EditorPaste(value)]
        }
    }
}

fn map_omni(event: KeyEvent, app: &App) -> Option<Action> {
    app.omni.as_ref()?;
    if app.key_bindings.matches("omni", event) {
        return Some(Action::OmniDismiss);
    }
    match (event.modifiers, event.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => Some(Action::OmniDismiss),
        (KeyModifiers::NONE, KeyCode::Esc) => Some(Action::OmniCancel),
        (KeyModifiers::NONE, KeyCode::Enter) => Some(Action::OmniConfirm),
        (KeyModifiers::NONE, KeyCode::Tab) => Some(Action::ToggleHelpPanel),
        (KeyModifiers::NONE, KeyCode::BackTab) => Some(Action::OmniShowActions),
        (KeyModifiers::NONE, KeyCode::Up) => Some(Action::OmniMove(-1)),
        (KeyModifiers::NONE, KeyCode::Down) => Some(Action::OmniMove(1)),
        (KeyModifiers::NONE, KeyCode::Char(character)) => Some(Action::OmniEdit(
            crate::model::text_input::TextInputEdit::Insert(character),
        )),
        (KeyModifiers::NONE, KeyCode::Backspace) => Some(Action::OmniEdit(
            crate::model::text_input::TextInputEdit::Backspace,
        )),
        (KeyModifiers::NONE, KeyCode::Delete) => Some(Action::OmniEdit(
            crate::model::text_input::TextInputEdit::Delete,
        )),
        (KeyModifiers::NONE, KeyCode::Left) => Some(Action::OmniEdit(
            crate::model::text_input::TextInputEdit::MoveLeft,
        )),
        (KeyModifiers::NONE, KeyCode::Right) => Some(Action::OmniEdit(
            crate::model::text_input::TextInputEdit::MoveRight,
        )),
        (KeyModifiers::NONE, KeyCode::Home) => Some(Action::OmniEdit(
            crate::model::text_input::TextInputEdit::MoveHome,
        )),
        (KeyModifiers::NONE, KeyCode::End) => Some(Action::OmniEdit(
            crate::model::text_input::TextInputEdit::MoveEnd,
        )),
        (KeyModifiers::CONTROL, KeyCode::Char('w')) => Some(Action::OmniEdit(
            crate::model::text_input::TextInputEdit::DeletePreviousWord,
        )),
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => Some(Action::OmniEdit(
            crate::model::text_input::TextInputEdit::DeleteToStart,
        )),
        (KeyModifiers::CONTROL, KeyCode::Char('z')) => Some(Action::OmniEdit(
            crate::model::text_input::TextInputEdit::Undo,
        )),
        (KeyModifiers::CONTROL, KeyCode::Char('y')) => Some(Action::OmniEdit(
            crate::model::text_input::TextInputEdit::Redo,
        )),
        _ => None,
    }
}

fn is_relation_data_focus(app: &App) -> bool {
    app.focus == Focus::Results
        && matches!(
            app.tabs.get(app.active_tab),
            Some(crate::model::tab::WorkspaceTab::Relation(tab))
                if tab.view == crate::model::relation::RelationView::Data && tab.query.focus.is_none()
        )
}

/// Editor session for any DDL-only read-only workspace tab.
///
/// Covers both the relation DDL view and the principal (user/role) DDL tab so
/// they share identical Vim and scroll behavior.
fn ddl_only_session_id(app: &App) -> Option<uuid::Uuid> {
    match app.tabs.get(app.active_tab) {
        Some(crate::model::tab::WorkspaceTab::Relation(tab))
            if tab.view == crate::model::relation::RelationView::Ddl =>
        {
            Some(tab.ddl_editor_id)
        }
        Some(crate::model::tab::WorkspaceTab::PrincipalDdl(tab))
            if tab.view == crate::model::principal::PrincipalView::Ddl =>
        {
            Some(tab.editor_id)
        }
        _ => None,
    }
}

fn is_ddl_only_focus(app: &App) -> bool {
    app.focus == Focus::Results && ddl_only_session_id(app).is_some()
}

fn relation_grid_is_browse(app: &App) -> bool {
    use crate::model::relation_edit::RelationGridMode;

    app.tabs
        .get(app.active_tab)
        .and_then(|tab| match tab {
            crate::model::tab::WorkspaceTab::Relation(tab) => tab.edit.as_ref(),
            _ => None,
        })
        .is_none_or(|edit| matches!(edit.mode, RelationGridMode::Browse))
}

fn is_relation_cell_editor(app: &App) -> bool {
    app.tabs
        .get(app.active_tab)
        .and_then(|tab| match tab {
            crate::model::tab::WorkspaceTab::Relation(tab) => tab.edit.as_ref(),
            _ => None,
        })
        .is_some_and(|edit| {
            matches!(
                edit.mode,
                crate::model::relation_edit::RelationGridMode::EditCell(_)
                    | crate::model::relation_edit::RelationGridMode::EditDocument(_)
            )
        })
}

fn is_grid_navigation_focus(app: &App) -> bool {
    if app.focus != Focus::Results {
        return false;
    }
    match app.tabs.get(app.active_tab) {
        Some(crate::model::tab::WorkspaceTab::Sql(tab)) => {
            tab.result_view == crate::model::tab::ResultView::Data
        }
        Some(crate::model::tab::WorkspaceTab::Relation(tab))
            if tab.view == crate::model::relation::RelationView::Data
                && tab.query.focus.is_none() =>
        {
            tab.edit.as_ref().is_none_or(|edit| {
                matches!(
                    edit.mode,
                    crate::model::relation_edit::RelationGridMode::Browse
                        | crate::model::relation_edit::RelationGridMode::VisualLine { .. }
                )
            })
        }
        Some(crate::model::tab::WorkspaceTab::Dashboard(tab))
            if tab.page == crate::model::dashboard::DashboardPage::Processes
                && !tab.process_filter_active =>
        {
            true
        }
        _ => false,
    }
}

fn is_sql_grid_focus(app: &App) -> bool {
    app.focus == Focus::Results
        && matches!(
            app.tabs.get(app.active_tab),
            Some(crate::model::tab::WorkspaceTab::Sql(tab))
                if tab.result_view == crate::model::tab::ResultView::Data
        )
}

fn is_read_only_grid_focus(app: &App) -> bool {
    is_grid_navigation_focus(app)
        && matches!(
            app.tabs.get(app.active_tab),
            Some(crate::model::tab::WorkspaceTab::Dashboard(tab))
                if !tab.process_filter_active
        )
}

fn is_redis_table_grid_focus(app: &App) -> bool {
    app.focus == Focus::Results
        && matches!(
            app.tabs.get(app.active_tab),
            Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab))
                if tab.focus == crate::model::redis_browser::RedisBrowserFocus::Preview
                    && tab.format.view() == crate::value_preview::ValueView::Table
                    && !tab.value_filter.editing
        )
}

fn map_relation_data(event: KeyEvent, app: &App) -> Option<Action> {
    use crate::model::relation_edit::RelationGridMode;

    let mode = app.tabs.get(app.active_tab).and_then(|tab| match tab {
        crate::model::tab::WorkspaceTab::Relation(tab) => tab.edit.as_ref().map(|edit| &edit.mode),
        _ => None,
    });
    if matches!(mode, Some(RelationGridMode::Busy)) {
        return None;
    }
    if let Some(RelationGridMode::EditDocument(state)) = mode {
        return match state.view {
            crate::model::relation_edit::DocumentEditorView::Ejson => {
                match (event.modifiers, event.code) {
                    (KeyModifiers::NONE, KeyCode::Tab) => {
                        Some(Action::RelationEditDocumentToggleView)
                    }
                    _ => map_relation_json_editor(event),
                }
            }
            crate::model::relation_edit::DocumentEditorView::Fields => {
                map_document_field_list(event, state)
            }
        };
    }
    if let Some(RelationGridMode::EditCell(_)) = mode {
        if event.modifiers == KeyModifiers::ALT {
            return match event.code {
                KeyCode::Char('d') => Some(Action::RelationEditRestoreDefault),
                KeyCode::Char('n') => Some(Action::RelationEditSetNull),
                KeyCode::Char('v') => Some(Action::RelationEditUseValue),
                _ => None,
            };
        }
        let json = matches!(mode, Some(RelationGridMode::EditCell(state))
        if matches!(
            state.input,
            crate::model::cell_editor::CellEditorBuffer { content: crate::model::cell_editor::CellEditorContent::Typed {
                kind: crate::model::cell_editor::CellEditorKind::Json,
                draft: crate::model::cell_editor::TypedDraft::Json(_),
                ..
            }, .. }
        ));
        if json {
            return map_relation_json_editor(event);
        }
        let boolean = matches!(mode, Some(RelationGridMode::EditCell(state))
        if matches!(
            state.input,
            crate::model::cell_editor::CellEditorBuffer { content: crate::model::cell_editor::CellEditorContent::Typed {
                kind: crate::model::cell_editor::CellEditorKind::Boolean,
                draft: crate::model::cell_editor::TypedDraft::Boolean(_),
                ..
            }, .. }
        ));
        if boolean {
            return match (event.modifiers, event.code) {
                (KeyModifiers::NONE, KeyCode::Left) => Some(Action::RelationEditBooleanMove(-1)),
                (KeyModifiers::NONE, KeyCode::Right) => Some(Action::RelationEditBooleanMove(1)),
                (KeyModifiers::NONE, KeyCode::Char(' ')) => Some(Action::RelationEditBooleanToggle),
                (KeyModifiers::NONE, KeyCode::Char('t' | 'T')) => {
                    Some(Action::RelationEditBooleanSet(true))
                }
                (KeyModifiers::NONE, KeyCode::Char('f' | 'F')) => {
                    Some(Action::RelationEditBooleanSet(false))
                }
                (KeyModifiers::NONE, KeyCode::Enter) => Some(Action::RelationEditConfirm),
                (KeyModifiers::NONE, KeyCode::Esc) => Some(Action::RelationEditCancel),
                _ => None,
            };
        }
        let temporal = matches!(mode, Some(RelationGridMode::EditCell(state))
        if matches!(
            state.input,
            crate::model::cell_editor::CellEditorBuffer { content: crate::model::cell_editor::CellEditorContent::Typed {
                draft: crate::model::cell_editor::TypedDraft::Temporal(_),
                ..
            }, .. }
        ));
        if temporal {
            return match (event.modifiers, event.code) {
                (KeyModifiers::NONE, KeyCode::Left) => Some(Action::RelationEditTemporalMove(-1)),
                (KeyModifiers::NONE, KeyCode::Right) => Some(Action::RelationEditTemporalMove(1)),
                (KeyModifiers::NONE, KeyCode::Char('[')) => {
                    Some(Action::RelationEditTemporalMonth(-1))
                }
                (KeyModifiers::NONE, KeyCode::Char(']')) => {
                    Some(Action::RelationEditTemporalMonth(1))
                }
                (KeyModifiers::NONE, KeyCode::Enter) => Some(Action::RelationEditConfirm),
                (KeyModifiers::NONE, KeyCode::Esc) => Some(Action::RelationEditCancel),
                _ => map_text_input_edit(event).map(|edit| match edit {
                    TextInputEdit::Insert(character) => Action::RelationEditInsert(character),
                    TextInputEdit::Backspace => Action::RelationEditBackspace,
                    TextInputEdit::DeletePreviousWord => Action::RelationEditDeletePreviousWord,
                    TextInputEdit::DeleteToStart | TextInputEdit::Clear => {
                        Action::RelationEditDeleteToStart
                    }
                    TextInputEdit::Delete => Action::RelationEditDelete,
                    TextInputEdit::MoveLeft => Action::RelationEditTemporalMove(-1),
                    TextInputEdit::MoveRight => Action::RelationEditTemporalMove(1),
                    TextInputEdit::MoveHome => Action::RelationEditMoveHome,
                    TextInputEdit::MoveEnd => Action::RelationEditMoveEnd,
                    TextInputEdit::Undo => Action::RelationEditUndo,
                    TextInputEdit::Redo => Action::RelationEditRedo,
                }),
            };
        }
        return match (event.modifiers, event.code) {
            (KeyModifiers::NONE, KeyCode::Enter) => Some(Action::RelationEditConfirm),
            (KeyModifiers::NONE, KeyCode::Esc) => Some(Action::RelationEditCancel),
            _ => map_text_input_edit(event).map(|edit| match edit {
                TextInputEdit::Insert(character) => Action::RelationEditInsert(character),
                TextInputEdit::Backspace => Action::RelationEditBackspace,
                TextInputEdit::DeletePreviousWord => Action::RelationEditDeletePreviousWord,
                TextInputEdit::DeleteToStart => Action::RelationEditDeleteToStart,
                TextInputEdit::Clear => Action::RelationEditDeleteToStart,
                TextInputEdit::Delete => Action::RelationEditDelete,
                TextInputEdit::MoveLeft => Action::RelationEditMoveLeft,
                TextInputEdit::MoveRight => Action::RelationEditMoveRight,
                TextInputEdit::MoveHome => Action::RelationEditMoveHome,
                TextInputEdit::MoveEnd => Action::RelationEditMoveEnd,
                TextInputEdit::Undo => Action::RelationEditUndo,
                TextInputEdit::Redo => Action::RelationEditRedo,
            }),
        };
    }

    if !event.modifiers.is_empty() {
        return match (event.modifiers, event.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('s')) => Some(Action::OpenTransactionControl),
            (KeyModifiers::CONTROL, KeyCode::Char('r')) => Some(Action::RelationRedo),
            _ => None,
        };
    }

    match mode {
        Some(RelationGridMode::VisualLine { .. }) => match event.code {
            KeyCode::Char('j') | KeyCode::Down => Some(Action::GridMove {
                rows: 1,
                columns: 0,
            }),
            KeyCode::Char('k') | KeyCode::Up => Some(Action::GridMove {
                rows: -1,
                columns: 0,
            }),
            KeyCode::Char('d') => Some(Action::RelationDeleteSelected),
            KeyCode::Char('y') => Some(Action::RelationYankSelected),
            KeyCode::Char('V') => Some(Action::RelationEditCancel),
            _ => None,
        },
        _ => match event.code {
            KeyCode::Char('e') => Some(Action::RelationEditCell),
            KeyCode::Char('V') => Some(Action::RelationVisualLine),
            KeyCode::Char('d') => {
                // Keep d available for the dd sequence instead of deleting immediately.
                None
            }
            KeyCode::Char('y') => None,
            KeyCode::Char('p') => Some(Action::RelationPaste),
            KeyCode::Char('a') => Some(Action::RelationInsertRow),
            KeyCode::Char('u') => Some(Action::RelationUndo),
            KeyCode::Char('=') => Some(Action::GridResetColumnWidth),
            _ => None,
        },
    }
}

/// Keys of the field-by-field view inside the MongoDB document modal.
///
/// The list navigates like the document grid and `Enter` opens the selected
/// field. A scalar opens the single-line input, where `Enter`/`Ctrl-S` apply and
/// `Esc` leaves it. An array or embedded document opens the multi-line field
/// modal instead, so `Enter` inserts a newline there and only `Ctrl-S` applies.
/// `Ctrl-S` in the list saves the whole document through the same path the EJSON
/// view uses.
fn map_document_field_list(
    event: KeyEvent,
    state: &crate::model::relation_edit::DocumentEditorState,
) -> Option<Action> {
    let Some(field) = state.field_edit.as_ref() else {
        return match (event.modifiers, event.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('s')) => Some(Action::RelationEditConfirm),
            (KeyModifiers::NONE, KeyCode::Tab) => Some(Action::RelationEditDocumentToggleView),
            (KeyModifiers::NONE, KeyCode::Enter) => Some(Action::RelationEditDocumentFieldEdit),
            (KeyModifiers::NONE, KeyCode::Esc) => Some(Action::RelationEditCancel),
            (KeyModifiers::NONE, KeyCode::Char('j') | KeyCode::Down) => {
                Some(Action::RelationEditDocumentFieldMove(1))
            }
            (KeyModifiers::NONE, KeyCode::Char('k') | KeyCode::Up) => {
                Some(Action::RelationEditDocumentFieldMove(-1))
            }
            _ => None,
        };
    };
    // The multi-line field modal owns `Enter`, because its value spans lines,
    // and its own cursor keys. Both kinds keep Ctrl-S and Esc.
    if field.input.is_multiline() {
        return match (event.modifiers, event.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('s')) => {
                Some(Action::RelationEditDocumentFieldApply)
            }
            (KeyModifiers::CONTROL, KeyCode::Char('f')) => {
                Some(Action::RelationEditDocumentFieldFormat)
            }
            (KeyModifiers::NONE, KeyCode::Enter) => Some(Action::RelationEditInsert('\n')),
            (KeyModifiers::NONE, KeyCode::Esc) => Some(Action::RelationEditDocumentFieldCancel),
            (KeyModifiers::NONE, KeyCode::Up) => {
                Some(Action::RelationEditDocumentFieldMoveLine(-1))
            }
            (KeyModifiers::NONE, KeyCode::Down) => {
                Some(Action::RelationEditDocumentFieldMoveLine(1))
            }
            _ => map_relation_edit_text(event),
        };
    }
    // A one-line field applies on Enter; Up/Down validates the draft before
    // opening the adjacent editable field.
    match (event.modifiers, event.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('s')) | (KeyModifiers::NONE, KeyCode::Enter) => {
            Some(Action::RelationEditDocumentFieldApply)
        }
        (KeyModifiers::NONE, KeyCode::Esc) => Some(Action::RelationEditDocumentFieldCancel),
        (KeyModifiers::NONE, KeyCode::Down) => Some(Action::RelationEditDocumentFieldMove(1)),
        (KeyModifiers::NONE, KeyCode::Up) => Some(Action::RelationEditDocumentFieldMove(-1)),
        _ => map_relation_edit_text(event),
    }
}

/// Keys of a multi-line JSON document editor.
///
/// The SQL JSON cell editor and the MongoDB document modal share it so both
/// save, cancel and insert newlines the same way.
fn map_relation_json_editor(event: KeyEvent) -> Option<Action> {
    match (event.modifiers, event.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('s')) => Some(Action::RelationEditConfirm),
        (KeyModifiers::CONTROL, KeyCode::Char('f')) => Some(Action::RelationEditJsonFormat),
        (KeyModifiers::NONE, KeyCode::Enter) => Some(Action::RelationEditInsert('\n')),
        (KeyModifiers::NONE, KeyCode::Esc) => Some(Action::RelationEditCancel),
        (KeyModifiers::NONE, KeyCode::Up) => Some(Action::RelationEditJsonMove(-1)),
        (KeyModifiers::NONE, KeyCode::Down) => Some(Action::RelationEditJsonMove(1)),
        _ => map_relation_edit_text(event),
    }
}

/// The text-editing keys the relation editors share.
fn map_relation_edit_text(event: KeyEvent) -> Option<Action> {
    map_text_input_edit(event).map(|edit| match edit {
        TextInputEdit::Insert(character) => Action::RelationEditInsert(character),
        TextInputEdit::Backspace => Action::RelationEditBackspace,
        TextInputEdit::DeletePreviousWord => Action::RelationEditDeletePreviousWord,
        TextInputEdit::DeleteToStart | TextInputEdit::Clear => Action::RelationEditDeleteToStart,
        TextInputEdit::Delete => Action::RelationEditDelete,
        TextInputEdit::MoveLeft => Action::RelationEditMoveLeft,
        TextInputEdit::MoveRight => Action::RelationEditMoveRight,
        TextInputEdit::MoveHome => Action::RelationEditMoveHome,
        TextInputEdit::MoveEnd => Action::RelationEditMoveEnd,
        TextInputEdit::Undo => Action::RelationEditUndo,
        TextInputEdit::Redo => Action::RelationEditRedo,
    })
}

fn map_text_input_edit(event: KeyEvent) -> Option<TextInputEdit> {
    if is_text_redo(event) {
        return Some(TextInputEdit::Redo);
    }
    if is_text_undo(event) {
        return Some(TextInputEdit::Undo);
    }
    if event.modifiers == KeyModifiers::CONTROL {
        return match event.code {
            KeyCode::Char('w') => Some(TextInputEdit::DeletePreviousWord),
            KeyCode::Char('u') => Some(TextInputEdit::DeleteToStart),
            KeyCode::Char('a') => Some(TextInputEdit::MoveHome),
            KeyCode::Char('e') => Some(TextInputEdit::MoveEnd),
            KeyCode::Char('h') => Some(TextInputEdit::Backspace),
            _ => None,
        };
    }
    if !event.modifiers.is_empty() && event.modifiers != KeyModifiers::SHIFT {
        return None;
    }
    match event.code {
        KeyCode::Backspace => Some(TextInputEdit::Backspace),
        KeyCode::Delete => Some(TextInputEdit::Delete),
        KeyCode::Left => Some(TextInputEdit::MoveLeft),
        KeyCode::Right => Some(TextInputEdit::MoveRight),
        KeyCode::Home => Some(TextInputEdit::MoveHome),
        KeyCode::End => Some(TextInputEdit::MoveEnd),
        KeyCode::Char(character) => Some(TextInputEdit::Insert(character)),
        _ => None,
    }
}

fn normalize_shift_tab(mut event: KeyEvent) -> KeyEvent {
    if matches!(event.code, KeyCode::Tab | KeyCode::BackTab)
        && event.modifiers.contains(KeyModifiers::SHIFT)
    {
        event.code = KeyCode::BackTab;
        event.modifiers.remove(KeyModifiers::SHIFT);
    }
    event
}

fn map_transaction_review(event: KeyEvent, app: &App) -> Option<Action> {
    let preview_focused = app
        .overlay
        .as_ref()
        .and_then(|overlay| match overlay {
            Overlay::RelationTransactionConfirm(review) => Some(
                review.focus
                    == crate::model::transaction_review::TransactionReviewFocus::SqlPreview,
            ),
            _ => None,
        })
        .unwrap_or(false);
    match event.code {
        KeyCode::Tab => Some(Action::TransactionReviewFocusNext),
        KeyCode::BackTab => Some(Action::TransactionReviewFocusPrevious),
        KeyCode::Esc if preview_focused && app.review_preview_has_pending_interaction() => {
            Some(Action::ReadOnlyEditorKey {
                session_id: app.review_preview_session_id()?,
                event,
            })
        }
        KeyCode::Esc => Some(Action::CancelTransactionExit),
        KeyCode::Enter if !preview_focused => Some(Action::ConfirmTransactionExit),
        KeyCode::Left if !preview_focused => Some(Action::TransactionReviewMoveButton(-1)),
        KeyCode::Right if !preview_focused => Some(Action::TransactionReviewMoveButton(1)),
        KeyCode::Left | KeyCode::Right if preview_focused => Some(Action::ReadOnlyEditorKey {
            session_id: app.review_preview_session_id()?,
            event,
        }),
        KeyCode::Up if !preview_focused => Some(Action::TransactionReviewFocusPrevious),
        KeyCode::Down if !preview_focused => Some(Action::TransactionReviewFocusNext),
        KeyCode::PageUp if preview_focused => Some(Action::ReadOnlyEditorKey {
            session_id: app.review_preview_session_id()?,
            event,
        }),
        KeyCode::PageDown if preview_focused => Some(Action::ReadOnlyEditorKey {
            session_id: app.review_preview_session_id()?,
            event,
        }),
        _ if preview_focused => Some(Action::ReadOnlyEditorKey {
            session_id: app.review_preview_session_id()?,
            event,
        }),
        _ => None,
    }
}

fn map_single_line_text_input_edit(event: KeyEvent) -> Option<TextInputEdit> {
    map_text_input_edit(event).map(|edit| match edit {
        TextInputEdit::DeleteToStart => TextInputEdit::Clear,
        edit => edit,
    })
}

fn map_profile_manager(event: KeyEvent, app: &App) -> Option<Action> {
    let manager = app.profile_manager.as_ref()?;
    if manager.page != ProfileManagerPage::Form && !event.modifiers.is_empty() {
        return None;
    }
    if manager.page == ProfileManagerPage::Form
        && (event.modifiers.contains(KeyModifiers::CONTROL)
            || is_text_undo(event)
            || is_text_redo(event))
        && ((event.modifiers & !(KeyModifiers::CONTROL | KeyModifiers::SHIFT)).is_empty()
            || is_text_undo(event)
            || is_text_redo(event))
    {
        if is_profile_history_field(manager.selected_field) {
            if is_text_redo(event) {
                return Some(Action::ProfileRedo);
            }
            if is_text_undo(event) {
                return Some(Action::ProfileUndo);
            }
        }
        return match event.code {
            KeyCode::Char('t') => Some(Action::ProfileTest),
            KeyCode::Char('w') if is_text_field(manager.selected_field) => {
                Some(Action::ProfileDeletePreviousWord)
            }
            KeyCode::Char('u') if is_text_field(manager.selected_field) => {
                Some(Action::ProfileDeleteToStart)
            }
            KeyCode::Char('a') if is_text_field(manager.selected_field) => {
                Some(Action::ProfileMoveHome)
            }
            KeyCode::Char('e') if is_text_field(manager.selected_field) => {
                Some(Action::ProfileMoveEnd)
            }
            _ => None,
        };
    }
    let alt_gr_text = manager.page == ProfileManagerPage::Form
        && is_text_field(manager.selected_field)
        && matches!(event.code, KeyCode::Char(_))
        && event
            .modifiers
            .contains(KeyModifiers::CONTROL | KeyModifiers::ALT)
        && (event.modifiers & !(KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::ALT))
            .is_empty();
    if manager.page == ProfileManagerPage::Form
        && !(event.modifiers & !KeyModifiers::SHIFT).is_empty()
        && !alt_gr_text
    {
        return None;
    }
    match manager.page {
        ProfileManagerPage::Form => map_profile_form(event, manager.selected_field),
        ProfileManagerPage::Scope => match event.code {
            KeyCode::Esc | KeyCode::Enter => Some(Action::ProfileScopeBack),
            KeyCode::Char('r') if !manager.scope_discovery_loading() => {
                Some(Action::ProfileRefreshScope)
            }
            KeyCode::Up | KeyCode::Char('k') => Some(Action::ProfileScopeMove(-1)),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::ProfileScopeMove(1)),
            KeyCode::Char(' ') if !manager.scope_discovery_loading() => manager
                .scope_selected_row
                .clone()
                .map(Action::ProfileToggleScopeRow),
            _ => None,
        },
        ProfileManagerPage::ConfirmDelete => {
            map_profile_delete_confirmation(event.code, manager.delete_focus)
        }
    }
}

fn map_profile_form(event: KeyEvent, field: ProfileField) -> Option<Action> {
    let code = event.code;
    match code {
        KeyCode::Esc => return Some(Action::CloseProfileManager),
        KeyCode::Tab if event.modifiers.contains(KeyModifiers::SHIFT) => {
            return Some(Action::ProfileFieldPrevious);
        }
        KeyCode::Tab => return Some(Action::ProfileFieldNext),
        KeyCode::BackTab => return Some(Action::ProfileFieldPrevious),
        _ => {}
    }
    if code == KeyCode::Enter {
        if !event.modifiers.is_empty() {
            return None;
        }
        return Some(match field {
            ProfileField::Test => Action::ProfileTest,
            ProfileField::Cancel => Action::CloseProfileManager,
            _ => Action::ProfileSave { connect: true },
        });
    }
    if is_text_field(field) {
        return match code {
            KeyCode::Char(character) => Some(Action::ProfileInsert(ProfileInput::from(character))),
            KeyCode::Backspace => Some(Action::ProfileBackspace),
            KeyCode::Delete => Some(Action::ProfileDeleteCharacter),
            KeyCode::Left => Some(Action::ProfileMoveLeft),
            KeyCode::Right => Some(Action::ProfileMoveRight),
            KeyCode::Up => Some(Action::ProfileFieldPrevious),
            KeyCode::Down => Some(Action::ProfileFieldNext),
            KeyCode::Home => Some(Action::ProfileMoveHome),
            KeyCode::End => Some(Action::ProfileMoveEnd),
            _ => None,
        };
    }
    if field == ProfileField::VisibleObjects {
        return match code {
            KeyCode::Char(' ') => Some(Action::ProfileOpenScope),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::ProfileFieldPrevious),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::ProfileFieldNext),
            _ => None,
        };
    }
    if matches!(field, ProfileField::Kind | ProfileField::DatabaseCategory) {
        return match code {
            KeyCode::Left | KeyCode::Char('h') => Some(Action::ProfileCycle(-1)),
            KeyCode::Right | KeyCode::Char('l') => Some(Action::ProfileCycle(1)),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::ProfileFieldPrevious),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::ProfileFieldNext),
            _ => None,
        };
    }
    if is_cycle_field(field) {
        return match code {
            KeyCode::Left | KeyCode::Char('h') => Some(Action::ProfileCycle(-1)),
            KeyCode::Right | KeyCode::Char(' ' | 'l') => Some(Action::ProfileCycle(1)),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::ProfileFieldPrevious),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::ProfileFieldNext),
            _ => None,
        };
    }
    if is_toggle_field(field) {
        return match code {
            KeyCode::Char(' ') => Some(Action::ProfileToggle),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::ProfileFieldPrevious),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::ProfileFieldNext),
            _ => None,
        };
    }
    match (field, code) {
        (_, KeyCode::Up | KeyCode::Char('k')) => Some(Action::ProfileFieldPrevious),
        (_, KeyCode::Down | KeyCode::Char('j')) => Some(Action::ProfileFieldNext),
        (ProfileField::Test | ProfileField::Save | ProfileField::Cancel, KeyCode::Char(' ')) => {
            Some(match field {
                ProfileField::Test => Action::ProfileTest,
                ProfileField::Save => Action::ProfileSave { connect: true },
                ProfileField::Cancel => Action::CloseProfileManager,
                _ => unreachable!(),
            })
        }
        _ => None,
    }
}

fn map_profile_delete_confirmation(
    code: KeyCode,
    focus: crate::model::profile_manager::ProfileDeleteFocus,
) -> Option<Action> {
    match code {
        KeyCode::Enter => Some(match focus {
            crate::model::profile_manager::ProfileDeleteFocus::Cancel => {
                Action::ProfileCancelDelete
            }
            crate::model::profile_manager::ProfileDeleteFocus::Delete => {
                Action::ProfileConfirmDelete
            }
        }),
        KeyCode::Esc => Some(Action::ProfileCancelDelete),
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
            Some(Action::ToggleProfileDeleteFocus)
        }
        _ => None,
    }
}

fn is_text_field(field: ProfileField) -> bool {
    matches!(
        field,
        ProfileField::Name
            | ProfileField::Host
            | ProfileField::Port
            | ProfileField::User
            | ProfileField::Password
            | ProfileField::Url
            | ProfileField::Database
            | ProfileField::Schema
            | ProfileField::SqlitePath
    )
}

fn is_profile_history_field(field: ProfileField) -> bool {
    is_text_field(field)
}

fn is_cycle_field(field: ProfileField) -> bool {
    matches!(
        field,
        ProfileField::UrlFormat
            | ProfileField::SslMode
            | ProfileField::Environment
            | ProfileField::PasswordStorage
    )
}

fn is_toggle_field(field: ProfileField) -> bool {
    matches!(field, ProfileField::ReadOnly | ProfileField::SqliteMemory)
}

fn map_explorer(code: KeyCode, app: &App) -> Option<Action> {
    let selected_profile = app
        .explorer
        .normalized
        .selected
        .as_ref()
        .and_then(|node| node.profile_id());
    match code {
        KeyCode::Char('y') => return Some(Action::CopyExplorerSelection),
        KeyCode::Char('/') => return Some(Action::ExplorerFindOpen),
        KeyCode::Char('f') => return Some(Action::ExplorerSearchOpen),
        KeyCode::Char('a') => {
            if let Some(ExplorerNodeId::RedisDatabase {
                profile_id,
                database,
            }) = app.explorer.normalized.selected.as_ref()
            {
                return Some(Action::OpenRedisObjectCreateAt {
                    profile_id: *profile_id,
                    database: *database,
                });
            }
            if matches!(
                app.explorer.normalized.selected,
                Some(ExplorerNodeId::Profile(_))
            ) {
                return Some(Action::OpenExplorerAdd);
            }
            if matches!(
                app.explorer.normalized.selected,
                Some(ExplorerNodeId::EmptyProfiles)
            ) {
                return Some(Action::OpenExplorerAdd);
            }
            if matches!(
                app.explorer.normalized.selected,
                Some(ExplorerNodeId::ConnectionGroup { .. })
            ) {
                return Some(Action::ProfileGroupCreate);
            }
            if matches!(
                app.explorer.normalized.selected,
                Some(
                    ExplorerNodeId::PrincipalGroup { .. }
                        | ExplorerNodeId::Principal { .. }
                        | ExplorerNodeId::PrincipalNotice { .. }
                )
            ) {
                return Some(Action::OpenExplorerAdd);
            }
            return crate::help::shortcut_is_available_in_app(
                app,
                crate::help::HelpShortcutId::ExplorerCreateCatalog,
            )
            .then_some(Action::OpenCatalogCreate);
        }
        KeyCode::Char('e') => {
            if matches!(
                app.explorer.normalized.selected,
                Some(ExplorerNodeId::Principal { .. })
            ) {
                return Some(Action::OpenCatalogEdit);
            }
            if matches!(
                app.explorer.normalized.selected,
                Some(ExplorerNodeId::ConnectionGroup { .. })
            ) {
                return Some(Action::ProfileGroupOpen);
            }
            let id = if matches!(
                app.explorer.normalized.selected,
                Some(ExplorerNodeId::Profile(_))
            ) {
                crate::help::HelpShortcutId::ExplorerEditProfile
            } else {
                crate::help::HelpShortcutId::ExplorerEditCatalog
            };
            return crate::help::shortcut_is_available_in_app(app, id)
                .then_some(Action::OpenCatalogEdit);
        }
        KeyCode::Char('d') => {
            return match app.explorer.normalized.selected.as_ref() {
                Some(ExplorerNodeId::ConnectionGroup { .. }) => {
                    Some(Action::ProfileGroupDeleteConfirm)
                }
                Some(ExplorerNodeId::Profile(profile_id)) => Some(Action::ProfileRequestDelete {
                    profile_id: *profile_id,
                }),
                Some(ExplorerNodeId::Catalog(id)) => {
                    Some(Action::RequestDropCatalogObject { id: id.clone() })
                }
                Some(ExplorerNodeId::Principal { .. }) => Some(Action::OpenPrincipalDrop),
                _ => None,
            };
        }
        KeyCode::Char('c') => {
            return selected_profile.map(|profile_id| Action::RequestProfileConnect { profile_id });
        }
        KeyCode::Char('x') => {
            return selected_profile
                .map(|profile_id| Action::RequestProfileDisconnect { profile_id });
        }
        KeyCode::Char('s') => return Some(Action::OpenProfileAccess),
        KeyCode::Char('g') => {
            return matches!(
                app.explorer.normalized.selected,
                Some(ExplorerNodeId::Profile(_))
            )
            .then_some(Action::ProfileGroupOpen);
        }
        _ => {}
    }
    match code {
        KeyCode::Down => Some(Action::ExplorerMove(1)),
        KeyCode::Up => Some(Action::ExplorerMove(-1)),
        KeyCode::Char('J') => Some(Action::ProfileGroupMove(1)),
        KeyCode::Char('K') => Some(Action::ProfileGroupMove(-1)),
        KeyCode::Char('G') => Some(Action::ExplorerSelectTarget(
            crate::model::explorer::ExplorerNodeTarget::Last,
        )),
        KeyCode::Char('H') => Some(Action::ExplorerSelectTarget(
            crate::model::explorer::ExplorerNodeTarget::ViewTop,
        )),
        KeyCode::Char('M') => Some(Action::ExplorerSelectTarget(
            crate::model::explorer::ExplorerNodeTarget::ViewMiddle,
        )),
        KeyCode::Char('L') => Some(Action::ExplorerSelectTarget(
            crate::model::explorer::ExplorerNodeTarget::ViewBottom,
        )),
        KeyCode::Right => Some(Action::ExplorerExpand),
        KeyCode::Left => Some(Action::ExplorerCollapse),
        KeyCode::Enter => Some(Action::ExplorerOpenSelected),
        KeyCode::Char('o') => Some(Action::ExplorerToggle),
        KeyCode::Char('r') => Some(Action::ExplorerRefresh),
        KeyCode::Char('p') => Some(Action::OpenSelectedRelation {
            view: crate::model::relation::RelationView::Data,
        }),
        KeyCode::Char('D') => Some(Action::OpenSelectedRelation {
            view: crate::model::relation::RelationView::Ddl,
        }),
        KeyCode::Home => Some(Action::ExplorerSelectTarget(
            crate::model::explorer::ExplorerNodeTarget::First,
        )),
        KeyCode::End => Some(Action::ExplorerSelectTarget(
            crate::model::explorer::ExplorerNodeTarget::Last,
        )),
        _ => None,
    }
}

fn is_read_only_editor_key(event: KeyEvent) -> bool {
    let visual_line = event.code == KeyCode::Char('V')
        && (event.modifiers.is_empty() || event.modifiers == KeyModifiers::SHIFT);
    if visual_line {
        return true;
    }
    match event.code {
        KeyCode::Char('o' | '1' | '2' | '3' | 'p' | 'D' | 'r') => false,
        KeyCode::Char('c') if event.modifiers == KeyModifiers::CONTROL => false,
        KeyCode::Char('w') if event.modifiers == KeyModifiers::CONTROL => false,
        KeyCode::F(1 | 5) | KeyCode::Tab | KeyCode::BackTab => false,
        _ => event.modifiers.is_empty() || event.modifiers == KeyModifiers::CONTROL,
    }
}

fn map_results(code: KeyCode, _app: &App) -> Option<Action> {
    match code {
        KeyCode::Char('0' | '^') => Some(Action::GridSelectColumn(
            crate::model::tab::GridColumnTarget::First,
        )),
        KeyCode::Char('$') => Some(Action::GridSelectColumn(
            crate::model::tab::GridColumnTarget::Last,
        )),
        KeyCode::Char('[') => Some(Action::GridResizeColumn(-1)),
        KeyCode::Char(']') => Some(Action::GridResizeColumn(1)),
        KeyCode::Char('=') => Some(Action::GridResetColumnWidth),
        KeyCode::Char('v') => Some(Action::OpenRecordView),
        KeyCode::Char('h') => Some(Action::GridMove {
            rows: 0,
            columns: -1,
        }),
        KeyCode::Char('j') => Some(Action::GridMove {
            rows: 1,
            columns: 0,
        }),
        KeyCode::Char('k') => Some(Action::GridMove {
            rows: -1,
            columns: 0,
        }),
        KeyCode::Char('l') => Some(Action::GridMove {
            rows: 0,
            columns: 1,
        }),
        KeyCode::Left => Some(Action::GridMove {
            rows: 0,
            columns: -1,
        }),
        KeyCode::Down => Some(Action::GridMove {
            rows: 1,
            columns: 0,
        }),
        KeyCode::Up => Some(Action::GridMove {
            rows: -1,
            columns: 0,
        }),
        KeyCode::Right => Some(Action::GridMove {
            rows: 0,
            columns: 1,
        }),
        KeyCode::Char('G') => Some(Action::GridSelectRow(
            crate::model::tab::GridRowTarget::Last,
        )),
        KeyCode::Char('H') => Some(Action::GridSelectRow(
            crate::model::tab::GridRowTarget::ViewTop,
        )),
        KeyCode::Char('M') => Some(Action::GridSelectRow(
            crate::model::tab::GridRowTarget::ViewMiddle,
        )),
        KeyCode::Char('L') => Some(Action::GridSelectRow(
            crate::model::tab::GridRowTarget::ViewBottom,
        )),
        KeyCode::Char('o') => match _app.tabs.get(_app.active_tab) {
            Some(crate::model::tab::WorkspaceTab::Relation(tab)) => {
                Some(Action::SetRelationView(match tab.view {
                    crate::model::relation::RelationView::Data => {
                        crate::model::relation::RelationView::Ddl
                    }
                    crate::model::relation::RelationView::Ddl => {
                        crate::model::relation::RelationView::Data
                    }
                }))
            }
            _ => Some(Action::ToggleResultView),
        },
        _ => None,
    }
}

fn map_relation(code: KeyCode, app: &App) -> Option<Action> {
    match code {
        KeyCode::Char('[') => Some(Action::GridResizeColumn(-1)),
        KeyCode::Char(']') => Some(Action::GridResizeColumn(1)),
        KeyCode::Char('=') => Some(Action::GridResetColumnWidth),
        KeyCode::Char('o') => map_results(code, app),
        KeyCode::Char('p') => Some(Action::SetRelationView(
            crate::model::relation::RelationView::Data,
        )),
        KeyCode::Char('D') => Some(Action::SetRelationView(
            crate::model::relation::RelationView::Ddl,
        )),
        KeyCode::Char('r') => Some(Action::RefreshActiveRelation),
        _ => map_results(code, app),
    }
}

fn map_data_query(event: KeyEvent, app: &App) -> Option<Action> {
    let query = match app.tabs.get(app.active_tab) {
        Some(crate::model::tab::WorkspaceTab::Relation(tab))
            if tab.view == crate::model::relation::RelationView::Data =>
        {
            Some(&tab.query)
        }
        Some(crate::model::tab::WorkspaceTab::Sql(tab))
            if tab.result_view == crate::model::tab::ResultView::Data =>
        {
            Some(&tab.query)
        }
        _ => None,
    }?;
    if !matches!(
        query.capability,
        crate::model::data_query::DataQueryCapability::Relation
            | crate::model::data_query::DataQueryCapability::Sql
    ) {
        return None;
    }
    if let Some(input) = query.focus {
        use crate::model::data_query::DataQueryInput;
        if query.completion.is_some() {
            match event.code {
                KeyCode::Down if event.modifiers.is_empty() => {
                    return Some(Action::DataQueryCompletionNext);
                }
                KeyCode::Char('n') if event.modifiers == KeyModifiers::CONTROL => {
                    return Some(Action::DataQueryCompletionNext);
                }
                KeyCode::Up if event.modifiers.is_empty() => {
                    return Some(Action::DataQueryCompletionPrevious);
                }
                KeyCode::Char('p') if event.modifiers == KeyModifiers::CONTROL => {
                    return Some(Action::DataQueryCompletionPrevious);
                }
                KeyCode::Tab | KeyCode::Enter if event.modifiers.is_empty() => {
                    return Some(Action::DataQueryCompletionAccept);
                }
                KeyCode::Esc if event.modifiers.is_empty() => {
                    return Some(Action::DataQueryCompletionDismiss);
                }
                _ => {}
            }
        }
        return match event.code {
            KeyCode::Esc if event.modifiers.is_empty() => Some(Action::CancelDataQueryInput),
            KeyCode::Enter if event.modifiers.is_empty() => Some(Action::SubmitDataQuery),
            KeyCode::Tab if event.modifiers.is_empty() => {
                Some(Action::FocusDataQueryInput(match input {
                    DataQueryInput::Where => DataQueryInput::OrderBy,
                    DataQueryInput::OrderBy => DataQueryInput::Where,
                }))
            }
            KeyCode::BackTab => Some(Action::FocusDataQueryInput(match input {
                DataQueryInput::Where => DataQueryInput::OrderBy,
                DataQueryInput::OrderBy => DataQueryInput::Where,
            })),
            _ => map_text_input_edit(event).map(|edit| match edit {
                TextInputEdit::Insert(character) => Action::DataQueryInsert(character),
                TextInputEdit::Backspace => Action::DataQueryBackspace,
                TextInputEdit::DeletePreviousWord => Action::DataQueryDeletePreviousWord,
                TextInputEdit::DeleteToStart => Action::DataQueryDeleteToStart,
                TextInputEdit::Clear => Action::DataQueryDeleteToStart,
                TextInputEdit::Delete => Action::DataQueryDelete,
                TextInputEdit::MoveLeft => Action::DataQueryMoveLeft,
                TextInputEdit::MoveRight => Action::DataQueryMoveRight,
                TextInputEdit::MoveHome => Action::DataQueryMoveHome,
                TextInputEdit::MoveEnd => Action::DataQueryMoveEnd,
                TextInputEdit::Undo => Action::DataQueryUndo,
                TextInputEdit::Redo => Action::DataQueryRedo,
            }),
        };
    }
    if app.focus == Focus::Results {
        return match event.code {
            KeyCode::Char('/') => Some(Action::FocusDataQueryInput(
                crate::model::data_query::DataQueryInput::Where,
            )),
            KeyCode::Char('s') => Some(Action::FocusDataQueryInput(
                crate::model::data_query::DataQueryInput::OrderBy,
            )),
            _ => None,
        };
    }
    None
}

fn active_data_query_has_focus(app: &App) -> bool {
    match app.tabs.get(app.active_tab) {
        Some(crate::model::tab::WorkspaceTab::Relation(tab)) => tab.query.focus.is_some(),
        Some(crate::model::tab::WorkspaceTab::Sql(tab)) => tab.query.focus.is_some(),
        Some(
            crate::model::tab::WorkspaceTab::Dashboard(_)
            | crate::model::tab::WorkspaceTab::MongoDashboard(_),
        ) => false,
        Some(crate::model::tab::WorkspaceTab::RedisBrowser(_)) => false,
        Some(crate::model::tab::WorkspaceTab::PrincipalDdl(_)) => false,
        Some(crate::model::tab::WorkspaceTab::MongoBrowser(_)) => false,
        Some(crate::model::tab::WorkspaceTab::MongoConsole(_)) => false,
        None => false,
    }
}

fn redis_help_binding_is_active(
    app: &App,
    event: KeyEvent,
    bindings: &crate::config::KeyBindings,
) -> bool {
    let Some(crate::model::tab::WorkspaceTab::RedisBrowser(tab)) = app.tabs.get(app.active_tab)
    else {
        return false;
    };
    if app.focus != Focus::Results {
        return false;
    }

    let printable_input = matches!(event.code, KeyCode::Char(_));
    let input_mode = match tab.focus {
        crate::model::redis_browser::RedisBrowserFocus::Keys => tab
            .find
            .as_ref()
            .is_some_and(|find| find.phase == crate::model::redis_browser::RedisFindPhase::Editing),
        crate::model::redis_browser::RedisBrowserFocus::Preview => matches!(
            app.active_read_only_editor_mode(),
            Some(EditorMode::Insert | EditorMode::Replace)
        ),
    };
    if input_mode && printable_input {
        return false;
    }
    bindings.matches("help", event)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{Keymap, Pending, PendingState, map_shortcut, map_text_input_edit};
    use crate::{
        action::Action,
        app::App,
        model::{
            relation::RelationTab,
            relation_edit::{RelationEditSession, RelationGridMode},
            tab::{
                GridColumnTarget, GridRowAlignment, GridRowTarget, GridScrollAmount, WorkspaceTab,
            },
            workspace::Focus,
            workspace::Overlay,
        },
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    fn relation_app(mode: RelationGridMode) -> App {
        let mut app = App::new(Vec::new());
        let mut tab = RelationTab::new("users");
        let mut edit = RelationEditSession::from_rows(vec![vec![]; 3]);
        edit.mode = mode;
        tab.edit = Some(edit);
        app.tabs.push(WorkspaceTab::Relation(tab));
        app.active_tab = 1;
        app.focus = Focus::Results;
        app
    }

    fn relation_app_without_edit() -> App {
        let mut app = App::new(Vec::new());
        app.tabs
            .push(WorkspaceTab::Relation(RelationTab::new("users")));
        app.active_tab = 1;
        app.focus = Focus::Results;
        app
    }

    #[test]
    fn advertised_shortcuts_use_the_same_contextual_keymap_as_keyboard_input() {
        let mut app = relation_app(RelationGridMode::Browse);
        app.overlay = Some(Overlay::Message {
            title: "test".into(),
            body: "test".into(),
        });
        assert_eq!(
            map_shortcut(&[KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)], &app,),
            Some(Action::DismissOverlay),
        );
        assert_eq!(map_shortcut(&[], &app), None);
    }

    #[test]
    fn text_history_keys_are_normalized_across_terminal_encodings() {
        let modifier = if cfg!(target_os = "macos") {
            KeyModifiers::SUPER
        } else {
            KeyModifiers::CONTROL
        };
        let other_modifier = if cfg!(target_os = "macos") {
            KeyModifiers::CONTROL
        } else {
            KeyModifiers::SUPER
        };

        assert_eq!(
            map_text_input_edit(KeyEvent::new(KeyCode::Char('z'), modifier)),
            Some(crate::model::text_input::TextInputEdit::Undo),
        );
        for event in [
            KeyEvent::new(KeyCode::Char('z'), modifier | KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::Char('Z'), modifier),
            KeyEvent::new(KeyCode::Char('Z'), modifier | KeyModifiers::SHIFT),
        ] {
            assert_eq!(
                map_text_input_edit(event),
                Some(crate::model::text_input::TextInputEdit::Redo),
            );
        }
        for event in [
            KeyEvent::new(KeyCode::Char('z'), other_modifier),
            KeyEvent::new(KeyCode::Char('z'), other_modifier | KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::Char('z'), modifier | KeyModifiers::ALT),
            KeyEvent::new(
                KeyCode::Char('Z'),
                modifier | KeyModifiers::SHIFT | KeyModifiers::ALT,
            ),
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::SHIFT),
        ] {
            assert!(!matches!(
                map_text_input_edit(event),
                Some(
                    crate::model::text_input::TextInputEdit::Undo
                        | crate::model::text_input::TextInputEdit::Redo
                )
            ));
        }
    }

    #[test]
    fn cell_editor_history_keys_are_owned_by_cell_input() {
        let app = relation_app(RelationGridMode::EditCell(Box::new(
            crate::model::relation_edit::CellEditorState {
                row: 0,
                column: 0,
                input: Default::default(),
                error: None,
            },
        )));
        let mut keymap = Keymap::default();
        assert_eq!(
            keymap.map(
                KeyEvent::new(
                    KeyCode::Char('z'),
                    if cfg!(target_os = "macos") {
                        KeyModifiers::SUPER
                    } else {
                        KeyModifiers::CONTROL
                    },
                ),
                &app,
            ),
            Some(Action::RelationEditUndo)
        );
        assert_eq!(
            keymap.map(
                KeyEvent::new(
                    KeyCode::Char('z'),
                    (if cfg!(target_os = "macos") {
                        KeyModifiers::SUPER
                    } else {
                        KeyModifiers::CONTROL
                    }) | KeyModifiers::SHIFT,
                ),
                &app,
            ),
            Some(Action::RelationEditRedo)
        );
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// A JSON cell editor buffer, the shape the SQL cell editor uses for a
    /// whole-value edit.
    fn json_cell_editor() -> RelationGridMode {
        RelationGridMode::EditCell(Box::new(crate::model::relation_edit::CellEditorState {
            row: 0,
            column: 0,
            input: crate::model::cell_editor::CellEditorBuffer {
                presence: crate::model::cell_editor::CellEditorPresence::Value,
                content: crate::model::cell_editor::CellEditorContent::Typed {
                    kind: crate::model::cell_editor::CellEditorKind::Json,
                    draft: crate::model::cell_editor::TypedDraft::Json(
                        crate::model::cell_editor::JsonBuffer::new(r#"{"a":1}"#),
                    ),
                },
                ..Default::default()
            },
            error: None,
        }))
    }

    fn document_modal() -> RelationGridMode {
        let namespace = crate::db::mongodb::types::MongoNamespace::collection("app", "bikes");
        let document = crate::db::mongodb::types::MongoDocument {
            bson: mongodb::bson::doc! { "_id": 1_i32, "name": "Ruedas Pro" },
            raw: mongodb::bson::raw::RawDocumentBuf::from_bytes(
                mongodb::bson::to_vec(&mongodb::bson::doc! { "_id": 1_i32, "name": "Ruedas Pro" })
                    .expect("encodes"),
            )
            .expect("raw"),
            editable: true,
        };
        let mut state =
            crate::model::relation_edit::DocumentEditorState::new(0, &namespace, &document)
                .expect("document state");
        // This fixture pins the EJSON view keys; the modal opens on the field
        // list, which has its own mapping.
        state.view = crate::model::relation_edit::DocumentEditorView::Ejson;
        RelationGridMode::EditDocument(Box::new(state))
    }

    #[test]
    fn the_document_modal_owns_save_cancel_and_newline_keys() {
        let app = relation_app(document_modal());
        let mut keymap = Keymap::default();

        assert_eq!(
            keymap.map(control_key('s'), &app),
            Some(Action::RelationEditConfirm),
            "Ctrl-S saves the document"
        );
        assert_eq!(
            keymap.map(key(KeyCode::Esc), &app),
            Some(Action::RelationEditCancel),
            "Esc cancels the document edit"
        );
        assert_eq!(
            keymap.map(key(KeyCode::Enter), &app),
            Some(Action::RelationEditInsert('\n')),
            "Enter inserts a newline instead of saving"
        );
        assert_eq!(
            keymap.map(key(KeyCode::Char('x')), &app),
            Some(Action::RelationEditInsert('x')),
            "printable keys edit the document"
        );
        assert_eq!(
            keymap.map(key(KeyCode::Up), &app),
            Some(Action::RelationEditJsonMove(-1))
        );
        assert_eq!(
            keymap.map(control_key('f'), &app),
            Some(Action::RelationEditJsonFormat)
        );
        assert_eq!(
            keymap.map(key(KeyCode::Backspace), &app),
            Some(Action::RelationEditBackspace)
        );
    }

    #[test]
    fn the_sql_json_cell_editor_keeps_its_own_keys() {
        let app = relation_app(json_cell_editor());
        let mut keymap = Keymap::default();

        assert_eq!(
            keymap.map(control_key('s'), &app),
            Some(Action::RelationEditConfirm)
        );
        assert_eq!(
            keymap.map(key(KeyCode::Enter), &app),
            Some(Action::RelationEditInsert('\n'))
        );
        assert_eq!(
            keymap.map(key(KeyCode::Esc), &app),
            Some(Action::RelationEditCancel)
        );
    }

    fn control_key(code: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(code), KeyModifiers::CONTROL)
    }

    fn control_shift_key(code: char) -> KeyEvent {
        KeyEvent::new(
            KeyCode::Char(code),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        )
    }

    #[test]
    fn smart_pane_binding_maps_cmd_ctrl_l_when_explicitly_configured() {
        let mut app = App::new(Vec::new());
        let mut config = crate::config::AppConfig::default();
        config
            .keybindings
            .panes
            .insert("smart-focus-pane-right".into(), vec!["Cmd+Ctrl+l".into()]);
        app.key_bindings = config.keybindings.key_bindings().unwrap();
        let mut keymap = Keymap::with_sequence_timeout_and_bindings(
            Duration::from_millis(750),
            app.key_bindings.clone(),
        );
        assert_eq!(
            keymap.map(
                KeyEvent::new(
                    KeyCode::Char('l'),
                    KeyModifiers::SUPER | KeyModifiers::CONTROL
                ),
                &app,
            ),
            Some(Action::SmartFocusPane(
                crate::model::pane_navigation::PaneDirection::Right
            ))
        );
    }

    #[test]
    fn smart_resize_bindings_map_all_cmd_ctrl_shift_directions() {
        let mut app = App::new(Vec::new());
        let mut config = crate::config::AppConfig::default();
        for (command, key) in [
            ("smart-resize-pane-left", 'h'),
            ("smart-resize-pane-down", 'j'),
            ("smart-resize-pane-up", 'k'),
            ("smart-resize-pane-right", 'l'),
        ] {
            config
                .keybindings
                .panes
                .insert(command.into(), vec![format!("Cmd+Ctrl+Shift+{key}")]);
        }
        app.key_bindings = config.keybindings.key_bindings().unwrap();
        let mut keymap = Keymap::with_sequence_timeout_and_bindings(
            Duration::from_millis(750),
            app.key_bindings.clone(),
        );
        for (key, direction) in [
            ('h', crate::model::pane_navigation::PaneDirection::Left),
            ('j', crate::model::pane_navigation::PaneDirection::Down),
            ('k', crate::model::pane_navigation::PaneDirection::Up),
            ('l', crate::model::pane_navigation::PaneDirection::Right),
        ] {
            assert_eq!(
                keymap.map(
                    KeyEvent::new(
                        KeyCode::Char(key),
                        KeyModifiers::SUPER | KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                    ),
                    &app,
                ),
                Some(Action::SmartResizePane(direction))
            );
        }
    }

    #[test]
    fn smart_resize_accepts_repeats_and_clears_pending_sequences() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Results;
        let mut config = crate::config::AppConfig::default();
        config.keybindings.panes.insert(
            "smart-resize-pane-left".into(),
            vec!["Cmd+Ctrl+Shift+h".into()],
        );
        let bindings = config.keybindings.key_bindings().unwrap();
        app.key_bindings = bindings.clone();
        let mut keymap =
            Keymap::with_sequence_timeout_and_bindings(Duration::from_millis(750), bindings);
        keymap.map(key(KeyCode::Char('g')), &app);
        assert!(keymap.pending.is_some());

        let mut resize = KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::SUPER | KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        resize.kind = KeyEventKind::Repeat;
        assert_eq!(
            keymap.map(resize, &app),
            Some(Action::SmartResizePane(
                crate::model::pane_navigation::PaneDirection::Left
            ))
        );
        assert!(keymap.pending.is_none());

        resize.kind = KeyEventKind::Release;
        assert_eq!(keymap.map(resize, &app), None);
    }

    #[test]
    fn smart_maximize_works_in_workspace_modes_and_cancels_pending_keys() {
        use crate::model::editor::EditorMode;

        let mut app = App::new(Vec::new());
        let mut config = crate::config::AppConfig::default();
        config.keybindings.panes.insert(
            "smart-toggle-pane-maximized".into(),
            vec!["Ctrl+Alt+F12".into()],
        );
        let mut keymap = Keymap::with_sequence_timeout_and_bindings(
            Duration::from_millis(750),
            config.keybindings.key_bindings().unwrap(),
        );
        let shortcut = KeyEvent::new(KeyCode::F(12), KeyModifiers::CONTROL | KeyModifiers::ALT);
        for focus in [Focus::Explorer, Focus::Results, Focus::Editor] {
            app.focus = focus;
            assert_eq!(
                keymap.map(shortcut, &app),
                Some(Action::SmartTogglePaneMaximized)
            );
        }
        app.update(Action::EditorKey(key(KeyCode::Esc)));
        for (enter_mode, mode) in [('i', EditorMode::Insert), ('v', EditorMode::VisualChar)] {
            app.update(Action::EditorKey(key(KeyCode::Char(enter_mode))));
            assert_eq!(app.active_editor_mode(), mode);
            assert_eq!(
                keymap.map(shortcut, &app),
                Some(Action::SmartTogglePaneMaximized)
            );
            app.update(Action::EditorKey(key(KeyCode::Esc)));
        }
        app.focus = Focus::Results;
        keymap.map(key(KeyCode::Char('g')), &app);
        assert!(keymap.pending.is_some());
        assert_eq!(
            keymap.map(shortcut, &app),
            Some(Action::SmartTogglePaneMaximized)
        );
        assert!(keymap.pending.is_none());
        for kind in [KeyEventKind::Repeat, KeyEventKind::Release] {
            assert_eq!(keymap.map(KeyEvent { kind, ..shortcut }, &app), None);
        }
        app.update(Action::ShowHelp);
        assert!(app.overlay.is_some());
        assert_eq!(keymap.map(shortcut, &app), None);
        app.overlay = None;
        app.update(Action::OpenOmni);
        assert!(app.omni.is_some());
        assert_eq!(keymap.map(shortcut, &app), None);
    }

    #[test]
    fn control_tab_shortcuts_work_only_in_workspace_panes() {
        let mut app = App::new(Vec::new());
        app.tabs.push(crate::model::tab::WorkspaceTab::Sql(
            crate::model::tab::ConsoleTab::new("test"),
        ));
        app.active_tab = 0;
        let mut keymap = Keymap::default();

        assert_eq!(keymap.map(control_key('n'), &app), Some(Action::NextTab));
        assert_eq!(
            keymap.map(control_key('p'), &app),
            Some(Action::PreviousTab)
        );
        assert_eq!(
            keymap.map(control_shift_key('n'), &app),
            Some(Action::MoveTabRight)
        );
        assert_eq!(
            keymap.map(control_shift_key('p'), &app),
            Some(Action::MoveTabLeft)
        );

        app.focus = Focus::Explorer;
        assert_eq!(keymap.map(control_key('n'), &app), None);
        assert_eq!(keymap.map(control_key('p'), &app), None);
        assert_eq!(keymap.map(control_shift_key('n'), &app), None);
        assert_eq!(keymap.map(control_shift_key('p'), &app), None);

        app.focus = Focus::Editor;
        assert_eq!(
            keymap.map(control_key('q'), &app),
            Some(Action::CloseActiveTab)
        );
        assert_eq!(
            keymap.map(control_shift_key('q'), &app),
            Some(Action::CloseOtherTabs)
        );
        assert_eq!(
            keymap.map(
                KeyEvent::new(
                    KeyCode::Char('Q'),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT
                ),
                &app
            ),
            Some(Action::CloseOtherTabs)
        );
    }

    #[test]
    fn tab_aliases_and_page_navigation_work_only_in_workspace_panes() {
        let mut app = App::new(Vec::new());
        app.tabs.push(crate::model::tab::WorkspaceTab::Relation(
            crate::model::relation::RelationTab::new("test"),
        ));
        app.active_tab = 0;
        let mut keymap = Keymap::default();

        app.focus = Focus::Results;
        keymap.map(key(KeyCode::Char('g')), &app);
        assert_eq!(
            keymap.map(key(KeyCode::Char('t')), &app),
            Some(Action::NextTab)
        );
        keymap.map(key(KeyCode::Char('[')), &app);
        assert_eq!(
            keymap.map(key(KeyCode::Char('t')), &app),
            Some(Action::PreviousTab)
        );
        keymap.map(key(KeyCode::Char(']')), &app);
        assert_eq!(
            keymap.map(key(KeyCode::Char('t')), &app),
            Some(Action::NextTab)
        );

        assert_eq!(
            keymap.map(
                KeyEvent::new(KeyCode::PageDown, KeyModifiers::CONTROL),
                &app
            ),
            Some(Action::NextTab)
        );
        assert_eq!(
            keymap.map(KeyEvent::new(KeyCode::PageUp, KeyModifiers::CONTROL), &app),
            Some(Action::PreviousTab)
        );

        app.focus = Focus::Explorer;
        keymap.map(key(KeyCode::Char('g')), &app);
        assert_eq!(keymap.map(key(KeyCode::Char('t')), &app), None);
        keymap.map(key(KeyCode::Char('[')), &app);
        assert_eq!(keymap.map(key(KeyCode::Char('t')), &app), None);
        keymap.map(key(KeyCode::Char(']')), &app);
        assert_eq!(keymap.map(key(KeyCode::Char('t')), &app), None);
        assert_eq!(
            keymap.map(
                KeyEvent::new(KeyCode::PageDown, KeyModifiers::CONTROL),
                &app
            ),
            None
        );
        assert_eq!(
            keymap.map(KeyEvent::new(KeyCode::PageUp, KeyModifiers::CONTROL), &app),
            None
        );
    }

    #[test]
    fn editor_normal_ctrl_o_and_ctrl_i_are_forwarded_to_the_editor() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Editor;
        app.update(Action::EditorKey(key(KeyCode::Esc)));
        let mut keymap = Keymap::default();

        assert!(matches!(
            keymap.map(control_key('o'), &app),
            Some(Action::EditorKey(event))
                if event.code == KeyCode::Char('o')
                    && event.modifiers == KeyModifiers::CONTROL
        ));
        assert!(matches!(
            keymap.map(control_key('i'), &app),
            Some(Action::EditorKey(event))
                if event.code == KeyCode::Char('i')
                    && event.modifiers == KeyModifiers::CONTROL
        ));
    }

    fn assert_leader_shortcut(app: &App, code: char, expected: Action) {
        let mut keymap = Keymap::default();
        keymap.map(key(KeyCode::Char(' ')), app);
        assert_eq!(keymap.map(key(KeyCode::Char(code)), app), Some(expected));
    }

    fn leader_action(app: &App, code: char) -> Option<Action> {
        let mut keymap = Keymap::default();
        keymap.map(key(KeyCode::Char(' ')), app);
        keymap.map(key(KeyCode::Char(code)), app)
    }

    #[test]
    fn global_leader_shortcuts_work_outside_explorer() {
        let mut editor = App::new(Vec::new());
        editor.focus = Focus::Editor;
        editor.update(Action::EditorKey(key(KeyCode::Esc)));
        assert_eq!(
            editor.active_editor_mode(),
            crate::model::editor::EditorMode::Normal
        );

        let mut keymap = Keymap::default();
        assert!(matches!(
            keymap.map(key(KeyCode::Char(' ')), &editor),
            Some(Action::EditorKey(_))
        ));
        assert!(matches!(
            keymap.map(key(KeyCode::Char('n')), &editor),
            Some(Action::EditorKey(_))
        ));

        let mut results = App::new(Vec::new());
        results.focus = Focus::Results;
        assert_eq!(leader_action(&results, 'n'), None);
        assert_eq!(leader_action(&results, 'e'), None);
        for (code, action) in [
            ('s', Action::OpenSqlEditorList),
            ('q', Action::CloseActiveTab),
            ('x', Action::RequestDeleteActiveConsole),
        ] {
            assert_leader_shortcut(&results, code, action);
        }

        let relation = relation_app(RelationGridMode::Browse);
        assert_eq!(leader_action(&relation, 'n'), None);
        assert_eq!(leader_action(&relation, 'e'), None);
        for (code, action) in [
            ('s', Action::OpenSqlEditorList),
            ('q', Action::CloseActiveTab),
        ] {
            assert_leader_shortcut(&relation, code, action);
        }
    }

    #[test]
    fn pending_sequences_remain_active_until_completed_or_cancelled() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Results;
        let mut keymap = Keymap::default();

        assert_eq!(keymap.map(key(KeyCode::Char('1')), &app), None);
        assert_eq!(keymap.map(key(KeyCode::Char('0')), &app), None);
        assert_eq!(
            keymap.map(
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
                &app
            ),
            None
        );
        assert!(matches!(
            keymap.pending,
            Some(PendingState {
                pending: Pending::Window { count: 10 },
                ..
            })
        ));
        let started = keymap.pending.as_ref().unwrap().started_at;
        assert!(
            keymap
                .sequence_state(&app, started + Duration::from_millis(749))
                .is_some()
        );
        assert_eq!(keymap.map(key(KeyCode::Char('x')), &app), None);
        assert!(keymap.sequence_state(&app, Instant::now()).is_some());
    }

    #[test]
    fn escape_and_ctrl_c_cancel_pending_sequences_without_quitting() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Results;
        let mut keymap = Keymap::default();

        keymap.map(key(KeyCode::Char('g')), &app);
        assert_eq!(keymap.map(key(KeyCode::Esc), &app), None);
        assert!(keymap.sequence_state(&app, Instant::now()).is_none());

        keymap.map(key(KeyCode::Char('g')), &app);
        assert_eq!(
            keymap.map(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                &app
            ),
            None
        );
        assert!(keymap.sequence_state(&app, Instant::now()).is_none());
        assert_eq!(
            keymap.map(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                &app
            ),
            Some(Action::Quit)
        );
    }

    #[test]
    fn sequence_state_exposes_semantic_prefixes_and_count() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Results;
        let mut keymap = Keymap::default();
        let now = Instant::now();

        for (event, prefix, display) in [
            (
                key(KeyCode::Char(' ')),
                crate::help::ShortcutPrefix::Leader,
                "Space",
            ),
            (
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
                crate::help::ShortcutPrefix::Window,
                "Ctrl-w",
            ),
            (
                key(KeyCode::Char('g')),
                crate::help::ShortcutPrefix::Goto,
                "g",
            ),
            (
                key(KeyCode::Char('z')),
                crate::help::ShortcutPrefix::GridAlign,
                "z",
            ),
            (
                key(KeyCode::Char('[')),
                crate::help::ShortcutPrefix::Previous,
                "[",
            ),
            (
                key(KeyCode::Char(']')),
                crate::help::ShortcutPrefix::Next,
                "]",
            ),
        ] {
            keymap.clear_pending();
            assert_eq!(keymap.map(event, &app), None);
            let state = keymap.sequence_state(&app, now).expect("pending state");
            assert_eq!(state.prefix, prefix);
            assert_eq!(state.display, display);
        }

        keymap.clear_pending();
        keymap.map(key(KeyCode::Char('5')), &app);
        let count_state = keymap
            .sequence_state(&app, Instant::now())
            .expect("count prefix state");
        assert_eq!(
            count_state.prefix,
            crate::help::ShortcutPrefix::WindowCount(5)
        );
        assert_eq!(count_state.display, "5");
        keymap.map(
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
            &app,
        );
        let state = keymap
            .sequence_state(&app, now)
            .expect("counted window state");
        assert_eq!(state.prefix, crate::help::ShortcutPrefix::Window);
        assert_eq!(state.display, "5 Ctrl-w");
    }

    #[test]
    fn sequence_state_covers_relation_and_record_prefixes() {
        let mut relation = relation_app(RelationGridMode::Browse);
        let mut keymap = Keymap::default();
        assert_eq!(
            keymap.map(key(KeyCode::Char('y')), &relation),
            Some(Action::CopyGridCell)
        );
        assert!(keymap.sequence_state(&relation, Instant::now()).is_none());
        keymap.clear_pending();
        assert_eq!(keymap.map(key(KeyCode::Char('d')), &relation), None);
        assert_eq!(
            keymap
                .sequence_state(&relation, Instant::now())
                .unwrap()
                .prefix,
            crate::help::ShortcutPrefix::RelationDelete
        );

        relation.overlay = Some(Overlay::RecordView(Default::default()));
        keymap.clear_pending();
        assert_eq!(keymap.map(key(KeyCode::Char('g')), &relation), None);
        assert_eq!(
            keymap
                .sequence_state(&relation, Instant::now())
                .unwrap()
                .prefix,
            crate::help::ShortcutPrefix::RecordViewGoto
        );
    }

    #[test]
    fn leader_menu_survives_idle_and_executes_after_selection() {
        for focus in [Focus::Explorer, Focus::Results] {
            let mut app = App::new(Vec::new());
            app.focus = focus;
            let mut keymap = Keymap::default();
            keymap.map(key(KeyCode::Char(' ')), &app);
            let started = keymap.pending.as_ref().unwrap().started_at;
            let later = started + Duration::from_secs(60);
            assert!(!keymap.expire_pending(&app, later));
            assert!(keymap.sequence_state(&app, later).is_some());
            assert_eq!(keymap.map(key(KeyCode::Down), &app), None);
            assert!(keymap.sequence_state(&app, later).is_some());
            assert!(matches!(
                keymap.map(key(KeyCode::Enter), &app),
                Some(Action::ExecuteHelpShortcut(_))
            ));
            assert!(keymap.sequence_state(&app, later).is_none());

            keymap.map(key(KeyCode::Char(' ')), &app);
            keymap.pending.as_mut().unwrap().started_at = Instant::now() - Duration::from_secs(60);
            assert_eq!(
                keymap.map(key(KeyCode::Char('s')), &app),
                Some(Action::OpenSqlEditorList)
            );
        }
    }

    #[test]
    fn leader_menu_editor_transaction_survives_idle() {
        let mut app = App::new(Vec::new());
        app.update(Action::EditorKey(key(KeyCode::Esc)));
        let mut keymap = Keymap::default();
        for code in [KeyCode::Char(' '), KeyCode::Char('t')] {
            let action = keymap.map(key(code), &app).unwrap();
            app.update(action);
            let later = Instant::now() + Duration::from_secs(60);
            assert!(!keymap.expire_pending(&app, later));
            let state = keymap.sequence_state(&app, later).unwrap();
            assert_eq!(
                state.display,
                if code == KeyCode::Char('t') {
                    "Space t"
                } else {
                    "Space"
                }
            );
        }
        let action = keymap.map(key(KeyCode::Char('c')), &app).unwrap();
        app.update(action);
        assert!(keymap.sequence_state(&app, Instant::now()).is_none());
    }

    #[test]
    fn sequence_state_expires_and_invalidates_on_focus_mode_and_tab_changes() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Results;
        let mut keymap = Keymap::with_sequence_timeout(Duration::from_millis(750));
        keymap.map(key(KeyCode::Char('g')), &app);
        let started = keymap.pending.as_ref().unwrap().started_at;
        assert!(keymap.sequence_state(&app, started).is_some());
        assert!(
            keymap
                .sequence_state(&app, started + Duration::from_millis(749))
                .is_some()
        );
        assert!(keymap.expire_pending(&app, started + Duration::from_millis(750)));
        assert!(keymap.sequence_state(&app, started).is_none());

        keymap.map(key(KeyCode::Char('g')), &app);
        app.focus = Focus::Explorer;
        assert_eq!(
            keymap.map(key(KeyCode::Char('j')), &app),
            Some(Action::ExplorerMove(1))
        );
        keymap.clear_pending();

        app.focus = Focus::Editor;
        keymap.map(key(KeyCode::Char('g')), &app);
        app.update(Action::EditorKey(key(KeyCode::Char('i'))));
        assert!(keymap.sequence_state(&app, Instant::now()).is_none());
        keymap.clear_pending();

        app.focus = Focus::Results;
        keymap.map(key(KeyCode::Char('g')), &app);
        app.tabs
            .push(WorkspaceTab::Sql(crate::model::tab::ConsoleTab::new(
                "other",
            )));
        app.active_tab = 1;
        assert!(keymap.sequence_state(&app, Instant::now()).is_none());
    }

    #[test]
    fn restoring_focus_mode_or_tab_does_not_restore_old_pending() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Results;
        let mut keymap = Keymap::default();

        keymap.map(key(KeyCode::Char('g')), &app);
        app.focus = Focus::Explorer;
        assert!(keymap.sequence_state(&app, Instant::now()).is_none());
        app.focus = Focus::Results;
        assert_eq!(keymap.map(key(KeyCode::Char('t')), &app), None);

        keymap.map(key(KeyCode::Char('g')), &app);
        app.focus = Focus::Editor;
        app.update(Action::EditorKey(key(KeyCode::Char('i'))));
        assert_eq!(
            keymap.map(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE), &app),
            Some(Action::ShowHelp)
        );
        app.focus = Focus::Results;
        assert!(keymap.sequence_state(&app, Instant::now()).is_none());
        assert_eq!(keymap.map(key(KeyCode::Char('t')), &app), None);

        keymap.map(key(KeyCode::Char('g')), &app);
        let original = app.tabs[app.active_tab].id();
        app.tabs
            .push(WorkspaceTab::Sql(crate::model::tab::ConsoleTab::new(
                "other",
            )));
        app.active_tab = 1;
        assert!(keymap.sequence_state(&app, Instant::now()).is_none());
        app.active_tab = app
            .tabs
            .iter()
            .position(|tab| tab.id() == original)
            .unwrap();
        assert_eq!(keymap.map(key(KeyCode::Char('t')), &app), None);
    }

    #[test]
    fn invalid_continuation_keeps_pending_sequence_visible() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Results;
        let mut keymap = Keymap::default();
        keymap.map(key(KeyCode::Char('g')), &app);
        assert!(keymap.sequence_state(&app, Instant::now()).is_some());
        assert_eq!(
            keymap.map(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE), &app),
            None
        );
        assert!(keymap.sequence_state(&app, Instant::now()).is_some());
    }

    #[test]
    fn arrows_select_and_enter_executes_a_pending_shortcut() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Results;
        let mut keymap = Keymap::default();

        keymap.map(key(KeyCode::Char(' ')), &app);
        assert_eq!(
            keymap.map(key(KeyCode::Down), &app),
            None,
            "selection should move without executing"
        );
        assert_eq!(
            keymap
                .sequence_state(&app, Instant::now())
                .unwrap()
                .selected,
            1
        );
        assert!(matches!(
            keymap.map(key(KeyCode::Enter), &app),
            Some(Action::ExecuteHelpShortcut(_))
        ));
        assert!(keymap.sequence_state(&app, Instant::now()).is_none());
    }

    #[test]
    fn four_way_shortcut_navigation_tracks_the_visible_column_layout() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Results;
        let mut keymap = Keymap::default();
        keymap.set_sequence_width(120);
        keymap.map(key(KeyCode::Char(' ')), &app);
        let count = crate::help::prefix_shortcuts_with_bindings(
            crate::help::shortcut_context(&app),
            crate::help::shortcut_capabilities(&app),
            crate::help::ShortcutPrefix::Leader,
            Some(&app.key_bindings),
        )
        .len();
        let layout = crate::input::sequence_navigation::Layout::new(120, count);
        assert!(layout.columns() > 1);

        assert_eq!(keymap.map(key(KeyCode::Right), &app), None);
        assert_eq!(
            keymap
                .sequence_state(&app, Instant::now())
                .unwrap()
                .selected,
            layout.rows()
        );
        assert_eq!(keymap.map(key(KeyCode::Down), &app), None);
        let selected = keymap
            .sequence_state(&app, Instant::now())
            .unwrap()
            .selected;
        assert_eq!(selected, layout.rows() + 1);
        assert_eq!(keymap.map(key(KeyCode::Left), &app), None);
        assert_eq!(
            keymap
                .sequence_state(&app, Instant::now())
                .unwrap()
                .selected,
            1
        );
        assert_eq!(app.focus, Focus::Results);

        let mut narrow_keymap = Keymap::default();
        narrow_keymap.map(key(KeyCode::Char(' ')), &app);
        assert_eq!(narrow_keymap.map(key(KeyCode::Left), &app), None);
        assert_eq!(
            narrow_keymap
                .sequence_state(&app, Instant::now())
                .unwrap()
                .selected,
            0,
            "left at a single-column boundary must not steal focus"
        );

        let expected = crate::help::prefix_shortcuts_with_bindings(
            crate::help::shortcut_context(&app),
            crate::help::shortcut_capabilities(&app),
            crate::help::ShortcutPrefix::Leader,
            Some(&app.key_bindings),
        )[1]
        .id;
        assert_eq!(
            keymap.map(key(KeyCode::Enter), &app),
            Some(Action::ExecuteHelpShortcut(expected))
        );
    }

    #[test]
    fn completed_and_cleared_sequences_have_no_snapshot() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Results;
        let mut keymap = Keymap::default();
        keymap.map(key(KeyCode::Char('g')), &app);
        assert!(keymap.sequence_state(&app, Instant::now()).is_some());
        assert_eq!(
            keymap.map(key(KeyCode::Char('t')), &app),
            Some(Action::NextTab)
        );
        assert!(keymap.sequence_state(&app, Instant::now()).is_none());
        keymap.map(key(KeyCode::Char('g')), &app);
        keymap.clear_pending();
        assert!(keymap.sequence_state(&app, Instant::now()).is_none());
    }

    #[test]
    fn terminal_selection_binding_maps_before_context_specific_keys() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Results;
        let mut keymap = Keymap::default();

        assert_eq!(
            keymap.map(
                KeyEvent::new(
                    KeyCode::Char('s'),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                ),
                &app,
            ),
            Some(Action::ToggleTerminalSelection)
        );
        assert!(keymap.sequence_state(&app, Instant::now()).is_none());
        assert_eq!(
            keymap.map(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                &app,
            ),
            Some(Action::Quit)
        );
    }

    #[test]
    fn explorer_search_preempts_normal_bindings_and_edits_query() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Explorer;
        app.explorer.open_search(
            Some(crate::identity::ConnectionIdentity {
                profile_id: uuid::Uuid::nil(),
                generation: 1,
            }),
            1,
        );
        let mut keymap = Keymap::default();

        assert_eq!(
            keymap.map(key(KeyCode::Char('x')), &app),
            Some(Action::ExplorerSearchInsert('x'))
        );
        assert_eq!(
            keymap.map(key(KeyCode::Down), &app),
            Some(Action::ExplorerSearchMove(1))
        );
        assert_eq!(
            keymap.map(key(KeyCode::Char('j')), &app),
            Some(Action::ExplorerSearchInsert('j'))
        );
        assert_eq!(
            keymap.map(key(KeyCode::Char('k')), &app),
            Some(Action::ExplorerSearchInsert('k'))
        );
        assert_eq!(
            keymap.map(key(KeyCode::Home), &app),
            Some(Action::ExplorerSearchMove(isize::MIN))
        );
        assert_eq!(
            keymap.map(key(KeyCode::End), &app),
            Some(Action::ExplorerSearchMove(isize::MAX))
        );
        assert_eq!(
            keymap.map(key(KeyCode::Char('r')), &app),
            Some(Action::ExplorerSearchInsert('r'))
        );
        assert_eq!(
            keymap.map(
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
                &app
            ),
            None
        );
        assert_eq!(
            keymap.map(key(KeyCode::Enter), &app),
            Some(Action::ExplorerSearchLocate)
        );
        assert_eq!(
            keymap.map(key(KeyCode::Esc), &app),
            Some(Action::ExplorerSearchClose)
        );
        assert_eq!(
            keymap.map(
                KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
                &app
            ),
            Some(Action::ExplorerSearchClear)
        );
    }

    #[test]
    fn relation_data_maps_cell_edit_lifecycle_and_text() {
        let app = relation_app(RelationGridMode::Browse);
        let mut keymap = Keymap::default();

        assert_eq!(
            keymap.map(key(KeyCode::Char('e')), &app),
            Some(Action::RelationEditCell)
        );
        assert_eq!(
            keymap.map(
                key(KeyCode::Char('x')),
                &relation_app(RelationGridMode::EditCell(Box::new(
                    crate::model::relation_edit::CellEditorState {
                        row: 0,
                        column: 0,
                        input: crate::model::cell_editor::CellEditorBuffer::default(),
                        error: None,
                    },
                )))
            ),
            Some(Action::RelationEditInsert('x'))
        );
        assert_eq!(
            keymap.map(
                key(KeyCode::Esc),
                &relation_app(RelationGridMode::EditCell(Box::new(
                    crate::model::relation_edit::CellEditorState {
                        row: 0,
                        column: 0,
                        input: Default::default(),
                        error: None,
                    },
                )))
            ),
            Some(Action::RelationEditCancel)
        );
    }

    #[test]
    fn cell_editor_maps_digits_and_enter_before_global_count_prefixes() {
        let app = relation_app(RelationGridMode::EditCell(Box::new(
            crate::model::relation_edit::CellEditorState {
                row: 0,
                column: 0,
                input: Default::default(),
                error: None,
            },
        )));
        let mut keymap = Keymap::default();

        for digit in ['0', '1', '9'] {
            assert_eq!(
                keymap.map(key(KeyCode::Char(digit)), &app),
                Some(Action::RelationEditInsert(digit))
            );
        }
        assert_eq!(
            keymap.map(key(KeyCode::Enter), &app),
            Some(Action::RelationEditConfirm)
        );
        assert!(keymap.pending.is_none());
    }

    #[test]
    fn cell_editor_maps_line_editing_controls_before_global_bindings() {
        let app = relation_app(RelationGridMode::EditCell(Box::new(
            crate::model::relation_edit::CellEditorState {
                row: 0,
                column: 0,
                input: Default::default(),
                error: None,
            },
        )));
        let mut keymap = Keymap::default();

        assert_eq!(
            keymap.map(
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
                &app,
            ),
            Some(Action::RelationEditDeletePreviousWord)
        );
        assert_eq!(
            keymap.map(
                KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
                &app,
            ),
            Some(Action::RelationEditDeleteToStart)
        );
    }

    #[test]
    fn relation_data_dd_remains_a_pending_sequence() {
        let app = relation_app(RelationGridMode::Browse);
        let mut keymap = Keymap::default();

        assert_eq!(keymap.map(key(KeyCode::Char('d')), &app), None);
        assert_eq!(
            keymap.map(key(KeyCode::Char('d')), &app),
            Some(Action::RelationDeleteCurrent)
        );
        assert!(keymap.pending.is_none());
    }

    #[test]
    fn relation_cell_copy_does_not_consume_followup_navigation() {
        let app = relation_app(RelationGridMode::Browse);
        let mut keymap = Keymap::default();

        assert_eq!(
            keymap.map(key(KeyCode::Char('y')), &app),
            Some(Action::CopyGridCell)
        );
        assert_eq!(
            keymap.map(key(KeyCode::Char('j')), &app),
            Some(Action::GridMove {
                rows: 1,
                columns: 0,
            })
        );
        assert!(keymap.pending.is_none());
    }

    #[test]
    fn relation_data_browse_maps_row_navigation_keys_to_grid_moves() {
        for app in [
            relation_app(RelationGridMode::Browse),
            relation_app_without_edit(),
        ] {
            let mut keymap = Keymap::default();

            for (code, rows) in [
                (KeyCode::Char('j'), 1),
                (KeyCode::Down, 1),
                (KeyCode::Char('k'), -1),
                (KeyCode::Up, -1),
            ] {
                assert_eq!(
                    keymap.map(KeyEvent::new(code, KeyModifiers::NONE), &app),
                    Some(Action::GridMove { rows, columns: 0 })
                );
            }

            assert_eq!(
                keymap.map(
                    KeyEvent::new_with_kind(
                        KeyCode::Char('j'),
                        KeyModifiers::NONE,
                        KeyEventKind::Release,
                    ),
                    &app,
                ),
                None
            );
            assert_eq!(
                keymap.map(
                    KeyEvent::new_with_kind(
                        KeyCode::Char('k'),
                        KeyModifiers::NONE,
                        KeyEventKind::Repeat,
                    ),
                    &app,
                ),
                Some(Action::GridMove {
                    rows: -1,
                    columns: 0,
                })
            );
        }
    }

    #[test]
    fn grid_maps_absolute_and_viewport_row_targets() {
        let app = relation_app(RelationGridMode::Browse);
        let mut keymap = Keymap::default();

        for (code, target) in [
            (KeyCode::Char('G'), GridRowTarget::Last),
            (KeyCode::Char('H'), GridRowTarget::ViewTop),
            (KeyCode::Char('M'), GridRowTarget::ViewMiddle),
            (KeyCode::Char('L'), GridRowTarget::ViewBottom),
        ] {
            assert_eq!(
                keymap.map(key(code), &app),
                Some(Action::GridSelectRow(target))
            );
        }

        assert_eq!(keymap.map(key(KeyCode::Char('g')), &app), None);
        assert_eq!(
            keymap.map(key(KeyCode::Char('g')), &app),
            Some(Action::GridSelectRow(GridRowTarget::First))
        );
    }

    #[test]
    fn grid_maps_current_row_column_targets() {
        let app = relation_app(RelationGridMode::Browse);
        let mut keymap = Keymap::default();

        assert_eq!(
            keymap.map(key(KeyCode::Char('^')), &app),
            Some(Action::GridSelectColumn(GridColumnTarget::First))
        );
        assert_eq!(
            keymap.map(key(KeyCode::Char('$')), &app),
            Some(Action::GridSelectColumn(GridColumnTarget::Last))
        );
    }

    #[test]
    fn grid_column_targets_do_not_steal_relation_cell_input() {
        let app = relation_app(RelationGridMode::EditCell(Box::new(
            crate::model::relation_edit::CellEditorState {
                row: 0,
                column: 0,
                input: Default::default(),
                error: None,
            },
        )));
        let mut keymap = Keymap::default();

        assert_eq!(
            keymap.map(key(KeyCode::Char('^')), &app),
            Some(Action::RelationEditInsert('^'))
        );
        assert_eq!(
            keymap.map(key(KeyCode::Char('$')), &app),
            Some(Action::RelationEditInsert('$'))
        );
    }

    #[test]
    fn grid_maps_page_scroll_control_keys() {
        let app = relation_app(RelationGridMode::Browse);
        let mut keymap = Keymap::default();

        for (character, direction, amount) in [
            ('d', 1, GridScrollAmount::HalfPage),
            ('u', -1, GridScrollAmount::HalfPage),
            ('f', 1, GridScrollAmount::Page),
            ('b', -1, GridScrollAmount::Page),
        ] {
            assert_eq!(
                keymap.map(
                    KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL),
                    &app,
                ),
                Some(Action::GridScrollRows { direction, amount })
            );
        }
    }

    #[test]
    fn grid_maps_selected_row_alignment_sequences() {
        let app = relation_app(RelationGridMode::Browse);
        let mut keymap = Keymap::default();

        for (suffix, alignment) in [
            ('z', GridRowAlignment::Middle),
            ('t', GridRowAlignment::Top),
            ('b', GridRowAlignment::Bottom),
        ] {
            assert_eq!(keymap.map(key(KeyCode::Char('z')), &app), None);
            assert_eq!(
                keymap.map(key(KeyCode::Char(suffix)), &app),
                Some(Action::GridAlignSelectedRow(alignment))
            );
        }
    }

    #[test]
    fn grid_navigation_does_not_steal_relation_cell_input() {
        let app = relation_app(RelationGridMode::EditCell(Box::new(
            crate::model::relation_edit::CellEditorState {
                row: 0,
                column: 0,
                input: Default::default(),
                error: None,
            },
        )));
        let mut keymap = Keymap::default();

        assert_eq!(
            keymap.map(key(KeyCode::Char('g')), &app),
            Some(Action::RelationEditInsert('g'))
        );
        assert_eq!(
            keymap.map(key(KeyCode::Char('z')), &app),
            Some(Action::RelationEditInsert('z'))
        );
        assert_eq!(
            keymap.map(
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
                &app,
            ),
            None
        );
    }

    #[test]
    fn relation_data_visual_and_transaction_bindings_are_scoped() {
        let app = relation_app(RelationGridMode::Browse);
        let mut keymap = Keymap::default();

        assert_eq!(
            keymap.map(key(KeyCode::Char('V')), &app),
            Some(Action::RelationVisualLine)
        );
        assert_eq!(
            keymap.map(key(KeyCode::Char('p')), &app),
            Some(Action::RelationPaste)
        );
        assert_eq!(
            keymap.map(key(KeyCode::Char('a')), &app),
            Some(Action::RelationInsertRow)
        );
        assert_eq!(
            keymap.map(
                KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
                &app,
            ),
            Some(Action::OpenTransactionControl)
        );
        assert_eq!(
            keymap.map(
                KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
                &app,
            ),
            None
        );
    }

    #[test]
    fn notification_history_uses_f8_and_leader_m_without_stealing_plain_m() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Explorer;
        let mut keymap = Keymap::default();
        assert_eq!(
            keymap.map(key(KeyCode::F(7)), &app),
            Some(Action::OpenSqlHistory)
        );
        assert_eq!(
            keymap.map(key(KeyCode::F(8)), &app),
            Some(Action::OpenNotificationHistory)
        );

        let mut app = App::new(Vec::new());
        app.focus = Focus::Explorer;
        assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
        assert_eq!(
            keymap.map(key(KeyCode::Char('m')), &app),
            Some(Action::OpenNotificationHistory)
        );
    }

    #[test]
    fn configured_global_binding_replaces_its_default() {
        let mut config = crate::config::AppConfig::default();
        config
            .keybindings
            .global
            .insert("help".into(), vec!["F11".into()]);
        let bindings = config.keybindings.key_bindings().unwrap();
        let mut keymap =
            Keymap::with_sequence_timeout_and_bindings(Duration::from_millis(750), bindings);
        let mut app = App::new(Vec::new());
        app.focus = Focus::Results;

        assert_eq!(
            keymap.map(key(KeyCode::F(11)), &app),
            Some(Action::ShowHelp)
        );
        assert_eq!(keymap.map(key(KeyCode::F(1)), &app), None);
    }

    #[test]
    fn configured_tab_sequence_replaces_the_default_goto_mapping() {
        let mut config = crate::config::AppConfig::default();
        config
            .keybindings
            .global
            .insert("next-tab".into(), vec!["g t".into()]);
        let bindings = config.keybindings.key_bindings().unwrap();
        let mut keymap =
            Keymap::with_sequence_timeout_and_bindings(Duration::from_millis(750), bindings);
        let mut app = App::new(Vec::new());
        app.focus = Focus::Results;

        assert_eq!(keymap.map(key(KeyCode::Char('g')), &app), None);
        assert_eq!(
            keymap.map(key(KeyCode::Char('t')), &app),
            Some(Action::NextTab)
        );
    }

    #[test]
    fn configured_leader_sequence_executes_its_command() {
        let bindings = crate::config::AppConfig::default()
            .keybindings
            .key_bindings()
            .unwrap();
        let mut keymap =
            Keymap::with_sequence_timeout_and_bindings(Duration::from_millis(750), bindings);
        let mut app = App::new(Vec::new());
        app.focus = Focus::Results;

        assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
        assert_eq!(
            keymap.map(key(KeyCode::Char('d')), &app),
            Some(Action::OpenTargetSelector)
        );
        assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
        assert_eq!(
            keymap.map(key(KeyCode::Char('D')), &app),
            Some(Action::OpenDatabaseSelector)
        );
    }

    #[test]
    fn configured_leader_commands_cover_the_default_sequences() {
        let bindings = crate::config::AppConfig::default()
            .keybindings
            .key_bindings()
            .unwrap();
        let mut keymap =
            Keymap::with_sequence_timeout_and_bindings(Duration::from_millis(750), bindings);
        let mut app = App::new(Vec::new());
        app.focus = Focus::Results;

        for (binding_key, expected) in [
            ('c', Action::Focus(Focus::Explorer)),
            ('s', Action::OpenSqlEditorList),
            ('r', Action::RunActiveSql),
            ('R', Action::RunAllSql),
            ('d', Action::OpenTargetSelector),
        ] {
            keymap.clear_pending();
            assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
            assert_eq!(
                keymap.map(key(KeyCode::Char(binding_key)), &app),
                Some(expected)
            );
        }
    }

    #[test]
    fn existing_relation_view_keys_keep_their_meaning() {
        let mut app = App::new(Vec::new());
        app.tabs
            .push(WorkspaceTab::Relation(RelationTab::new("users")));
        app.active_tab = 1;
        app.focus = Focus::Results;
        let mut keymap = Keymap::default();

        assert_eq!(
            keymap.map(key(KeyCode::Char('p')), &app),
            Some(Action::RelationPaste)
        );
        app.update(Action::SetRelationView(
            crate::model::relation::RelationView::Ddl,
        ));
        assert_eq!(
            keymap.map(key(KeyCode::Char('p')), &app),
            Some(Action::SetRelationView(
                crate::model::relation::RelationView::Data
            ))
        );
        assert_eq!(
            keymap.map(key(KeyCode::Char('o')), &app),
            Some(Action::SetRelationView(
                crate::model::relation::RelationView::Data
            ))
        );
        assert_eq!(
            keymap.map(key(KeyCode::Char('r')), &app),
            Some(Action::RefreshActiveRelation)
        );
    }

    #[test]
    fn relation_ddl_window_command_can_focus_explorer() {
        let mut app = App::new(Vec::new());
        app.tabs
            .push(WorkspaceTab::Relation(RelationTab::new("users")));
        app.active_tab = 1;
        app.focus = Focus::Results;
        app.update(Action::SetRelationView(
            crate::model::relation::RelationView::Ddl,
        ));
        let mut keymap = Keymap::default();

        assert_eq!(
            keymap.map(
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
                &app,
            ),
            None
        );
        assert_eq!(
            keymap.map(key(KeyCode::Char('h')), &app),
            Some(Action::Focus(Focus::Explorer))
        );
    }

    #[test]
    fn empty_workspace_navigation_keeps_explorer_focus() {
        let profile = crate::profile::import_connection_url(":memory:", Some("saved"))
            .unwrap()
            .profile;
        let mut app = App::new(vec![profile]);
        app.focus = Focus::Explorer;
        let mut keymap = Keymap::default();

        for event in [key(KeyCode::Tab), key(KeyCode::BackTab)] {
            let action = keymap.map(event, &app).expect("focus action");
            app.update(action);
            assert_eq!(app.focus, Focus::Explorer);
        }

        for direction in ['h', 'j', 'k', 'l', 'w'] {
            let mut keymap = Keymap::default();
            assert_eq!(
                keymap.map(
                    KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
                    &app,
                ),
                None
            );
            let action = keymap.map(key(KeyCode::Char(direction)), &app);
            if direction == 'l' {
                assert!(action.is_some());
            }
            if let Some(action) = action {
                app.update(action);
            }
            assert_eq!(app.focus, Focus::Explorer);
        }
    }

    #[test]
    fn dashboard_window_right_focuses_results_instead_of_missing_editor() {
        let mut app = App::new(Vec::new());
        app.tabs.clear();
        app.tabs.push(WorkspaceTab::Dashboard(
            crate::model::dashboard::DashboardTab::new(),
        ));
        app.active_tab = 0;
        app.focus = Focus::Explorer;
        let mut keymap = Keymap::default();

        assert_eq!(
            keymap.map(
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
                &app,
            ),
            None
        );
        assert_eq!(
            keymap.map(key(KeyCode::Char('l')), &app),
            Some(Action::Focus(Focus::Results))
        );
    }

    #[test]
    fn dashboard_controls_remain_available_on_processes_page() {
        let mut app = App::new(Vec::new());
        app.tabs.clear();
        let mut dashboard = crate::model::dashboard::DashboardTab::new();
        dashboard.page = crate::model::dashboard::DashboardPage::Processes;
        app.tabs.push(WorkspaceTab::Dashboard(dashboard));
        app.active_tab = 0;
        app.focus = Focus::Results;

        assert_eq!(
            Keymap::default().map(key(KeyCode::Char('o')), &app),
            Some(Action::DashboardSetPage(
                crate::model::dashboard::DashboardPage::Overview
            ))
        );
        for code in ['1', '2', '3'] {
            assert_eq!(Keymap::default().map(key(KeyCode::Char(code)), &app), None);
        }
        app.update(Action::DashboardSetPage(
            crate::model::dashboard::DashboardPage::Overview,
        ));
        assert_eq!(
            Keymap::default().map(key(KeyCode::Char('o')), &app),
            Some(Action::DashboardSetPage(
                crate::model::dashboard::DashboardPage::Processes
            ))
        );
        assert_eq!(
            Keymap::default().map(key(KeyCode::Char('r')), &app),
            Some(Action::DashboardRefresh)
        );
    }
}
