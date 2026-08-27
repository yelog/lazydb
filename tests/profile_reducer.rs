use lazydb::{
    action::{Action, Command},
    app::App,
    db::{
        ServerInfo,
        catalog::{
            CatalogCapabilities, CatalogDiscovery, ColumnMetadataCapabilities, DiscoveredDatabase,
            NamespaceModel, ObjectGroup,
        },
    },
    model::{
        explorer::{ExplorerConnectionStatus, ExplorerNodeId},
        profile_manager::{
            CatalogDiscoveryState, CatalogScopeMode, DiscoveryFingerprint, ProfileDraft,
            ProfileField, ProfileManagerPage, ProfileOperation,
        },
        workspace::{ConnectionIdentity, ConnectionStatus, Overlay, QueryStatus},
    },
    persistence::secrets::SecretStoreAvailability,
    profile::{
        CatalogScope, CatalogSelection, ConnectionProfile, DatabaseKind, DatabaseScope,
        import_connection_url,
    },
};

fn sqlite_profile(name: &str) -> ConnectionProfile {
    import_connection_url(":memory:", Some(name))
        .unwrap()
        .profile
}

#[test]
fn profile_root_actions_target_exact_uuid_and_activation_never_disconnects() {
    let first = sqlite_profile("first");
    let second = sqlite_profile("second");
    let first_id = first.id;
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);

    app.update(Action::ProfileStartEdit {
        profile_id: second_id,
    });
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

    app.explorer.normalized.selected = Some(ExplorerNodeId::Profile(first_id));
    assert!(matches!(
        app.update(Action::ExplorerToggle).as_slice(),
        [Command::Connect { profile_id, .. }] if *profile_id == first_id
    ));
    app.explorer
        .normalized
        .profiles
        .get_mut(&first_id)
        .unwrap()
        .status = ExplorerConnectionStatus::Online;
    assert!(app.update(Action::ExplorerToggle).is_empty());
    assert!(
        app.explorer
            .normalized
            .expanded
            .contains(&ExplorerNodeId::Profile(first_id))
    );
    assert!(app.update(Action::ExplorerToggle).is_empty());
    assert!(
        !app.explorer
            .normalized
            .expanded
            .contains(&ExplorerNodeId::Profile(first_id))
    );
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

fn capabilities() -> CatalogCapabilities {
    CatalogCapabilities {
        namespace_model: NamespaceModel::DatabaseAndSchema,
        top_level_groups: vec![
            ObjectGroup::Tables,
            ObjectGroup::Views,
            ObjectGroup::Functions,
            ObjectGroup::Procedures,
        ],
        column_metadata: ColumnMetadataCapabilities::default(),
        supports_lazy_children: false,
    }
}

fn discovery(database: &str, schemas: &[&str]) -> CatalogDiscovery {
    CatalogDiscovery {
        databases: vec![DiscoveredDatabase {
            name: database.into(),
            schemas: schemas.iter().map(|schema| (*schema).into()).collect(),
        }],
        warnings: Vec::new(),
    }
}

#[test]
fn opening_visible_objects_starts_discovery_and_preserves_saved_scope() {
    let profile = sqlite_profile("scope");
    let saved_scope = profile.catalog_scope.clone();
    let mut app = App::new(vec![profile.clone()]);
    app.update(Action::ProfileStartEdit {
        profile_id: profile.id,
    });

    let commands = app.update(Action::ProfileOpenScope);
    let [
        Command::DiscoverProfileCatalog {
            request_id,
            submission,
        },
    ] = commands.as_slice()
    else {
        panic!("unexpected commands: {commands:?}")
    };
    let manager = app.profile_manager.as_ref().unwrap();
    assert_eq!(manager.page, ProfileManagerPage::Scope);
    assert!(manager.scope_discovery_loading());
    assert_eq!(manager.draft.as_ref().unwrap().catalog_scope, saved_scope);
    assert_eq!(
        manager.scope_discovery_request,
        Some((*request_id, submission.discovery_fingerprint))
    );
}

#[test]
fn scope_discovery_response_preserves_scope_and_refresh_starts_new_request() {
    let profile = sqlite_profile("scope");
    let saved_scope = profile.catalog_scope.clone();
    let mut app = App::new(vec![profile.clone()]);
    app.update(Action::ProfileStartEdit {
        profile_id: profile.id,
    });
    let (request_id, fingerprint) = match app.update(Action::ProfileOpenScope).as_slice() {
        [
            Command::DiscoverProfileCatalog {
                request_id,
                submission,
            },
        ] => (*request_id, submission.discovery_fingerprint),
        commands => panic!("unexpected commands: {commands:?}"),
    };

    app.update(Action::ProfileCatalogDiscoverySucceeded {
        request_id,
        fingerprint,
        server: ServerInfo {
            kind: DatabaseKind::Sqlite,
            version: "3.50".into(),
            database: ":memory:".into(),
        },
        capabilities: capabilities(),
        discovery: discovery(":memory:", &["main", "temp"]),
    });
    let manager = app.profile_manager.as_ref().unwrap();
    assert!(!manager.scope_discovery_loading());
    assert_eq!(manager.draft.as_ref().unwrap().catalog_scope, saved_scope);
    assert!(manager.scope_row("database::memory::schema:temp").is_some());

    assert!(matches!(
        app.update(Action::ProfileRefreshScope).as_slice(),
        [Command::DiscoverProfileCatalog { request_id: next, .. }] if *next > request_id
    ));
}

fn profile_test_succeeded(
    request_id: u64,
    fingerprint: DiscoveryFingerprint,
    discovery: Result<CatalogDiscovery, String>,
) -> Action {
    Action::ProfileTestSucceeded {
        request_id,
        fingerprint,
        server: server(),
        capabilities: capabilities(),
        discovery,
    }
}

#[test]
fn opening_manager_always_uses_a_new_form() {
    let mut populated = App::new(vec![sqlite_profile("primary")]);
    assert!(populated.update(Action::OpenProfileManager).is_empty());
    assert_eq!(populated.overlay, Some(Overlay::ProfileManager));
    assert_eq!(
        populated.profile_manager.as_ref().unwrap().page,
        ProfileManagerPage::Form
    );
    assert_eq!(
        populated
            .profile_manager
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .kind,
        DatabaseKind::Postgres
    );

    let mut empty = App::new(Vec::new());
    empty.update(Action::OpenProfileManager);
    let manager = empty.profile_manager.as_ref().unwrap();
    assert_eq!(manager.page, ProfileManagerPage::Form);
    assert_eq!(manager.draft.as_ref().unwrap().kind, DatabaseKind::Postgres);
}

#[test]
fn profile_test_form_open_and_edit_are_pure() {
    let profile = sqlite_profile("primary");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);

    assert!(
        app.update(Action::ProfileStartEdit { profile_id })
            .is_empty()
    );
    assert_eq!(
        app.profile_manager.as_ref().unwrap().page,
        ProfileManagerPage::Form
    );
}

#[test]
fn uuid_targeted_new_edit_cancel_and_delete_confirmation_are_pure() {
    let first = sqlite_profile("first");
    let second = sqlite_profile("second");
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);
    app.update(Action::ProfileStartEdit {
        profile_id: second_id,
    });
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
    assert!(app.profile_manager.is_none());
    app.update(Action::ProfileStartNew);
    assert_eq!(
        app.profile_manager.as_ref().unwrap().page,
        ProfileManagerPage::Form
    );
    app.update(Action::CloseProfileManager);

    app.update(Action::ProfileRequestDelete {
        profile_id: second_id,
    });
    assert_eq!(
        app.profile_manager.as_ref().unwrap().page,
        ProfileManagerPage::ConfirmDelete
    );
    app.update(Action::ProfileCancelDelete);
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
        ProfileField::VisibleObjects
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
    app.update(Action::ProfileToggleField(ProfileField::ReadOnly));
    app.update(Action::ProfileFocusField(ProfileField::PasswordStorage));
    app.update(Action::SystemCredentialAvailability(
        SecretStoreAvailability::Available,
    ));
    app.update(Action::ProfileCycle(1));

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
    assert_eq!(
        draft.password_storage,
        lazydb::profile::PasswordStorageChoice::System
    );

    app.update(Action::ProfileCycle(-1));
    assert_eq!(
        app.profile_manager
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .password_storage,
        lazydb::profile::PasswordStorageChoice::LocalEncrypted
    );
}

#[test]
fn unavailable_system_store_hides_option_but_available_store_shows_it() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileFocusField(ProfileField::PasswordStorage));
    assert_eq!(
        app.profile_manager
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .password_storage_choices(),
        &[lazydb::profile::PasswordStorageChoice::LocalEncrypted]
    );

    app.update(Action::SystemCredentialAvailability(
        SecretStoreAvailability::Available,
    ));
    assert_eq!(
        app.profile_manager
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .password_storage_choices()
            .len(),
        2
    );
}

#[test]
fn direct_driver_selection_reuses_driver_migration_rules() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    app.update(Action::ProfileSelectDriver(DatabaseKind::MySql));

    let draft = app
        .profile_manager
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap();
    assert_eq!(draft.kind, DatabaseKind::MySql);
    assert_eq!(draft.port.value(), "3306");

    app.update(Action::ProfileSelectDriver(DatabaseKind::Sqlite));
    let draft = app
        .profile_manager
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap();
    assert_eq!(draft.kind, DatabaseKind::Sqlite);
    assert_eq!(draft.ssl_mode, lazydb::profile::SslMode::Disable);

    app.profile_manager.as_mut().unwrap().operation = Some(ProfileOperation::Testing);
    app.update(Action::ProfileSelectDriver(DatabaseKind::Postgres));
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
}

#[test]
fn test_commits_pending_url_atomically_before_validation() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    let manager = app.profile_manager.as_mut().unwrap();
    manager.draft.as_mut().unwrap().name.set("url-profile");
    manager.focus_field(ProfileField::Url);
    let draft = manager.draft.as_mut().unwrap();
    draft.move_home(ProfileField::Url);
    while draft.url_cursor() < draft.url_display().chars().count() {
        draft.delete(ProfileField::Url);
    }
    draft.paste(
        ProfileField::Url,
        "postgresql://alice:secret@db.example:5440/app?sslmode=require",
    );

    let commands = app.update(Action::ProfileTest);
    let [Command::TestProfile { submission, .. }] = commands.as_slice() else {
        panic!("unexpected commands: {commands:?}");
    };
    assert_eq!(submission.profile.host.as_deref(), Some("db.example"));
    assert_eq!(submission.profile.port, Some(5440));
    assert!(!format!("{submission:?}").contains("secret"));
    assert!(
        !app.profile_manager
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .url_display()
            .contains("secret")
    );
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
    let (request_id, fingerprint) = match commands.as_slice() {
        [
            Command::TestProfile {
                request_id,
                submission,
            },
        ] => (*request_id, submission.discovery_fingerprint),
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

    app.update(profile_test_succeeded(
        request_id,
        fingerprint,
        Ok(discovery("lazydb", &["public"])),
    ));
    let manager = app.profile_manager.as_ref().unwrap();
    assert_eq!(manager.operation, None);
    assert!(manager.message.as_deref().unwrap().contains("16.4"));
    assert!(matches!(
        manager.draft.as_ref().unwrap().catalog_discovery,
        CatalogDiscoveryState::Fresh(_)
    ));
}

#[test]
fn profile_test_discovery_failure_is_success_with_a_warning_and_preserves_scope() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    valid_new_profile(&mut app, "primary");
    let scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: "lazydb".into(),
            schemas: CatalogSelection::Selected(vec!["public".into(), "audit".into()]),
        }]),
    };
    app.profile_manager
        .as_mut()
        .unwrap()
        .draft
        .as_mut()
        .unwrap()
        .catalog_scope = scope.clone();
    app.profile_manager
        .as_mut()
        .unwrap()
        .draft
        .as_mut()
        .unwrap()
        .catalog_scope_mode = CatalogScopeMode::Explicit;
    let (request_id, fingerprint) = match app.update(Action::ProfileTest).as_slice() {
        [
            Command::TestProfile {
                request_id,
                submission,
            },
        ] => (*request_id, submission.discovery_fingerprint),
        commands => panic!("unexpected commands: {commands:?}"),
    };

    app.update(profile_test_succeeded(
        request_id,
        fingerprint,
        Err("catalog permission denied".into()),
    ));

    let manager = app.profile_manager.as_ref().unwrap();
    assert_eq!(manager.operation, None);
    let message = manager.message.as_deref().unwrap();
    assert!(message.contains("Connection succeeded"));
    assert!(message.contains("catalog permission denied"));
    let draft = manager.draft.as_ref().unwrap();
    assert_eq!(draft.catalog_scope, scope);
    let CatalogDiscoveryState::Fresh(snapshot) = &draft.catalog_discovery else {
        panic!("probe success must retain the discovery warning")
    };
    assert_eq!(snapshot.fingerprint, fingerprint);
    assert!(matches!(
        &snapshot.discovery,
        Err(warning) if warning == "catalog permission denied"
    ));
}

#[test]
fn profile_test_stale_generation_cannot_replace_newer_discovery() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    valid_new_profile(&mut app, "primary");
    let (first_request, first_fingerprint) = match app.update(Action::ProfileTest).as_slice() {
        [
            Command::TestProfile {
                request_id,
                submission,
            },
        ] => (*request_id, submission.discovery_fingerprint),
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(profile_test_succeeded(
        first_request,
        first_fingerprint,
        Ok(discovery("first", &["public"])),
    ));

    let (second_request, second_fingerprint) = match app.update(Action::ProfileTest).as_slice() {
        [
            Command::TestProfile {
                request_id,
                submission,
            },
        ] => (*request_id, submission.discovery_fingerprint),
        commands => panic!("unexpected commands: {commands:?}"),
    };
    let before_stale = app
        .profile_manager
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap()
        .catalog_discovery
        .clone();
    app.update(profile_test_succeeded(
        first_request,
        first_fingerprint,
        Ok(discovery("stale", &["ignored"])),
    ));
    let manager = app.profile_manager.as_ref().unwrap();
    assert_eq!(manager.operation, Some(ProfileOperation::Testing));
    assert_eq!(
        manager.draft.as_ref().unwrap().catalog_discovery,
        before_stale
    );

    app.update(profile_test_succeeded(
        second_request,
        second_fingerprint,
        Ok(discovery("current", &["public"])),
    ));
    let current = app
        .profile_manager
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap()
        .catalog_discovery
        .clone();
    app.update(profile_test_succeeded(
        first_request,
        first_fingerprint,
        Ok(discovery("late", &["ignored"])),
    ));
    assert_eq!(
        app.profile_manager
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .catalog_discovery,
        current
    );
}

#[test]
fn profile_test_connection_edits_mark_discovery_stale_and_preserve_scope() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    valid_new_profile(&mut app, "primary");
    let scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![DatabaseScope {
            name: "lazydb".into(),
            schemas: CatalogSelection::Selected(vec!["public".into(), "audit".into()]),
        }]),
    };
    app.profile_manager
        .as_mut()
        .unwrap()
        .draft
        .as_mut()
        .unwrap()
        .catalog_scope = scope.clone();
    app.profile_manager
        .as_mut()
        .unwrap()
        .draft
        .as_mut()
        .unwrap()
        .catalog_scope_mode = CatalogScopeMode::Explicit;
    let (request_id, fingerprint) = match app.update(Action::ProfileTest).as_slice() {
        [
            Command::TestProfile {
                request_id,
                submission,
            },
        ] => (*request_id, submission.discovery_fingerprint),
        commands => panic!("unexpected commands: {commands:?}"),
    };
    app.update(profile_test_succeeded(
        request_id,
        fingerprint,
        Ok(discovery("lazydb", &["public", "audit"])),
    ));

    app.update(Action::ProfileFocusField(ProfileField::Host));
    app.update(Action::ProfileInsert('2'.into()));

    let draft = app
        .profile_manager
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap();
    assert!(matches!(
        draft.catalog_discovery,
        CatalogDiscoveryState::Stale(_)
    ));
    assert!(draft.discovery_fingerprint.is_none());
    assert_eq!(draft.catalog_scope, scope);
}

#[test]
fn profile_test_fingerprint_never_depends_on_password_contents() {
    let mut first = ProfileDraft::new(DatabaseKind::Postgres);
    first.name.set("primary");
    first.database.set("lazydb");
    first.set_password("first-password");
    let first_fingerprint = first.validate(&[]).unwrap().discovery_fingerprint;

    let mut same_revision = ProfileDraft::new(DatabaseKind::Postgres);
    same_revision.name.set("other-name");
    same_revision.database.set("lazydb");
    same_revision.set_password("different-password");
    assert_eq!(
        same_revision.validate(&[]).unwrap().discovery_fingerprint,
        first_fingerprint
    );

    first.set_password("replacement-password");
    assert_ne!(
        first.validate(&[]).unwrap().discovery_fingerprint,
        first_fingerprint
    );
    let debug = format!("{first:?}");
    assert!(!debug.contains("first-password"));
    assert!(!debug.contains("replacement-password"));
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
    let second_id = second.id;
    let expected_ids = [first.id, second.id];
    let mut app = App::new(vec![first, second]);
    app.update(Action::ProfileStartEdit {
        profile_id: second_id,
    });
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
        change: lazydb::model::profile_manager::ProfileChange {
            connection_settings_changed: false,
            catalog_scope_changed: false,
            display_only_changed: false,
            credentials_changed: false,
        },
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
    assert!(app.profile_manager.is_none());
}

#[test]
fn delete_success_removes_the_profile_and_clamps_selection() {
    let first = sqlite_profile("first");
    let second = sqlite_profile("second");
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);
    app.update(Action::ProfileRequestDelete {
        profile_id: second_id,
    });
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
    assert!(app.profile_manager.is_none());
}

#[test]
fn deleting_or_saving_a_pending_profile_clears_its_connection_state() {
    let profile = sqlite_profile("pending");
    let profile_id = profile.id;
    let mut deleting = App::new(vec![profile.clone()]);
    deleting.connection.pending_profile_id = Some(profile_id);
    deleting.connection.pending_generation = Some(1);
    deleting.connection.status = ConnectionStatus::Connecting;
    deleting.update(Action::ProfileRequestDelete { profile_id });
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
    saving.update(Action::ProfileStartEdit { profile_id });
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
                change: lazydb::model::profile_manager::ProfileChange { connection_settings_changed: false, catalog_scope_changed: false, display_only_changed: false, credentials_changed: false },
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
    app.update(Action::ProfileStartEdit { profile_id });
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
        change: lazydb::model::profile_manager::ProfileChange {
            connection_settings_changed: true,
            catalog_scope_changed: false,
            display_only_changed: false,
            credentials_changed: false,
        },
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
fn active_scope_only_save_keeps_connection_clears_completion_and_reloads_catalog() {
    let mut profile = sqlite_profile("active");
    let profile_id = profile.id;
    profile.catalog_scope =
        CatalogScope::for_profile(DatabaseKind::Sqlite, ":memory:", Some("main"));
    let mut app = App::new(vec![profile.clone()]);
    app.connection.profile_id = Some(profile_id);
    app.connection.generation = 3;
    app.connection.status = ConnectionStatus::Connected;
    app.active_console_mut().completion = Some(Default::default());
    app.update(Action::ProfileStartEdit { profile_id });
    let request_id = match app
        .update(Action::ProfileSave { connect: false })
        .as_slice()
    {
        [Command::SaveProfile { request_id, .. }] => *request_id,
        commands => panic!("unexpected commands: {commands:?}"),
    };
    let mut saved = app
        .profile_manager
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap()
        .validate(&app.profiles)
        .unwrap()
        .profile;
    saved.catalog_scope = CatalogScope::for_profile(DatabaseKind::Sqlite, ":memory:", None);
    let old_epoch = app
        .explorer
        .normalized
        .profiles
        .get(&profile_id)
        .unwrap()
        .catalog_epoch;
    let commands = app.update(Action::ProfileSaved {
        request_id,
        profile: saved,
        warning: None,
        change: lazydb::model::profile_manager::ProfileChange {
            connection_settings_changed: false,
            catalog_scope_changed: true,
            display_only_changed: false,
            credentials_changed: false,
        },
        connect: false,
    });
    assert!(matches!(commands.as_slice(), [Command::LoadCatalogPage(_)]));
    assert_eq!(app.connection.profile_id, Some(profile_id));
    assert_eq!(app.connection.status, ConnectionStatus::Connected);
    assert!(app.active_console().completion.is_none());
    assert!(
        app.explorer
            .normalized
            .profiles
            .get(&profile_id)
            .unwrap()
            .catalog_epoch
            > old_epoch
    );
}

#[test]
fn query_started_while_save_is_in_flight_preserves_the_active_connection() {
    let profile = sqlite_profile("active");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.connection.profile_id = Some(profile_id);
    app.connection.generation = 3;
    app.connection.status = ConnectionStatus::Connected;
    app.update(Action::ProfileStartEdit { profile_id });
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
            change: lazydb::model::profile_manager::ProfileChange {
                connection_settings_changed: true,
                catalog_scope_changed: false,
                display_only_changed: false,
                credentials_changed: false,
            },
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
    app.update(Action::ProfileRequestDelete { profile_id });
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
    let other_id = other.id;
    let mut app = App::new(vec![active, other]);
    app.connection.profile_id = Some(active_id);
    app.connection.status = ConnectionStatus::Connected;
    app.active_console_mut().query_status = QueryStatus::Running;
    assert!(
        app.update(Action::RequestProfileConnect {
            profile_id: other_id,
        })
        .is_empty()
    );

    app.update(Action::ProfileRequestDelete {
        profile_id: active_id,
    });
    assert_eq!(
        app.profile_manager.as_ref().unwrap().page,
        ProfileManagerPage::ConfirmDelete
    );
    assert!(app.profile_manager.as_ref().unwrap().message.is_some());

    app.update(Action::ProfileStartEdit {
        profile_id: active_id,
    });
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
    app.update(Action::ProfileStartEdit { profile_id });
    let (request_id, fingerprint) = match app.update(Action::ProfileTest).as_slice() {
        [
            Command::TestProfile {
                request_id,
                submission,
            },
        ] => (*request_id, submission.discovery_fingerprint),
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

    app.update(profile_test_succeeded(
        request_id,
        fingerprint,
        Ok(discovery("lazydb", &["public"])),
    ));
    assert_eq!(app.profile_manager.as_ref().unwrap().operation, None);
}

#[test]
fn starting_another_profile_form_during_operation_preserves_request_generation() {
    let first = sqlite_profile("first");
    let second = sqlite_profile("second");
    let first_id = first.id;
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);
    app.update(Action::ProfileStartEdit {
        profile_id: first_id,
    });
    let request_id = match app.update(Action::ProfileTest).as_slice() {
        [Command::TestProfile { request_id, .. }] => *request_id,
        commands => panic!("unexpected commands: {commands:?}"),
    };

    app.update(Action::ProfileStartEdit {
        profile_id: second_id,
    });
    let manager = app.profile_manager.as_ref().unwrap();
    assert_eq!(manager.request_generation, request_id);
    assert_eq!(manager.operation, Some(ProfileOperation::Testing));
    app.update(Action::ProfileTestFailed {
        request_id,
        message: "late".into(),
    });
    assert_eq!(
        app.profile_manager.as_ref().unwrap().message.as_deref(),
        Some("late")
    );
}

#[test]
fn invalidating_active_connection_cancels_pending_switch() {
    let first = sqlite_profile("first");
    let second = sqlite_profile("second");
    let first_id = first.id;
    let second_id = second.id;
    let mut app = App::new(vec![first, second]);
    app.connection.profile_id = Some(first_id);
    app.connection.generation = 1;
    app.connection.status = ConnectionStatus::Connected;
    app.update(Action::RequestProfileConnect {
        profile_id: second_id,
    });

    app.update(Action::ConnectionInvalidated {
        connection: ConnectionIdentity {
            profile_id: first_id,
            generation: 1,
        },
        message: "connection lost".into(),
    });

    assert!(app.connection.pending_profile_id.is_none());
    assert_eq!(app.connection.status, ConnectionStatus::Failed);
    assert_eq!(
        app.explorer.normalized.profiles[&first_id].status,
        ExplorerConnectionStatus::Failed
    );
    assert_eq!(
        app.explorer.normalized.profiles[&second_id].status,
        ExplorerConnectionStatus::Failed
    );
    assert_eq!(
        app.explorer.normalized.profiles[&second_id]
            .last_error
            .as_deref(),
        Some("connection lost")
    );
}
