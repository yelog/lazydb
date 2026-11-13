use lazydb::{
    action::{Action, Command},
    app::App,
    model::{
        profile_manager::{DiscoveryFingerprint, ProfileManagerPage, ProfileOperation},
        workspace::Overlay,
    },
};

#[test]
fn closing_a_connection_test_cancels_work_and_ignores_its_late_result() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    let manager = app.profile_manager.as_mut().unwrap();
    manager.operation = Some(ProfileOperation::Testing);
    let commands = app.update(Action::CloseProfileManager);
    assert!(matches!(
        commands.as_slice(),
        [Command::CancelProfileConnections]
    ));
    assert!(app.profile_manager.is_none());
    assert!(app.overlay.is_none());
    assert!(
        app.update(Action::ProfileTestFailed {
            request_id: 1,
            message: "Late SSH failure".to_owned(),
        })
        .is_empty()
    );
    assert!(app.profile_manager.is_none());
}

#[test]
fn closing_a_profile_during_save_remains_blocked() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    app.profile_manager.as_mut().unwrap().operation = Some(ProfileOperation::Saving);
    assert!(app.update(Action::CloseProfileManager).is_empty());
    assert!(app.profile_manager.is_some());
    assert_eq!(app.overlay, Some(Overlay::ProfileManager));
}

#[test]
fn backing_out_of_discovery_cancels_its_pending_connection() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    let manager = app.profile_manager.as_mut().unwrap();
    manager.page = ProfileManagerPage::Scope;
    let profile = lazydb::profile::import_connection_url(
        "postgresql://user@localhost/lazydb",
        Some("SSH cancellation"),
    )
    .unwrap()
    .profile;
    let fingerprint = DiscoveryFingerprint::for_profile(&profile, false, 0);
    manager.begin_scope_discovery(1, fingerprint);
    let commands = app.update(Action::ProfileScopeBack);
    assert!(matches!(
        commands.as_slice(),
        [Command::CancelProfileConnections]
    ));
    let manager = app.profile_manager.as_ref().unwrap();
    assert_eq!(manager.page, ProfileManagerPage::Form);
    assert!(!manager.scope_discovery_loading());
}
