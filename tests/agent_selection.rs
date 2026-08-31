use std::fs;

use lazydb::{
    agent::{
        context::AgentProjectContext,
        selection::{AgentErrorCode, SelectionReason, select_profile},
    },
    profile::{ConnectionProfile, ProfileAccess, import_connection_url},
};
use tempfile::TempDir;
use uuid::Uuid;

fn profile(name: &str) -> ConnectionProfile {
    import_connection_url(":memory:", Some(name))
        .unwrap()
        .profile
}

fn context() -> (TempDir, AgentProjectContext) {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    let context = AgentProjectContext::resolve(Some(temp.path())).unwrap();
    (temp, context)
}

#[test]
fn selects_explicit_uuid_and_name_only_from_visible_profiles() {
    let (_temp, context) = context();
    let mut current = profile("current");
    current.access = ProfileAccess::Projects {
        roots: vec![context.root().to_owned()],
    };
    let global = profile("global");
    let mut other = profile("other");
    other.access = ProfileAccess::Projects {
        roots: vec![context.root().join("sibling")],
    };
    let other_id = other.id;
    let profiles = [current, global, other];
    let visible = context.visible_profiles(&profiles);

    let selected = select_profile(&visible, Some(&visible[0].profile.id.to_string())).unwrap();
    assert_eq!(selected.profile.name, "current");
    assert_eq!(selected.reason, SelectionReason::ExplicitUuid);

    let selected = select_profile(&visible, Some("global")).unwrap();
    assert_eq!(selected.profile.name, "global");
    assert_eq!(selected.reason, SelectionReason::ExplicitName);

    let error = select_profile(&visible, Some(&other_id.to_string())).unwrap_err();
    assert_eq!(error.code, AgentErrorCode::ConnectionNotFound);
}

#[test]
fn implicit_selection_is_project_first_then_sole_global() {
    let (_temp, context) = context();
    let mut current = profile("current");
    current.access = ProfileAccess::Projects {
        roots: vec![context.root().to_owned()],
    };
    let global = profile("global");
    let profiles = [current, global.clone()];
    let visible = context.visible_profiles(&profiles);

    let selected = select_profile(&visible, None).unwrap();
    assert_eq!(selected.profile.name, "current");
    assert_eq!(selected.reason, SelectionReason::SoleProject);

    let globals = [global];
    let visible = context.visible_profiles(&globals);
    let selected = select_profile(&visible, None).unwrap();
    assert_eq!(selected.profile.name, "global");
    assert_eq!(selected.reason, SelectionReason::SoleGlobal);
}

#[test]
fn multiple_candidates_and_duplicate_names_fail_closed() {
    let (_temp, context) = context();
    let mut first = profile("same");
    first.access = ProfileAccess::Projects {
        roots: vec![context.root().to_owned()],
    };
    let mut second = profile("same");
    second.access = ProfileAccess::Projects {
        roots: vec![context.root().to_owned()],
    };
    let profiles = [first, second];
    let visible = context.visible_profiles(&profiles);

    let error = select_profile(&visible, None).unwrap_err();
    assert_eq!(error.code, AgentErrorCode::ConnectionAmbiguous);
    let error = select_profile(&visible, Some("same")).unwrap_err();
    assert_eq!(error.code, AgentErrorCode::ConnectionAmbiguous);

    let visible = context.visible_profiles(&[]);
    let error = select_profile(&visible, None).unwrap_err();
    assert_eq!(error.code, AgentErrorCode::NoVisibleConnections);
}

#[test]
fn uuid_parser_does_not_turn_unknown_names_into_uuid_errors() {
    let (_temp, context) = context();
    let profile = profile("not-a-uuid");
    let profiles = [profile];
    let visible = context.visible_profiles(&profiles);
    let error = select_profile(&visible, Some(&Uuid::new_v4().to_string())).unwrap_err();
    assert_eq!(error.code, AgentErrorCode::ConnectionNotFound);
}
