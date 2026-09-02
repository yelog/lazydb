use std::fs;

use lazydb::{
    persistence::profiles::{PersistenceError, ProfileStore},
    profile::{
        CatalogScope, CatalogSelection, ConnectionProfile, ConnectionUrlFormat, CredentialPolicy,
        DatabaseScope, Environment, ProfileAccess, import_connection_url,
    },
};
use tempfile::TempDir;

#[derive(serde::Serialize)]
struct ProfileFileFixture<'a> {
    version: u16,
    profiles: &'a [ConnectionProfile],
}

#[test]
fn version_two_profiles_migrate_credential_policy_and_save_as_version_five() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    let store = ProfileStore::new(path.clone());
    let mut first = import_connection_url(
        "postgres://alice:alice-password-42@db.example.com/app?sslmode=require",
        Some("app"),
    )
    .unwrap()
    .profile;
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

    let mut legacy = toml::Value::try_from(ProfileFileFixture {
        version: 3,
        profiles: &[first.clone(), second.clone()],
    })
    .unwrap();
    let root = legacy.as_table_mut().unwrap();
    root.insert("version".into(), toml::Value::Integer(2));
    let profiles = root.get_mut("profiles").unwrap().as_array_mut().unwrap();
    profiles[0]
        .as_table_mut()
        .unwrap()
        .remove("credential_policy");
    profiles[0].as_table_mut().unwrap().remove("url_format");
    profiles[0].as_table_mut().unwrap().remove("access");
    profiles[0].as_table_mut().unwrap().insert(
        "secret_ref".into(),
        toml::Value::String("keyring:profile-id".into()),
    );
    profiles[1]
        .as_table_mut()
        .unwrap()
        .remove("credential_policy");
    profiles[1].as_table_mut().unwrap().remove("url_format");
    profiles[1].as_table_mut().unwrap().remove("access");
    fs::write(&path, toml::to_string_pretty(&legacy).unwrap()).unwrap();

    let loaded = store.load().unwrap();

    assert_eq!(
        loaded[0].credential_policy,
        CredentialPolicy::System("keyring:profile-id".into())
    );
    assert_eq!(loaded[1].credential_policy, CredentialPolicy::None);
    first.credential_policy = CredentialPolicy::System("keyring:profile-id".into());
    first.url_format = ConnectionUrlFormat::PostgreSql;
    assert_eq!(loaded, vec![first, second]);

    store.save(&loaded).unwrap();
    let serialized = fs::read_to_string(path).unwrap();
    assert!(serialized.contains("version = 6"));
    assert!(serialized.contains("policy = \"system\""));
    assert!(!serialized.contains("secret_ref"));
    assert!(serialized.contains("catalog_scope"));
    assert!(!serialized.contains("include_databases"));
    assert!(!serialized.contains("include_schemas"));
    assert!(serialized.contains("keyring:profile-id"));
    assert!(!serialized.contains("alice-password-42"));
    assert!(serialized.contains("url_format"));
    assert!(!serialized.contains("postgresql://"));
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
            expected: 6
        }
    ));
}

#[test]
fn version_three_keyring_profiles_migrate_to_system_without_secret_access() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    let store = ProfileStore::new(path.clone());
    let mut profile = import_connection_url("postgres://db.example.com/app", Some("app"))
        .unwrap()
        .profile;
    profile.credential_policy = CredentialPolicy::Keyring("keyring:app".into());
    fs::write(
        &path,
        toml::to_string(&ProfileFileFixture {
            version: 3,
            profiles: &[profile],
        })
        .unwrap(),
    )
    .unwrap();

    let loaded = store.load().unwrap();
    assert_eq!(
        loaded[0].credential_policy,
        CredentialPolicy::System("keyring:app".into())
    );
}

#[test]
fn credential_storage_variants_serialize_without_plaintext() {
    let mut profile = import_connection_url("postgres://db.example.com/app", Some("app"))
        .unwrap()
        .profile;
    profile.credential_policy = CredentialPolicy::LocalEncrypted(
        lazydb::persistence::local_credentials::EncryptedCredential {
            version: 1,
            nonce: "nonce".into(),
            ciphertext: "ciphertext".into(),
        },
    );
    let serialized = toml::to_string(&profile).unwrap();
    assert!(serialized.contains("policy = \"local_encrypted\""));
    assert!(serialized.contains("ciphertext = \"ciphertext\""));

    profile.credential_policy = CredentialPolicy::System("keyring:app".into());
    let serialized = toml::to_string(&profile).unwrap();
    assert!(serialized.contains("policy = \"system\""));
    assert!(serialized.contains("keyring:app"));
}

#[test]
fn version_five_round_trips_global_and_multi_project_access() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    let store = ProfileStore::new(path.clone());
    let mut profile = import_connection_url("sqlite:///tmp/app.db", Some("app"))
        .unwrap()
        .profile;
    profile.access = ProfileAccess::Projects {
        roots: vec!["/Users/me/code/alpha".into(), "/Users/me/code/zeta".into()],
    };
    store.save(&[profile.clone()]).unwrap();
    let loaded = store.load().unwrap();

    assert_eq!(loaded, vec![profile]);
    let serialized = fs::read_to_string(path).unwrap();
    assert!(serialized.contains("version = 6"));
    assert!(serialized.contains("scope = \"projects\""));
    assert!(serialized.contains("/Users/me/code/zeta"));
    assert!(serialized.contains("/Users/me/code/alpha"));
}

#[test]
fn project_access_allows_an_empty_unassigned_root_list() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    let store = ProfileStore::new(path);
    let mut profile = import_connection_url(":memory:", Some("unassigned"))
        .unwrap()
        .profile;
    profile.access = ProfileAccess::Projects { roots: Vec::new() };

    store.save(&[profile.clone()]).unwrap();

    assert_eq!(store.load().unwrap(), vec![profile]);
}

#[test]
fn version_five_rejects_relative_and_duplicate_project_roots() {
    let temp = TempDir::new().unwrap();
    let store = ProfileStore::new(temp.path().join("connections.toml"));
    let mut profile = import_connection_url(":memory:", Some("invalid"))
        .unwrap()
        .profile;

    profile.access = ProfileAccess::Projects {
        roots: vec!["relative/project".into()],
    };
    assert!(matches!(
        store.save(&[profile.clone()]),
        Err(PersistenceError::InvalidProjectRoot(_))
    ));

    profile.access = ProfileAccess::Projects {
        roots: vec!["/same/project".into(), "/same/project".into()],
    };
    assert!(matches!(
        store.save(&[profile]),
        Err(PersistenceError::DuplicateProjectRoot(_))
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
            version: 4,
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
