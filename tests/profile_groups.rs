use std::fs;

use lazydb::{
    persistence::profiles::{PersistenceError, ProfileStore},
    profile::{ConnectionGroup, ProfileCollection, import_connection_url},
};
use tempfile::TempDir;
use uuid::Uuid;

#[test]
fn missing_file_loads_an_empty_collection() {
    let temp = TempDir::new().unwrap();
    let collection = ProfileStore::new(temp.path().join("connections.toml"))
        .load()
        .unwrap();
    assert!(collection.groups.is_empty());
    assert!(collection.profiles.is_empty());
}

#[test]
fn v6_round_trip_preserves_group_and_profile_order() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    let store = ProfileStore::new(path);
    let groups = vec![
        ConnectionGroup::new(Uuid::from_u128(1), "Production").unwrap(),
        ConnectionGroup::new(Uuid::from_u128(2), "Development").unwrap(),
    ];
    let mut first = import_connection_url("sqlite::memory:", Some("first"))
        .unwrap()
        .profile;
    first.id = Uuid::from_u128(3);
    first.group_id = Some(groups[0].id);
    let mut second = import_connection_url("sqlite::memory:", Some("second"))
        .unwrap()
        .profile;
    second.id = Uuid::from_u128(4);
    second.group_id = Some(groups[1].id);
    let mut third = import_connection_url("sqlite::memory:", Some("third"))
        .unwrap()
        .profile;
    third.id = Uuid::from_u128(5);
    let expected = ProfileCollection {
        groups,
        profiles: vec![first, second, third],
    };

    store.save(&expected).unwrap();

    assert_eq!(store.load().unwrap(), expected);
    assert!(
        fs::read_to_string(store.path())
            .unwrap()
            .contains("version = 6")
    );
}

#[test]
fn invalid_group_integrity_is_rejected() {
    let temp = TempDir::new().unwrap();
    let store = ProfileStore::new(temp.path().join("connections.toml"));
    let mut profile = import_connection_url("sqlite::memory:", Some("profile"))
        .unwrap()
        .profile;
    profile.group_id = Some(Uuid::from_u128(99));
    let collection = ProfileCollection {
        groups: vec![ConnectionGroup::new(Uuid::from_u128(1), "Production").unwrap()],
        profiles: vec![profile],
    };
    assert!(matches!(
        store.save(&collection),
        Err(PersistenceError::UnknownProfileGroup { .. })
    ));
}
