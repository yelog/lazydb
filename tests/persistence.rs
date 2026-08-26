use std::fs;

use lazydb::{
    persistence::profiles::{PersistenceError, ProfileStore},
    profile::{
        CatalogScope, CatalogSelection, ConnectionProfile, DatabaseScope, Environment,
        import_connection_url,
    },
};
use tempfile::TempDir;

#[derive(serde::Serialize)]
struct ProfileFileFixture<'a> {
    version: u16,
    profiles: &'a [ConnectionProfile],
}

#[test]
fn version_two_profiles_round_trip_hierarchical_scope() {
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
    first.catalog_scope = CatalogScope {
        databases: CatalogSelection::Selected(vec![
            DatabaseScope {
                name: "app".into(),
                schemas: CatalogSelection::Selected(vec!["public".into(), "audit".into()]),
            },
            DatabaseScope {
                name: "analytics".into(),
                schemas: CatalogSelection::All,
            },
        ]),
    };
    let second = import_connection_url("sqlite:///tmp/demo.db", Some("demo"))
        .unwrap()
        .profile;

    store.save(&[first.clone(), second.clone()]).unwrap();
    let loaded = store.load().unwrap();

    assert_eq!(loaded, vec![first, second]);
    let serialized = fs::read_to_string(path).unwrap();
    assert!(serialized.contains("version = 2"));
    assert!(serialized.contains("catalog_scope"));
    assert!(!serialized.contains("include_databases"));
    assert!(!serialized.contains("include_schemas"));
    assert!(serialized.contains("keyring:profile-id"));
    assert!(!serialized.contains("alice-password-42"));
}

#[test]
fn version_one_is_rejected_before_profile_shape_decoding() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    fs::write(
        &path,
        r#"version = 1

[[profiles]]
id = "5f43b849-2a01-4e4a-a388-3fbc89af7ae2"
name = "legacy"
kind = "postgres"
host = "localhost"
port = 5432
user = "alice"
database = "app"
default_schema = "public"
ssl_mode = "prefer"
read_only = false
environment = "development"
include_databases = ["app"]
include_schemas = ["public"]
"#,
    )
    .unwrap();

    let error = ProfileStore::new(path).load().unwrap_err();

    assert!(matches!(
        error,
        PersistenceError::UnsupportedVersion {
            found: 1,
            expected: 2
        }
    ));
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

#[test]
fn rejects_duplicate_profile_uuids_on_save_and_load() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    let store = ProfileStore::new(path.clone());
    let profile = import_connection_url(":memory:", Some("duplicate"))
        .unwrap()
        .profile;

    assert!(store.save(&[profile.clone(), profile.clone()]).is_err());
    assert!(!path.exists());

    let duplicates = [profile.clone(), profile.clone()];
    fs::write(
        &path,
        toml::to_string(&ProfileFileFixture {
            version: 2,
            profiles: &duplicates,
        })
        .unwrap(),
    )
    .unwrap();

    assert!(matches!(
        store.load(),
        Err(PersistenceError::DuplicateProfileId(duplicate)) if duplicate == profile.id
    ));
}
