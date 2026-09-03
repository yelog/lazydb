use lazydb::model::profile_group::ProfileGroupOverlay;
use lazydb::{
    action::{Action, Command, ProfileOrganizationMutation},
    app::App,
    db::{
        ServerInfo,
        catalog::{
            CatalogCapabilities, CatalogDiscovery, ColumnMetadataCapabilities, DiscoveredDatabase,
            NamespaceModel, ObjectGroup,
        },
    },
    model::{
        explorer::{ExplorerConnectionStatus, ExplorerNodeId, ProfilePlacement},
        profile_manager::{
            CatalogDiscoveryState, CatalogScopeMode, DiscoveryFingerprint, ProfileDraft,
            ProfileField, ProfileManagerPage, ProfileOperation,
        },
        text_input::TextInputEdit,
        workspace::{ConnectionIdentity, ConnectionStatus, Overlay, QueryStatus},
    },
    persistence::secrets::SecretStoreAvailability,
    profile::{
        CatalogScope, CatalogSelection, ConnectionProfile, DatabaseKind, DatabaseScope,
        ProfileAccess, import_connection_url,
    },
};
use uuid::Uuid;

fn sqlite_profile(name: &str) -> ConnectionProfile {
    import_connection_url(":memory:", Some(name))
        .unwrap()
        .profile
}

#[test]
fn profile_group_overlay_creates_renames_assigns_unassigns_and_deletes() {
    let mut profile = sqlite_profile("one");
    profile.id = Uuid::from_u128(1);
    let mut app = App::new(vec![profile.clone()]);
    app.explorer.normalized.selected = Some(ExplorerNodeId::Profile(profile.id));
    app.overlay = Some(Overlay::ProfileGroup(ProfileGroupOverlay::Edit {
        group_id: None,
        name: "  Prod  ".into(),
        error: None,
        busy: false,
    }));
    let commands = app.update(Action::ProfileGroupConfirm);
    let Command::UpdateProfileOrganization {
        mutation: ProfileOrganizationMutation::CreateGroup { id, name },
        ..
    } = commands.into_iter().next().unwrap()
    else {
        panic!()
    };
    assert_eq!(name, "Prod");
    let group = lazydb::profile::ConnectionGroup::new(id, name).unwrap();
    app.update(Action::ProfileOrganizationSaved {
        request_id: 1,
        collection: lazydb::profile::ProfileCollection {
            groups: vec![group.clone()],
            profiles: vec![profile.clone()],
        },
    });
    app.explorer.normalized.selected = Some(ExplorerNodeId::Profile(profile.id));
    app.update(Action::ProfileGroupOpen);
    app.update(Action::ProfileGroupSelect(1));
    let commands = app.update(Action::ProfileGroupConfirm);
    assert!(matches!(
        commands.as_slice(),
        [Command::UpdateProfileOrganization {
            mutation: ProfileOrganizationMutation::AssignProfile {
                profile_id,
                group_id: Some(found),
            },
            ..
        }] if *profile_id == profile.id && *found == group.id
    ));
    app.overlay = Some(Overlay::ProfileGroup(ProfileGroupOverlay::Picker {
        profile_id: profile.id,
        selected: 0,
        busy: false,
    }));
    let commands = app.update(Action::ProfileGroupConfirm);
    assert!(matches!(
        commands.as_slice(),
        [Command::UpdateProfileOrganization {
            mutation: ProfileOrganizationMutation::AssignProfile { group_id: None, .. },
            ..
        }]
    ));
}

#[test]
fn profile_group_overlay_cancel_and_invalid_name_do_not_emit_commands() {
    let mut app = App::new(vec![sqlite_profile("one")]);
    app.overlay = Some(Overlay::ProfileGroup(ProfileGroupOverlay::Edit {
        group_id: None,
        name: "".into(),
        error: None,
        busy: false,
    }));
    assert!(app.update(Action::ProfileGroupConfirm).is_empty());
    assert!(matches!(
        app.overlay,
        Some(Overlay::ProfileGroup(ProfileGroupOverlay::Edit {
            error: Some(_),
            ..
        }))
    ));
    assert!(app.update(Action::ProfileGroupCancel).is_empty());
    assert!(app.overlay.is_none());
}

#[test]
fn profile_group_editor_applies_shared_cursor_and_deletion_edits() {
    let mut app = App::new(Vec::new());
    app.overlay = Some(Overlay::ProfileGroup(ProfileGroupOverlay::Edit {
        group_id: None,
        name: "alpha beta".into(),
        error: None,
        busy: false,
    }));

    app.update(Action::ProfileGroupEdit(TextInputEdit::MoveHome));
    app.update(Action::ProfileGroupEdit(TextInputEdit::MoveRight));
    app.update(Action::ProfileGroupEdit(TextInputEdit::Insert('-')));
    app.update(Action::ProfileGroupEdit(TextInputEdit::MoveEnd));
    app.update(Action::ProfileGroupEdit(TextInputEdit::DeletePreviousWord));

    let Some(Overlay::ProfileGroup(ProfileGroupOverlay::Edit { name, .. })) = &app.overlay else {
        panic!("profile group editor should remain open");
    };
    assert_eq!(name.value(), "a-lpha ");

    app.update(Action::ProfileGroupEdit(TextInputEdit::Clear));
    let Some(Overlay::ProfileGroup(ProfileGroupOverlay::Edit { name, .. })) = &app.overlay else {
        panic!("profile group editor should remain open");
    };
    assert_eq!(name.value(), "");
    assert_eq!(name.cursor(), 0);
}

#[test]
fn profile_group_picker_reaches_create_option_and_emits_create_command() {
    let profile = sqlite_profile("one");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.explorer.normalized.selected = Some(ExplorerNodeId::Profile(profile_id));
    app.update(Action::ProfileGroupOpen);
    app.update(Action::ProfileGroupSelect(1));
    assert!(app.update(Action::ProfileGroupConfirm).is_empty());
    assert!(matches!(
        app.overlay,
        Some(Overlay::ProfileGroup(ProfileGroupOverlay::Edit {
            group_id: None,
            ..
        }))
    ));
    for character in "Prod".chars() {
        app.update(Action::ProfileGroupEdit(TextInputEdit::Insert(character)));
    }
    assert!(matches!(
        app.update(Action::ProfileGroupConfirm).as_slice(),
        [Command::UpdateProfileOrganization {
            mutation: ProfileOrganizationMutation::CreateGroup { name, .. },
            ..
        }] if name == "Prod"
    ));
}

#[test]
fn connection_group_o_and_enter_toggle_expansion() {
    let profile = sqlite_profile("one");
    let group_id = Uuid::from_u128(99);
    let mut app = App::new(vec![profile.clone()]);
    app.update(Action::ProfileOrganizationSaved {
        request_id: 1,
        collection: lazydb::profile::ProfileCollection {
            groups: vec![lazydb::profile::ConnectionGroup {
                id: group_id,
                name: "Production".into(),
            }],
            profiles: vec![profile],
        },
    });
    let group = ExplorerNodeId::ConnectionGroup {
        group_id,
        region: lazydb::model::explorer::ProfileRegion::Primary,
    };
    app.explorer.normalized.selected = Some(group.clone());

    assert!(app.update(Action::ExplorerToggle).is_empty());
    assert!(app.explorer.normalized.expanded.contains(&group));

    assert!(app.update(Action::ExplorerOpenSelected).is_empty());
    assert!(!app.explorer.normalized.expanded.contains(&group));
}

#[test]
fn profile_group_overlay_emits_rename_delete_and_reorder_commands() {
    let mut first = sqlite_profile("first");
    first.id = Uuid::from_u128(10);
    let mut second = sqlite_profile("second");
    second.id = Uuid::from_u128(11);
    let group_id = Uuid::from_u128(12);
    let mut app = App::from_profile_collection(lazydb::profile::ProfileCollection {
        groups: vec![lazydb::profile::ConnectionGroup::new(group_id, "Production").unwrap()],
        profiles: vec![first.clone(), second],
    });
    app.overlay = Some(Overlay::ProfileGroup(ProfileGroupOverlay::Edit {
        group_id: Some(group_id),
        name: "Renamed".into(),
        error: None,
        busy: false,
    }));
    assert!(matches!(
        app.update(Action::ProfileGroupConfirm).as_slice(),
        [Command::UpdateProfileOrganization { mutation: ProfileOrganizationMutation::RenameGroup { group_id: found, name }, .. }] if *found == group_id && name == "Renamed"
    ));
    app.overlay = Some(Overlay::ProfileGroup(ProfileGroupOverlay::DeleteConfirm {
        group_id,
        member_count: 2,
        busy: false,
    }));
    assert!(matches!(
        app.update(Action::ProfileGroupConfirm).as_slice(),
        [Command::UpdateProfileOrganization { mutation: ProfileOrganizationMutation::DeleteGroup { group_id: found }, .. }] if *found == group_id
    ));
    app.explorer.normalized.selected = Some(ExplorerNodeId::Profile(first.id));
    assert!(matches!(
        app.update(Action::ProfileGroupMove(1)).as_slice(),
        [Command::UpdateProfileOrganization { mutation: ProfileOrganizationMutation::MoveProfile { profile_id, direction: lazydb::model::profile_organization::MoveDirection::Down, .. }, .. }] if *profile_id == first.id
    ));
}

#[test]
fn new_saved_profiles_default_to_global_access() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    let draft = app
        .profile_manager
        .as_mut()
        .unwrap()
        .draft
        .as_mut()
        .unwrap();
    draft.name.set("project-local");
    draft.kind = DatabaseKind::Sqlite;
    draft.sqlite_memory = true;

    let commands = app.update(Action::ProfileSave { connect: false });
    let [Command::SaveProfile { submission, .. }] = commands.as_slice() else {
        panic!("expected a save command");
    };

    assert_eq!(submission.profile.access, ProfileAccess::Global);
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

#[test]
fn other_profiles_group_opens_with_toggle_and_enter_actions() {
    let profile = sqlite_profile("other");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.explorer
        .normalized
        .profiles
        .get_mut(&profile_id)
        .unwrap()
        .placement = ProfilePlacement::OtherProject;
    app.explorer.normalized.selected = Some(ExplorerNodeId::Others);

    assert!(app.update(Action::ExplorerToggle).is_empty());
    assert!(
        app.explorer
            .normalized
            .expanded
            .contains(&ExplorerNodeId::Others)
    );
    assert!(app.update(Action::ExplorerOpenSelected).is_empty());
    assert!(
        !app.explorer
            .normalized
            .expanded
            .contains(&ExplorerNodeId::Others)
    );
}

#[test]
fn opening_an_offline_profile_expands_it_after_connection_succeeds() {
    let profile = sqlite_profile("first");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.explorer.normalized.selected = Some(ExplorerNodeId::Profile(profile_id));

    let commands = app.update(Action::ExplorerToggle);
    let generation = match commands.as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };

    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: server(),
        mutation_capabilities: Default::default(),
    });

    assert!(
        app.explorer
            .normalized
            .expanded
            .contains(&ExplorerNodeId::Profile(profile_id))
    );
}

#[test]
fn activating_an_offline_profile_expands_it_after_connection_succeeds() {
    let profile = sqlite_profile("first");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.explorer.normalized.selected = Some(ExplorerNodeId::Profile(profile_id));

    let commands = app.update(Action::ExplorerOpenSelected);
    let generation = match commands.as_slice() {
        [Command::Connect { generation, .. }] => *generation,
        commands => panic!("unexpected commands: {commands:?}"),
    };

    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation,
        server: server(),
        mutation_capabilities: Default::default(),
    });

    assert!(
        app.explorer
            .normalized
            .expanded
            .contains(&ExplorerNodeId::Profile(profile_id))
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
        current_user: Some("postgres".into()),
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
fn pending_scope_discovery_blocks_toggle_and_refresh_but_allows_back() {
    let profile = sqlite_profile("scope");
    let mut app = App::new(vec![profile.clone()]);
    app.update(Action::ProfileStartEdit {
        profile_id: profile.id,
    });
    let request = app.update(Action::ProfileOpenScope);
    let Command::DiscoverProfileCatalog {
        request_id,
        submission,
    } = request.into_iter().next().unwrap()
    else {
        panic!("expected discovery command");
    };
    let before = app
        .profile_manager
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap()
        .catalog_scope
        .clone();

    assert!(
        app.update(Action::ProfileToggleScopeRow("database::memory:".into()))
            .is_empty()
    );
    assert!(app.update(Action::ProfileRefreshScope).is_empty());
    assert_eq!(
        app.profile_manager
            .as_ref()
            .unwrap()
            .scope_discovery_request,
        Some((request_id, submission.discovery_fingerprint))
    );
    assert_eq!(
        app.profile_manager
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .catalog_scope,
        before
    );
    app.update(Action::ProfileScopeBack);
    assert_eq!(
        app.profile_manager.as_ref().unwrap().page,
        ProfileManagerPage::Form
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
            current_user: None,
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
    assert!(message.contains("Connection verified"));
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
    assert_eq!(
        app.notifications.history().next().unwrap().body,
        "Saved successfully"
    );
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
    let commands = deleting.update(Action::ProfileDeleted {
        request_id,
        profile_id,
        active_connection: None,
    });
    assert!(matches!(
        commands.as_slice(),
        [Command::Disconnect { connection }, Command::PersistWorkspace(_)]
            if *connection == ConnectionIdentity { profile_id, generation: 1 }
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
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation: 3,
        server: server(),
        mutation_capabilities: Default::default(),
    });
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
fn snapshot_without_an_installed_profile_workspace_stays_empty() {
    let profile = sqlite_profile("connected-without-workspace");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.connection.profile_id = Some(profile_id);
    app.connection.status = ConnectionStatus::Connected;

    let snapshot = app.workspace_snapshot();

    assert_eq!(app.active_workspace_profile, None);
    assert!(app.tabs.is_empty());
    assert!(snapshot.profiles.is_empty());
    assert!(snapshot.consoles.is_empty());
    assert!(snapshot.sql.is_empty());
}

#[test]
fn query_started_while_save_is_in_flight_preserves_the_active_connection() {
    let profile = sqlite_profile("active");
    let profile_id = profile.id;
    let mut app = App::new(vec![profile]);
    app.update(Action::ConnectionSucceeded {
        profile_id,
        generation: 3,
        server: server(),
        mutation_capabilities: Default::default(),
    });
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
        [Command::Disconnect { connection: disconnected }, Command::PersistWorkspace(_)]
            if *disconnected == connection
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
    app.update(Action::ConnectionSucceeded {
        profile_id: active_id,
        generation: 1,
        server: server(),
        mutation_capabilities: Default::default(),
    });
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
