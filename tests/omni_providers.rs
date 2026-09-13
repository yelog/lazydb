use lazydb::{
    action::Action,
    app::App,
    commands::CommandId,
    model::{omni::OmniItemId, text_input::TextInputEdit},
};

#[test]
fn local_command_and_console_results_are_available_without_database_io() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenOmni);
    let omni = app.omni.as_ref().unwrap();

    assert!(
        omni.items
            .iter()
            .any(|item| { item.id == OmniItemId::Command(CommandId::NewConsole) })
    );
    assert!(
        omni.items.iter().any(|item| {
            matches!(item.id, OmniItemId::Console { .. }) && item.title == "console"
        })
    );
    assert!(
        omni.visible_items()
            .iter()
            .any(|item| item.title == "New Console")
    );
}

#[test]
fn command_filter_is_immediate_and_does_not_emit_database_commands() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenOmni);
    for character in "> format".chars() {
        app.update(Action::OmniEdit(TextInputEdit::Insert(character)));
    }
    let omni = app.omni.as_ref().unwrap();

    assert_eq!(omni.visible_items().len(), 1);
    assert_eq!(
        omni.visible_items()[0].id,
        OmniItemId::Command(CommandId::FormatSql)
    );
}

#[test]
fn selecting_a_connection_filters_by_profile_id_without_switching_connection() {
    let first = lazydb::profile::import_connection_url("sqlite::memory:", Some("local"))
        .unwrap()
        .profile;
    let second = lazydb::profile::import_connection_url("sqlite:/tmp/other.db", Some("other"))
        .unwrap()
        .profile;
    let first_id = first.id;
    let mut app = App::new(vec![first, second]);
    app.update(Action::OpenOmni);
    let original_connection = app.connection.active_identity();

    let omni = app.omni.as_mut().unwrap();
    omni.selected = Some(OmniItemId::Profile(first_id));
    app.update(Action::OmniConfirm);
    let omni = app.omni.as_ref().unwrap();

    assert_eq!(omni.profile_scope, Some(first_id));
    assert!(
        omni.visible_items()
            .iter()
            .all(|item| item.context.profile_id == Some(first_id))
    );
    assert_eq!(app.connection.active_identity(), original_connection);
}
