use std::fs;

use lazydb::{
    persistence::profiles::ProfileStore,
    profile::{Environment, import_connection_url},
};
use tempfile::TempDir;

#[test]
fn round_trips_profiles_without_passwords() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    let store = ProfileStore::new(path.clone());
    let mut first = import_connection_url(
        "postgres://alice:alice-password-42@db.example.com/app?sslmode=require",
        Some("app"),
    )
    .unwrap()
    .profile;
    first.secret_ref = Some("keyring:profile-id".into());
    first.environment = Environment::Production;
    let second = import_connection_url("sqlite:///tmp/demo.db", Some("demo"))
        .unwrap()
        .profile;

    store.save(&[first.clone(), second.clone()]).unwrap();
    let loaded = store.load().unwrap();

    assert_eq!(loaded, vec![first, second]);
    let serialized = fs::read_to_string(path).unwrap();
    assert!(serialized.contains("keyring:profile-id"));
    assert!(!serialized.contains("alice-password-42"));
}

#[test]
fn missing_store_loads_as_empty() {
    let temp = TempDir::new().unwrap();
    let store = ProfileStore::new(temp.path().join("missing.toml"));

    assert!(store.load().unwrap().is_empty());
}

#[test]
fn malformed_store_is_not_overwritten() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    fs::write(&path, "not = [valid").unwrap();
    let store = ProfileStore::new(path.clone());

    assert!(store.load().is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "not = [valid");
}

#[test]
fn successful_save_leaves_no_temporary_file() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("nested/connections.toml");
    let store = ProfileStore::new(path);

    store.save(&[]).unwrap();

    let names = fs::read_dir(temp.path().join("nested"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["connections.toml"]);
}
