use lazydb::profile::{
    ConnectionUrlFormat, DatabaseKind, ProfileError, SslMode, format_connection_url,
    import_connection_url, parse_connection_url,
};
use secrecy::ExposeSecret;
use uuid::Uuid;

use lazydb::profile::{ConnectionGroup, ProfileCollection};

#[test]
fn imported_profiles_are_ungrouped() {
    let imported = import_connection_url("postgres://localhost/app", Some("app"))
        .unwrap()
        .profile;
    assert_eq!(imported.group_id, None);
}

#[test]
fn profile_collection_preserves_declared_order() {
    let first = ConnectionGroup::new(Uuid::from_u128(1), "Production").unwrap();
    let second = ConnectionGroup::new(Uuid::from_u128(2), "Development").unwrap();
    let collection = ProfileCollection {
        groups: vec![first.clone(), second.clone()],
        profiles: vec![],
    };
    assert_eq!(collection.groups, vec![first, second]);
}

#[test]
fn parses_all_server_formats_and_connection_settings() {
    let cases = [
        ("postgres://db/app", ConnectionUrlFormat::Postgres),
        ("postgresql://db/app", ConnectionUrlFormat::PostgreSql),
        (
            "jdbc:postgresql://db/app",
            ConnectionUrlFormat::JdbcPostgreSql,
        ),
        ("mysql://db/app", ConnectionUrlFormat::MySql),
        ("jdbc:mysql://db/app", ConnectionUrlFormat::JdbcMySql),
    ];
    for (url, format) in cases {
        assert_eq!(parse_connection_url(url).unwrap().format, format);
    }

    let parsed = parse_connection_url(
        "postgresql://a%20b:p%40ss@db.example:5440/app%20db?currentSchema=my%20schema&sslmode=verify-full&readOnly=true",
    )
    .unwrap();
    assert_eq!(parsed.kind, DatabaseKind::Postgres);
    assert_eq!(parsed.user.as_deref(), Some("a b"));
    assert_eq!(parsed.password.unwrap().expose_secret(), "p@ss");
    assert_eq!(parsed.database.as_deref(), Some("app db"));
    assert_eq!(parsed.default_schema.as_deref(), Some("my schema"));
    assert_eq!(parsed.ssl_mode, SslMode::VerifyFull);
    assert!(parsed.read_only);
}

#[test]
fn parses_sqlite_file_jdbc_and_memory_formats() {
    let cases = [
        ("sqlite:/tmp/a%20b.db", ConnectionUrlFormat::Sqlite, false),
        (
            "file:/tmp/a%20b.db?mode=ro",
            ConnectionUrlFormat::FileUri,
            true,
        ),
        (
            "jdbc:sqlite:relative%20db.sqlite",
            ConnectionUrlFormat::JdbcSqlite,
            false,
        ),
        ("sqlite::memory:", ConnectionUrlFormat::Sqlite, false),
        (":memory:", ConnectionUrlFormat::Sqlite, false),
    ];
    for (url, format, read_only) in cases {
        let parsed = parse_connection_url(url).unwrap();
        assert_eq!(parsed.format, format);
        assert_eq!(parsed.read_only, read_only);
    }
    assert_eq!(
        parse_connection_url("file:///tmp/standard.db")
            .unwrap()
            .sqlite_path
            .unwrap()
            .to_string_lossy(),
        "/tmp/standard.db"
    );
}

#[test]
fn rejects_unknown_conflicting_and_invalid_query_parameters_without_raw_url() {
    let secret = "do-not-leak-this";
    let error = parse_connection_url(&format!(
        "postgresql://user:{secret}@db/app?application_name=lazydb"
    ))
    .unwrap_err();
    assert!(matches!(error, ProfileError::UnknownQueryParameter(_)));
    assert!(error.to_string().contains("application_name"));
    assert!(!error.to_string().contains(secret));

    assert!(matches!(
        parse_connection_url("mysql://db/app?useSSL=true&sslMode=require").unwrap_err(),
        ProfileError::ConflictingQueryParameter(_)
    ));
    assert!(matches!(
        parse_connection_url("file:test.db?mode=invalid").unwrap_err(),
        ProfileError::InvalidQueryParameter(_)
    ));
    assert!(matches!(
        parse_connection_url("mysql://db/app?currentSchema=ignored").unwrap_err(),
        ProfileError::UnknownQueryParameter(_)
    ));
    assert!(matches!(
        parse_connection_url("postgresql://db/app?useSSL=true").unwrap_err(),
        ProfileError::UnknownQueryParameter(_)
    ));
}

#[test]
fn formatter_is_password_free_percent_encoded_and_driver_compatible() {
    let imported = import_connection_url(
        "postgresql://a%20b:secret@db.example:5440/app%20db?currentSchema=my%20schema&sslmode=require&readOnly=true",
        None,
    )
    .unwrap();
    let url =
        format_connection_url(&imported.profile, ConnectionUrlFormat::JdbcPostgreSql).unwrap();
    assert_eq!(
        url,
        "jdbc:postgresql://a%20b@db.example:5440/app%20db?currentSchema=my%20schema&sslmode=require&readOnly=true"
    );
    assert!(!url.contains("secret"));
    assert!(matches!(
        format_connection_url(&imported.profile, ConnectionUrlFormat::MySql),
        Err(ProfileError::IncompatibleFormat)
    ));
}

#[test]
fn formatter_round_trips_sqlite_path_memory_and_read_only() {
    for input in ["file:/tmp/a%20b.db?mode=ro", "jdbc:sqlite::memory:"] {
        let imported = import_connection_url(input, None).unwrap();
        let formatted =
            format_connection_url(&imported.profile, imported.profile.url_format).unwrap();
        let reparsed = parse_connection_url(&formatted).unwrap();
        assert_eq!(reparsed.sqlite_path, imported.profile.sqlite_path);
        assert_eq!(reparsed.database, imported.profile.database);
        assert_eq!(reparsed.read_only, imported.profile.read_only);
    }
}

#[test]
fn formatter_escapes_query_delimiters() {
    let mut imported = import_connection_url("postgresql://db/app", None)
        .unwrap()
        .profile;
    imported.default_schema = Some("a&b=c+d".into());
    let url = format_connection_url(&imported, imported.url_format).unwrap();
    assert!(url.contains("currentSchema=a%26b%3Dc%2Bd"));
    assert_eq!(
        parse_connection_url(&url)
            .unwrap()
            .default_schema
            .as_deref(),
        Some("a&b=c+d")
    );
}
