use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use lazydb::{
    action::Action,
    app::App,
    input::keymap::{Keymap, map_paste},
    model::{editor::EditorMode, text_input::TextInputEdit},
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn global_f2_opens_omni_before_insert_mode_receives_the_key() {
    let mut app = App::new(Vec::new());
    app.update(Action::EditorKey(key(KeyCode::Char('i'))));
    assert_eq!(app.active_editor_mode(), EditorMode::Insert);

    let mut keymap = Keymap::default();
    let action = keymap.map(key(KeyCode::F(2)), &app);
    assert_eq!(action, Some(Action::OpenOmni));
    app.update(action.unwrap());
    assert!(app.omni.is_some());
    assert_eq!(app.active_editor_mode(), EditorMode::Insert);
}

#[test]
fn omni_input_and_paste_never_reach_the_underlying_editor() {
    let mut app = App::new(Vec::new());
    let before = app.active_editor_text().unwrap();
    let mut keymap = Keymap::default();
    app.update(Action::OpenOmni);

    let action = keymap.map(key(KeyCode::Char('u')), &app).unwrap();
    assert!(matches!(
        action,
        Action::OmniEdit(TextInputEdit::Insert('u'))
    ));
    app.update(action);
    for action in map_paste("users".to_owned(), &app) {
        app.update(action);
    }
    assert_eq!(app.omni.as_ref().unwrap().query(), "uusers");
    assert_eq!(app.active_editor_text().unwrap(), before);
}

#[test]
fn escape_restores_a_real_overlay_and_ctrl_c_does_not_quit() {
    let mut app = App::new(Vec::new());
    app.update(Action::ShowHelp);
    let help = app.overlay.clone();
    app.update(Action::OpenOmni);
    assert!(app.overlay.is_none());
    app.update(Action::OmniDismiss);

    assert_eq!(app.overlay, help);
    assert!(!app.should_quit);
}

#[test]
fn omni_blocks_unrelated_actions_while_open() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenOmni);
    app.update(Action::EditorKey(key(KeyCode::Char('x'))));
    assert_eq!(app.active_editor_text().unwrap(), "");
}
