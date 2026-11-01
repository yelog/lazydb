use lazydb::{
    model::{execution_target::ExecutionTarget, profile_manager::ProfileDraft},
    profile::{
        ConnectionUrlFormat, DatabaseKind, SslMode, format_connection_url, import_connection_url,
        parse_connection_url,
    },
};
use secrecy::ExposeSecret;

#[test]
fn native_urls_round_trip_credentials_ipv6_and_tls() {
    for (url, mode) in [
        (
            "elasticsearch://alice:p%40ss@localhost:9201",
            SslMode::Disable,
        ),
        (
            "elasticsearchs://alice:p%40ss@[::1]:9200/team",
            SslMode::VerifyFull,
        ),
    ] {
        let imported = import_connection_url(url, Some("search")).unwrap();
        let profile = imported.profile;
        assert_eq!(profile.kind, DatabaseKind::Elasticsearch);
        assert!(!profile.kind.is_relational());
        assert_eq!(profile.ssl_mode, mode);
        assert_eq!(imported.transient_password.unwrap().expose_secret(), "p@ss");
        let formatted = format_connection_url(&profile, profile.url_format).unwrap();
        assert!(!formatted.contains("p%40ss"));
        assert!(!formatted.contains("p@ss"));
        let parsed = parse_connection_url(&formatted).unwrap();
        assert_eq!(parsed.host, profile.host);
        assert_eq!(parsed.database, profile.database);
        assert_eq!(parsed.ssl_mode, mode);
        let target = ExecutionTarget::from_profile(&profile);
        assert!(target.is_valid(&profile));
        assert!(target.apply_to_profile(&profile).is_some());
    }
}

#[test]
fn cluster_without_database_has_valid_default_execution_target() {
    let imported = import_connection_url("elasticsearch://localhost", Some("search")).unwrap();
    assert_eq!(imported.profile.port, Some(9200));
    let target = ExecutionTarget::from_profile(&imported.profile);
    assert_eq!(target.database, "default");
    assert_eq!(target.schema.as_deref(), Some("default"));
    assert!(target.is_valid(&imported.profile));
    let draft = ProfileDraft::edit(&imported.profile, false);
    assert_eq!(draft.kind, DatabaseKind::Elasticsearch);
    assert!(draft.validate(&[]).is_ok());
    let mut new = ProfileDraft::new(DatabaseKind::Elasticsearch);
    new.name.set("new-search");
    assert!(new.validate(&[]).is_ok());
}

#[test]
fn invalid_connection_options_are_rejected() {
    for url in [
        "elasticsearch://localhost:0",
        "elasticsearch://localhost?apiKey=secret",
        "elasticsearchs://localhost?sslMode=disable",
        "elasticsearch://localhost#fragment",
    ] {
        assert!(parse_connection_url(url).is_err(), "{url}");
    }
    let mut profile = import_connection_url("elasticsearch://localhost", None)
        .unwrap()
        .profile;
    profile.ssl_mode = SslMode::Disable;
    assert!(format_connection_url(&profile, ConnectionUrlFormat::ElasticsearchTls).is_err());
}
