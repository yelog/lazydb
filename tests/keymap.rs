use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use lazydb::{
    action::Action,
    app::App,
    input::keymap::Keymap,
    model::{editor::EditorMode, workspace::Focus},
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(character: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL)
}

#[test]
fn maps_global_sequences_and_function_keys() {
    let mut keymap = Keymap::default();
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;

    assert_eq!(
        keymap.map(key(KeyCode::Char('?')), &app),
        Some(Action::ShowHelp)
    );
    assert_eq!(
        keymap.map(key(KeyCode::F(5)), &app),
        Some(Action::RunActiveSql)
    );
    assert_eq!(keymap.map(key(KeyCode::F(1)), &app), Some(Action::ShowHelp));
    assert_eq!(keymap.map(ctrl('w'), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('h')), &app),
        Some(Action::Focus(Focus::Explorer))
    );
    assert_eq!(keymap.map(key(KeyCode::Char(']')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('t')), &app),
        Some(Action::NextTab)
    );
    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('n')), &app),
        Some(Action::NewConsole)
    );
}

#[test]
fn insert_mode_preserves_printable_characters() {
    let mut keymap = Keymap::default();
    let app = App::new(Vec::new());
    assert_eq!(app.active_console().editor.mode, EditorMode::Insert);

    assert_eq!(
        keymap.map(key(KeyCode::Char('?')), &app),
        Some(Action::InsertCharacter('?'))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('q')), &app),
        Some(Action::InsertCharacter('q'))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Esc), &app),
        Some(Action::EnterNormalMode)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Tab), &app),
        Some(Action::InsertCharacter('\t'))
    );
    assert_eq!(keymap.map(ctrl('c'), &app), Some(Action::EnterNormalMode));
}

#[test]
fn maps_vim_editor_navigation_in_normal_mode() {
    let mut keymap = Keymap::default();
    let mut app = App::new(Vec::new());
    app.active_console_mut().editor.mode = EditorMode::Normal;

    assert_eq!(
        keymap.map(key(KeyCode::Char('h')), &app),
        Some(Action::MoveLeft)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::MoveDown)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('k')), &app),
        Some(Action::MoveUp)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('l')), &app),
        Some(Action::MoveRight)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('i')), &app),
        Some(Action::EnterInsertMode)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('a')), &app),
        Some(Action::EnterAppendMode)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('o')), &app),
        Some(Action::OpenLineBelow)
    );
    assert_eq!(keymap.map(key(KeyCode::Char(' ')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('r')), &app),
        Some(Action::RunActiveSql)
    );
}

#[test]
fn maps_explorer_and_result_actions_by_context() {
    let mut keymap = Keymap::default();
    let mut app = App::new(Vec::new());
    app.focus = Focus::Explorer;

    assert_eq!(
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::ExplorerMove(1))
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('l')), &app),
        Some(Action::ExplorerToggle)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('p')), &app),
        Some(Action::PreviewSelected)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('D')), &app),
        Some(Action::DdlSelected)
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('r')), &app),
        Some(Action::RefreshCatalog)
    );

    app.focus = Focus::Results;
    assert_eq!(
        keymap.map(key(KeyCode::Char('j')), &app),
        Some(Action::GridMove {
            rows: 1,
            columns: 0
        })
    );
    assert_eq!(
        keymap.map(key(KeyCode::Char('o')), &app),
        Some(Action::ToggleResultView)
    );
}

#[test]
fn never_uses_lowercase_q_as_a_global_exit() {
    let mut keymap = Keymap::default();
    let mut app = App::new(Vec::new());
    app.focus = Focus::Results;

    assert_eq!(keymap.map(key(KeyCode::Char('q')), &app), None);
    assert_eq!(
        keymap.map(key(KeyCode::Char('Q')), &app),
        Some(Action::Quit)
    );
}
