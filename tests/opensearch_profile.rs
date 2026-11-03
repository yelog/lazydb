use lazydb::{
    model::{
        execution_target::ExecutionTarget,
        profile_manager::{
            CredentialUpdate, DiscoveryFingerprint, ProfileDraft, ProfileField, ProfileManagerState,
        },
    },
    persistence::profiles::ProfileStore,
    profile::{
        ConnectionProfile, ConnectionUrlFormat, CredentialPolicy, DatabaseKind, ElasticsearchAuth,
        ElasticsearchConnectionOptions, OpenSearchAuth, OpenSearchConnectionOptions, SslMode,
        format_connection_url, import_connection_url, parse_connection_url,
        parse_connection_url_with_driver,
    },
};
use secrecy::ExposeSecret;
use tempfile::tempdir;

#[test]
fn native_urls_keep_driver_identity_and_transient_credentials_without_exporting_them() {
    for (url, mode, format) in [
        (
            "opensearch://alice:p%40ss@localhost:9201",
            SslMode::Disable,
            ConnectionUrlFormat::OpenSearch,
        ),
        (
            "opensearchs://alice:p%40ss@[::1]/team%20search?readOnly=true",
            SslMode::VerifyFull,
            ConnectionUrlFormat::OpenSearchTls,
        ),
    ] {
        let imported = import_connection_url(url, Some("search")).unwrap();
        let profile = imported.profile;
        assert_eq!(profile.kind, DatabaseKind::OpenSearch);
        assert!(profile.kind.is_search_rest());
        assert!(!profile.kind.is_relational());
        assert_eq!(profile.ssl_mode, mode);
        assert_eq!(profile.url_format, format);
        assert_eq!(profile.user.as_deref(), Some("alice"));
        assert_eq!(imported.transient_password.unwrap().expose_secret(), "p@ss");
        let formatted = format_connection_url(&profile, format).unwrap();
        assert!(!formatted.contains("alice"));
        assert!(!formatted.contains('@'));
        assert!(!formatted.contains("p%40ss"));
        assert!(!formatted.contains("p@ss"));
        let parsed = parse_connection_url(&formatted).unwrap();
        assert_eq!(parsed.kind, profile.kind);
        assert_eq!(parsed.host, profile.host);
        assert_eq!(parsed.port, profile.port);
        assert_eq!(parsed.database, profile.database);
        assert_eq!(parsed.ssl_mode, mode);
        assert_eq!(parsed.read_only, profile.read_only);
        assert!(parsed.user.is_none());
        assert!(parsed.password.is_none());
    }
    assert!(DatabaseKind::Elasticsearch.is_search_rest());
    assert!(!DatabaseKind::Postgres.is_search_rest());
    assert_eq!(
        ConnectionUrlFormat::compatible_formats(DatabaseKind::OpenSearch),
        &[
            ConnectionUrlFormat::OpenSearch,
            ConnectionUrlFormat::OpenSearchTls
        ]
    );
    assert!(
        parse_connection_url_with_driver(
            "elasticsearch://localhost",
            Some(DatabaseKind::OpenSearch)
        )
        .is_err()
    );
    assert!(
        parse_connection_url_with_driver(
            "opensearch://localhost",
            Some(DatabaseKind::Elasticsearch)
        )
        .is_err()
    );
}

#[test]
fn cluster_defaults_and_virtual_execution_target_are_valid() {
    let profile = import_connection_url("opensearch://localhost", Some("search"))
        .unwrap()
        .profile;
    assert_eq!(profile.port, Some(9200));
    assert_eq!(profile.ssl_mode, SslMode::Disable);
    let target = ExecutionTarget::from_profile(&profile);
    assert_eq!(target.database, "default");
    assert_eq!(target.schema.as_deref(), Some("default"));
    assert!(target.is_valid(&profile));
    assert!(target.apply_to_profile(&profile).is_some());
    assert!(profile.catalog_scope.allows_database("default"));
    assert!(profile.catalog_scope.allows_schema("default", "default"));
    let draft = ProfileDraft::edit(&profile, false);
    assert_eq!(draft.opensearch_auth, OpenSearchAuth::Basic);
    assert!(draft.validate(&[]).is_ok());
    let mut draft = ProfileDraft::new(DatabaseKind::OpenSearch);
    draft.name.set("new-search");
    let submission = draft.validate(&[]).unwrap();
    assert_eq!(submission.profile.port, Some(9200));
    assert_eq!(
        submission.profile.opensearch_options,
        Some(OpenSearchConnectionOptions::default())
    );
    assert!(submission.profile.elasticsearch_options.is_none());
}

#[test]
fn invalid_urls_and_incompatible_transports_are_rejected() {
    for url in [
        "opensearch://localhost:0",
        "opensearch://localhost?apiKey=secret",
        "opensearch://localhost?auth=none",
        "opensearch://localhost?ca_cert_path=ca.pem",
        "opensearchs://localhost?sslMode=disable",
        "opensearch://localhost?readOnly=true&read_only=false",
        "opensearch://localhost?readOnly=true&READONLY=true",
        "opensearch://localhost?readOnly=invalid",
        "opensearch://localhost#fragment",
        "jdbc:opensearch://localhost",
    ] {
        assert!(parse_connection_url(url).is_err(), "accepted {url}");
    }
    let mut profile = import_connection_url("opensearch://localhost", None)
        .unwrap()
        .profile;
    assert!(format_connection_url(&profile, ConnectionUrlFormat::OpenSearchTls).is_err());
    assert!(format_connection_url(&profile, ConnectionUrlFormat::Elasticsearch).is_err());
    profile.ssl_mode = SslMode::Prefer;
    assert!(format_connection_url(&profile, ConnectionUrlFormat::OpenSearch).is_err());
    profile.ssl_mode = SslMode::Disable;
    profile.port = Some(0);
    assert!(format_connection_url(&profile, ConnectionUrlFormat::OpenSearch).is_err());
}

#[test]
fn independent_auth_and_ca_round_trip_and_cleared_options_stay_cleared() {
    let temp = tempdir().unwrap();
    let store = ProfileStore::new(temp.path().join("connections.toml"));
    let mut profile = import_connection_url("opensearchs://alice@localhost", Some("search"))
        .unwrap()
        .profile;
    profile.opensearch_options = Some(OpenSearchConnectionOptions {
        auth: OpenSearchAuth::Basic,
        ca_cert_path: Some(temp.path().join("ca.pem")),
    });
    store.save(vec![profile.clone()]).unwrap();
    let loaded = store.load().unwrap().profiles.remove(0);
    assert_eq!(loaded.user.as_deref(), Some("alice"));
    let mut draft = ProfileDraft::edit(&loaded, false);
    draft.set_opensearch_auth(OpenSearchAuth::None);
    profile = draft.validate(&[]).unwrap().profile;
    assert!(profile.user.is_none());
    store.save(vec![profile.clone()]).unwrap();
    let persisted: toml::Value =
        toml::from_str(&std::fs::read_to_string(store.path()).unwrap()).unwrap();
    assert!(persisted["profiles"][0].get("user").is_none());
    let loaded = store.load().unwrap().profiles.remove(0);
    assert!(loaded.user.is_none());
    assert_eq!(loaded, profile);
    let draft = ProfileDraft::edit(&loaded, false);
    assert_eq!(draft.opensearch_auth, OpenSearchAuth::None);
    assert_eq!(
        draft.validate(&[]).unwrap().profile.opensearch_options,
        profile.opensearch_options
    );
    profile.opensearch_options.as_mut().unwrap().ca_cert_path = None;
    store.save(vec![profile.clone()]).unwrap();
    assert_eq!(store.load().unwrap().profiles[0], profile);
    profile.opensearch_options = None;
    store.save(vec![profile.clone()]).unwrap();
    assert_eq!(store.load().unwrap().profiles[0], profile);
    assert_eq!(
        ProfileDraft::edit(&profile, false).opensearch_auth,
        OpenSearchAuth::Basic
    );

    let mut value = serde_json::to_value(&profile).unwrap();
    value["opensearch_options"] = serde_json::json!({"ca_cert_path": "/test/ca.pem"});
    let parsed: ConnectionProfile = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(
        parsed.opensearch_options.unwrap().auth,
        OpenSearchAuth::Basic
    );
    value["opensearch_options"]["auth"] = serde_json::json!("api_key");
    assert!(serde_json::from_value::<ConnectionProfile>(value).is_err());

    let mut es = import_connection_url("elasticsearchs://localhost", Some("es"))
        .unwrap()
        .profile;
    es.elasticsearch_options = Some(ElasticsearchConnectionOptions {
        auth: ElasticsearchAuth::ApiKey,
        ca_cert_path: Some(temp.path().join("es-ca.pem")),
    });
    store.save(vec![es.clone()]).unwrap();
    assert_eq!(store.load().unwrap().profiles[0], es);
    assert_eq!(
        ProfileDraft::edit(&es, false).elasticsearch_auth,
        ElasticsearchAuth::ApiKey
    );
    assert!(es.opensearch_options.is_none());
}

#[test]
fn auth_switch_hides_and_forgets_credentials_and_cycles_only_basic_and_none() {
    let mut profile = import_connection_url("opensearch://alice@localhost", Some("search"))
        .unwrap()
        .profile;
    profile.credential_policy = CredentialPolicy::Prompt;
    let mut draft = ProfileDraft::edit(&profile, true);
    draft.set_password("pending-basic-password");
    assert!(draft.visible_fields().contains(&ProfileField::User));
    assert!(draft.visible_fields().contains(&ProfileField::Password));
    assert!(
        !draft
            .visible_fields()
            .contains(&ProfileField::ElasticsearchAuth)
    );
    let mut manager = ProfileManagerState::new(false);
    manager.draft = Some(draft);
    manager.selected_field = ProfileField::OpenSearchAuth;
    manager.cycle(1);
    let draft = manager.draft.as_ref().unwrap();
    assert_eq!(draft.opensearch_auth, OpenSearchAuth::None);
    assert_eq!(draft.password_len(), 0);
    assert!(!draft.has_stored_credential);
    for field in [
        ProfileField::User,
        ProfileField::Password,
        ProfileField::PasswordStorage,
    ] {
        assert!(!draft.visible_fields().contains(&field));
    }
    let submission = draft.validate(&[]).unwrap();
    assert!(submission.profile.user.is_none());
    assert_eq!(submission.profile.credential_policy, CredentialPolicy::None);
    assert!(matches!(submission.credential, CredentialUpdate::Forget));
    manager.cycle(1);
    let draft = manager.draft.as_mut().unwrap();
    assert_eq!(draft.opensearch_auth, OpenSearchAuth::Basic);
    draft.set_password("new-basic-password");
    assert!(matches!(
        draft.validate(&[]).unwrap().credential,
        CredentialUpdate::LocalEncrypted(_)
    ));
}

fn replace_url(draft: &mut ProfileDraft, url: &str) {
    draft.move_home(ProfileField::Url);
    for _ in 0..draft.url_display().chars().count() {
        draft.delete(ProfileField::Url);
    }
    draft.paste(ProfileField::Url, url);
    draft.commit_url().unwrap();
}

#[test]
fn secret_free_url_edits_preserve_existing_basic_user_and_credentials() {
    let mut profile = import_connection_url("opensearchs://alice@localhost", Some("search"))
        .unwrap()
        .profile;
    profile.credential_policy = CredentialPolicy::Prompt;
    let mut draft = ProfileDraft::edit(&profile, true);
    assert!(!draft.url_display().contains("alice"));
    let edited_url = draft.url_display().replace("localhost", "other-host");
    replace_url(&mut draft, &edited_url);
    assert_eq!(draft.user.value(), "alice");
    assert!(draft.has_stored_credential);
    let submission = draft.validate(&[]).unwrap();
    assert_eq!(submission.profile.host.as_deref(), Some("other-host"));
    assert_eq!(submission.profile.user.as_deref(), Some("alice"));
    assert!(matches!(submission.credential, CredentialUpdate::Preserve));

    replace_url(&mut draft, "opensearchs://bob:new-password@other-host");
    assert_eq!(draft.user.value(), "bob");
    assert_eq!(draft.password().expose_secret(), "new-password");
    replace_url(&mut draft, "opensearchs://third-host");
    assert_eq!(draft.user.value(), "bob");
    assert_eq!(draft.password().expose_secret(), "new-password");

    replace_url(&mut draft, "elasticsearchs://localhost");
    assert!(draft.user.value().is_empty());
    draft.user.set("es-user");
    replace_url(&mut draft, "opensearchs://localhost");
    assert!(draft.user.value().is_empty());
    replace_url(&mut draft, "elasticsearchs://localhost");
    replace_url(&mut draft, "opensearchs://carol:replacement@localhost");
    assert_eq!(draft.user.value(), "carol");
    assert_eq!(draft.password().expose_secret(), "replacement");
}

#[test]
fn url_edits_preserve_none_and_ca_but_explicit_credentials_select_basic() {
    let mut draft = ProfileDraft::new(DatabaseKind::OpenSearch);
    draft.name.set("search");
    draft.set_opensearch_auth(OpenSearchAuth::None);
    replace_url(&mut draft, "opensearchs://localhost");
    draft.ca_cert_path.set("/test/ca.pem");
    replace_url(&mut draft, "opensearchs://other-host");
    assert_eq!(draft.opensearch_auth, OpenSearchAuth::None);
    assert_eq!(draft.ca_cert_path.value(), "/test/ca.pem");
    replace_url(&mut draft, "opensearchs://bob:new-password@localhost");
    assert_eq!(draft.opensearch_auth, OpenSearchAuth::Basic);
    assert_eq!(draft.password().expose_secret(), "new-password");
    assert!(!draft.url_display().contains("new-password"));
    assert!(!draft.url_display().contains("bob"));
    replace_url(&mut draft, "elasticsearchs://localhost");
    assert_eq!(draft.password_len(), 0);
    assert!(draft.ca_cert_path.value().is_empty());
    assert!(
        draft
            .validate(&[])
            .unwrap()
            .profile
            .opensearch_options
            .is_none()
    );
    draft.set_elasticsearch_auth(ElasticsearchAuth::ApiKey);
    draft.set_password("es-api-key");
    replace_url(&mut draft, "opensearchs://localhost");
    assert_eq!(draft.opensearch_auth, OpenSearchAuth::Basic);
    assert_eq!(draft.password_len(), 0);
    assert!(
        draft
            .validate(&[])
            .unwrap()
            .profile
            .elasticsearch_options
            .is_none()
    );
}

#[test]
fn ca_requires_https_and_transport_auth_and_ca_change_fingerprint() {
    let mut draft = ProfileDraft::new(DatabaseKind::OpenSearch);
    draft.name.set("search");
    draft.ca_cert_path.set("/test/ca.pem");
    assert_eq!(
        draft.validate(&[]).unwrap_err().field,
        ProfileField::CaCertPath
    );
    let mut manager = ProfileManagerState::new(false);
    manager.draft = Some(draft);
    manager.selected_field = ProfileField::SslMode;
    manager.cycle(1);
    let draft = manager.draft.as_mut().unwrap();
    assert_eq!(draft.url_format, ConnectionUrlFormat::OpenSearchTls);
    assert_eq!(draft.ssl_mode, SslMode::VerifyFull);
    let profile = draft.validate(&[]).unwrap().profile;
    let fingerprint = DiscoveryFingerprint::for_profile(&profile, false, 0);
    for change in [0, 1] {
        let mut changed = profile.clone();
        let options = changed.opensearch_options.as_mut().unwrap();
        if change == 0 {
            options.auth = OpenSearchAuth::None;
        } else {
            options.ca_cert_path = Some("/test/other.pem".into());
        }
        assert_ne!(
            fingerprint,
            DiscoveryFingerprint::for_profile(&changed, false, 0)
        );
    }
    manager.draft.as_mut().unwrap().ca_cert_path.set("");
    manager.selected_field = ProfileField::UrlFormat;
    manager.cycle(1);
    let draft = manager.draft.as_ref().unwrap();
    assert_eq!(draft.url_format, ConnectionUrlFormat::OpenSearch);
    assert_eq!(draft.ssl_mode, SslMode::Disable);
    assert_ne!(
        fingerprint,
        draft.validate(&[]).unwrap().discovery_fingerprint
    );
}

#[test]
fn rendered_form_shows_basic_none_and_ca_without_api_key_or_secret() {
    use lazydb::{action::Action, app::App, ui};
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = Box::new(App::new(Vec::new()));
    app.update(Action::OpenProfileManager);
    let mut draft = ProfileDraft::new(DatabaseKind::OpenSearch);
    draft.name.set("search");
    draft.set_password("do-not-render-this-password");
    draft.ca_cert_path.set("/test/ca.pem");
    let manager = app.profile_manager.as_mut().unwrap();
    manager.draft = Some(draft);
    manager.selected_field = ProfileField::OpenSearchAuth;
    let mut terminal = Terminal::new(TestBackend::new(150, 55)).unwrap();
    terminal.draw(|frame| ui::render(frame, &app)).unwrap();
    let output: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    for label in [
        "OpenSearch",
        "Authentication",
        "Basic Auth",
        "CA file (optional)",
        "Password storage",
    ] {
        assert!(output.contains(label), "missing {label}");
    }
    assert!(!output.contains("API Key"));
    assert!(!output.contains("API key"));
    assert!(!output.contains("do-not-render-this-password"));
    app.profile_manager.as_mut().unwrap().cycle(1);
    terminal.draw(|frame| ui::render(frame, &app)).unwrap();
    let output: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(output.contains("None"));
    assert!(!output.contains("Password storage"));
    assert!(!output.contains("API Key"));
}

struct NoNativeSecretStore;

#[async_trait::async_trait]
impl lazydb::persistence::secrets::SecretStore for NoNativeSecretStore {
    async fn available(&self) -> Result<(), lazydb::persistence::secrets::SecretStoreError> {
        Err(lazydb::persistence::secrets::SecretStoreError::Unavailable)
    }

    async fn get(
        &self,
        _: uuid::Uuid,
    ) -> Result<Option<secrecy::SecretString>, lazydb::persistence::secrets::SecretStoreError> {
        panic!("local credentials must not read the native store")
    }

    async fn set(
        &self,
        _: uuid::Uuid,
        _: &secrecy::SecretString,
    ) -> Result<(), lazydb::persistence::secrets::SecretStoreError> {
        panic!("local credentials must not write the native store")
    }

    async fn delete(
        &self,
        _: uuid::Uuid,
    ) -> Result<(), lazydb::persistence::secrets::SecretStoreError> {
        panic!("local credentials must not delete from the native store")
    }
}

#[tokio::test]
async fn save_encrypts_basic_password_and_auth_and_ca_changes_require_reconnect() {
    use lazydb::{
        action::{Action, Command},
        persistence::local_credentials::LocalCredentialStore,
        runtime::Runtime,
    };
    use std::{
        collections::{HashMap, HashSet},
        sync::Arc,
        time::Duration,
    };
    use tokio::{sync::mpsc, time::timeout};

    let temp = tempdir().unwrap();
    let store = ProfileStore::new(temp.path().join("connections.toml"));
    let mut profile = import_connection_url("opensearchs://alice@localhost", Some("search"))
        .unwrap()
        .profile;
    profile.opensearch_options = Some(OpenSearchConnectionOptions::default());
    store.save(vec![profile.clone()]).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut runtime = Runtime::new(
        vec![profile.clone()],
        HashSet::from([profile.id]),
        HashMap::new(),
        None,
        store.clone(),
        Arc::new(NoNativeSecretStore),
        sender,
    );
    let mut draft = ProfileDraft::edit(&profile, false);
    draft.set_password("test-opensearch-password");
    runtime.dispatch(Command::SaveProfile {
        request_id: 1,
        submission: draft.validate(&[]).unwrap(),
        connect: false,
    });
    let saved = match timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap()
    {
        Action::ProfileSaved {
            request_id: 1,
            profile,
            change,
            ..
        } => {
            assert!(change.credentials_changed);
            profile
        }
        action => panic!("unexpected action: {action:?}"),
    };
    let CredentialPolicy::LocalEncrypted(encrypted) = &saved.credential_policy else {
        panic!("Basic password was not encrypted");
    };
    let local = LocalCredentialStore::new(store.credential_key_path(), "lazydb");
    assert_eq!(
        local.decrypt(saved.id, encrypted).unwrap().expose_secret(),
        "test-opensearch-password"
    );
    assert!(
        !std::fs::read_to_string(store.path())
            .unwrap()
            .contains("test-opensearch-password")
    );

    let mut draft = ProfileDraft::edit(&saved, true);
    draft.ca_cert_path.set("/test/custom-ca.pem");
    runtime.dispatch(Command::SaveProfile {
        request_id: 2,
        submission: draft.validate(&[]).unwrap(),
        connect: false,
    });
    let saved = match timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap()
    {
        Action::ProfileSaved {
            request_id: 2,
            profile,
            change,
            ..
        } => {
            assert!(change.connection_settings_changed);
            assert!(!change.credentials_changed);
            assert_eq!(profile.credential_policy, saved.credential_policy);
            profile
        }
        action => panic!("unexpected action: {action:?}"),
    };
    // Keep the same username so only auth, rather than username removal, proves
    // that OpenSearch options participate in the reconnect comparison.
    let mut changed = saved.clone();
    changed.opensearch_options.as_mut().unwrap().auth = OpenSearchAuth::None;
    runtime.dispatch(Command::SaveProfile {
        request_id: 3,
        submission: lazydb::model::profile_manager::ProfileSubmission::new(
            changed,
            CredentialUpdate::Forget,
            0,
        ),
        connect: false,
    });
    match timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap()
    {
        Action::ProfileSaved {
            request_id: 3,
            profile,
            change,
            ..
        } => {
            assert!(change.connection_settings_changed);
            assert!(change.credentials_changed);
            assert_eq!(profile.credential_policy, CredentialPolicy::None);
            let report = store.load_report().unwrap();
            assert!(report.unavailable.is_empty(), "{:?}", report.unavailable);
            assert_eq!(report.collection.profiles, vec![profile]);
            let persisted: toml::Value =
                toml::from_str(&std::fs::read_to_string(store.path()).unwrap()).unwrap();
            let policy = &persisted["profiles"][0]["credential_policy"];
            assert_eq!(policy["policy"].as_str(), Some("none"));
            assert!(policy.get("reference").is_none());
        }
        action => panic!("unexpected action: {action:?}"),
    }
    runtime.shutdown().await;
}
