use lazydb::{
    action::Action,
    app::App,
    commands::CommandId,
    model::{omni::OmniItemId, workspace::Overlay},
};

#[test]
fn idle_profile_draft_is_suspended_and_restored_by_session_id() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    app.profile_manager
        .as_mut()
        .unwrap()
        .draft
        .as_mut()
        .unwrap()
        .name
        .set("work profile");

    app.update(Action::OpenOmni);
    let dashboard = OmniItemId::Command(CommandId::OpenDashboard);
    app.omni.as_mut().unwrap().selected = Some(dashboard);
    app.update(Action::OmniConfirm);

    assert!(!matches!(app.overlay, Some(Overlay::ProfileManager)));
    assert!(app.omni.is_none());

    app.update(Action::OpenOmni);
    let suspended = app
        .omni
        .as_ref()
        .unwrap()
        .items
        .iter()
        .find_map(|item| match item.action {
            lazydb::model::omni::OmniItemAction::ResumeInteraction(id) => Some(id),
            _ => None,
        })
        .unwrap();
    assert!(
        app.omni
            .as_ref()
            .unwrap()
            .items
            .iter()
            .any(|item| { item.id == OmniItemId::SuspendedSession(suspended) }),
    );
    app.omni.as_mut().unwrap().selected = Some(OmniItemId::SuspendedSession(suspended));
    app.update(Action::OmniConfirm);

    assert!(matches!(app.overlay, Some(Overlay::ProfileManager)));
    assert_eq!(
        app.profile_manager
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .name
            .value(),
        "work profile"
    );
}

#[test]
fn busy_profile_operation_is_not_suspended_or_abandoned() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    app.profile_manager.as_mut().unwrap().operation =
        Some(lazydb::model::profile_manager::ProfileOperation::Testing);
    app.update(Action::OpenOmni);
    app.omni.as_mut().unwrap().selected = Some(OmniItemId::Command(CommandId::OpenDashboard));
    app.update(Action::OmniConfirm);

    assert!(app.omni.is_some());
    assert!(app.omni.as_ref().unwrap().status.is_some());
}
