use lazydb::{persistence::profiles::ProfileStore, profile::import_connection_url};
use tempfile::tempdir;

fn profile(name: &str) -> lazydb::profile::ConnectionProfile {
    import_connection_url("postgresql://localhost/app", Some(name))
        .unwrap()
        .profile
}

#[test]
fn reconcile_save_merges_external_add_and_rejects_same_profile_conflicts() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("connections.toml");
    let store = ProfileStore::new(path);
    let first = profile("first");
    store.save(vec![first.clone()]).unwrap();

    let expected = store.load().unwrap();
    let added = profile("added-externally");
    store
        .mutate(|collection| {
            collection.profiles.push(added.clone());
            Ok(((), true))
        })
        .unwrap();

    let mut desired = expected.clone();
    desired.profiles[0].name = "first-renamed".to_owned();
    store.reconcile_save(&expected, desired.clone()).unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.profiles.len(), 2);
    assert_eq!(loaded.profiles[0].name, "first-renamed");
    assert_eq!(loaded.profiles[1].id, added.id);

    let expected = store.load().unwrap();
    let mut desired = expected.clone();
    desired.profiles[0].name = "stale-edit".to_owned();
    store
        .mutate(|collection| {
            collection.profiles[0].name = "external-edit".to_owned();
            Ok(((), true))
        })
        .unwrap();
    let error = store.reconcile_save(&expected, desired).unwrap_err();
    assert!(error.to_string().contains("profile_conflict"));
    assert_eq!(store.load().unwrap().profiles[0].name, "external-edit");
}
