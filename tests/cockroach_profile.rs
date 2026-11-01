use std::{fs, path::Path, process::Command};

use lazydb::{
    model::profile_manager::{
        DiscoveryFingerprint, ProfileDraft, ProfileField, ProfileManagerState,
    },
    persistence::profiles::ProfileStore,
    profile::{
        ConnectionProfile, ConnectionUrlFormat, DatabaseKind, PgConnectionOptions, ProfileError,
        SslMode, format_connection_url, import_connection_url, import_connection_url_with_driver,
        parse_connection_url, parse_connection_url_with_driver,
    },
};
use secrecy::ExposeSecret;
use tempfile::tempdir;

const TLS_URL: &str = "cockroachdb://a%20b:p%40ss@db.example:26258/app%2Fdb?currentSchema=my%20schema&sslmode=verify-full&sslrootcert=%2Fmissing%20certs%2F根%25.crt&sslcert=client%2Bcert.crt&sslkey=client%26key%3D1.key&options=--cluster%3Dteam%3Atest%20-c%20application_name%3Dlazy%2Bdb&readOnly=true";

fn replace_url(draft: &mut ProfileDraft, url: &str) {
    draft.move_end(ProfileField::Url);
    draft.delete_to_start(ProfileField::Url);
    draft.paste(ProfileField::Url, url);
}

#[test]
fn cockroach_identity_and_defaults_are_distinct_from_postgres() {
    assert_eq!(
        serde_json::to_string(&DatabaseKind::CockroachDb).unwrap(),
        "\"cockroachdb\""
    );
    assert_eq!(
        serde_json::to_string(&ConnectionUrlFormat::CockroachDb).unwrap(),
        "\"cockroachdb\""
    );
    assert!(DatabaseKind::CockroachDb.is_relational());
    assert!(DatabaseKind::CockroachDb.is_postgres_compatible());
    assert_eq!(
        ConnectionUrlFormat::default_for(DatabaseKind::CockroachDb),
        ConnectionUrlFormat::CockroachDb
    );
    for format in ConnectionUrlFormat::compatible_formats(DatabaseKind::CockroachDb) {
        assert!(format.is_compatible(DatabaseKind::CockroachDb));
    }
    assert!(!ConnectionUrlFormat::CockroachDb.is_compatible(DatabaseKind::Postgres));

    let cockroach = parse_connection_url("cockroachdb://localhost").unwrap();
    assert_eq!(cockroach.kind, DatabaseKind::CockroachDb);
    assert_eq!(cockroach.port, Some(26257));
    assert_eq!(cockroach.database.as_deref(), Some("defaultdb"));
    let postgres = parse_connection_url("postgresql://localhost").unwrap();
    assert_eq!(postgres.kind, DatabaseKind::Postgres);
    assert_eq!(postgres.port, Some(5432));
    assert!(postgres.database.is_none());
}

#[test]
fn explicit_driver_resolves_pg_url_defaults_before_import() {
    for scheme in ["postgres", "postgresql", "jdbc:postgresql", "cockroachdb"] {
        let input = format!("{scheme}://db.example");
        let parsed =
            parse_connection_url_with_driver(&input, Some(DatabaseKind::CockroachDb)).unwrap();
        assert_eq!(parsed.kind, DatabaseKind::CockroachDb);
        assert_eq!(parsed.port, Some(26257));
        assert_eq!(parsed.database.as_deref(), Some("defaultdb"));
        let imported = import_connection_url_with_driver(
            &input,
            Some("crdb"),
            Some(DatabaseKind::CockroachDb),
        )
        .unwrap();
        assert_eq!(imported.profile.kind, DatabaseKind::CockroachDb);
        assert!(imported.profile.catalog_scope.allows_database("defaultdb"));
        let explicit = parse_connection_url_with_driver(
            &format!("{scheme}://db.example:5433/app"),
            Some(DatabaseKind::CockroachDb),
        )
        .unwrap();
        assert_eq!(explicit.port, Some(5433));
        assert_eq!(explicit.database.as_deref(), Some("app"));
    }
    assert!(matches!(
        parse_connection_url_with_driver("mysql://db/app", Some(DatabaseKind::CockroachDb)),
        Err(ProfileError::IncompatibleFormat)
    ));
}

#[test]
fn percent_encoded_tls_paths_and_options_round_trip_without_reading_files() {
    let imported = import_connection_url(TLS_URL, Some("secure")).unwrap();
    assert_eq!(
        imported
            .transient_password
            .as_ref()
            .unwrap()
            .expose_secret(),
        "p@ss"
    );
    let profile = imported.profile;
    let expected = PgConnectionOptions {
        ssl_root_cert: Some("/missing certs/根%.crt".into()),
        ssl_client_cert: Some("client+cert.crt".into()),
        ssl_client_key: Some("client&key=1.key".into()),
        options: Some("--cluster=team:test -c application_name=lazy+db".into()),
    };
    assert_eq!(profile.pg_options.as_ref(), Some(&expected));
    assert_eq!(profile.user.as_deref(), Some("a b"));
    assert_eq!(profile.database.as_deref(), Some("app/db"));
    assert_eq!(profile.default_schema.as_deref(), Some("my schema"));
    assert_eq!(profile.ssl_mode, SslMode::VerifyFull);
    assert!(profile.read_only);
    let persisted = toml::to_string(&profile).unwrap();
    let restored: ConnectionProfile = toml::from_str(&persisted).unwrap();
    assert_eq!(restored, profile);
    for format in ConnectionUrlFormat::compatible_formats(DatabaseKind::CockroachDb) {
        let output = format_connection_url(&restored, *format).unwrap();
        assert!(!output.contains("p%40ss"));
        assert!(output.contains("sslcert=client%2Bcert.crt"));
        assert!(output.contains("sslkey=client%26key%3D1.key"));
        let parsed =
            parse_connection_url_with_driver(&output, Some(DatabaseKind::CockroachDb)).unwrap();
        assert_eq!(parsed.pg_options.as_ref(), Some(&expected));
        assert_eq!(parsed.database, profile.database);
        assert_eq!(parsed.user, profile.user);
        assert_eq!(parsed.port, profile.port);
        assert_eq!(parsed.ssl_mode, profile.ssl_mode);
        assert_eq!(parsed.default_schema, profile.default_schema);
        assert!(parsed.password.is_none());
    }
}

#[test]
fn postgres_profiles_also_retain_protocol_options_and_reject_unknown_or_duplicate_keys() {
    let imported =
        import_connection_url(&TLS_URL.replacen("cockroachdb:", "postgresql:", 1), None).unwrap();
    assert_eq!(imported.profile.kind, DatabaseKind::Postgres);
    let output = format_connection_url(&imported.profile, ConnectionUrlFormat::PostgreSql).unwrap();
    assert_eq!(
        parse_connection_url(&output).unwrap().pg_options,
        imported.profile.pg_options
    );
    for input in [
        "cockroachdb://db?application_name=unsupported",
        "cockroachdb://db?sslrootcert=a&SSLROOTCERT=b",
        "cockroachdb://db?sslcert=a&sslcert=b",
        "cockroachdb://db?sslkey=a&sslkey=b",
        "cockroachdb://db?options=a&OPTIONS=b",
        "mysql://db?sslrootcert=a",
    ] {
        assert!(parse_connection_url(input).is_err(), "accepted {input}");
    }
}

#[test]
fn legacy_profiles_without_pg_options_still_deserialize() {
    let profile = import_connection_url("postgresql://db/app", None)
        .unwrap()
        .profile;
    let mut value = serde_json::to_value(&profile).unwrap();
    value.as_object_mut().unwrap().remove("pg_options");
    let restored: ConnectionProfile = serde_json::from_value(value).unwrap();
    assert_eq!(restored, profile);
    assert!(restored.pg_options.is_none());
}

#[test]
fn cockroach_draft_defaults_and_pg_paste_preserve_explicit_identity() {
    let mut draft = ProfileDraft::new(DatabaseKind::CockroachDb);
    draft.name.set("crdb");
    assert_eq!(draft.port.value(), "26257");
    assert_eq!(draft.database.value(), "defaultdb");
    assert_eq!(draft.schema.value(), "public");
    assert!(draft.visible_fields().contains(&ProfileField::Schema));
    assert!(
        draft
            .url_display()
            .starts_with("cockroachdb://localhost:26257/defaultdb")
    );
    for scheme in ["postgres", "postgresql", "jdbc:postgresql"] {
        replace_url(
            &mut draft,
            &format!(
                "{scheme}://root@db.example?sslmode=verify-full&sslrootcert=ca.crt&options=--cluster%3Dteam"
            ),
        );
        draft.commit_url().unwrap();
        let profile = draft.validate(&[]).unwrap().profile;
        assert_eq!(profile.kind, DatabaseKind::CockroachDb);
        assert_eq!(profile.port, Some(26257));
        assert_eq!(profile.database.as_deref(), Some("defaultdb"));
        assert_eq!(
            profile
                .pg_options
                .as_ref()
                .unwrap()
                .ssl_root_cert
                .as_deref(),
            Some(Path::new("ca.crt"))
        );
        assert_eq!(
            profile.pg_options.as_ref().unwrap().options.as_deref(),
            Some("--cluster=team")
        );
    }
}

#[test]
fn draft_edit_save_export_and_driver_switch_keep_tls_options() {
    let profile = import_connection_url(TLS_URL, Some("secure"))
        .unwrap()
        .profile;
    let mut manager = ProfileManagerState::new(false);
    manager.start_edit(&profile, false);
    let draft = manager.draft.as_mut().unwrap();
    draft.move_end(ProfileField::Host);
    draft.insert(ProfileField::Host, '2');
    let saved = draft.validate(&[]).unwrap().profile;
    assert_eq!(saved.host.as_deref(), Some("db.example2"));
    assert_eq!(saved.pg_options, profile.pg_options);
    assert_eq!(
        parse_connection_url(&draft.url_display())
            .unwrap()
            .pg_options,
        profile.pg_options
    );

    manager.select_driver(DatabaseKind::MySql);
    assert_eq!(
        manager.draft.as_ref().unwrap().pg_options,
        profile.pg_options
    );
    manager.select_driver(DatabaseKind::CockroachDb);
    let saved = manager
        .draft
        .as_ref()
        .unwrap()
        .validate(&[])
        .unwrap()
        .profile;
    assert_eq!(saved.pg_options, profile.pg_options);
    assert_eq!(
        parse_connection_url(&manager.draft.as_ref().unwrap().url_display())
            .unwrap()
            .pg_options,
        profile.pg_options
    );

    // URL-based target selection also preserves inactive protocol options.
    let draft = manager.draft.as_mut().unwrap();
    replace_url(draft, "mysql://db.example/app");
    draft.commit_url().unwrap();
    assert_eq!(draft.kind, DatabaseKind::MySql);
    manager.select_driver(DatabaseKind::CockroachDb);
    assert_eq!(
        manager
            .draft
            .as_ref()
            .unwrap()
            .validate(&[])
            .unwrap()
            .profile
            .pg_options,
        profile.pg_options
    );
}

#[test]
fn tls_paths_and_startup_options_change_discovery_fingerprint() {
    let original = import_connection_url(TLS_URL, None).unwrap().profile;
    let baseline = DiscoveryFingerprint::for_profile(&original, false, 0);
    for field in ["root", "cert", "key", "options"] {
        let mut changed = original.clone();
        let options = changed.pg_options.as_mut().unwrap();
        match field {
            "root" => options.ssl_root_cert = Some("other-ca.crt".into()),
            "cert" => options.ssl_client_cert = Some("other-client.crt".into()),
            "key" => options.ssl_client_key = Some("other-client.key".into()),
            _ => options.options = Some("--cluster=other".into()),
        }
        assert_ne!(
            DiscoveryFingerprint::for_profile(&changed, false, 0),
            baseline
        );
    }
}

#[test]
fn connection_add_driver_flag_persists_cockroach_identity_tls_and_defaults() {
    let temp = tempdir().unwrap();
    let config = temp.path().join("connections.toml");
    let output = Command::new(env!("CARGO_BIN_EXE_lazydb"))
        .arg("--config").arg(&config)
        .args([
            "connections", "add", "--name", "crdb", "--driver", "cockroachdb",
            "--url", "postgresql://root@db.example?sslmode=verify-full&sslrootcert=%2Fmissing%20ca.crt&sslcert=client.crt&sslkey=client.key&options=--cluster%3Dtest",
            "--scope", "global", "--json",
        ])
        .env("LAZYDB_CONFIG_HOME", temp.path())
        .current_dir(temp.path())
        .output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["data"]["kind"], "cockroachdb");
    assert_eq!(value["data"]["port"], 26257);
    assert_eq!(value["data"]["database"], "defaultdb");
    let profiles = ProfileStore::new(config.clone()).load().unwrap();
    let profile = &profiles.profiles[0];
    assert_eq!(profile.kind, DatabaseKind::CockroachDb);
    assert_eq!(
        profile
            .pg_options
            .as_ref()
            .unwrap()
            .ssl_root_cert
            .as_deref(),
        Some(Path::new("/missing ca.crt"))
    );
    assert_eq!(
        profile.pg_options.as_ref().unwrap().options.as_deref(),
        Some("--cluster=test")
    );
    assert!(fs::read_to_string(config).unwrap().contains("cockroachdb"));
}

#[test]
fn connection_add_rejects_incompatible_explicit_driver_without_writing_profile() {
    let temp = tempdir().unwrap();
    let config = temp.path().join("connections.toml");
    let output = Command::new(env!("CARGO_BIN_EXE_lazydb"))
        .arg("--config")
        .arg(&config)
        .args([
            "connections",
            "add",
            "--name",
            "bad",
            "--driver",
            "cockroachdb",
            "--url",
            "mysql://db/app",
            "--scope",
            "global",
            "--json",
        ])
        .env("LAZYDB_CONFIG_HOME", temp.path())
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["code"], "invalid_profile");
    assert!(!config.exists());
}
