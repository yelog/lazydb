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
    RelationDelete,
    RelationYank,
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
        if app.focus == Focus::Explorer && app.explorer.search.is_some() {
            self.pending = None;
            if event.modifiers == KeyModifiers::CONTROL && event.code == KeyCode::Char('u') {
                return Some(Action::ExplorerSearchClear);
            }
            if event.modifiers == KeyModifiers::CONTROL && event.code == KeyCode::Char('r') {
                return Some(Action::ExplorerSearchRetry);
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
        if app.focus == Focus::Editor && app.active_editor_mode() == EditorMode::Normal {
            match event.code {
                KeyCode::Char('?') => return Some(Action::ShowHelp),
                KeyCode::Tab => return Some(Action::FocusNext),
                KeyCode::BackTab => return Some(Action::FocusPrevious),
                _ => {}
            }
        }
        if app.focus == Focus::Editor
            && app.active_editor_mode() == EditorMode::Insert
            && event.code == KeyCode::Esc
        {
            return Some(Action::EditorKey(event));
        }
        if app.overlay.is_some() {
            self.pending = None;
            return match event.code {
                KeyCode::Esc | KeyCode::Char('q') => Some(Action::DismissOverlay),
                _ => None,
            };
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
            && let Some(action) = map_pending(pending, event, app)
        {
            return Some(action);
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
            return match event.code {
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
        if event.code == KeyCode::Char('Q')
            && (app.focus != Focus::Editor || app.active_editor_mode() == EditorMode::Normal)
        {
            return Some(Action::Quit);
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
        || (pending == Pending::Window && event.modifiers == KeyModifiers::CONTROL);
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
        (Pending::Window, KeyCode::Char('h')) => Some(Action::Focus(Focus::Explorer)),
        (Pending::Window, KeyCode::Char('j')) => Some(Action::Focus(Focus::Results)),
        (Pending::Window, KeyCode::Char('k' | 'l')) => {
            Some(Action::Focus(if app.is_active_relation_tab() {
                Focus::Results
            } else {
                Focus::Editor
            }))
        }
        (Pending::Previous, KeyCode::Char('t')) => Some(Action::PreviousTab),
        (Pending::Next, KeyCode::Char('t')) => Some(Action::NextTab),
        (Pending::RelationDelete, KeyCode::Char('d')) => Some(Action::RelationDeleteCurrent),
        (Pending::RelationYank, KeyCode::Char('y')) => Some(Action::RelationYank),
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

fn map_relation_data(event: KeyEvent, app: &App) -> Option<Action> {
    use crate::model::relation_edit::RelationGridMode;

    let mode = app.tabs.get(app.active_tab).and_then(|tab| match tab {
        crate::model::tab::WorkspaceTab::Relation(tab) => tab.edit.as_ref().map(|edit| &edit.mode),
        _ => None,
    });
    if let Some(RelationGridMode::EditCell(_)) = mode {
        if !event.modifiers.is_empty() {
            return None;
        }
        return match event.code {
            KeyCode::Enter => Some(Action::RelationEditConfirm),
            KeyCode::Esc => Some(Action::RelationEditCancel),
            KeyCode::Backspace => Some(Action::RelationEditBackspace),
            KeyCode::Delete => Some(Action::RelationEditDelete),
            KeyCode::Left => Some(Action::RelationEditMoveLeft),
            KeyCode::Right => Some(Action::RelationEditMoveRight),
            KeyCode::Home => Some(Action::RelationEditMoveHome),
            KeyCode::End => Some(Action::RelationEditMoveEnd),
            KeyCode::Char(character) => Some(Action::RelationEditInsert(character)),
            _ => None,
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
        KeyCode::Char('/') => return Some(Action::ExplorerSearchOpen),
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
        _ => {}
    }
    match code {
        KeyCode::Char('j') | KeyCode::Down => Some(Action::ExplorerMove(1)),
        KeyCode::Char('k') | KeyCode::Up => Some(Action::ExplorerMove(-1)),
        KeyCode::Char('l') | KeyCode::Right => Some(Action::ExplorerExpand),
        KeyCode::Char('h') | KeyCode::Left => Some(Action::ExplorerCollapse),
        KeyCode::Enter => Some(Action::ExplorerOpenSelected),
        KeyCode::Char('o') => Some(Action::ExplorerToggle),
        KeyCode::Char('r') => Some(Action::ExplorerRefresh),
        KeyCode::Char('p') => Some(Action::OpenSelectedRelation {
            view: crate::model::relation::RelationView::Data,
        }),
        KeyCode::Char('D') => Some(Action::OpenSelectedRelation {
            view: crate::model::relation::RelationView::Structure,
        }),
        KeyCode::Home => Some(Action::ExplorerMove(isize::MIN)),
        KeyCode::End => Some(Action::ExplorerMove(isize::MAX)),
        _ => None,
    }
}

fn map_results(code: KeyCode, app: &App) -> Option<Action> {
    match code {
        KeyCode::Char('[') => Some(Action::GridResizeColumn(-1)),
        KeyCode::Char(']') => Some(Action::GridResizeColumn(1)),
        KeyCode::Char('=') => Some(Action::GridResetColumnWidth),
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
        KeyCode::Char('o') => match app.tabs.get(app.active_tab) {
            Some(crate::model::tab::WorkspaceTab::Relation(tab)) => {
                Some(Action::SetRelationView(match tab.view {
                    crate::model::relation::RelationView::Data => {
                        crate::model::relation::RelationView::Structure
                    }
                    crate::model::relation::RelationView::Structure => {
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
            crate::model::relation::RelationView::Structure,
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
        if event.modifiers.contains(KeyModifiers::CONTROL) {
            return match event.code {
                KeyCode::Char('u') => Some(Action::DataQueryClear),
                _ => None,
            };
        }
        return match event.code {
            KeyCode::Esc => Some(Action::CancelDataQueryInput),
            KeyCode::Enter => Some(Action::SubmitDataQuery),
            KeyCode::Tab => Some(Action::FocusDataQueryInput(match input {
                DataQueryInput::Where => DataQueryInput::OrderBy,
                DataQueryInput::OrderBy => DataQueryInput::Where,
            })),
            KeyCode::BackTab => Some(Action::FocusDataQueryInput(match input {
                DataQueryInput::Where => DataQueryInput::OrderBy,
                DataQueryInput::OrderBy => DataQueryInput::Where,
            })),
            KeyCode::Backspace => Some(Action::DataQueryBackspace),
            KeyCode::Delete => Some(Action::DataQueryDelete),
            KeyCode::Left => Some(Action::DataQueryMoveLeft),
            KeyCode::Right => Some(Action::DataQueryMoveRight),
            KeyCode::Home => Some(Action::DataQueryMoveHome),
            KeyCode::End => Some(Action::DataQueryMoveEnd),
            KeyCode::Char(character) => Some(Action::DataQueryInsert(character)),
            _ => None,
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

#[cfg(test)]
mod tests {
    use super::Keymap;
    use crate::{
        action::Action,
        app::App,
        model::{
            relation::RelationTab,
            relation_edit::{RelationEditSession, RelationGridMode},
            tab::WorkspaceTab,
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
            Some(Action::ExplorerSearchRetry)
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
            crate::model::relation::RelationView::Structure,
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
}
