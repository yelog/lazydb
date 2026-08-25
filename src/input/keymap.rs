use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::{
    action::Action,
    app::App,
    model::{editor::EditorMode, workspace::Focus},
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
    pending: Option<(Pending, Instant)>,
}

impl Keymap {
    pub fn map(&mut self, event: KeyEvent, app: &App) -> Option<Action> {
        if matches!(event.kind, KeyEventKind::Release) {
            return None;
        }
        if let Some(overlay) = &app.overlay {
            let _ = overlay;
            return match event.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                    Some(Action::DismissOverlay)
                }
                _ => None,
            };
        }

        if let Some((pending, started)) = self.pending.take()
            && started.elapsed() <= SEQUENCE_TIMEOUT
            && let Some(action) = map_pending(pending, event.code)
        {
            return Some(action);
        }

        if event.modifiers.contains(KeyModifiers::CONTROL) {
            return match event.code {
                KeyCode::Char('w') => {
                    self.pending = Some((Pending::Window, Instant::now()));
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
                self.pending = Some((Pending::Leader, Instant::now()));
                return None;
            }
            KeyCode::Char('[') => {
                self.pending = Some((Pending::Previous, Instant::now()));
                return None;
            }
            KeyCode::Char(']') => {
                self.pending = Some((Pending::Next, Instant::now()));
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
}

fn map_pending(pending: Pending, code: KeyCode) -> Option<Action> {
    match (pending, code) {
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
