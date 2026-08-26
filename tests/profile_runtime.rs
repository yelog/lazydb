use std::{
    collections::{HashMap, HashSet},
    fs,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use lazydb::{
    action::{Action, Command},
    app::App,
    db::catalog::{NamespaceModel, ObjectGroup},
    model::{
        profile_manager::{CredentialUpdate, ProfileSubmission},
        workspace::ConnectionIdentity,
    },
    persistence::{
        profiles::ProfileStore,
        secrets::{SecretStore, SecretStoreError, keyring_ref},
    },
    profile::{ConnectionProfile, import_connection_url},
    runtime::Runtime,
};
use secrecy::{ExposeSecret, SecretString};
use tempfile::TempDir;
use tokio::{sync::mpsc, time::timeout};
use uuid::Uuid;

#[derive(Default)]
struct FakeSecretStore {
    state: Mutex<FakeSecretState>,
}

#[derive(Default)]
struct FakeSecretState {
    values: HashMap<Uuid, SecretString>,
    available_error: Option<SecretStoreError>,
    get_error: Option<SecretStoreError>,
    set_error: Option<SecretStoreError>,
    delete_error: Option<SecretStoreError>,
    available_calls: usize,
    get_calls: usize,
    set_calls: usize,
    delete_calls: usize,
}

impl FakeSecretStore {
    fn seed(&self, profile_id: Uuid, password: &str) {
        self.state
            .lock()
            .unwrap()
            .values
            .insert(profile_id, SecretString::from(password.to_owned()));
    }

    fn set_available_error(&self, error: SecretStoreError) {
        self.state.lock().unwrap().available_error = Some(error);
    }

    fn contains(&self, profile_id: Uuid) -> bool {
        self.state.lock().unwrap().values.contains_key(&profile_id)
    }

    fn matches(&self, profile_id: Uuid, expected: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .values
            .get(&profile_id)
            .is_some_and(|password| password.expose_secret() == expected)
    }

    fn calls(&self) -> (usize, usize, usize, usize) {
        let state = self.state.lock().unwrap();
        (
            state.available_calls,
            state.get_calls,
            state.set_calls,
            state.delete_calls,
        )
    }
}

#[async_trait]
impl SecretStore for FakeSecretStore {
    async fn available(&self) -> Result<(), SecretStoreError> {
        let mut state = self.state.lock().unwrap();
        state.available_calls += 1;
        state.available_error.map_or(Ok(()), Err)
    }

    async fn get(&self, profile_id: Uuid) -> Result<Option<SecretString>, SecretStoreError> {
        let mut state = self.state.lock().unwrap();
        state.get_calls += 1;
        if let Some(error) = state.get_error {
            return Err(error);
        }
        Ok(state.values.get(&profile_id).cloned())
    }

    async fn set(&self, profile_id: Uuid, password: &SecretString) -> Result<(), SecretStoreError> {
        let mut state = self.state.lock().unwrap();
        state.set_calls += 1;
        if let Some(error) = state.set_error {
            return Err(error);
        }
        state.values.insert(profile_id, password.clone());
        Ok(())
    }

    async fn delete(&self, profile_id: Uuid) -> Result<(), SecretStoreError> {
        let mut state = self.state.lock().unwrap();
        state.delete_calls += 1;
        if let Some(error) = state.delete_error {
            return Err(error);
        }
        state.values.remove(&profile_id);
        Ok(())
    }
}

fn postgres_profile(name: &str) -> ConnectionProfile {
    import_connection_url("postgres://localhost/lazydb", Some(name))
        .unwrap()
        .profile
}

fn sqlite_profile(name: &str) -> ConnectionProfile {
    import_connection_url(":memory:", Some(name))
        .unwrap()
        .profile
}

fn submission(profile: ConnectionProfile, credential: CredentialUpdate) -> ProfileSubmission {
    ProfileSubmission::new(profile, credential, 0)
}

fn runtime(
    profiles: Vec<ConnectionProfile>,
    persisted: HashSet<Uuid>,
    profile_store: ProfileStore,
    secret_store: Arc<FakeSecretStore>,
) -> (Runtime, mpsc::UnboundedReceiver<Action>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    (
        Runtime::new(
            profiles,
            persisted,
            HashMap::new(),
            None,
            profile_store,
            secret_store,
            sender,
        ),
        receiver,
    )
}

async fn next_action(receiver: &mut mpsc::UnboundedReceiver<Action>) -> Action {
    timeout(Duration::from_secs(3), receiver.recv())
        .await
        .expect("runtime action timed out")
        .expect("runtime action channel closed")
}

#[tokio::test]
async fn profile_test_discovers_scope_without_mutating_or_persisting_it() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    let profile = sqlite_profile("scratch");
    let fake = Arc::new(FakeSecretStore::default());
    let (mut runtime, mut receiver) = runtime(
        Vec::new(),
        HashSet::new(),
        ProfileStore::new(path.clone()),
        fake,
    );

    let submission = submission(profile, CredentialUpdate::Preserve);
    let expected_fingerprint = submission.discovery_fingerprint;
    runtime.dispatch(Command::TestProfile {
        request_id: 1,
        submission,
    });

    match next_action(&mut receiver).await {
        Action::ProfileTestSucceeded {
            request_id: 1,
            fingerprint,
            server,
            capabilities,
            discovery: Ok(discovery),
        } => {
            assert_eq!(fingerprint, expected_fingerprint);
            assert_eq!(server.database, ":memory:");
            assert_eq!(
                capabilities.namespace_model,
                NamespaceModel::DatabaseAndSchema
            );
            assert_eq!(
                capabilities.top_level_groups,
                [
                    ObjectGroup::Tables,
                    ObjectGroup::Views,
                    ObjectGroup::Triggers
                ]
            );
            assert!(capabilities.supports_lazy_children);
            assert_eq!(discovery.databases.len(), 1);
            assert_eq!(discovery.databases[0].name, ":memory:");
            assert_eq!(discovery.databases[0].schemas, ["main"]);
        }
        action => panic!("unexpected action: {action:?}"),
    }
    assert!(!path.exists());
    runtime.shutdown().await;
}

#[tokio::test]
async fn session_password_is_never_persisted_or_sent_to_keyring() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    let profile = postgres_profile("session");
    let profile_id = profile.id;
    let password_text = "session-password-never-persist";
    let fake = Arc::new(FakeSecretStore::default());
    let (mut runtime, mut receiver) = runtime(
        Vec::new(),
        HashSet::new(),
        ProfileStore::new(path.clone()),
        Arc::clone(&fake),
    );

    runtime.dispatch(Command::SaveProfile {
        request_id: 2,
        submission: submission(
            profile,
            CredentialUpdate::Session(SecretString::from(password_text.to_owned())),
        ),
        connect: false,
    });

    let saved = match next_action(&mut receiver).await {
        Action::ProfileSaved {
            request_id: 2,
            profile,
            warning,
            ..
        } => {
            assert!(warning.is_none());
            profile
        }
        action => panic!("unexpected action: {action:?}"),
    };
    assert_eq!(saved.id, profile_id);
    assert!(saved.secret_ref.is_none());
    assert!(!fake.contains(profile_id));
    assert_eq!(fake.calls(), (0, 0, 0, 0));
    let contents = fs::read_to_string(path).unwrap();
    assert!(!contents.contains(password_text));
    runtime.shutdown().await;
}

#[tokio::test]
async fn remember_writes_keyring_reference_but_never_password_text() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    let profile = postgres_profile("remembered");
    let profile_id = profile.id;
    let password_text = "remembered-password-never-persist";
    let fake = Arc::new(FakeSecretStore::default());
    let (mut runtime, mut receiver) = runtime(
        Vec::new(),
        HashSet::new(),
        ProfileStore::new(path.clone()),
        Arc::clone(&fake),
    );

    runtime.dispatch(Command::SaveProfile {
        request_id: 3,
        submission: submission(
            profile,
            CredentialUpdate::Remember(SecretString::from(password_text.to_owned())),
        ),
        connect: false,
    });

    let saved = match next_action(&mut receiver).await {
        Action::ProfileSaved {
            request_id: 3,
            profile,
            warning,
            ..
        } => {
            assert!(warning.is_none());
            profile
        }
        action => panic!("unexpected action: {action:?}"),
    };
    assert_eq!(
        saved.secret_ref.as_deref(),
        Some(keyring_ref(profile_id).as_str())
    );
    assert!(fake.matches(profile_id, password_text));
    assert_eq!(fake.calls(), (1, 1, 1, 0));
    let contents = fs::read_to_string(path).unwrap();
    assert!(contents.contains(&keyring_ref(profile_id)));
    assert!(!contents.contains(password_text));
    runtime.shutdown().await;
}

#[tokio::test]
async fn unavailable_keyring_downgrades_remember_to_session_only() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    let profile = postgres_profile("fallback");
    let profile_id = profile.id;
    let fake = Arc::new(FakeSecretStore::default());
    fake.set_available_error(SecretStoreError::Unavailable);
    let (mut runtime, mut receiver) = runtime(
        Vec::new(),
        HashSet::new(),
        ProfileStore::new(path.clone()),
        Arc::clone(&fake),
    );

    runtime.dispatch(Command::SaveProfile {
        request_id: 4,
        submission: submission(
            profile,
            CredentialUpdate::Remember(SecretString::from("temporary".to_owned())),
        ),
        connect: false,
    });

    match next_action(&mut receiver).await {
        Action::ProfileSaved {
            request_id: 4,
            profile,
            warning,
            ..
        } => {
            assert!(profile.secret_ref.is_none());
            assert!(warning.unwrap().contains("session"));
        }
        action => panic!("unexpected action: {action:?}"),
    }
    assert!(!fake.contains(profile_id));
    assert_eq!(fake.calls(), (1, 0, 0, 0));
    assert!(!fs::read_to_string(path).unwrap().contains("temporary"));
    runtime.shutdown().await;
}

#[tokio::test]
async fn preserve_derives_secret_references_from_runtime_state() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    let mut existing = postgres_profile("existing");
    let existing_id = existing.id;
    existing.secret_ref = Some(keyring_ref(existing_id));
    let store = ProfileStore::new(path.clone());
    store.save(std::slice::from_ref(&existing)).unwrap();
    let fake = Arc::new(FakeSecretStore::default());
    fake.seed(existing_id, "stored-password");
    let (mut runtime, mut receiver) = runtime(
        vec![existing.clone()],
        HashSet::from([existing_id]),
        store,
        Arc::clone(&fake),
    );

    existing.secret_ref = None;
    runtime.dispatch(Command::SaveProfile {
        request_id: 5,
        submission: submission(existing, CredentialUpdate::Preserve),
        connect: false,
    });
    match next_action(&mut receiver).await {
        Action::ProfileSaved {
            request_id: 5,
            profile,
            ..
        } => assert_eq!(
            profile.secret_ref.as_deref(),
            Some(keyring_ref(existing_id).as_str())
        ),
        action => panic!("unexpected action: {action:?}"),
    }

    let mut new_profile = postgres_profile("new");
    let new_id = new_profile.id;
    new_profile.secret_ref = Some(keyring_ref(new_id));
    runtime.dispatch(Command::SaveProfile {
        request_id: 6,
        submission: submission(new_profile, CredentialUpdate::Preserve),
        connect: false,
    });
    match next_action(&mut receiver).await {
        Action::ProfileSaved {
            request_id: 6,
            profile,
            ..
        } => assert!(profile.secret_ref.is_none()),
        action => panic!("unexpected action: {action:?}"),
    }
    assert_eq!(fake.calls(), (0, 0, 0, 0));
    runtime.shutdown().await;
}

#[tokio::test]
async fn edits_preserve_profile_order_and_can_replace_then_forget_a_secret() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    let first = postgres_profile("first");
    let mut second = postgres_profile("second");
    second.secret_ref = Some(keyring_ref(second.id));
    let second_id = second.id;
    let ids = [first.id, second.id];
    let store = ProfileStore::new(path.clone());
    store.save(&[first.clone(), second.clone()]).unwrap();
    let fake = Arc::new(FakeSecretStore::default());
    fake.seed(second_id, "old-password");
    let (mut runtime, mut receiver) = runtime(
        vec![first, second.clone()],
        HashSet::from(ids),
        store,
        Arc::clone(&fake),
    );

    second.name = "renamed".into();
    runtime.dispatch(Command::SaveProfile {
        request_id: 7,
        submission: submission(
            second,
            CredentialUpdate::Remember(SecretString::from("new-password".to_owned())),
        ),
        connect: false,
    });
    let saved = match next_action(&mut receiver).await {
        Action::ProfileSaved {
            request_id: 7,
            profile,
            ..
        } => profile,
        action => panic!("unexpected action: {action:?}"),
    };
    assert!(fake.matches(second_id, "new-password"));
    let loaded = ProfileStore::new(path.clone()).load().unwrap();
    assert_eq!(
        loaded.iter().map(|profile| profile.id).collect::<Vec<_>>(),
        ids
    );
    assert_eq!(loaded[1].name, "renamed");

    runtime.dispatch(Command::SaveProfile {
        request_id: 8,
        submission: submission(saved, CredentialUpdate::Forget),
        connect: false,
    });
    match next_action(&mut receiver).await {
        Action::ProfileSaved {
            request_id: 8,
            profile,
            ..
        } => assert!(profile.secret_ref.is_none()),
        action => panic!("unexpected action: {action:?}"),
    }
    assert!(!fake.contains(second_id));
    runtime.shutdown().await;
}

#[tokio::test]
async fn profile_save_failure_restores_the_previous_keyring_value() {
    let temp = TempDir::new().unwrap();
    let blocked_parent = temp.path().join("blocked");
    fs::write(&blocked_parent, "not a directory").unwrap();
    let mut profile = postgres_profile("rollback");
    let profile_id = profile.id;
    profile.secret_ref = Some(keyring_ref(profile_id));
    let fake = Arc::new(FakeSecretStore::default());
    fake.seed(profile_id, "old-password");
    let (mut runtime, mut receiver) = runtime(
        vec![profile.clone()],
        HashSet::from([profile_id]),
        ProfileStore::new(blocked_parent.join("connections.toml")),
        Arc::clone(&fake),
    );

    runtime.dispatch(Command::SaveProfile {
        request_id: 7,
        submission: submission(
            profile,
            CredentialUpdate::Remember(SecretString::from("new-password".to_owned())),
        ),
        connect: false,
    });

    match next_action(&mut receiver).await {
        Action::ProfileSaveFailed {
            request_id: 7,
            message,
        } => assert!(message.contains("Unable to save")),
        action => panic!("unexpected action: {action:?}"),
    }
    assert!(fake.matches(profile_id, "old-password"));
    assert_eq!(fake.calls().2, 2);
    runtime.shutdown().await;
}

#[tokio::test]
async fn delete_removes_metadata_and_keyring_value() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    let mut profile = postgres_profile("delete");
    let profile_id = profile.id;
    profile.secret_ref = Some(keyring_ref(profile_id));
    let store = ProfileStore::new(path.clone());
    store.save(std::slice::from_ref(&profile)).unwrap();
    let fake = Arc::new(FakeSecretStore::default());
    fake.seed(profile_id, "stored-password");
    let (mut runtime, mut receiver) = runtime(
        vec![profile],
        HashSet::from([profile_id]),
        store,
        Arc::clone(&fake),
    );

    runtime.dispatch(Command::DeleteProfile {
        request_id: 8,
        profile_id,
    });

    assert!(matches!(
        next_action(&mut receiver).await,
        Action::ProfileDeleted {
            request_id: 8,
            profile_id: deleted,
            active_connection: None,
        } if deleted == profile_id
    ));
    assert!(!fake.contains(profile_id));
    assert!(ProfileStore::new(path).load().unwrap().is_empty());
    runtime.shutdown().await;
}

#[tokio::test]
async fn delete_persistence_failure_restores_the_keyring_value() {
    let temp = TempDir::new().unwrap();
    let blocked_parent = temp.path().join("blocked");
    fs::write(&blocked_parent, "not a directory").unwrap();
    let mut profile = postgres_profile("delete-rollback");
    let profile_id = profile.id;
    profile.secret_ref = Some(keyring_ref(profile_id));
    let fake = Arc::new(FakeSecretStore::default());
    fake.seed(profile_id, "stored-password");
    let (mut runtime, mut receiver) = runtime(
        vec![profile],
        HashSet::from([profile_id]),
        ProfileStore::new(blocked_parent.join("connections.toml")),
        Arc::clone(&fake),
    );

    runtime.dispatch(Command::DeleteProfile {
        request_id: 9,
        profile_id,
    });

    assert!(matches!(
        next_action(&mut receiver).await,
        Action::ProfileDeleteFailed { request_id: 9, .. }
    ));
    assert!(fake.matches(profile_id, "stored-password"));
    runtime.shutdown().await;
}

#[tokio::test]
async fn deleting_the_active_profile_requests_an_explicit_disconnect() {
    let temp = TempDir::new().unwrap();
    let profile = sqlite_profile("active");
    let profile_id = profile.id;
    let store = ProfileStore::new(temp.path().join("connections.toml"));
    store.save(std::slice::from_ref(&profile)).unwrap();
    let fake = Arc::new(FakeSecretStore::default());
    let (mut runtime, mut receiver) =
        runtime(vec![profile], HashSet::from([profile_id]), store, fake);

    runtime.dispatch(Command::Connect {
        profile_id,
        generation: 1,
    });
    assert!(matches!(
        next_action(&mut receiver).await,
        Action::ConnectionSucceeded {
            profile_id: connected,
            generation: 1,
            ..
        } if connected == profile_id
    ));

    runtime.dispatch(Command::DeleteProfile {
        request_id: 10,
        profile_id,
    });
    assert!(matches!(
        next_action(&mut receiver).await,
        Action::ProfileDeleted {
            request_id: 10,
            profile_id: deleted,
            active_connection: Some(ConnectionIdentity {
                profile_id: active,
                generation: 1,
            }),
        } if deleted == profile_id && active == profile_id
    ));

    runtime.dispatch(Command::Disconnect {
        connection: ConnectionIdentity {
            profile_id,
            generation: 1,
        },
    });
    assert!(matches!(
        next_action(&mut receiver).await,
        Action::DisconnectCompleted {
            connection: ConnectionIdentity {
                profile_id: disconnected,
                generation: 1,
            },
        } if disconnected == profile_id
    ));
    runtime.shutdown().await;
}

#[tokio::test]
async fn deleting_a_profile_invalidates_its_in_flight_connection() {
    let temp = TempDir::new().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let profile = import_connection_url(
        &format!("postgres://127.0.0.1:{port}/lazydb"),
        Some("pending"),
    )
    .unwrap()
    .profile;
    let profile_id = profile.id;
    let store = ProfileStore::new(temp.path().join("connections.toml"));
    store.save(std::slice::from_ref(&profile)).unwrap();
    let fake = Arc::new(FakeSecretStore::default());
    let (mut runtime, mut receiver) =
        runtime(vec![profile], HashSet::from([profile_id]), store, fake);
    let (accepted_sender, accepted_receiver) = tokio::sync::oneshot::channel();
    let (release_sender, release_receiver) = std::sync::mpsc::channel();
    let server = tokio::task::spawn_blocking(move || {
        let (socket, _) = listener.accept().unwrap();
        let _ = accepted_sender.send(());
        let _ = release_receiver.recv_timeout(Duration::from_secs(3));
        drop(socket);
    });

    runtime.dispatch(Command::Connect {
        profile_id,
        generation: 1,
    });
    timeout(Duration::from_secs(3), accepted_receiver)
        .await
        .unwrap()
        .unwrap();
    runtime.dispatch(Command::DeleteProfile {
        request_id: 11,
        profile_id,
    });
    assert!(matches!(
        next_action(&mut receiver).await,
        Action::ProfileDeleted {
            request_id: 11,
            profile_id: deleted,
            active_connection: None,
        } if deleted == profile_id
    ));

    release_sender.send(()).unwrap();
    server.await.unwrap();
    assert!(
        timeout(Duration::from_millis(100), receiver.recv())
            .await
            .is_err()
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn ad_hoc_profiles_stay_out_of_toml_until_explicitly_saved() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    let ad_hoc = sqlite_profile("ad-hoc");
    let saved = sqlite_profile("saved");
    let ad_hoc_id = ad_hoc.id;
    let saved_id = saved.id;
    let fake = Arc::new(FakeSecretStore::default());
    let (mut runtime, mut receiver) = runtime(
        vec![ad_hoc.clone()],
        HashSet::new(),
        ProfileStore::new(path.clone()),
        fake,
    );

    runtime.dispatch(Command::SaveProfile {
        request_id: 11,
        submission: submission(saved, CredentialUpdate::Preserve),
        connect: false,
    });
    assert!(matches!(
        next_action(&mut receiver).await,
        Action::ProfileSaved { request_id: 11, .. }
    ));
    let loaded = ProfileStore::new(path.clone()).load().unwrap();
    assert_eq!(
        loaded.iter().map(|profile| profile.id).collect::<Vec<_>>(),
        [saved_id]
    );

    runtime.dispatch(Command::SaveProfile {
        request_id: 12,
        submission: submission(ad_hoc, CredentialUpdate::Preserve),
        connect: false,
    });
    assert!(matches!(
        next_action(&mut receiver).await,
        Action::ProfileSaved { request_id: 12, .. }
    ));
    let loaded = ProfileStore::new(path).load().unwrap();
    assert_eq!(
        loaded.iter().map(|profile| profile.id).collect::<Vec<_>>(),
        [ad_hoc_id, saved_id]
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn app_changes_only_after_runtime_completion_is_applied() {
    let temp = TempDir::new().unwrap();
    let fake = Arc::new(FakeSecretStore::default());
    let (mut runtime, mut receiver) = runtime(
        Vec::new(),
        HashSet::new(),
        ProfileStore::new(temp.path().join("connections.toml")),
        fake,
    );
    let mut app = App::new(Vec::new());
    app.update(Action::OpenProfileManager);
    let draft = app
        .profile_manager
        .as_mut()
        .unwrap()
        .draft
        .as_mut()
        .unwrap();
    draft.name.set("app-owned");
    draft.database.set("lazydb");
    let commands = app.update(Action::ProfileSave { connect: false });
    assert!(app.profiles.is_empty());
    for command in commands {
        runtime.dispatch(command);
    }

    let completion = next_action(&mut receiver).await;
    assert!(app.profiles.is_empty());
    app.update(completion);
    assert_eq!(app.profiles.len(), 1);
    runtime.shutdown().await;
}
