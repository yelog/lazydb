use std::fs;

use lazydb::{
    agent::context::{AgentProfileScope, AgentProjectContext},
    profile::{ProfileAccess, import_connection_url},
};
use tempfile::TempDir;

fn profile(name: &str) -> lazydb::profile::ConnectionProfile {
    import_connection_url(":memory:", Some(name))
        .unwrap()
        .profile
}

#[test]
fn resolves_a_git_file_from_a_nested_directory() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join(".git"), "gitdir: /tmp/worktree").unwrap();
    let nested = temp.path().join("src").join("nested");
    fs::create_dir_all(&nested).unwrap();

    let context = AgentProjectContext::resolve(Some(&nested)).unwrap();

    assert_eq!(context.root(), temp.path().canonicalize().unwrap());
}

#[test]
fn exposes_current_project_and_global_profiles_but_not_other_projects() {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    let current_root = temp.path().canonicalize().unwrap();
    let other_root = temp.path().join("other");
    fs::create_dir_all(&other_root).unwrap();

    let mut current = profile("current");
    current.access = ProfileAccess::Projects {
        roots: vec![current_root.clone()],
    };
    let global = profile("global");
    let mut other = profile("other");
    other.access = ProfileAccess::Projects {
        roots: vec![other_root],
    };
    let mut multi = profile("multi-root");
    multi.access = ProfileAccess::Projects {
        roots: vec![current_root],
    };

    let context = AgentProjectContext::resolve(Some(temp.path())).unwrap();
    let profiles = [current, global, other, multi];
    let visible = context.visible_profiles(&profiles);

    assert_eq!(
        visible
            .iter()
            .map(|entry| entry.profile.name.as_str())
            .collect::<Vec<_>>(),
        ["current", "global", "multi-root"]
    );
    assert_eq!(visible[0].scope, AgentProfileScope::CurrentProject);
    assert_eq!(visible[1].scope, AgentProfileScope::Global);
    assert_eq!(visible[2].scope, AgentProfileScope::CurrentProject);
}
