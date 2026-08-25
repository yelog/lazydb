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
        if app.overlay == Some(Overlay::ProfileManager) {
            self.pending = None;
            return map_profile_manager(event, app);
        }
        if app.overlay.is_some() {
            self.pending = None;
            return match event.code {
                KeyCode::Esc | KeyCode::Char('q') => Some(Action::DismissOverlay),
                _ => None,
            };
        }

        if let Some((pending, started, focus, editor_mode, tab_id)) = self.pending.take()
            && started.elapsed() <= SEQUENCE_TIMEOUT
            && focus == app.focus
            && editor_mode == app.active_console().editor.mode
            && tab_id == app.active_console().id
            && let Some(action) = map_pending(pending, event)
        {
            return Some(action);
        }

        if event.modifiers.contains(KeyModifiers::CONTROL) {
            return match event.code {
                KeyCode::Char('w') => {
                    self.set_pending(Pending::Window, app);
                    None
                }
                KeyCode::Char('c') => {
                    if app.active_console().query_status
                        == crate::model::workspace::QueryStatus::Running
                    {
                        Some(Action::CancelActiveQuery)
                    } else if app.focus == Focus::Editor
                        && app.active_console().editor.mode == EditorMode::Insert
                    {
                        Some(Action::EnterNormalMode)
                    } else {
                        None
                    }
                }
                KeyCode::Char('h') if app.focus == Focus::Editor => Some(Action::Backspace),
                _ => None,
            };
        }
        if event.code == KeyCode::F(5) {
            return Some(Action::RunActiveSql);
        }
        if event.code == KeyCode::F(1) {
            return Some(Action::ShowHelp);
        }
        if event.code == KeyCode::Char('Q') {
            return Some(Action::Quit);
        }

        let editor = &app.active_console().editor;
        if app.focus == Focus::Editor && editor.mode == EditorMode::Insert {
            return map_insert(event.code);
        }

        if event.code == KeyCode::Tab {
            return Some(Action::FocusNext);
        }
        if event.code == KeyCode::BackTab {
            return Some(Action::FocusPrevious);
        }

        match event.code {
            KeyCode::Char('?') => return Some(Action::ShowHelp),
            KeyCode::Esc if app.focus == Focus::Editor => return Some(Action::EnterNormalMode),
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

        match app.focus {
            Focus::Explorer => map_explorer(event.code),
            Focus::Editor => map_normal_editor(event.code),
            Focus::Results => map_results(event.code),
        }
    }

    fn set_pending(&mut self, pending: Pending, app: &App) {
        self.pending = Some((
            pending,
            Instant::now(),
            app.focus,
            app.active_console().editor.mode,
            app.active_console().id,
        ));
    }

    pub fn clear_pending(&mut self) {
        self.pending = None;
    }
}

fn map_pending(pending: Pending, event: KeyEvent) -> Option<Action> {
    let valid_modifiers = event.modifiers.is_empty()
        || (pending == Pending::Window && event.modifiers == KeyModifiers::CONTROL);
    if !valid_modifiers {
        return None;
    }
    match (pending, event.code) {
        (Pending::Leader, KeyCode::Char('c')) => Some(Action::OpenProfileManager),
        (Pending::Leader, KeyCode::Char('n')) => Some(Action::NewConsole),
        (Pending::Leader, KeyCode::Char('r')) => Some(Action::RunActiveSql),
        (Pending::Window, KeyCode::Char('h')) => Some(Action::Focus(Focus::Explorer)),
        (Pending::Window, KeyCode::Char('j')) => Some(Action::Focus(Focus::Results)),
        (Pending::Window, KeyCode::Char('k' | 'l')) => Some(Action::Focus(Focus::Editor)),
        (Pending::Previous, KeyCode::Char('t')) => Some(Action::PreviousTab),
        (Pending::Next, KeyCode::Char('t')) => Some(Action::NextTab),
        _ => None,
    }
}

pub fn map_paste(value: String, app: &App) -> Vec<Action> {
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
    if app.focus != Focus::Editor || app.active_console().editor.mode != EditorMode::Insert {
        return Vec::new();
    }
    value
        .chars()
        .map(|character| {
            if character == '\n' {
                Action::InsertNewline
            } else {
                Action::InsertCharacter(character)
            }
        })
        .collect()
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
        ProfileManagerPage::List => map_profile_list(event.code),
        ProfileManagerPage::Form => map_profile_form(event.code, manager.selected_field),
        ProfileManagerPage::ConfirmDelete => map_profile_delete_confirmation(event.code),
    }
}

fn map_profile_list(code: KeyCode) -> Option<Action> {
    match code {
        KeyCode::Char('j') | KeyCode::Down => Some(Action::ProfileMove(1)),
        KeyCode::Char('k') | KeyCode::Up => Some(Action::ProfileMove(-1)),
        KeyCode::Home => Some(Action::ProfileMove(isize::MIN)),
        KeyCode::End => Some(Action::ProfileMove(isize::MAX)),
        KeyCode::Enter => Some(Action::ProfileConnectSelected),
        KeyCode::Char('n') => Some(Action::ProfileStartNew),
        KeyCode::Char('e') => Some(Action::ProfileStartEdit),
        KeyCode::Char('d') => Some(Action::ProfileRequestDelete),
        KeyCode::Esc | KeyCode::Char('q') => Some(Action::CloseProfileManager),
        _ => None,
    }
}

fn map_profile_form(code: KeyCode, field: ProfileField) -> Option<Action> {
    match code {
        KeyCode::Esc => return Some(Action::CloseProfileManager),
        KeyCode::Tab => return Some(Action::ProfileFieldNext),
        KeyCode::BackTab => return Some(Action::ProfileFieldPrevious),
        _ => {}
    }
    if is_text_field(field) {
        return match code {
            KeyCode::Char(character) => Some(Action::ProfileInsert(ProfileInput::from(character))),
            KeyCode::Backspace => Some(Action::ProfileBackspace),
            KeyCode::Delete => Some(Action::ProfileDeleteCharacter),
            KeyCode::Left => Some(Action::ProfileMoveLeft),
            KeyCode::Right => Some(Action::ProfileMoveRight),
            KeyCode::Home => Some(Action::ProfileMoveHome),
            KeyCode::End => Some(Action::ProfileMoveEnd),
            _ => None,
        };
    }
    if is_cycle_field(field) {
        return match code {
            KeyCode::Left | KeyCode::Up | KeyCode::Char('h' | 'k') => {
                Some(Action::ProfileCycle(-1))
            }
            KeyCode::Right | KeyCode::Down | KeyCode::Enter | KeyCode::Char(' ' | 'j' | 'l') => {
                Some(Action::ProfileCycle(1))
            }
            _ => None,
        };
    }
    if is_toggle_field(field) {
        return matches!(code, KeyCode::Enter | KeyCode::Char(' '))
            .then_some(Action::ProfileToggle);
    }
    match (field, code) {
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
            | ProfileField::Database
            | ProfileField::Schema
            | ProfileField::SqlitePath
    )
}

fn is_cycle_field(field: ProfileField) -> bool {
    matches!(
        field,
        ProfileField::Kind | ProfileField::SslMode | ProfileField::Environment
    )
}

fn is_toggle_field(field: ProfileField) -> bool {
    matches!(
        field,
        ProfileField::ReadOnly | ProfileField::RememberPassword | ProfileField::SqliteMemory
    )
}

fn map_insert(code: KeyCode) -> Option<Action> {
    match code {
        KeyCode::Esc => Some(Action::EnterNormalMode),
        KeyCode::Char(character) => Some(Action::InsertCharacter(character)),
        KeyCode::Tab => Some(Action::InsertCharacter('\t')),
        KeyCode::Enter => Some(Action::InsertNewline),
        KeyCode::Backspace => Some(Action::Backspace),
        KeyCode::Delete => Some(Action::Delete),
        KeyCode::Left => Some(Action::MoveLeft),
        KeyCode::Right => Some(Action::MoveRight),
        KeyCode::Up => Some(Action::MoveUp),
        KeyCode::Down => Some(Action::MoveDown),
        KeyCode::Home => Some(Action::MoveHome),
        KeyCode::End => Some(Action::MoveEnd),
        _ => None,
    }
}

fn map_normal_editor(code: KeyCode) -> Option<Action> {
    match code {
        KeyCode::Char('h') | KeyCode::Left => Some(Action::MoveLeft),
        KeyCode::Char('j') | KeyCode::Down => Some(Action::MoveDown),
        KeyCode::Char('k') | KeyCode::Up => Some(Action::MoveUp),
        KeyCode::Char('l') | KeyCode::Right => Some(Action::MoveRight),
        KeyCode::Char('i') => Some(Action::EnterInsertMode),
        KeyCode::Char('a') => Some(Action::EnterAppendMode),
        KeyCode::Char('o') => Some(Action::OpenLineBelow),
        KeyCode::Char('x') | KeyCode::Delete => Some(Action::Delete),
        KeyCode::Char('0') | KeyCode::Home => Some(Action::MoveHome),
        KeyCode::Char('$') | KeyCode::End => Some(Action::MoveEnd),
        _ => None,
    }
}

fn map_explorer(code: KeyCode) -> Option<Action> {
    match code {
        KeyCode::Char('j') | KeyCode::Down => Some(Action::ExplorerMove(1)),
        KeyCode::Char('k') | KeyCode::Up => Some(Action::ExplorerMove(-1)),
        KeyCode::Char('h')
        | KeyCode::Char('l')
        | KeyCode::Enter
        | KeyCode::Right
        | KeyCode::Left => Some(Action::ExplorerToggle),
        KeyCode::Char('r') => Some(Action::RefreshCatalog),
        KeyCode::Char('p') => Some(Action::PreviewSelected),
        KeyCode::Char('D') => Some(Action::DdlSelected),
        KeyCode::Home => Some(Action::ExplorerMove(isize::MIN)),
        KeyCode::End => Some(Action::ExplorerMove(isize::MAX)),
        _ => None,
    }
}

fn map_results(code: KeyCode) -> Option<Action> {
    match code {
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
        KeyCode::Char('o') => Some(Action::ToggleResultView),
        _ => None,
    }
}
