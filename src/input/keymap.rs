use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use uuid::Uuid;

use crate::{
    action::Action,
    app::App,
    model::{
        editor::EditorMode,
        profile_manager::{ProfileField, ProfileInput, ProfileManagerPage},
        workspace::{Focus, Overlay},
    },
};

const SEQUENCE_TIMEOUT: Duration = Duration::from_millis(750);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Pending {
    Leader,
    Window,
    Previous,
    Next,
    RelationYank,
    RelationDelete,
    GridAlign,
    RecordViewGoto,
    ExplorerAlign,
    Goto,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TextInputEdit {
    Insert(char),
    Backspace,
    DeletePreviousWord,
    DeleteToStart,
    Delete,
    MoveLeft,
    MoveRight,
    MoveHome,
    MoveEnd,
}

#[derive(Debug, Default)]
pub struct Keymap {
    pending: Option<(Pending, Instant, Focus, EditorMode, Uuid)>,
}

impl Keymap {
    pub fn map(&mut self, event: KeyEvent, app: &App) -> Option<Action> {
        if matches!(event.kind, KeyEventKind::Release) {
            return None;
        }
        if event.modifiers == KeyModifiers::CONTROL && event.code == KeyCode::Char('c') {
            return Some(Action::Quit);
        }
        if app
            .overlay
            .as_ref()
            .is_some_and(|overlay| matches!(overlay, Overlay::Help(_)))
        {
            self.pending = None;
            if event.modifiers == KeyModifiers::NONE {
                return match event.code {
                    KeyCode::Char(character) => Some(Action::HelpInsert(character)),
                    KeyCode::Backspace => Some(Action::HelpBackspace),
                    KeyCode::Up => Some(Action::HelpMove(-1)),
                    KeyCode::Down => Some(Action::HelpMove(1)),
                    KeyCode::Enter => app.help_selected_id().map(Action::ExecuteHelpShortcut),
                    KeyCode::Esc => Some(Action::DismissOverlay),
                    _ => None,
                };
            }
            if event.modifiers == KeyModifiers::CONTROL && event.code == KeyCode::Char('u') {
                return Some(Action::HelpClear);
            }
            return None;
        }
        if app.overlay == Some(Overlay::ProfileManager) {
            self.pending = None;
            return map_profile_manager(event, app);
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
        if matches!(app.overlay, Some(Overlay::RecordView(_))) {
            let pending = self.pending.take();
            return match event.code {
                KeyCode::Char('g') => {
                    if let Some((Pending::RecordViewGoto, started, focus, editor_mode, tab_id)) =
                        pending
                    {
                        let current_tab = app
                            .tabs
                            .get(app.active_tab)
                            .map_or(Uuid::nil(), |tab| tab.id());
                        if started.elapsed() <= SEQUENCE_TIMEOUT
                            && focus == app.focus
                            && editor_mode == app.active_editor_mode()
                            && tab_id == current_tab
                        {
                            return Some(Action::RecordViewJumpFirstField);
                        }
                    }
                    self.pending = Some((
                        Pending::RecordViewGoto,
                        Instant::now(),
                        app.focus,
                        app.active_editor_mode(),
                        app.tabs
                            .get(app.active_tab)
                            .map_or(Uuid::nil(), |tab| tab.id()),
                    ));
                    None
                }
                KeyCode::Char('j') | KeyCode::Down => Some(Action::RecordViewMoveFields(1)),
                KeyCode::Char('k') | KeyCode::Up => Some(Action::RecordViewMoveFields(-1)),
                KeyCode::Char('h') | KeyCode::Left => Some(Action::RecordViewMoveRow(-1)),
                KeyCode::Char('l') | KeyCode::Right => Some(Action::RecordViewMoveRow(1)),
                KeyCode::Char('G') | KeyCode::End => Some(Action::RecordViewJumpLastField),
                KeyCode::Home => Some(Action::RecordViewJumpFirstField),
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('v') => {
                    Some(Action::CloseRecordView)
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
                KeyCode::Enter | KeyCode::Char('e') | KeyCode::Char('y') => {
                    Some(Action::ConfirmExecution)
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => {
                    Some(Action::CancelExecution)
                }
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                    Some(Action::ToggleExecutionConfirmationFocus)
                }
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::ManualCancelConfirm { .. })) {
            self.pending = None;
            return match event.code {
                KeyCode::Enter | KeyCode::Char('c') => Some(Action::ConfirmManualCancellation),
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
        if matches!(app.overlay, Some(Overlay::ClearTransactionOutcome { .. })) {
            self.pending = None;
            return match event.code {
                KeyCode::Enter | KeyCode::Char('y') => Some(Action::ConfirmClearTransactionOutcome),
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => {
                    Some(Action::CancelClearTransactionOutcome)
                }
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
        if app.focus == Focus::Explorer && app.explorer.find.is_some() {
            let confirmed = app.explorer.find.as_ref().is_some_and(|find| {
                find.phase == crate::model::workspace::ExplorerSearchPhase::Confirmed
            });
            if !confirmed {
                self.pending = None;
                return match event.code {
                    KeyCode::Esc => Some(Action::ExplorerFindClose),
                    KeyCode::Enter => Some(Action::ExplorerFindConfirm),
                    KeyCode::Backspace => Some(Action::ExplorerFindBackspace),
                    KeyCode::Char('u') if event.modifiers == KeyModifiers::CONTROL => {
                        Some(Action::ExplorerFindClear)
                    }
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
            return match event.code {
                KeyCode::Enter => app
                    .sql_editors
                    .iter()
                    .filter(|record| {
                        crate::model::sql_editor_list::SqlEditorListState::matches(
                            &record.name,
                            &list.query,
                        )
                    })
                    .nth(list.selected)
                    .map(|record| Action::ActivateSqlEditor(record.id)),
                KeyCode::Esc => Some(Action::DismissOverlay),
                KeyCode::Up | KeyCode::Char('k') => Some(Action::SqlEditorListMove(-1)),
                KeyCode::Down | KeyCode::Char('j') => Some(Action::SqlEditorListMove(1)),
                KeyCode::Backspace => Some(Action::SqlEditorListBackspace),
                KeyCode::Char(value) => Some(Action::SqlEditorListInsert(value)),
                _ => None,
            };
        }
        if matches!(app.overlay, Some(Overlay::DeleteConsole { .. })) {
            self.pending = None;
            return match event.code {
                KeyCode::Enter => Some(Action::ConfirmDeleteConsole),
                KeyCode::Esc => Some(Action::CancelDeleteConsole),
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
        if active_data_query_has_focus(app)
            && let Some(action) = map_data_query(event, app)
        {
            return Some(action);
        }
        if app.focus == Focus::Editor && app.active_editor_mode() == EditorMode::Normal {
            match event.code {
                KeyCode::Char('?') => return Some(Action::ShowHelp),
                KeyCode::Tab => return Some(Action::FocusNext),
                KeyCode::BackTab => return Some(Action::FocusPrevious),
                KeyCode::Char('g') => {
                    self.set_pending(Pending::Goto, app);
                    return None;
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
        if app.active_console_opt().is_some_and(|tab| {
            tab.completion.is_some() && app.active_editor_mode() == EditorMode::Insert
        }) {
            let completion_action = match event.code {
                KeyCode::Char('n') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    Some(Action::CompletionNext)
                }
                KeyCode::Char('p') if event.modifiers.contains(KeyModifiers::CONTROL) => {
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

        if let Some((pending, started, focus, editor_mode, tab_id)) = self.pending.take()
            && started.elapsed() <= SEQUENCE_TIMEOUT
            && focus == app.focus
            && editor_mode == app.active_editor_mode()
            && app
                .tabs
                .get(app.active_tab)
                .is_some_and(|tab| tab_id == tab.id())
        {
            if let Some(action) = map_pending(pending, event, app) {
                return Some(action);
            }
            if matches!(
                pending,
                Pending::Window
                    | Pending::RecordViewGoto
                    | Pending::GridAlign
                    | Pending::ExplorerAlign
            ) {
                return None;
            }
        }

        // Resolve multi-key window commands before treating `h`/`j`/`k`/`l`
        // as DDL scrolling commands.
        if is_relation_ddl_focus(app)
            && let Some(action) = map_relation_ddl(event)
        {
            return Some(action);
        }

        if event.modifiers.is_empty()
            && event.code == KeyCode::Char('g')
            && (app.focus == Focus::Explorer
                || (is_grid_navigation_focus(app) && relation_grid_is_browse(app)))
        {
            self.set_pending(Pending::Goto, app);
            return None;
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

        if is_sql_grid_focus(app) && event.modifiers.is_empty() {
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
            if event.modifiers.is_empty() && relation_grid_is_browse(app) {
                match event.code {
                    KeyCode::Char('d') => {
                        self.set_pending(Pending::RelationDelete, app);
                        return None;
                    }
                    KeyCode::Char('y') => {
                        self.set_pending(Pending::RelationYank, app);
                        return None;
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

        if let Some(action) = map_data_query(event, app) {
            return Some(action);
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
            if event.modifiers == KeyModifiers::CONTROL {
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
                KeyCode::Char('w') if app.focus == Focus::Editor => Some(Action::EditorKey(event)),
                KeyCode::Char('w') => {
                    self.set_pending(Pending::Window, app);
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
        if event.code == KeyCode::F(5) {
            return if event.modifiers.contains(KeyModifiers::SHIFT) {
                Some(Action::RunAllSql)
            } else {
                Some(Action::RunActiveSql)
            };
        }
        if event.code == KeyCode::F(1) {
            return Some(Action::ShowHelp);
        }
        if app.focus == Focus::Editor {
            return Some(Action::EditorKey(event));
        }

        if event.code == KeyCode::Tab {
            return Some(Action::FocusNext);
        }
        if event.code == KeyCode::BackTab {
            return Some(Action::FocusPrevious);
        }

        match event.code {
            KeyCode::Char('?') => return Some(Action::ShowHelp),
            KeyCode::Char(' ') => {
                self.set_pending(Pending::Leader, app);
                return None;
            }
            KeyCode::Char('[') => {
                self.set_pending(Pending::Previous, app);
                return None;
            }
            KeyCode::Char('g')
                if app.focus != Focus::Editor
                    && !is_relation_ddl_focus(app)
                    && !is_relation_data_focus(app) =>
            {
                self.set_pending(Pending::Goto, app);
                return None;
            }
            KeyCode::Char(']') => {
                self.set_pending(Pending::Next, app);
                return None;
            }
            _ => {}
        }

        let relation_tab = matches!(
            app.tabs.get(app.active_tab),
            Some(crate::model::tab::WorkspaceTab::Relation(_))
        );
        if relation_tab
            && app.focus == Focus::Results
            && matches!(event.code, KeyCode::Char('o' | 'p' | 'D' | 'r'))
        {
            return map_relation(event.code, app);
        }
        if relation_tab && app.focus == Focus::Results {
            return map_relation(event.code, app);
        }
        match app.focus {
            Focus::Explorer => map_explorer(event.code, app),
            Focus::Editor => None,
            Focus::Results => map_results(event.code, app),
        }
    }

    fn set_pending(&mut self, pending: Pending, app: &App) {
        self.pending = Some((
            pending,
            Instant::now(),
            app.focus,
            app.active_editor_mode(),
            app.tabs
                .get(app.active_tab)
                .map_or(Uuid::nil(), |tab| tab.id()),
        ));
    }

    pub fn clear_pending(&mut self) {
        self.pending = None;
    }
}

fn map_pending(pending: Pending, event: KeyEvent, app: &App) -> Option<Action> {
    let valid_modifiers = event.modifiers.is_empty()
        || (pending == Pending::Leader
            && event.modifiers == KeyModifiers::SHIFT
            && event.code == KeyCode::Char('R'))
        || (pending == Pending::Goto
            && event.modifiers == KeyModifiers::SHIFT
            && event.code == KeyCode::Char('T'))
        || pending == Pending::Window;
    if !valid_modifiers {
        return None;
    }
    match (pending, event.code) {
        (Pending::Leader, KeyCode::Char('c')) => Some(Action::Focus(Focus::Explorer)),
        (Pending::Leader, KeyCode::Char('n')) => Some(Action::NewConsole),
        (Pending::Leader, KeyCode::Char('s')) => Some(Action::GotoSqlConsole),
        (Pending::Leader, KeyCode::Char('r')) => Some(Action::RunActiveSql),
        (Pending::Leader, KeyCode::Char('R')) => Some(Action::RunAllSql),
        (Pending::Leader, KeyCode::Char('d')) => Some(Action::OpenTargetSelector),
        (Pending::Leader, KeyCode::Char('q')) => Some(Action::CloseActiveTab),
        (Pending::Leader, KeyCode::Char('x')) => Some(Action::RequestDeleteActiveConsole),
        (Pending::Leader, KeyCode::Char('e')) => Some(Action::OpenSqlEditorList),
        (Pending::Leader, KeyCode::Char('Y')) if is_grid_navigation_focus(app) => {
            Some(Action::CopyGridRow {
                include_headers: true,
            })
        }
        (Pending::RelationYank, KeyCode::Char('y')) => Some(Action::RelationYank),
        (Pending::Window, KeyCode::Char('j')) if app.focus == Focus::Editor => {
            Some(Action::Focus(Focus::Results))
        }
        (Pending::Window, KeyCode::Char('k'))
            if app.focus == Focus::Results && !app.is_active_relation_tab() =>
        {
            Some(Action::Focus(Focus::Editor))
        }
        (Pending::Window, KeyCode::Char('l')) if app.focus == Focus::Explorer => {
            Some(Action::Focus(if app.is_active_relation_tab() {
                Focus::Results
            } else {
                Focus::Editor
            }))
        }
        (Pending::Window, KeyCode::Char('h')) if app.focus == Focus::Editor => {
            Some(Action::Focus(Focus::Explorer))
        }
        (Pending::Window, KeyCode::Char('h')) if app.focus == Focus::Results => {
            Some(Action::Focus(Focus::Explorer))
        }
        (Pending::Window, KeyCode::Char('j')) if app.focus == Focus::Explorer => None,
        (Pending::Goto, KeyCode::Char('g')) if app.focus == Focus::Explorer => Some(
            Action::ExplorerSelectTarget(crate::model::explorer::ExplorerNodeTarget::First),
        ),
        (Pending::Goto, KeyCode::Char('g')) if is_grid_navigation_focus(app) => Some(
            Action::GridSelectRow(crate::model::tab::GridRowTarget::First),
        ),
        (Pending::Goto, KeyCode::Char('t')) => Some(Action::NextTab),
        (Pending::Goto, KeyCode::Char('T')) => Some(Action::PreviousTab),
        (Pending::Previous, KeyCode::Char('t')) => Some(Action::PreviousTab),
        (Pending::Next, KeyCode::Char('t')) => Some(Action::NextTab),
        (Pending::RelationDelete, KeyCode::Char('d')) => Some(Action::RelationDeleteCurrent),
        (Pending::RecordViewGoto, KeyCode::Char('g')) => Some(Action::RecordViewJumpFirstField),
        (Pending::GridAlign, KeyCode::Char('z')) => Some(Action::GridAlignSelectedRow(
            crate::model::tab::GridRowAlignment::Middle,
        )),
        (Pending::GridAlign, KeyCode::Char('t')) => Some(Action::GridAlignSelectedRow(
            crate::model::tab::GridRowAlignment::Top,
        )),
        (Pending::GridAlign, KeyCode::Char('b')) => Some(Action::GridAlignSelectedRow(
            crate::model::tab::GridRowAlignment::Bottom,
        )),
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

pub fn map_paste(value: String, app: &App) -> Vec<Action> {
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
    if app.overlay.is_some() {
        return Vec::new();
    }
    if is_relation_data_focus(app) {
        return vec![Action::RelationPaste];
    }
    if app.focus != Focus::Editor || app.active_editor_mode() != EditorMode::Insert {
        return Vec::new();
    }
    vec![Action::EditorPaste(value)]
}

fn is_relation_data_focus(app: &App) -> bool {
    app.focus == Focus::Results
        && matches!(
            app.tabs.get(app.active_tab),
            Some(crate::model::tab::WorkspaceTab::Relation(tab))
                if tab.view == crate::model::relation::RelationView::Data && tab.query.focus.is_none()
        )
}

fn is_relation_ddl_focus(app: &App) -> bool {
    app.focus == Focus::Results
        && matches!(
            app.tabs.get(app.active_tab),
            Some(crate::model::tab::WorkspaceTab::Relation(tab))
                if tab.view == crate::model::relation::RelationView::Ddl
        )
}

fn map_relation_ddl(event: KeyEvent) -> Option<Action> {
    if !(event.modifiers.is_empty()
        || event.modifiers == KeyModifiers::SHIFT && event.code == KeyCode::Char('G'))
    {
        return None;
    }
    match event.code {
        KeyCode::Char('j') | KeyCode::Down => Some(Action::DdlScroll {
            rows: 1,
            columns: 0,
        }),
        KeyCode::Char('k') | KeyCode::Up => Some(Action::DdlScroll {
            rows: -1,
            columns: 0,
        }),
        KeyCode::Char('h') | KeyCode::Left => Some(Action::DdlScroll {
            rows: 0,
            columns: -1,
        }),
        KeyCode::Char('l') | KeyCode::Right => Some(Action::DdlScroll {
            rows: 0,
            columns: 1,
        }),
        KeyCode::Char('g') => Some(Action::DdlScrollToStart),
        KeyCode::Char('G') => Some(Action::DdlScrollToEnd),
        _ => None,
    }
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

fn map_relation_data(event: KeyEvent, app: &App) -> Option<Action> {
    use crate::model::relation_edit::RelationGridMode;

    let mode = app.tabs.get(app.active_tab).and_then(|tab| match tab {
        crate::model::tab::WorkspaceTab::Relation(tab) => tab.edit.as_ref().map(|edit| &edit.mode),
        _ => None,
    });
    if let Some(RelationGridMode::EditCell(_)) = mode {
        return match (event.modifiers, event.code) {
            (KeyModifiers::NONE, KeyCode::Enter) => Some(Action::RelationEditConfirm),
            (KeyModifiers::NONE, KeyCode::Esc) => Some(Action::RelationEditCancel),
            _ => map_text_input_edit(event).map(|edit| match edit {
                TextInputEdit::Insert(character) => Action::RelationEditInsert(character),
                TextInputEdit::Backspace => Action::RelationEditBackspace,
                TextInputEdit::DeletePreviousWord => Action::RelationEditDeletePreviousWord,
                TextInputEdit::DeleteToStart => Action::RelationEditDeleteToStart,
                TextInputEdit::Delete => Action::RelationEditDelete,
                TextInputEdit::MoveLeft => Action::RelationEditMoveLeft,
                TextInputEdit::MoveRight => Action::RelationEditMoveRight,
                TextInputEdit::MoveHome => Action::RelationEditMoveHome,
                TextInputEdit::MoveEnd => Action::RelationEditMoveEnd,
            }),
        };
    }

    if !event.modifiers.is_empty() {
        return match (event.modifiers, event.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('s')) => Some(Action::RelationCommit),
            (KeyModifiers::CONTROL, KeyCode::Char('x')) => Some(Action::RelationRollback),
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
            KeyCode::Char('i') => Some(Action::RelationEditCell),
            KeyCode::Char('V') => Some(Action::RelationVisualLine),
            KeyCode::Char('d') => {
                // Keep d available for the dd sequence instead of deleting immediately.
                None
            }
            KeyCode::Char('y') => None,
            KeyCode::Char('p') => Some(Action::RelationPaste),
            KeyCode::Char('a') => Some(Action::RelationInsertRow),
            KeyCode::Char('u') => Some(Action::RelationUndo),
            _ => None,
        },
    }
}

fn map_text_input_edit(event: KeyEvent) -> Option<TextInputEdit> {
    if event.modifiers == KeyModifiers::CONTROL {
        return match event.code {
            KeyCode::Char('w') => Some(TextInputEdit::DeletePreviousWord),
            KeyCode::Char('u') => Some(TextInputEdit::DeleteToStart),
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

fn map_profile_manager(event: KeyEvent, app: &App) -> Option<Action> {
    let manager = app.profile_manager.as_ref()?;
    if manager.page != ProfileManagerPage::Form && !event.modifiers.is_empty() {
        return None;
    }
    if manager.page == ProfileManagerPage::Form && event.modifiers == KeyModifiers::CONTROL {
        return match event.code {
            KeyCode::Char('s') => Some(Action::ProfileSave { connect: false }),
            KeyCode::Enter => Some(Action::ProfileSave { connect: true }),
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
    if manager.page == ProfileManagerPage::Form && event.code == KeyCode::F(5) {
        return Some(Action::ProfileTest);
    }
    match manager.page {
        ProfileManagerPage::Form => map_profile_form(event, manager.selected_field),
        ProfileManagerPage::Scope => match event.code {
            KeyCode::Esc | KeyCode::Enter => Some(Action::ProfileScopeBack),
            KeyCode::Char('r') => Some(Action::ProfileRefreshScope),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::ProfileScopeMove(-1)),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::ProfileScopeMove(1)),
            KeyCode::Char(' ') => manager
                .scope_selected_row
                .clone()
                .map(Action::ProfileToggleScopeRow),
            _ => None,
        },
        ProfileManagerPage::ConfirmDelete => map_profile_delete_confirmation(event.code),
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
    if is_text_field(field) {
        return match code {
            KeyCode::Enter if field == ProfileField::Url => Some(Action::ProfileCommitUrl),
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
            KeyCode::Enter | KeyCode::Char(' ') => Some(Action::ProfileOpenScope),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::ProfileFieldPrevious),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::ProfileFieldNext),
            _ => None,
        };
    }
    if field == ProfileField::Kind {
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
            KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ' | 'l') => {
                Some(Action::ProfileCycle(1))
            }
            KeyCode::Up | KeyCode::Char('k') => Some(Action::ProfileFieldPrevious),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::ProfileFieldNext),
            _ => None,
        };
    }
    if is_toggle_field(field) {
        return match code {
            KeyCode::Enter | KeyCode::Char(' ') => Some(Action::ProfileToggle),
            KeyCode::Up | KeyCode::Char('k') => Some(Action::ProfileFieldPrevious),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::ProfileFieldNext),
            _ => None,
        };
    }
    match (field, code) {
        (_, KeyCode::Up | KeyCode::Char('k')) => Some(Action::ProfileFieldPrevious),
        (_, KeyCode::Down | KeyCode::Char('j')) => Some(Action::ProfileFieldNext),
        (ProfileField::Test, KeyCode::Enter | KeyCode::Char(' ')) => Some(Action::ProfileTest),
        (ProfileField::Save, KeyCode::Enter | KeyCode::Char(' ')) => {
            Some(Action::ProfileSave { connect: false })
        }
        (ProfileField::SaveAndConnect, KeyCode::Enter | KeyCode::Char(' ')) => {
            Some(Action::ProfileSave { connect: true })
        }
        (ProfileField::Cancel, KeyCode::Enter | KeyCode::Char(' ')) => {
            Some(Action::CloseProfileManager)
        }
        _ => None,
    }
}

fn map_profile_delete_confirmation(code: KeyCode) -> Option<Action> {
    match code {
        KeyCode::Enter | KeyCode::Char('y') => Some(Action::ProfileConfirmDelete),
        KeyCode::Esc | KeyCode::Char('q' | 'n') => Some(Action::ProfileCancelDelete),
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
        KeyCode::Char('/') => return Some(Action::ExplorerFindOpen),
        KeyCode::Char('f') => return Some(Action::ExplorerSearchOpen),
        KeyCode::Char('n') => return Some(Action::ProfileStartNew),
        KeyCode::Char('e') => {
            return selected_profile.map(|profile_id| Action::ProfileStartEdit { profile_id });
        }
        KeyCode::Char('d') => {
            return selected_profile.map(|profile_id| Action::ProfileRequestDelete { profile_id });
        }
        KeyCode::Char('c') => {
            return selected_profile.map(|profile_id| Action::RequestProfileConnect { profile_id });
        }
        KeyCode::Char('x') => {
            return selected_profile
                .map(|profile_id| Action::RequestProfileDisconnect { profile_id });
        }
        KeyCode::Char('s') => return Some(Action::OpenProfileAccess),
        _ => {}
    }
    match code {
        KeyCode::Char('j') | KeyCode::Down => Some(Action::ExplorerMove(1)),
        KeyCode::Char('k') | KeyCode::Up => Some(Action::ExplorerMove(-1)),
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
        KeyCode::Char('l') | KeyCode::Right => Some(Action::ExplorerExpand),
        KeyCode::Char('h') | KeyCode::Left => Some(Action::ExplorerCollapse),
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

fn map_results(code: KeyCode, app: &App) -> Option<Action> {
    match code {
        KeyCode::Char('[') => Some(Action::GridResizeColumn(-1)),
        KeyCode::Char(']') => Some(Action::GridResizeColumn(1)),
        KeyCode::Char('=') => Some(Action::GridResetColumnWidth),
        KeyCode::Char('v')
            if app.active_grid_dimensions_for_input().0 > 0
                && app.active_grid_dimensions_for_input().1 > 0 =>
        {
            Some(Action::OpenRecordView)
        }
        KeyCode::Char('h') | KeyCode::Left => Some(Action::GridMove {
            rows: 0,
            columns: -1,
        }),
        KeyCode::Char('j') | KeyCode::Down => Some(Action::GridMove {
            rows: 1,
            columns: 0,
        }),
        KeyCode::Char('k') | KeyCode::Up => Some(Action::GridMove {
            rows: -1,
            columns: 0,
        }),
        KeyCode::Char('l') | KeyCode::Right => Some(Action::GridMove {
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
        KeyCode::Char('o') => match app.tabs.get(app.active_tab) {
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
        KeyCode::Char('1') => match app.tabs.get(app.active_tab) {
            Some(crate::model::tab::WorkspaceTab::Relation(_)) => Some(Action::SetRelationView(
                crate::model::relation::RelationView::Data,
            )),
            Some(crate::model::tab::WorkspaceTab::Sql(_)) => {
                Some(Action::SetResultView(crate::model::tab::ResultView::Data))
            }
            _ => None,
        },
        KeyCode::Char('2') => match app.tabs.get(app.active_tab) {
            Some(crate::model::tab::WorkspaceTab::Relation(_)) => Some(Action::SetRelationView(
                crate::model::relation::RelationView::Ddl,
            )),
            Some(crate::model::tab::WorkspaceTab::Sql(_)) => {
                Some(Action::SetResultView(crate::model::tab::ResultView::Output))
            }
            _ => None,
        },
        KeyCode::Char('3') => match app.tabs.get(app.active_tab) {
            Some(crate::model::tab::WorkspaceTab::Sql(_)) => {
                Some(Action::SetResultView(crate::model::tab::ResultView::Plan))
            }
            _ => None,
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
                KeyCode::Char('n') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Some(Action::DataQueryCompletionNext);
                }
                KeyCode::Char('p') if event.modifiers.contains(KeyModifiers::CONTROL) => {
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
                TextInputEdit::Delete => Action::DataQueryDelete,
                TextInputEdit::MoveLeft => Action::DataQueryMoveLeft,
                TextInputEdit::MoveRight => Action::DataQueryMoveRight,
                TextInputEdit::MoveHome => Action::DataQueryMoveHome,
                TextInputEdit::MoveEnd => Action::DataQueryMoveEnd,
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
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::Keymap;
    use crate::{
        action::Action,
        app::App,
        model::{
            relation::RelationTab,
            relation_edit::{RelationEditSession, RelationGridMode},
            tab::{GridRowAlignment, GridRowTarget, GridScrollAmount, WorkspaceTab},
            workspace::Focus,
        },
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
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
            keymap.map(key(KeyCode::Char('i')), &app),
            Some(Action::RelationEditCell)
        );
        assert_eq!(
            keymap.map(
                key(KeyCode::Char('x')),
                &relation_app(RelationGridMode::EditCell(
                    crate::model::relation_edit::CellEditorState {
                        row: 0,
                        column: 0,
                        input: Default::default(),
                    },
                ))
            ),
            Some(Action::RelationEditInsert('x'))
        );
        assert_eq!(
            keymap.map(
                key(KeyCode::Esc),
                &relation_app(RelationGridMode::EditCell(
                    crate::model::relation_edit::CellEditorState {
                        row: 0,
                        column: 0,
                        input: Default::default(),
                    },
                )),
            ),
            Some(Action::RelationEditCancel)
        );
    }

    #[test]
    fn cell_editor_maps_line_editing_controls_before_global_bindings() {
        let app = relation_app(RelationGridMode::EditCell(
            crate::model::relation_edit::CellEditorState {
                row: 0,
                column: 0,
                input: Default::default(),
            },
        ));
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
    fn relation_data_dd_and_yy_are_pending_sequences() {
        let app = relation_app(RelationGridMode::Browse);
        let mut keymap = Keymap::default();

        assert_eq!(keymap.map(key(KeyCode::Char('d')), &app), None);
        assert_eq!(
            keymap.map(key(KeyCode::Char('d')), &app),
            Some(Action::RelationDeleteCurrent)
        );
        assert_eq!(keymap.map(key(KeyCode::Char('y')), &app), None);
        assert_eq!(
            keymap.map(key(KeyCode::Char('y')), &app),
            Some(Action::RelationYank)
        );
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
        let app = relation_app(RelationGridMode::EditCell(
            crate::model::relation_edit::CellEditorState {
                row: 0,
                column: 0,
                input: Default::default(),
            },
        ));
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
            Some(Action::RelationCommit)
        );
        assert_eq!(
            keymap.map(
                KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
                &app,
            ),
            Some(Action::RelationRollback)
        );
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
}
