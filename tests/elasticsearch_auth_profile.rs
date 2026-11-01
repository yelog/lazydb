use lazydb::{
    model::profile_manager::{CredentialUpdate, ProfileDraft, ProfileField, ProfileManagerState},
    persistence::profiles::ProfileStore,
    profile::{
        CredentialPolicy, DatabaseKind, ElasticsearchAuth, ElasticsearchConnectionOptions,
        format_connection_url, import_connection_url,
    },
};
use tempfile::tempdir;

#[test]
fn auth_and_ca_options_persist_without_secret_and_old_profiles_default_to_basic() {
    let temp = tempdir().unwrap();
    let mut profile =
        import_connection_url("elasticsearchs://localhost:19201", Some("secure-search"))
            .unwrap()
            .profile;
    profile.elasticsearch_options = Some(ElasticsearchConnectionOptions {
        auth: ElasticsearchAuth::ApiKey,
        ca_cert_path: Some(temp.path().join("ca.pem")),
    });
    let store = ProfileStore::new(temp.path().join("connections.toml"));
    store.save(vec![profile.clone()]).unwrap();
    let saved = store.load().unwrap().profiles.remove(0);
    assert_eq!(saved, profile);
    let draft = ProfileDraft::edit(&saved, true);
    assert_eq!(draft.elasticsearch_auth, ElasticsearchAuth::ApiKey);
    assert_eq!(
        draft.validate(&[]).unwrap().profile.elasticsearch_options,
        profile.elasticsearch_options
    );
    assert!(!draft.visible_fields().contains(&ProfileField::User));
    let mut legacy = profile;
    legacy.elasticsearch_options.as_mut().unwrap().ca_cert_path = None;
    store.save(vec![legacy.clone()]).unwrap();
    assert!(
        store.load().unwrap().profiles[0]
            .elasticsearch_options
            .as_ref()
            .unwrap()
            .ca_cert_path
            .is_none()
    );
    legacy.elasticsearch_options = None;
    store.save(vec![legacy]).unwrap();
    let loaded = store.load().unwrap().profiles.remove(0);
    assert_eq!(
        ProfileDraft::edit(&loaded, false).elasticsearch_auth,
        ElasticsearchAuth::Basic
    );
}

#[test]
fn existing_prompt_api_key_profile_can_preserve_its_runtime_credential() {
    let mut profile = import_connection_url("elasticsearchs://localhost:19201", Some("search"))
        .unwrap()
        .profile;
    profile.credential_policy = CredentialPolicy::Prompt;
    profile.elasticsearch_options = Some(ElasticsearchConnectionOptions {
        auth: ElasticsearchAuth::ApiKey,
        ca_cert_path: None,
    });
    let mut draft = ProfileDraft::edit(&profile, false);
    draft.name.set("renamed-search");
    draft.ca_cert_path.set("/test/ca.pem");
    let submission = draft.validate(&[]).unwrap();
    assert_eq!(
        submission.profile.credential_policy,
        CredentialPolicy::Prompt
    );
    assert!(matches!(submission.credential, CredentialUpdate::Preserve));
}

#[test]
fn switching_auth_clears_pending_and_stored_credentials() {
    let mut profile = import_connection_url("elasticsearch://alice@localhost", Some("search"))
        .unwrap()
        .profile;
    profile.credential_policy = CredentialPolicy::Prompt;
    let mut draft = ProfileDraft::edit(&profile, true);
    draft.set_password("old-basic-password");
    draft.set_elasticsearch_auth(ElasticsearchAuth::ApiKey);
    assert_eq!(draft.password_len(), 0);
    assert!(!draft.has_stored_credential);
    assert_eq!(
        draft.validate(&[]).unwrap_err().field,
        ProfileField::Password
    );
    draft.set_password("encoded-test-api-key");
    let submitted = draft.validate(&[]).unwrap();
    assert!(matches!(
        submitted.credential,
        CredentialUpdate::LocalEncrypted(_)
    ));
    assert!(submitted.profile.user.is_none());
    let url = format_connection_url(&submitted.profile, submitted.profile.url_format).unwrap();
    assert!(!url.contains("encoded-test-api-key"));
    assert!(!url.contains("alice"));
    draft.set_elasticsearch_auth(ElasticsearchAuth::Basic);
    assert!(matches!(
        draft.validate(&[]).unwrap().credential,
        CredentialUpdate::Forget
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
fn url_edits_preserve_api_key_options_but_basic_urls_and_other_drivers_clear_the_key() {
    let mut draft = ProfileDraft::new(DatabaseKind::Elasticsearch);
    draft.name.set("search");
    draft.set_elasticsearch_auth(ElasticsearchAuth::ApiKey);
    draft.set_password("encoded-test-api-key");
    replace_url(&mut draft, "elasticsearchs://localhost:19201");
    draft.ca_cert_path.set("/test/ca.pem");
    replace_url(&mut draft, "elasticsearchs://other-host:19201");
    assert_eq!(draft.elasticsearch_auth, ElasticsearchAuth::ApiKey);
    assert_eq!(draft.password_len(), "encoded-test-api-key".len());
    assert_eq!(draft.ca_cert_path.value(), "/test/ca.pem");
    replace_url(
        &mut draft,
        "elasticsearchs://bob:new-basic-password@localhost:19201",
    );
    assert_eq!(draft.elasticsearch_auth, ElasticsearchAuth::Basic);
    assert_eq!(draft.password_len(), "new-basic-password".len());
    draft.set_elasticsearch_auth(ElasticsearchAuth::ApiKey);
    draft.set_password("another-test-api-key");
    replace_url(&mut draft, "postgres://alice@localhost/app");
    assert_eq!(draft.password_len(), 0);
    assert!(draft.ca_cert_path.value().is_empty());
    assert!(matches!(
        draft.validate(&[]).unwrap().credential,
        CredentialUpdate::Forget
    ));
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
fn api_key_form_masks_secret_and_displays_auth_and_ca_fields() {
    use lazydb::{action::Action, app::App, ui};
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = Box::new(App::new(Vec::new()));
    app.update(Action::OpenProfileManager);
    let manager = app.profile_manager.as_mut().unwrap();
    let mut draft = ProfileDraft::new(DatabaseKind::Elasticsearch);
    draft.set_elasticsearch_auth(ElasticsearchAuth::ApiKey);
    draft.set_password("do-not-render-this-key");
    draft.ca_cert_path.set("/test/ca.pem");
    manager.draft = Some(draft);
    manager.selected_field = ProfileField::Password;
    let mut terminal = Terminal::new(TestBackend::new(150, 55)).unwrap();
    terminal.draw(|frame| ui::render(frame, &app)).unwrap();
    let output: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(output.contains("Authentication"));
    assert!(output.contains("API key (encoded)"));
    assert!(output.contains("CA file (optional)"));
    assert!(!output.contains("do-not-render-this-key"));
}

#[test]
fn ca_requires_https_and_changes_discovery_fingerprint() {
    let mut draft = ProfileDraft::new(DatabaseKind::Elasticsearch);
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
    let first = manager.draft.as_ref().unwrap().validate(&[]).unwrap();
    manager
        .draft
        .as_mut()
        .unwrap()
        .ca_cert_path
        .set("/test/other.pem");
    let second = manager.draft.as_ref().unwrap().validate(&[]).unwrap();
    assert_ne!(first.discovery_fingerprint, second.discovery_fingerprint);
}
