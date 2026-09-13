use lazydb::{
    action::Action,
    app::App,
    commands::CommandId,
    model::{omni::OmniItemId, text_input::TextInputEdit},
};

#[test]
fn new_console_flow_cancels_without_creating_and_confirm_creates_named_console() {
    let mut app = App::new(Vec::new());
    let before = app.sql_editors.len();
    app.update(Action::OpenOmni);
    app.omni.as_mut().unwrap().selected = Some(OmniItemId::Command(CommandId::NewConsole));
    app.update(Action::OmniConfirm);

    assert_eq!(app.sql_editors.len(), before);
    assert!(matches!(
        app.omni.as_ref().unwrap().step,
        lazydb::model::omni::OmniStep::NameConsole { .. }
    ));

    for character in "scratch".chars() {
        app.update(Action::OmniEdit(TextInputEdit::Insert(character)));
    }
    app.update(Action::OmniConfirm);

    assert!(app.omni.is_none());
    assert_eq!(app.sql_editors.len(), before + 1);
    assert_eq!(app.active_console().name, "scratch");
}

#[test]
fn escape_returns_from_object_action_step_before_closing_omni() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenOmni);
    app.omni
        .as_mut()
        .unwrap()
        .push_step(lazydb::model::omni::OmniStep::PickConnection);
    app.update(Action::OmniCancel);

    assert!(app.omni.is_some());
    assert_eq!(
        app.omni.as_ref().unwrap().step,
        lazydb::model::omni::OmniStep::Root
    );
    app.update(Action::OmniCancel);
    assert!(app.omni.is_none());
}
