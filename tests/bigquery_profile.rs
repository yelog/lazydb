use lazydb::{
    model::profile_manager::{
        CatalogDiscoveryState, CredentialUpdate, DiscoveryFingerprint, ProfileCatalogDiscovery,
        ProfileDraft, ProfileField,
    },
    persistence::profiles::ProfileStore,
    profile::{
        BigQueryOptions, CatalogScope, ConnectionProfile, ConnectionUrlFormat, CredentialPolicy,
        DatabaseKind, SslMode, format_connection_url, import_connection_url, parse_connection_url,
    },
};
use secrecy::ExposeSecret;
use tempfile::TempDir;

#[test]
fn bigquery_url_preserves_execution_and_data_projects_without_credentials() {
    let imported = import_connection_url(
        "bigquery://billing-project/analytics?location=US&data_project=data-project&maximum_bytes_billed=2000000000&credentials_file=%2Ftmp%2Fadc+file.json&query_timeout_secs=90",
        Some("BigQuery"),
    ).unwrap();
    let profile = imported.profile;
    assert_eq!(profile.kind, DatabaseKind::BigQuery);
    assert_eq!(profile.url_format, ConnectionUrlFormat::BigQuery);
    assert_eq!(profile.database.as_deref(), Some("billing-project"));
    assert_eq!(profile.default_schema.as_deref(), Some("analytics"));
    assert_eq!(profile.catalog_database(), Some("data-project"));
    assert!(
        profile
            .catalog_scope
            .allows_schema("data-project", "analytics")
    );
    assert!(!profile.catalog_scope.allows_database("billing-project"));
    assert!(profile.host.is_none() && profile.port.is_none() && profile.user.is_none());
    assert!(imported.transient_password.is_none());
    let options = profile.bigquery_options.as_ref().unwrap();
    assert_eq!(
        options.credentials_file.as_deref(),
        Some("/tmp/adc file.json")
    );
    assert_eq!(options.location.as_deref(), Some("US"));
    assert_eq!(options.maximum_bytes_billed, 2_000_000_000);
    assert_eq!(options.query_timeout_secs, 90);
    let output = format_connection_url(&profile, profile.url_format).unwrap();
    let parsed = parse_connection_url(&output).unwrap();
    assert_eq!(parsed.database, profile.database);
    assert_eq!(parsed.default_schema, profile.default_schema);
    assert_eq!(parsed.bigquery_options, profile.bigquery_options);
    assert!(parsed.password.is_none());
}

#[test]
fn bigquery_read_only_urls_round_trip_and_reject_conflicting_aliases() {
    for key in ["readOnly", "read_only", "READONLY"] {
        let profile = import_connection_url(
            &format!("bigquery://billing-project/analytics?{key}=true"),
            None,
        )
        .unwrap()
        .profile;
        assert!(profile.read_only);
        let url = format_connection_url(&profile, profile.url_format).unwrap();
        let parsed = parse_connection_url(&url).unwrap();
        assert!(parsed.read_only && parsed.read_only_explicit);
        let mut draft = ProfileDraft::edit(&profile, false);
        replace_url(&mut draft, &url);
        draft.commit_url().unwrap();
        assert!(draft.validate(&[]).unwrap().profile.read_only);
    }
    for query in [
        "readOnly=true&read_only=false",
        "readOnly=true&READONLY=true",
        "readOnly=invalid",
    ] {
        assert!(parse_connection_url(&format!("bigquery://billing-project?{query}")).is_err());
    }
    let parsed = parse_connection_url("bigquery://billing-project?read_only=false").unwrap();
    assert!(!parsed.read_only && parsed.read_only_explicit);
}

#[test]
fn bigquery_defaults_survive_serialization_and_keep_implicit_data_project() {
    let profile = import_connection_url("bigquery://billing-project", None)
        .unwrap()
        .profile;
    assert!(profile.default_schema.is_none());
    let options = profile.bigquery_options.as_ref().unwrap();
    assert_eq!(options.maximum_bytes_billed, 1_000_000_000);
    assert_eq!(options.query_timeout_secs, 120);
    assert!(options.credentials_file.is_none());
    assert!(options.data_project.is_none());
    assert_eq!(
        options.effective_data_project("billing-project"),
        "billing-project"
    );
    assert_eq!(profile.catalog_database(), Some("billing-project"));
    let encoded = toml::to_string(&profile).unwrap();
    let decoded: lazydb::profile::ConnectionProfile = toml::from_str(&encoded).unwrap();
    assert_eq!(decoded, profile);
    let options: lazydb::profile::BigQueryOptions = toml::from_str("location = 'EU'").unwrap();
    assert_eq!(options.maximum_bytes_billed, 1_000_000_000);
    assert_eq!(options.query_timeout_secs, 120);
    let mut value = serde_json::to_value(&profile).unwrap();
    value.as_object_mut().unwrap().remove("bigquery_options");
    let decoded: lazydb::profile::ConnectionProfile = serde_json::from_value(value).unwrap();
    assert!(decoded.bigquery_options.is_none());
    decoded.validate_bigquery().unwrap();
}

#[test]
fn bigquery_rejects_credentials_endpoints_invalid_identifiers_and_limits() {
    for input in [
        "bigquery://user:secret@billing-project/dataset",
        "bigquery://user@billing-project/dataset",
        "bigquery://billing-project:443/dataset",
        "bigquery://billing-project/dataset#fragment",
        "bigquery://short/dataset",
        "bigquery://123456/dataset",
        "bigquery://Invalid-project/dataset",
        "bigquery://billing-project-/dataset",
        "bigquery://billing.example/dataset",
        "bigquery://billing-project/dataset/extra",
        "bigquery://billing-project/dataset%2Fextra",
        "bigquery://billing-project/../dataset",
        "bigquery://billing-project/%2e%2e/dataset",
        "bigquery://@billing-project/dataset",
        "bigquery://billing-project/dataset?credentials_file=%0Aprivate",
        "bigquery://billing-project/dataset?query_timeout_secs=%2B120",
        "bigquery://billing-project/dataset?data_project=invalid.project",
        "bigquery://billing-project/dataset?endpoint=https%3A%2F%2Fexample.com",
        "bigquery://billing-project/dataset?access_token=secret",
        "bigquery://billing-project/dataset?password=secret",
        "bigquery://billing-project/dataset?credentials_file=",
        "bigquery://billing-project/dataset?location=",
        "bigquery://billing-project/dataset?location=US&LOCATION=EU",
        "bigquery://billing-project/dataset?maximum_bytes_billed=0",
        "bigquery://billing-project/dataset?maximum_bytes_billed=-1",
        "bigquery://billing-project/dataset?maximum_bytes_billed=1.5",
        "bigquery://billing-project/dataset?maximum_bytes_billed=9223372036854775808",
        "bigquery://billing-project/dataset?query_timeout_secs=0",
        "bigquery://billing-project/dataset?query_timeout_secs=86401",
        "bigquery://billing-project/dataset?query_timeout_secs=18446744073709551616",
    ] {
        assert!(parse_connection_url(input).is_err(), "accepted {input}");
    }
}

fn profile() -> ConnectionProfile {
    import_connection_url(
        "bigquery://billing-project/analytics?data_project=data-project",
        Some("BigQuery"),
    )
    .unwrap()
    .profile
}

fn replace_url(draft: &mut ProfileDraft, url: &str) {
    draft.move_home(ProfileField::Url);
    for _ in 0..draft.url_display().chars().count() {
        draft.delete(ProfileField::Url);
    }
    draft.paste(ProfileField::Url, url);
}

#[test]
fn bigquery_profile_store_round_trip_and_compatibility_report() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("connections.toml");
    let store = ProfileStore::new(path.clone());
    let mut profile = profile();
    profile.bigquery_options.as_mut().unwrap().credentials_file =
        Some("/path/that/need/not/exist/adc.json".into());
    store.save(vec![profile.clone()]).unwrap();
    let report = store.load_report().unwrap();
    assert!(report.unavailable.is_empty());
    assert_eq!(report.collection.profiles, vec![profile.clone()]);
    let encoded = std::fs::read_to_string(&path).unwrap();
    assert!(encoded.contains("kind = \"bigquery\""));
    assert!(encoded.contains("maximum_bytes_billed = 1000000000"));
    assert!(encoded.contains("query_timeout_secs = 120"));
    assert!(!encoded.contains("private_key"));
    let json = serde_json::to_value(&profile).unwrap();
    assert_eq!(json["kind"], "bigquery");
    assert_eq!(json["url_format"], "bigquery");
}

#[test]
fn bigquery_draft_exposes_adc_fields_and_never_submits_a_password() {
    let mut draft = ProfileDraft::new(DatabaseKind::BigQuery);
    draft.name.set("BigQuery");
    draft.paste(ProfileField::Database, "billing-project");
    draft.paste(ProfileField::Schema, "analytics");
    draft.paste(ProfileField::DataProject, "data-project");
    draft.paste(ProfileField::Location, "US");
    draft.paste(ProfileField::CredentialsFile, "/tmp/adc file.json");
    for field in [
        ProfileField::CredentialsFile,
        ProfileField::Location,
        ProfileField::DataProject,
        ProfileField::MaximumBytesBilled,
        ProfileField::QueryTimeoutSecs,
    ] {
        assert!(draft.visible_fields().contains(&field));
    }
    for field in [
        ProfileField::Host,
        ProfileField::Port,
        ProfileField::User,
        ProfileField::Password,
        ProfileField::PasswordStorage,
        ProfileField::SslMode,
    ] {
        assert!(!draft.visible_fields().contains(&field));
    }
    assert!(draft.password_storage_choices().is_empty());
    draft.set_password("unrelated-old-password");
    let submission = draft.validate(&[]).unwrap();
    assert!(matches!(submission.credential, CredentialUpdate::Preserve));
    assert_eq!(submission.profile.credential_policy, CredentialPolicy::None);
    assert_eq!(submission.profile.ssl_mode, SslMode::VerifyFull);
    assert!(
        submission
            .profile
            .catalog_scope
            .allows_schema("data-project", "analytics")
    );
    assert!(!draft.url_display().contains("unrelated-old-password"));
    let options = submission.profile.bigquery_options.as_ref().unwrap();
    assert_eq!(
        options.credentials_file.as_deref(),
        Some("/tmp/adc file.json")
    );
    let edited = ProfileDraft::edit(&submission.profile, false)
        .validate(&[])
        .unwrap();
    assert_eq!(edited.profile, submission.profile);
}

#[test]
fn bigquery_url_switch_clears_password_and_other_driver_options_atomically() {
    let postgres = import_connection_url(
        "postgresql://alice@localhost/db?sslrootcert=/tmp/cert",
        Some("switch"),
    )
    .unwrap()
    .profile;
    let mut draft = ProfileDraft::edit(&postgres, true);
    draft.set_password("previous-password");
    replace_url(
        &mut draft,
        "bigquery://billing-project/analytics?data_project=data-project&location=EU",
    );
    draft.commit_url().unwrap();
    assert!(draft.password().expose_secret().is_empty());
    assert!(!draft.has_stored_credential);
    let submission = draft.validate(&[]).unwrap();
    assert!(submission.profile.pg_options.is_none());
    assert!(submission.profile.host.is_none());
    assert_eq!(submission.profile.catalog_database(), Some("data-project"));
    assert_eq!(draft.data_project.value(), "data-project");
    let before = submission.profile;
    replace_url(
        &mut draft,
        "bigquery://billing-project/analytics?query_timeout_secs=0",
    );
    assert!(draft.commit_url().is_err());
    assert_eq!(draft.kind, before.kind);
    assert_eq!(draft.database.value(), before.database.as_deref().unwrap());
    assert_eq!(draft.location.value(), "EU");
}

#[test]
fn switching_through_bigquery_does_not_restore_stored_passwords() {
    for use_url in [false, true] {
        let mut original = import_connection_url("postgresql://alice@old-host/db", Some("switch"))
            .unwrap()
            .profile;
        original.credential_policy = CredentialPolicy::System("fixture-reference".into());
        let mut manager = lazydb::model::profile_manager::ProfileManagerState::new(true);
        manager.draft = Some(ProfileDraft::edit(&original, true));
        if use_url {
            let draft = manager.draft.as_mut().unwrap();
            replace_url(draft, "bigquery://billing-project/analytics");
            draft.commit_url().unwrap();
            replace_url(draft, "postgresql://bob@new-host/db");
            draft.commit_url().unwrap();
        } else {
            manager.select_driver(DatabaseKind::BigQuery);
            manager.select_driver(DatabaseKind::Postgres);
            manager.draft.as_mut().unwrap().host.set("new-host");
        }
        let draft = manager.draft.as_ref().unwrap();
        assert!(!draft.has_stored_credential);
        assert_eq!(draft.password_len(), 0);
        let submission = draft.validate(&[]).unwrap();
        assert!(matches!(submission.credential, CredentialUpdate::Forget));
        assert_eq!(submission.profile.credential_policy, CredentialPolicy::None);
    }
}

#[test]
fn bigquery_driver_switch_does_not_restore_previous_transport_fields_on_save() {
    let mut postgres = import_connection_url(
        "postgresql://alice@localhost:5432/db?sslrootcert=/tmp/cert",
        Some("switch"),
    )
    .unwrap()
    .profile;
    postgres.credential_policy = CredentialPolicy::System("fixture-reference".into());
    let mut draft = ProfileDraft::edit(&postgres, true);
    replace_url(
        &mut draft,
        "bigquery://billing-project/analytics?data_project=data-project&location=EU",
    );
    draft.commit_url().unwrap();
    let bigquery = draft.validate(&[]).unwrap().profile;
    let temp = TempDir::new().unwrap();
    let store = ProfileStore::new(temp.path().join("connections.toml"));
    store.save(vec![postgres]).unwrap();
    store.save(vec![bigquery.clone()]).unwrap();
    let report = store.load_report().unwrap();
    assert!(report.unavailable.is_empty(), "{:?}", report.unavailable);
    assert_eq!(report.collection.profiles, vec![bigquery]);
}

#[test]
fn opensearch_bigquery_url_switches_clear_other_driver_options_and_credentials() {
    let original = import_connection_url("opensearchs://alice@localhost", Some("switch"))
        .unwrap()
        .profile;
    let mut draft = ProfileDraft::edit(&original, true);
    draft.ca_cert_path.set("/tmp/search-ca.pem");
    draft.set_password("fixture-search-password");
    replace_url(
        &mut draft,
        "bigquery://billing-project/analytics?data_project=data-project&location=US",
    );
    draft.commit_url().unwrap();
    assert!(draft.password().expose_secret().is_empty());
    assert!(!draft.has_stored_credential);
    assert!(draft.ca_cert_path.value().is_empty());
    let bigquery = draft.validate(&[]).unwrap().profile;
    assert!(bigquery.opensearch_options.is_none());
    assert_eq!(bigquery.credential_policy, CredentialPolicy::None);
    assert_eq!(bigquery.catalog_database(), Some("data-project"));

    replace_url(&mut draft, "opensearch://localhost");
    draft.commit_url().unwrap();
    draft.set_opensearch_auth(lazydb::profile::OpenSearchAuth::None);
    let opensearch = draft.validate(&[]).unwrap().profile;
    assert!(opensearch.bigquery_options.is_none());
    assert_eq!(opensearch.credential_policy, CredentialPolicy::None);
    assert!(opensearch.user.is_none());
    assert_eq!(
        opensearch.opensearch_options.as_ref().unwrap().auth,
        lazydb::profile::OpenSearchAuth::None
    );

    let temp = TempDir::new().unwrap();
    let store = ProfileStore::new(temp.path().join("connections.toml"));
    for profile in [original, bigquery, opensearch] {
        store.save(vec![profile.clone()]).unwrap();
        assert_eq!(
            store.load_report().unwrap().collection.profiles,
            vec![profile]
        );
    }
}

#[test]
fn bigquery_draft_validation_reports_specific_fields() {
    for (field, value) in [
        (ProfileField::Database, "bad"),
        (ProfileField::Schema, "invalid/dataset"),
        (ProfileField::DataProject, "invalid.project"),
        (ProfileField::MaximumBytesBilled, "0"),
        (ProfileField::MaximumBytesBilled, "9223372036854775808"),
        (ProfileField::QueryTimeoutSecs, "-1"),
        (ProfileField::QueryTimeoutSecs, "1.5"),
        (ProfileField::QueryTimeoutSecs, "86401"),
    ] {
        let mut draft = ProfileDraft::edit(&profile(), false);
        draft.move_end(field);
        draft.delete_to_start(field);
        draft.paste(field, value);
        assert_eq!(draft.validate(&[]).unwrap_err().field, field);
    }
}

#[test]
fn bigquery_project_edits_preserve_separate_identity_and_explicit_scope() {
    let mut implicit = ProfileDraft::edit(
        &import_connection_url("bigquery://billing-project/analytics", Some("implicit"))
            .unwrap()
            .profile,
        false,
    );
    implicit.paste(ProfileField::Database, "-two");
    let submission = implicit.validate(&[]).unwrap();
    assert!(
        submission
            .profile
            .bigquery_options
            .unwrap()
            .data_project
            .is_none()
    );
    assert!(
        submission
            .profile
            .catalog_scope
            .allows_schema("billing-project-two", "analytics")
    );
    let mut explicit = ProfileDraft::edit(&profile(), false);
    explicit.paste(ProfileField::Database, "-two");
    assert_eq!(
        explicit.validate(&[]).unwrap().profile.catalog_database(),
        Some("data-project")
    );
    explicit.catalog_scope_mode = lazydb::model::profile_manager::CatalogScopeMode::Explicit;
    explicit.catalog_scope =
        CatalogScope::for_profile(DatabaseKind::BigQuery, "other-project", Some("another"));
    explicit.paste(ProfileField::DataProject, "-two");
    assert!(
        explicit
            .validate(&[])
            .unwrap()
            .profile
            .catalog_scope
            .allows_schema("other-project", "another")
    );
}

#[test]
fn bigquery_fingerprints_track_every_option_and_field_edits_invalidate_discovery() {
    let original = profile();
    let original_fingerprint = DiscoveryFingerprint::for_profile(&original, false, 0);
    for (field, suffix) in [
        (ProfileField::Database, "-two"),
        (ProfileField::Schema, "_two"),
        (ProfileField::DataProject, "-two"),
        (ProfileField::Location, "US"),
        (ProfileField::CredentialsFile, "/tmp/adc.json"),
        (ProfileField::MaximumBytesBilled, "0"),
        (ProfileField::QueryTimeoutSecs, "0"),
    ] {
        let mut draft = ProfileDraft::edit(&original, false);
        let fingerprint = draft.validate(&[]).unwrap().discovery_fingerprint;
        let snapshot = ProfileCatalogDiscovery {
            fingerprint,
            server: lazydb::db::ServerInfo {
                kind: DatabaseKind::BigQuery,
                version: "test".into(),
                database: "billing-project".into(),
                current_user: None,
            },
            capabilities: lazydb::db::catalog::CatalogCapabilities {
                namespace_model: lazydb::db::catalog::NamespaceModel::DatabaseAndSchema,
                top_level_groups: vec![],
                column_metadata: Default::default(),
                supports_lazy_children: true,
            },
            discovery: None,
        };
        draft.begin_catalog_discovery(fingerprint);
        draft.apply_catalog_discovery(snapshot.clone());
        assert!(matches!(
            draft.catalog_discovery,
            CatalogDiscoveryState::Fresh(_)
        ));
        draft.paste(field, suffix);
        assert!(matches!(
            draft.catalog_discovery,
            CatalogDiscoveryState::Stale(_)
        ));
        assert!(draft.discovery_fingerprint.is_none());
        let changed = draft.validate(&[]).unwrap().profile;
        assert_ne!(
            DiscoveryFingerprint::for_profile(&changed, false, 0),
            original_fingerprint
        );
        draft.apply_catalog_discovery(snapshot);
        assert!(!matches!(
            draft.catalog_discovery,
            CatalogDiscoveryState::Fresh(_)
        ));
    }
}

#[test]
fn bigquery_numeric_boundaries_accept_api_byte_limit_and_one_day_timeout() {
    let parsed = parse_connection_url("bigquery://billing-project/fixture?maximum_bytes_billed=9223372036854775807&query_timeout_secs=86400").unwrap();
    let options = parsed.bigquery_options.unwrap();
    assert_eq!(options.maximum_bytes_billed, i64::MAX as u64);
    assert_eq!(options.query_timeout_secs, 86_400);
}

#[test]
fn bigquery_options_defaults_match_embedded_configuration_template() {
    let defaults = BigQueryOptions::default();
    let template = include_str!("../config/default.toml");
    assert!(template.contains(&format!(
        "# maximum_bytes_billed = {}",
        defaults.maximum_bytes_billed
    )));
    assert!(template.contains(&format!(
        "# query_timeout_secs = {}",
        defaults.query_timeout_secs
    )));
}
