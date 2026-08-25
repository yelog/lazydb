use lazydb::{
    action::{Action, Command},
    app::App,
    db::ServerInfo,
    model::{
        profile_manager::{ProfileField, ProfileManagerPage, ProfileOperation},
        workspace::{ConnectionIdentity, ConnectionStatus, Overlay, QueryStatus},
    },
    profile::{ConnectionProfile, DatabaseKind, import_connection_url},
};

fn sqlite_profile(name: &str) -> ConnectionProfile {
    import_connection_url(":memory:", Some(name))
        .unwrap()
        .profile
}

fn valid_new_profile(app: &mut App, name: &str) {
    let manager = app.profile_manager.as_mut().unwrap();
    let draft = manager.draft.as_mut().unwrap();
    draft.name.set(name);
    draft.database.set("lazydb");
}

fn server() -> ServerInfo {
    ServerInfo {
        kind: DatabaseKind::Postgres,
        version: "16.4".into(),
        database: "lazydb".into(),
    }
}

#[test]
fn opening_manager_uses_the_list_or_a_new_form() {
    let mut populated = App::new(vec![sqlite_profile("primary")]);
    assert!(populated.update(Action::OpenProfileManager).is_empty());
    assert_eq!(populated.overlay, Some(Overlay::ProfileManager));
    assert_eq!(
        populated.profile_manager.as_ref().unwrap().page,
        ProfileManagerPage::List
    );

    let mut empty = App::new(Vec::new());
    empty.update(Action::OpenProfileManager);
    let manager = empty.profile_manager.as_ref().unwrap();
    assert_eq!(manager.page, ProfileManagerPage::Form);
    assert_eq!(manager.draft.as_ref().unwrap().kind, DatabaseKind::Postgres);
}

#[test]
fn list_navigation_new_edit_cancel_and_delete_confirmation_are_pure() {
    let first = sqlite_profile("first");
    let second = sqlite_profile("second");
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);
    app.update(Action::OpenProfileManager);

    app.update(Action::ProfileMove(1));
    assert_eq!(app.profile_manager.as_ref().unwrap().selected, 1);
    app.update(Action::ProfileStartEdit);
    assert_eq!(
        app.profile_manager
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .profile_id(),
        second_id
    );

    app.update(Action::CloseProfileManager);
    assert_eq!(
        app.profile_manager.as_ref().unwrap().page,
        ProfileManagerPage::List
    );
    app.update(Action::ProfileStartNew);
    assert_eq!(
        app.profile_manager.as_ref().unwrap().page,
        ProfileManagerPage::Form
    );
    app.update(Action::CloseProfileManager);

    app.update(Action::ProfileRequestDelete);
    assert_eq!(
        app.profile_manager.as_ref().unwrap().page,
        ProfileManagerPage::ConfirmDelete
    );
    app.update(Action::ProfileCancelDelete);
    assert_eq!(
        app.profile_manager.as_ref().unwrap().page,
        ProfileManagerPage::List
    );

    app.update(Action::CloseProfileManager);
    assert!(app.profile_manager.is_none());
    assert!(app.overlay.is_none());
}

#[test]
fn form_navigation_skips_fields_hidden_by_driver_and_mode() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);

    app.update(Action::ProfileFocusField(ProfileField::Kind));
    app.update(Action::ProfileCycle(-1));
    assert_eq!(
        app.profile_manager
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .kind,
        DatabaseKind::Sqlite
    );
    app.update(Action::ProfileFocusField(ProfileField::SqliteMemory));
    app.update(Action::ProfileToggle);
    app.update(Action::ProfileFieldNext);
    assert_eq!(
        app.profile_manager.as_ref().unwrap().selected_field,
        ProfileField::ReadOnly
    );
    app.update(Action::ProfileFieldPrevious);
    assert_eq!(
        app.profile_manager.as_ref().unwrap().selected_field,
        ProfileField::SqliteMemory
    );
}

#[test]
fn text_actions_edit_only_the_focused_text_or_password_field() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileFocusField(ProfileField::Name));
    app.update(Action::ProfileInsert('数'.into()));
    app.update(Action::ProfilePaste("据库".into()));
    app.update(Action::ProfileMoveLeft);
    app.update(Action::ProfileBackspace);

    let draft = app
        .profile_manager
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap();
    assert_eq!(draft.name.value(), "数库");
    assert_eq!(draft.host.value(), "localhost");

    app.update(Action::ProfileFocusField(ProfileField::Password));
    let password_input = Action::ProfilePaste("do-not-print".into());
    assert!(!format!("{password_input:?}").contains("do-not-print"));
    app.update(password_input);
    app.update(Action::ProfileMoveLeft);
    app.update(Action::ProfileDeleteCharacter);
    let draft = app
        .profile_manager
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap();
    assert_eq!(draft.password_len(), "do-not-prin".chars().count());
    assert!(!format!("{draft:?}").contains("do-not-print"));
}

#[test]
fn cycle_and_toggle_actions_only_change_supported_fields() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);

    app.update(Action::ProfileFocusField(ProfileField::Kind));
    app.update(Action::ProfileCycle(1));
    app.update(Action::ProfileFocusField(ProfileField::Environment));
    app.update(Action::ProfileCycle(1));
    app.update(Action::ProfileFocusField(ProfileField::ReadOnly));
    app.update(Action::ProfileToggle);
    app.update(Action::ProfileFocusField(ProfileField::RememberPassword));
    app.update(Action::ProfileToggle);

    let draft = app
        .profile_manager
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap();
    assert_eq!(draft.kind, DatabaseKind::MySql);
    assert_eq!(draft.environment, lazydb::profile::Environment::Staging);
    assert!(draft.read_only);
    assert!(draft.remember_password);
}

#[test]
fn test_rejects_invalid_drafts_and_tracks_matching_results() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);

    assert!(app.update(Action::ProfileTest).is_empty());
    let manager = app.profile_manager.as_ref().unwrap();
    assert_eq!(manager.selected_field, ProfileField::Name);
    assert!(manager.message.as_deref().unwrap().contains("required"));

    valid_new_profile(&mut app, "primary");
    let commands = app.update(Action::ProfileTest);
    let request_id = match commands.as_slice() {
        [Command::TestProfile { request_id, .. }] => *request_id,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    assert_eq!(
        app.profile_manager.as_ref().unwrap().operation,
        Some(ProfileOperation::Testing)
    );

    app.update(Action::OpenProfileManager);
    assert_eq!(
        app.profile_manager.as_ref().unwrap().operation,
        Some(ProfileOperation::Testing)
    );

    app.update(Action::ProfileTestFailed {
        request_id: request_id + 1,
        message: "stale".into(),
    });
    assert_eq!(
        app.profile_manager.as_ref().unwrap().operation,
        Some(ProfileOperation::Testing)
    );

    app.update(Action::ProfileTestSucceeded {
        request_id,
        server: server(),
    });
    let manager = app.profile_manager.as_ref().unwrap();
    assert_eq!(manager.operation, None);
    assert!(manager.message.as_deref().unwrap().contains("16.4"));
}

#[test]
fn save_and_save_and_connect_emit_distinct_commands() {
    for connect in [false, true] {
        let mut app = App::new(Vec::new());
        app.update(Action::OpenProfileManager);
        valid_new_profile(&mut app, "primary");

        let commands = app.update(Action::ProfileSave { connect });
        match commands.as_slice() {
            [
                Command::SaveProfile {
                    connect: command_connect,
                    ..
                },
            ] => assert_eq!(*command_connect, connect),
            commands => panic!("unexpected commands: {commands:?}"),
        }
        assert_eq!(
            app.profile_manager.as_ref().unwrap().operation,
            Some(if connect {
                ProfileOperation::SavingAndConnecting
            } else {
                ProfileOperation::Saving
            })
        );
    }
}

#[test]
fn save_success_upserts_without_reordering_profiles() {
    let first = sqlite_profile("first");
    let second = sqlite_profile("second");
    let expected_ids = [first.id, second.id];
    let mut app = App::new(vec![first, second]);
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileMove(1));
    app.update(Action::ProfileStartEdit);
    app.profile_manager
        .as_mut()
        .unwrap()
        .draft
        .as_mut()
        .unwrap()
        .name
        .set("renamed");
    let request_id = match app
        .update(Action::ProfileSave { connect: false })
        .as_slice()
    {
        [Command::SaveProfile { request_id, .. }] => *request_id,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    let saved = app
        .profile_manager
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap()
        .validate(&app.profiles)
        .unwrap()
        .profile;

    app.update(Action::ProfileSaved {
        request_id,
        profile: saved,
        warning: None,
        connect: false,
    });

    assert_eq!(
        app.profiles
            .iter()
            .map(|profile| profile.id)
            .collect::<Vec<_>>(),
        expected_ids
    );
    assert_eq!(app.profiles[1].name, "renamed");
    assert_eq!(
        app.profile_manager.as_ref().unwrap().page,
        ProfileManagerPage::List
    );
}

#[test]
fn delete_success_removes_the_profile_and_clamps_selection() {
    let first = sqlite_profile("first");
    let second = sqlite_profile("second");
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileMove(1));
    app.update(Action::ProfileRequestDelete);
    let request_id = match app.update(Action::ProfileConfirmDelete).as_slice() {
        [
            Command::DeleteProfile {
                request_id,
                profile_id,
            },
        ] => {
            assert_eq!(*profile_id, second_id);
            *request_id
        }
        commands => panic!("unexpected commands: {commands:?}"),
    };

    app.update(Action::ProfileDeleted {
        request_id,
        profile_id: second_id,
        active_connection: None,
    });
    assert_eq!(app.profiles.len(), 1);
    assert_eq!(app.profile_manager.as_ref().unwrap().selected, 0);
    assert_eq!(
        app.profile_manager.as_ref().unwrap().page,
        ProfileManagerPage::List
    );
}

#[test]
fn deleting_or_saving_a_pending_profile_clears_its_connection_state() {
    let profile = sqlite_profile("pending");
    let profile_id = profile.id;
    let mut deleting = App::new(vec![profile.clone()]);
    deleting.connection.pending_profile_id = Some(profile_id);
    deleting.connection.pending_generation = Some(1);
    deleting.connection.status = ConnectionStatus::Connecting;
    deleting.update(Action::OpenProfileManager);
    deleting.update(Action::ProfileRequestDelete);
    let request_id = match deleting.update(Action::ProfileConfirmDelete).as_slice() {
        [Command::DeleteProfile { request_id, .. }] => *request_id,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    assert!(matches!(
        deleting
            .update(Action::ProfileDeleted {
                request_id,
                profile_id,
                active_connection: None,
            })
            .as_slice(),
        [Command::Disconnect { connection }] if *connection == ConnectionIdentity {
            profile_id,
            generation: 1,
        }
    ));

    let mut saving = App::new(vec![profile]);
    saving.connection.pending_profile_id = Some(profile_id);
    saving.connection.pending_generation = Some(1);
    saving.connection.status = ConnectionStatus::Connecting;
    saving.update(Action::OpenProfileManager);
    saving.update(Action::ProfileStartEdit);
    let request_id = match saving
        .update(Action::ProfileSave { connect: false })
        .as_slice()
    {
        [Command::SaveProfile { request_id, .. }] => *request_id,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    let saved = saving
        .profile_manager
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap()
        .validate(&saving.profiles)
        .unwrap()
        .profile;
    assert!(matches!(
        saving
            .update(Action::ProfileSaved {
                request_id,
                profile: saved,
                warning: None,
                connect: false,
            })
            .as_slice(),
        [Command::Disconnect { connection }] if *connection == ConnectionIdentity {
            profile_id,
            generation: 1,
        }
    ));
    saving.update(Action::DisconnectCompleted {
        connection: ConnectionIdentity {
            profile_id,
            generation: 1,
        },
    });
    assert_eq!(saving.connection.status, ConnectionStatus::Disconnected);
    assert!(saving.connection.profile_id.is_none());
}

#[test]
fn saving_an_active_profile_retires_the_old_connection() {
    let profile = sqlite_profile("active");
    let profile_id = profile.id;
    let connection = ConnectionIdentity {
        profile_id,
        generation: 9,
    };
    let mut app = App::new(vec![profile]);
    app.connection.profile_id = Some(profile_id);
    app.connection.generation = connection.generation;
    app.connection.status = ConnectionStatus::Connected;
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileStartEdit);
    let request_id = match app
        .update(Action::ProfileSave { connect: false })
        .as_slice()
    {
        [Command::SaveProfile { request_id, .. }] => *request_id,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ReplaceEditor("SELECT 1".into()));
    assert!(app.update(Action::RunActiveSql).is_empty());
    let saved = app
        .profile_manager
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap()
        .validate(&app.profiles)
        .unwrap()
        .profile;

    let commands = app.update(Action::ProfileSaved {
        request_id,
        profile: saved,
        warning: None,
        connect: false,
    });
    assert!(matches!(
        commands.as_slice(),
        [Command::Disconnect { connection: disconnected }] if *disconnected == connection
    ));
    assert!(app.connection.profile_id.is_none());
    assert_eq!(app.connection.status, ConnectionStatus::Disconnected);
    assert!(app.update(Action::RunActiveSql).is_empty());
}

#[test]
fn query_started_while_save_is_in_flight_preserves_the_active_connection() {
    let profile = sqlite_profile("active");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.connection.profile_id = Some(profile_id);
    app.connection.generation = 3;
    app.connection.status = ConnectionStatus::Connected;
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileStartEdit);
    let request_id = match app
        .update(Action::ProfileSave { connect: false })
        .as_slice()
    {
        [Command::SaveProfile { request_id, .. }] => *request_id,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    let saved = app
        .profile_manager
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap()
        .validate(&app.profiles)
        .unwrap()
        .profile;
    app.active_console_mut().query_status = QueryStatus::Running;

    assert!(
        app.update(Action::ProfileSaved {
            request_id,
            profile: saved,
            warning: None,
            connect: false,
        })
        .is_empty()
    );
    assert_eq!(app.connection.profile_id, Some(profile_id));
    assert_eq!(app.connection.status, ConnectionStatus::Connected);
    assert!(
        app.profile_manager
            .as_ref()
            .unwrap()
            .message
            .as_deref()
            .unwrap()
            .contains("running")
    );
}

#[test]
fn deleting_an_active_profile_retires_it_before_disconnect_completes() {
    let profile = sqlite_profile("active");
    let profile_id = profile.id;
    let connection = ConnectionIdentity {
        profile_id,
        generation: 7,
    };
    let mut app = App::new(vec![profile]);
    app.connection.profile_id = Some(profile_id);
    app.connection.generation = connection.generation;
    app.connection.status = ConnectionStatus::Connected;
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileRequestDelete);
    let request_id = match app.update(Action::ProfileConfirmDelete).as_slice() {
        [Command::DeleteProfile { request_id, .. }] => *request_id,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(Action::ReplaceEditor("SELECT 1".into()));
    assert!(app.update(Action::RunActiveSql).is_empty());

    let commands = app.update(Action::ProfileDeleted {
        request_id,
        profile_id,
        active_connection: Some(connection),
    });
    assert!(matches!(
        commands.as_slice(),
        [Command::Disconnect { connection: disconnected }] if *disconnected == connection
    ));
    assert!(app.connection.profile_id.is_none());
    assert_eq!(app.connection.status, ConnectionStatus::Disconnected);
    assert!(app.explorer.nodes.is_empty());
    assert!(app.update(Action::RunActiveSql).is_empty());
}

#[test]
fn running_queries_block_switching_active_profile_saves_and_deletion() {
    let active = sqlite_profile("active");
    let other = sqlite_profile("other");
    let active_id = active.id;
    let mut app = App::new(vec![active, other]);
    app.connection.profile_id = Some(active_id);
    app.connection.status = ConnectionStatus::Connected;
    app.active_console_mut().query_status = QueryStatus::Running;
    app.update(Action::OpenProfileManager);

    app.update(Action::ProfileMove(1));
    assert!(app.update(Action::ProfileConnectSelected).is_empty());
    assert!(
        app.profile_manager
            .as_ref()
            .unwrap()
            .message
            .as_deref()
            .unwrap()
            .contains("running")
    );

    app.update(Action::ProfileMove(-1));
    app.update(Action::ProfileRequestDelete);
    assert_eq!(
        app.profile_manager.as_ref().unwrap().page,
        ProfileManagerPage::List
    );

    app.update(Action::ProfileStartEdit);
    assert!(
        app.update(Action::ProfileSave { connect: false })
            .is_empty()
    );
    assert!(
        app.profile_manager
            .as_ref()
            .unwrap()
            .message
            .as_deref()
            .unwrap()
            .contains("running")
    );
}

#[test]
fn credentials_required_opens_the_matching_profile_at_password() {
    let profile = sqlite_profile("primary");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.connection.pending_profile_id = Some(profile_id);
    app.connection.pending_generation = Some(7);
    app.connection.status = ConnectionStatus::Connecting;

    app.update(Action::CredentialsRequired {
        profile_id,
        generation: 8,
        message: "stale".into(),
    });
    assert!(app.profile_manager.is_none());

    app.update(Action::CredentialsRequired {
        profile_id,
        generation: 7,
        message: "Password required".into(),
    });
    let manager = app.profile_manager.as_ref().unwrap();
    assert_eq!(app.overlay, Some(Overlay::ProfileManager));
    assert_eq!(manager.page, ProfileManagerPage::Form);
    assert_eq!(manager.selected_field, ProfileField::Password);
    assert_eq!(manager.message.as_deref(), Some("Password required"));
}

#[test]
fn credentials_required_does_not_replace_an_in_flight_profile_operation() {
    let profile = sqlite_profile("primary");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.connection.pending_profile_id = Some(profile_id);
    app.connection.pending_generation = Some(4);
    app.connection.status = ConnectionStatus::Connecting;
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileStartEdit);
    let request_id = match app.update(Action::ProfileTest).as_slice() {
        [Command::TestProfile { request_id, .. }] => *request_id,
        commands => panic!("unexpected commands: {commands:?}"),
    };

    app.update(Action::CredentialsRequired {
        profile_id,
        generation: 4,
        message: "Password required".into(),
    });
    let manager = app.profile_manager.as_ref().unwrap();
    assert_eq!(manager.operation, Some(ProfileOperation::Testing));
    assert_eq!(manager.request_generation, request_id);

    app.update(Action::ProfileTestSucceeded {
        request_id,
        server: server(),
    });
    assert_eq!(app.profile_manager.as_ref().unwrap().operation, None);
}
