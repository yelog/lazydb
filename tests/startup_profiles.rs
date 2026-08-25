use std::{
    collections::HashSet,
    sync::{Mutex, OnceLock},
};

use clap::Parser;
use lazydb::{
    app::App, cli::Cli, persistence::profiles::ProfileStore, profile::import_connection_url,
    runtime::load_startup_profiles,
};
use secrecy::ExposeSecret;
use tempfile::TempDir;

fn cli(config: &std::path::Path, extra: &[&str]) -> Cli {
    let mut args = vec!["lazydb", "--config", config.to_str().unwrap()];
    args.extend_from_slice(extra);
    Cli::try_parse_from(args).unwrap()
}

#[test]
fn empty_store_has_no_implicit_profile_and_opens_a_new_form() {
    let temp = TempDir::new().unwrap();
    let startup = load_startup_profiles(&cli(&temp.path().join("connections.toml"), &[])).unwrap();

    assert!(startup.profiles.is_empty());
    assert!(startup.persisted.is_empty());
    assert!(startup.selected.is_none());

    let mut app = App::new(startup.profiles);
    app.update(lazydb::action::Action::OpenProfileManager);
    assert!(app.profile_manager.as_ref().unwrap().draft.is_some());
}

#[test]
fn profile_flag_selects_the_named_persisted_profile() {
    let temp = TempDir::new().unwrap();
    let first = import_connection_url(":memory:", Some("first"))
        .unwrap()
        .profile;
    let second = import_connection_url(":memory:", Some("second"))
        .unwrap()
        .profile;
    let path = temp.path().join("connections.toml");
    ProfileStore::new(path.clone())
        .save(&[first.clone(), second.clone()])
        .unwrap();

    let startup = load_startup_profiles(&cli(&path, &["--profile", "second"])).unwrap();
    assert_eq!(startup.selected, Some(second.id));
    assert_eq!(startup.profiles, [first.clone(), second.clone()]);
    assert_eq!(startup.persisted, HashSet::from([first.id, second.id]));
}

#[test]
fn unknown_profile_name_is_an_actionable_startup_error() {
    let temp = TempDir::new().unwrap();
    let result = load_startup_profiles(&cli(
        &temp.path().join("connections.toml"),
        &["--profile", "missing"],
    ));
    let error = match result {
        Ok(_) => panic!("expected an unknown profile error"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("connection profile not found"));
}

#[test]
fn direct_url_is_ad_hoc_and_not_marked_persisted() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    let startup =
        load_startup_profiles(&cli(&path, &["--url", "sqlite://adhoc.db", "--read-only"])).unwrap();

    assert_eq!(startup.profiles.len(), 1);
    assert!(startup.persisted.is_empty());
    assert_eq!(startup.selected, Some(startup.profiles[0].id));
    assert!(startup.profiles[0].read_only);
    assert!(ProfileStore::new(path).load().unwrap().is_empty());
}

#[test]
fn startup_password_is_bound_only_to_the_selected_profile() {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
    let temp = TempDir::new().unwrap();
    let first = import_connection_url(":memory:", Some("first"))
        .unwrap()
        .profile;
    let second = import_connection_url(":memory:", Some("second"))
        .unwrap()
        .profile;
    let path = temp.path().join("connections.toml");
    ProfileStore::new(path.clone())
        .save(&[first.clone(), second.clone()])
        .unwrap();

    unsafe { std::env::set_var("LAZYDB_PASSWORD", "startup-only") };
    let startup = load_startup_profiles(&cli(&path, &["--profile", "second"])).unwrap();
    unsafe { std::env::remove_var("LAZYDB_PASSWORD") };

    let (profile_id, password) = startup.startup_password.unwrap();
    assert_eq!(profile_id, second.id);
    assert_eq!(password.expose_secret(), "startup-only");
    assert_ne!(profile_id, first.id);

    unsafe { std::env::set_var("LAZYDB_PASSWORD", "must-not-win") };
    let adhoc = load_startup_profiles(&cli(
        &temp.path().join("adhoc.toml"),
        &[
            "--url",
            "postgres://alice:url-secret@db.example.com/app",
            "--profile",
            "named-ad-hoc",
        ],
    ))
    .unwrap();
    unsafe { std::env::remove_var("LAZYDB_PASSWORD") };
    assert_eq!(adhoc.selected, Some(adhoc.profiles[0].id));
    assert_eq!(adhoc.profiles[0].name, "named-ad-hoc");
    assert!(adhoc.startup_password.is_none());
    assert_eq!(adhoc.session_secrets.len(), 1);
}
