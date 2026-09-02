use lazydb::profile::{
    ConnectionUrlFormat, DatabaseKind, ProfileError, SslMode, format_connection_url,
    import_connection_url, parse_connection_url,
};
use secrecy::ExposeSecret;

#[test]
fn sql_server_kind_and_url_formats_serialize_stably() {
    #[derive(serde::Serialize)]
    struct KindFixture {
        kind: DatabaseKind,
    }

    assert_eq!(
        toml::to_string(&KindFixture {
            kind: DatabaseKind::SqlServer,
        })
        .unwrap()
        .trim(),
        "kind = \"sqlserver\""
    );
    assert_eq!(
        ConnectionUrlFormat::default_for(DatabaseKind::SqlServer),
        ConnectionUrlFormat::SqlServer
    );
    assert_eq!(
        ConnectionUrlFormat::compatible_formats(DatabaseKind::SqlServer),
        &[
            ConnectionUrlFormat::SqlServer,
            ConnectionUrlFormat::MsSql,
            ConnectionUrlFormat::JdbcSqlServer,
        ]
    );
    for format in ConnectionUrlFormat::compatible_formats(DatabaseKind::SqlServer) {
        assert!(format.is_compatible(DatabaseKind::SqlServer));
        assert!(!format.is_compatible(DatabaseKind::Postgres));
    }
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
        ("sqlserver://db/app", ConnectionUrlFormat::SqlServer),
        ("mssql://db/app", ConnectionUrlFormat::MsSql),
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
fn parses_sql_server_uri_settings_and_tls_modes() {
    let parsed = parse_connection_url(
        "sqlserver://sa:s%40cret@localhost:1444/app?schema=dbo&encrypt=true&trustServerCertificate=false&readOnly=true",
    )
    .unwrap();
    assert_eq!(parsed.kind, DatabaseKind::SqlServer);
    assert_eq!(parsed.format, ConnectionUrlFormat::SqlServer);
    assert_eq!(parsed.host.as_deref(), Some("localhost"));
    assert_eq!(parsed.port, Some(1444));
    assert_eq!(parsed.user.as_deref(), Some("sa"));
    assert_eq!(parsed.password.unwrap().expose_secret(), "s@cret");
    assert_eq!(parsed.database.as_deref(), Some("app"));
    assert_eq!(parsed.default_schema.as_deref(), Some("dbo"));
    assert_eq!(parsed.ssl_mode, SslMode::VerifyFull);
    assert!(parsed.read_only);

    let parsed =
        parse_connection_url("mssql://sa@localhost/app?currentSchema=sales&encrypt=false").unwrap();
    assert_eq!(parsed.format, ConnectionUrlFormat::MsSql);
    assert_eq!(parsed.port, Some(1433));
    assert_eq!(parsed.default_schema.as_deref(), Some("sales"));
    assert_eq!(parsed.ssl_mode, SslMode::Disable);
    assert_eq!(
        parse_connection_url("sqlserver://localhost/app?encrypt=true&trustServerCertificate=true")
            .unwrap()
            .ssl_mode,
        SslMode::Require
    );
}

#[test]
fn parses_jdbc_sql_server_semicolon_properties_case_insensitively() {
    let parsed = parse_connection_url(
        "jdbc:SQLSERVER://localhost:1433;DatabaseName={app;archive};USER=sa;PASSWORD={s;ec}}ret};ENCRYPT=true;TrustServerCertificate=false;CurrentSchema=sales;ReadOnly=true",
    )
    .unwrap();

    assert_eq!(parsed.kind, DatabaseKind::SqlServer);
    assert_eq!(parsed.format, ConnectionUrlFormat::JdbcSqlServer);
    assert_eq!(parsed.host.as_deref(), Some("localhost"));
    assert_eq!(parsed.port, Some(1433));
    assert_eq!(parsed.database.as_deref(), Some("app;archive"));
    assert_eq!(parsed.user.as_deref(), Some("sa"));
    assert_eq!(parsed.password.unwrap().expose_secret(), "s;ec}ret");
    assert_eq!(parsed.default_schema.as_deref(), Some("sales"));
    assert_eq!(parsed.ssl_mode, SslMode::VerifyFull);
    assert!(parsed.read_only);
}

#[test]
fn sql_server_rejects_duplicate_conflicting_unknown_and_unsupported_properties() {
    for input in [
        "sqlserver://db/app?schema=dbo&currentSchema=sales",
        "sqlserver://db/app?encrypt=true&encrypt=false",
        "jdbc:sqlserver://db;databaseName=one;DATABASENAME=two",
        "jdbc:sqlserver://db;user=one;USER=two",
        "sqlserver://db/app?trustServerCertificate=true",
        "sqlserver://db/app?encrypt=false&trustServerCertificate=true",
    ] {
        assert!(matches!(
            parse_connection_url(input).unwrap_err(),
            ProfileError::ConflictingQueryParameter(_)
        ));
    }
    assert!(matches!(
        parse_connection_url("jdbc:sqlserver://db;applicationName=lazydb").unwrap_err(),
        ProfileError::UnknownQueryParameter(_)
    ));
    for property in [
        "integratedSecurity=true",
        "authentication=ActiveDirectoryPassword",
        "instanceName=SQLEXPRESS",
        "accessToken=token",
    ] {
        let error = parse_connection_url(&format!("jdbc:sqlserver://db;{property}")).unwrap_err();
        assert!(matches!(error, ProfileError::UnsupportedProperty(_)));
        assert!(error.to_string().contains("username/password"));
        assert!(error.to_string().contains("explicit TCP host and port"));
    }
    assert!(matches!(
        parse_connection_url("jdbc:sqlserver://db\\SQLEXPRESS;databaseName=app").unwrap_err(),
        ProfileError::UnsupportedProperty(_)
    ));
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

#[test]
fn sql_server_formatter_preserves_uri_formats_and_round_trips_without_password() {
    for input in [
        "sqlserver://sa:secret@localhost:1433/app?schema=dbo&encrypt=true&trustServerCertificate=false&readOnly=true",
        "mssql://sa:secret@localhost/app?schema=sales&encrypt=false",
    ] {
        let imported = import_connection_url(input, None).unwrap();
        let formatted =
            format_connection_url(&imported.profile, imported.profile.url_format).unwrap();
        assert!(!formatted.contains("secret"));
        let reparsed = parse_connection_url(&formatted).unwrap();
        assert_eq!(reparsed.format, imported.profile.url_format);
        assert_eq!(reparsed.host, imported.profile.host);
        assert_eq!(reparsed.port, imported.profile.port);
        assert_eq!(reparsed.user, imported.profile.user);
        assert_eq!(reparsed.database, imported.profile.database);
        assert_eq!(reparsed.default_schema, imported.profile.default_schema);
        assert_eq!(reparsed.ssl_mode, imported.profile.ssl_mode);
        assert_eq!(reparsed.read_only, imported.profile.read_only);
    }
}

#[test]
fn jdbc_sql_server_formatter_emits_semicolon_properties_and_round_trips() {
    let imported = import_connection_url(
        "jdbc:sqlserver://localhost:1433;databaseName={app;archive};user={domain;user};password={never;print};encrypt=true;trustServerCertificate=true;currentSchema={sales=west};readOnly=true",
        None,
    )
    .unwrap();
    let formatted = format_connection_url(&imported.profile, imported.profile.url_format).unwrap();
    assert_eq!(
        formatted,
        "jdbc:sqlserver://localhost:1433;databaseName={app;archive};user={domain;user};currentSchema={sales=west};encrypt=true;trustServerCertificate=true;readOnly=true"
    );
    assert!(!formatted.contains("never"));

    let reparsed = parse_connection_url(&formatted).unwrap();
    assert_eq!(reparsed.format, ConnectionUrlFormat::JdbcSqlServer);
    assert_eq!(reparsed.database, imported.profile.database);
    assert_eq!(reparsed.user, imported.profile.user);
    assert_eq!(reparsed.default_schema, imported.profile.default_schema);
    assert_eq!(reparsed.ssl_mode, imported.profile.ssl_mode);
    assert_eq!(reparsed.read_only, imported.profile.read_only);
    assert!(reparsed.password.is_none());
}
