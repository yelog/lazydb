use lazydb::{
    cli::{LspDialect, capabilities},
    db::descriptor::{DRIVERS, DatabaseCategory, descriptor},
    model::profile_manager::{
        CredentialUpdate, DRIVER_ORDER, ProfileDraft, ProfileField, ProfileManagerState,
    },
    profile::{
        CatalogScope, CatalogSelection, ConnectionProfile, ConnectionUrlFormat, CredentialPolicy,
        DatabaseKind, PasswordStorageChoice, ProfileError, SslMode, format_connection_url,
        import_connection_url, parse_connection_url,
    },
};
use secrecy::ExposeSecret;

fn replace_url(draft: &mut ProfileDraft, url: &str) {
    draft.move_home(ProfileField::Url);
    for _ in 0..draft.url_display().chars().count() {
        draft.delete(ProfileField::Url);
    }
    draft.paste(ProfileField::Url, url);
}

#[test]
fn tidb_urls_have_independent_identity_and_server_defaults() {
    let parsed = parse_connection_url("tidb://localhost/app").unwrap();
    assert_eq!(parsed.kind, DatabaseKind::TiDb);
    assert_eq!(parsed.format, ConnectionUrlFormat::TiDb);
    assert_eq!(parsed.host.as_deref(), Some("localhost"));
    assert_eq!(parsed.port, Some(4000));
    assert_eq!(parsed.user.as_deref(), Some("root"));
    assert_eq!(parsed.database.as_deref(), Some("app"));
    assert_eq!(parsed.default_schema, None);
    assert_eq!(parsed.ssl_mode, SslMode::Prefer);
    assert!(parsed.password.is_none());
    assert!(DatabaseKind::TiDb.is_relational());

    let mysql = parse_connection_url("mysql://localhost/app").unwrap();
    assert_eq!(mysql.kind, DatabaseKind::MySql);
    assert_eq!(mysql.port, Some(3306));
}

#[test]
fn tidb_url_round_trip_preserves_fields_tls_and_read_only_without_password() {
    let imported = import_connection_url(
        "tidb://cloud.user:p%40ss%3Aword@[::1]:4400/app%20data?sslMode=VERIFY_IDENTITY&readOnly=true",
        Some("TiDB Cloud"),
    )
    .unwrap();
    let profile = &imported.profile;
    assert_eq!(profile.name, "TiDB Cloud");
    assert_eq!(profile.kind, DatabaseKind::TiDb);
    assert_eq!(profile.port, Some(4400));
    assert_eq!(profile.user.as_deref(), Some("cloud.user"));
    assert_eq!(profile.database.as_deref(), Some("app data"));
    assert_eq!(profile.ssl_mode, SslMode::VerifyFull);
    assert!(profile.read_only);
    assert!(imported.read_only_explicit);
    assert_eq!(
        imported
            .transient_password
            .as_ref()
            .unwrap()
            .expose_secret(),
        "p@ss:word"
    );
    assert_eq!(profile.credential_policy, CredentialPolicy::None);
    assert!(!format!("{imported:?}").contains("p@ss:word"));

    let formatted = format_connection_url(profile, ConnectionUrlFormat::TiDb).unwrap();
    assert_eq!(
        formatted,
        "tidb://cloud.user@[::1]:4400/app%20data?sslMode=verify-full&readOnly=true"
    );
    let reparsed = parse_connection_url(&formatted).unwrap();
    assert_eq!(reparsed.kind, DatabaseKind::TiDb);
    assert_eq!(reparsed.port, profile.port);
    assert_eq!(reparsed.user, profile.user);
    assert_eq!(reparsed.database, profile.database);
    assert_eq!(reparsed.ssl_mode, profile.ssl_mode);
    assert!(reparsed.read_only);
    assert!(reparsed.password.is_none());
}

#[test]
fn tidb_accepts_mysql_tls_options_and_rejects_conflicts() {
    for (query, ssl_mode) in [
        ("useSSL=false", SslMode::Disable),
        ("useSSL=true", SslMode::Require),
        ("sslMode=PREFERRED", SslMode::Prefer),
        ("sslMode=REQUIRED", SslMode::Require),
        ("sslMode=VERIFY_CA", SslMode::VerifyCa),
        ("sslMode=VERIFY_IDENTITY", SslMode::VerifyFull),
    ] {
        let profile = import_connection_url(&format!("tidb://host/app?{query}"), None)
            .unwrap()
            .profile;
        assert_eq!(profile.ssl_mode, ssl_mode);
        let formatted = format_connection_url(&profile, profile.url_format).unwrap();
        assert_eq!(parse_connection_url(&formatted).unwrap().ssl_mode, ssl_mode);
    }
    assert!(matches!(
        parse_connection_url("tidb://host/app?useSSL=true&sslMode=disable"),
        Err(ProfileError::ConflictingQueryParameter(_))
    ));
    for query in ["sslMode=unknown", "password=secret", "currentSchema=other"] {
        assert!(parse_connection_url(&format!("tidb://host/app?{query}")).is_err());
    }
    assert!(parse_connection_url("jdbc:tidb://host/app").is_err());
}

#[test]
fn tidb_kind_and_native_format_serialize_without_exposing_credentials() {
    let imported = import_connection_url("tidb://root:private-password@host/app", None).unwrap();
    let encoded = serde_json::to_value(&imported.profile).unwrap();
    assert_eq!(encoded["kind"], "tidb");
    assert_eq!(encoded["url_format"], "tidb");
    assert!(!encoded.to_string().contains("private-password"));
    let decoded: ConnectionProfile = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, imported.profile);
    let encoded = toml::to_string(&decoded).unwrap();
    assert!(encoded.contains("kind = \"tidb\""));
    assert!(encoded.contains("url_format = \"tidb\""));
    assert!(!encoded.contains("private-password"));
    assert_eq!(
        toml::from_str::<ConnectionProfile>(&encoded).unwrap(),
        decoded
    );

    assert_eq!(
        ConnectionUrlFormat::default_for(DatabaseKind::TiDb),
        ConnectionUrlFormat::TiDb
    );
    assert_eq!(
        ConnectionUrlFormat::compatible_formats(DatabaseKind::TiDb),
        &[ConnectionUrlFormat::TiDb]
    );
    for format in [
        ConnectionUrlFormat::MySql,
        ConnectionUrlFormat::JdbcMySql,
        ConnectionUrlFormat::MariaDb,
    ] {
        assert!(!format.is_compatible(DatabaseKind::TiDb));
        assert!(matches!(
            format_connection_url(&decoded, format),
            Err(ProfileError::IncompatibleFormat)
        ));
    }
    assert!(!ConnectionUrlFormat::TiDb.is_compatible(DatabaseKind::MySql));
}

#[test]
fn tidb_draft_uses_mysql_fields_and_keeps_tls_secrets_in_credential_updates() {
    let mut draft = ProfileDraft::new(DatabaseKind::TiDb);
    assert_eq!(draft.host.value(), "localhost");
    assert_eq!(draft.port.value(), "4000");
    assert_eq!(draft.user.value(), "root");
    assert_eq!(draft.url_format, ConnectionUrlFormat::TiDb);
    assert_eq!(draft.ssl_mode, SslMode::Prefer);
    assert!(
        draft
            .url_display()
            .starts_with("tidb://root@localhost:4000")
    );
    assert_eq!(
        draft.visible_fields(),
        ProfileDraft::new(DatabaseKind::MySql).visible_fields()
    );
    assert!(!draft.visible_fields().contains(&ProfileField::Schema));
    draft.name.set("TiDB Cloud");
    replace_url(
        &mut draft,
        "tidb://cloud.user:draft-secret@cluster.test/app?sslMode=VERIFY_IDENTITY",
    );
    assert!(!draft.url_display().contains("draft-secret"));
    draft.commit_url().unwrap();
    assert_eq!(draft.kind, DatabaseKind::TiDb);
    assert_eq!(draft.ssl_mode, SslMode::VerifyFull);
    assert_eq!(draft.password().expose_secret(), "draft-secret");

    let submission = draft.validate(&[]).unwrap();
    assert_eq!(submission.profile.ssl_mode, SslMode::VerifyFull);
    assert_eq!(submission.profile.default_schema, None);
    assert_eq!(
        submission.profile.credential_policy,
        CredentialPolicy::Prompt
    );
    assert!(
        matches!(&submission.credential, CredentialUpdate::LocalEncrypted(value) if value.expose_secret() == "draft-secret")
    );
    assert!(!format!("{draft:?} {submission:?}").contains("draft-secret"));
    assert!(
        !toml::to_string(&submission.profile)
            .unwrap()
            .contains("draft-secret")
    );
    assert!(!draft.url_display().contains("draft-secret"));

    draft.password_storage = PasswordStorageChoice::System;
    assert!(matches!(
        draft.validate(&[]).unwrap().credential,
        CredentialUpdate::System(_)
    ));
    let mut profile = submission.profile;
    profile.credential_policy = CredentialPolicy::System("tidb-credential".into());
    let edited = ProfileDraft::edit(&profile, true).validate(&[]).unwrap();
    assert_eq!(edited.profile, profile);
    assert!(matches!(edited.credential, CredentialUpdate::Preserve));
}

#[test]
fn tidb_scope_treats_database_and_schema_as_the_same_namespace() {
    let scope = CatalogScope::for_profile(DatabaseKind::TiDb, "app", Some("ignored"));
    assert!(
        matches!(&scope.databases, CatalogSelection::Selected(items) if items.len() == 1 && items[0].schemas == CatalogSelection::All)
    );
    assert!(scope.allows_schema("app", "app"));
    assert!(!scope.allows_database("other"));

    let mut manager = ProfileManagerState::new(false);
    manager.start_new(DatabaseKind::TiDb);
    let draft = manager.draft.as_mut().unwrap();
    draft.name.set("TiDB");
    draft.paste(ProfileField::Database, "app");
    manager.open_scope_picker();
    let row_id = "database:app:schema:app";
    assert!(manager.scope_row(row_id).unwrap().read_only);
    let before = manager.draft.as_ref().unwrap().catalog_scope.clone();
    assert!(!manager.toggle_scope_row(row_id));
    assert_eq!(manager.draft.as_ref().unwrap().catalog_scope, before);
}

#[test]
fn tidb_registration_preserves_existing_indexes_and_uses_mysql_lsp_dialect() {
    assert_eq!(descriptor(DatabaseKind::TiDb).name, "tidb");
    assert_eq!(descriptor(DatabaseKind::TiDb).display_name, "TiDB");
    assert_eq!(
        descriptor(DatabaseKind::TiDb).category,
        DatabaseCategory::Relational
    );
    assert_eq!(DRIVERS[11].kind, DatabaseKind::TiDb);
    assert_eq!(DRIVER_ORDER[1], DatabaseKind::MySql);
    assert_eq!(DRIVER_ORDER[2], DatabaseKind::MariaDb);
    assert_eq!(DRIVER_ORDER[10], DatabaseKind::ClickHouse);
    assert_eq!(DRIVER_ORDER[11], DatabaseKind::TiDb);
    let capabilities = serde_json::to_value(capabilities()).unwrap();
    assert!(
        capabilities["drivers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "tidb")
    );
    assert_eq!(LspDialect::from(DatabaseKind::TiDb), LspDialect::MySql);
}
