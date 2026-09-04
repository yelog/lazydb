use lazydb::{
    action::{Action, Command},
    app::App,
    model::{update::UpdateState, workspace::Overlay},
    update::{InstallationManager, UpdateChannel, UpdateInspection, UpdateStatus},
};

fn inspection(status: UpdateStatus) -> UpdateInspection {
    UpdateInspection {
        manager: InstallationManager::Native,
        channel: UpdateChannel::Stable,
        running_version: env!("CARGO_PKG_VERSION").to_owned(),
        installed_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        target_version: Some("9.9.9".to_owned()),
        status,
        action: None,
        launcher_path: Some("/tmp/lazydb".into()),
    }
}

#[test]
fn opening_update_center_starts_only_one_check() {
    let mut app = App::new(Vec::new());
    assert!(matches!(
        app.update(Action::OpenUpdateCenter).as_slice(),
        [Command::CheckForUpdate {
            request_id: 1,
            automatic: false
        }]
    ));
    assert!(matches!(
        app.update_state,
        UpdateState::Checking {
            request_id: 1,
            automatic: false
        }
    ));
    assert!(app.update(Action::OpenUpdateCenter).is_empty());
    assert!(matches!(
        app.update_state,
        UpdateState::Checking { request_id: 1, .. }
    ));
}

#[test]
fn stale_completion_is_ignored() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenUpdateCenter);
    app.update(Action::UpdateCheckCompleted {
        request_id: 99,
        inspection: inspection(UpdateStatus::Available),
    });
    assert!(matches!(
        app.update_state,
        UpdateState::Checking { request_id: 1, .. }
    ));
}

#[test]
fn later_closes_overlay_and_keeps_available_state() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenUpdateCenter);
    app.update(Action::UpdateCheckCompleted {
        request_id: 1,
        inspection: inspection(UpdateStatus::Available),
    });
    app.update(Action::UpdateOverlayConfirm);
    assert!(app.overlay.is_none());
    assert!(matches!(app.update_state, UpdateState::Available(_)));
}

#[test]
fn primary_confirmation_starts_native_install() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenUpdateCenter);
    app.update(Action::UpdateCheckCompleted {
        request_id: 1,
        inspection: inspection(UpdateStatus::Available),
    });
    app.update(Action::UpdateOverlayToggleFocus);
    assert!(matches!(
        app.update(Action::UpdateOverlayConfirm).as_slice(),
        [Command::InstallUpdate {
            request_id: 2,
            channel: UpdateChannel::Stable
        }]
    ));
    assert!(matches!(
        app.update_state,
        UpdateState::Installing { request_id: 2, .. }
    ));
}

#[test]
fn install_failure_is_retryable() {
    let mut app = App::new(Vec::new());
    app.update(Action::OpenUpdateCenter);
    app.update(Action::UpdateCheckCompleted {
        request_id: 1,
        inspection: inspection(UpdateStatus::Available),
    });
    app.update(Action::UpdateOverlayToggleFocus);
    app.update(Action::UpdateOverlayConfirm);
    app.update(Action::UpdateInstallFailed {
        request_id: 2,
        message: "network down".into(),
    });
    assert!(matches!(app.update_state, UpdateState::Failed { .. }));
    assert!(matches!(app.overlay, Some(Overlay::Update(_))));
}
