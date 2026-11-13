use std::{fs, path::Path, process::Command};

use clap::Parser;
use lazydb::{
    cli::{Cli, ConnectionsCommand},
    config::{AppConfig, ConfigError, DEFAULT_CONFIG_TOML, SshConfig},
    model::profile_manager::{
        DiscoveryFingerprint, ProfileDraft, ProfileField, ProfileManagerState,
    },
    persistence::profiles::{PersistenceError, ProfileStore},
    profile::{ConnectionProfile, DatabaseKind, SshOptions, import_connection_url},
};
use serde_json::Value;
use tempfile::tempdir;

fn ssh_options() -> SshOptions {
    SshOptions {
        host: "bastion.example.com".into(),
        port: Some(2222),
        user: Some("alice".into()),
        identity_file: Some("/home/alice/.ssh/id_ed25519".into()),
        config_file: Some("/home/alice/.ssh/config".into()),
        known_hosts_file: Some("/home/alice/.ssh/known_hosts".into()),
        proxy_jump: Some("alice@jump.example.com:2200,ops@[2001:db8::1]:22".into()),
    }
}

fn postgres_profile() -> ConnectionProfile {
    import_connection_url("postgresql://localhost:5432/app", Some("app"))
        .unwrap()
        .profile
}

fn run_connection_add(config: &Path, project: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_lazydb"))
        .arg("--config")
        .arg(config)
        .arg("connections")
        .args(args)
        .env("LAZYDB_CONFIG_HOME", config.parent().unwrap())
        .current_dir(project)
        .output()
        .unwrap()
}

#[test]
fn ssh_options_validate_host_user_jump_port_and_paths() {
    assert_eq!(ssh_options().validate(), Ok(()));

    for host in [
        "",
        "-oProxyCommand=bad",
        "host name",
        "host\nname",
        "host\rname",
        "host\0name",
        "host/name",
    ] {
        let mut options = ssh_options();
        options.host = host.into();
        assert!(options.validate().is_err(), "accepted host {host:?}");
    }

    let mut options = ssh_options();
    options.port = Some(0);
    assert!(options.validate().is_err());

    for user in [
        "",
        "-oProxyCommand=bad",
        "alice bob",
        "alice@example.com",
        "alice\n",
        "alice\r",
        "alice\0",
    ] {
        let mut options = ssh_options();
        options.user = Some(user.into());
        assert!(options.validate().is_err(), "accepted user {user:?}");
    }

    for proxy_jump in [
        "",
        "-J bastion",
        "alice@",
        "@bastion",
        "alice@@bastion",
        "bastion:0",
        "bastion:not-a-port",
        "bastion,,jump",
        "bastion jump",
        "bastion\njump",
        "[2001:db8::1",
    ] {
        let mut options = ssh_options();
        options.proxy_jump = Some(proxy_jump.into());
        assert!(
            options.validate().is_err(),
            "accepted proxy jump {proxy_jump:?}"
        );
    }

    for path in ["", "-identity", "line\nbreak", "nul\0byte"] {
        let mut options = ssh_options();
        options.identity_file = Some(path.into());
        assert!(options.validate().is_err(), "accepted path {path:?}");
    }
}

#[test]
fn profile_ssh_validation_matches_the_initially_supported_drivers() {
    let mut profile = postgres_profile();
    profile.ssh = Some(ssh_options());
    assert_eq!(profile.validate_ssh(), Ok(()));

    for kind in [
        DatabaseKind::BigQuery,
        DatabaseKind::Snowflake,
        DatabaseKind::Sqlite,
        DatabaseKind::DuckDb,
        DatabaseKind::Oracle,
    ] {
        profile.kind = kind;
        assert!(profile.validate_ssh().is_err(), "accepted {kind:?}");
    }

    profile.ssh = None;
    assert_eq!(profile.validate_ssh(), Ok(()));
}

#[test]
fn profile_ssh_round_trips_at_version_six_and_clears_stale_toml() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("connections.toml");
    let store = ProfileStore::new(path.clone());
    let mut profile = postgres_profile();
    profile.ssh = Some(ssh_options());

    store.save(vec![profile.clone()]).unwrap();
    let contents = fs::read_to_string(&path).unwrap();
    assert!(contents.contains("version = 6"));
    assert!(contents.contains("[profiles.ssh]"));
    assert_eq!(store.load().unwrap(), vec![profile.clone()]);

    profile.ssh = None;
    store.save(vec![profile.clone()]).unwrap();
    let contents = fs::read_to_string(&path).unwrap();
    assert!(!contents.contains("[profiles.ssh]"));
    assert_eq!(store.load().unwrap(), vec![profile]);
}

#[derive(serde::Serialize)]
struct ProfileFileFixture<'a> {
    version: u16,
    profiles: &'a [ConnectionProfile],
}

#[test]
fn unsupported_ssh_profile_is_loaded_as_unavailable_instead_of_failing_the_file() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("connections.toml");
    let mut profile = postgres_profile();
    profile.kind = DatabaseKind::Sqlite;
    profile.ssh = Some(ssh_options());
    fs::write(
        &path,
        toml::to_string_pretty(&ProfileFileFixture {
            version: 6,
            profiles: &[profile],
        })
        .unwrap(),
    )
    .unwrap();

    let report = ProfileStore::new(path).load_report().unwrap();

    assert!(report.collection.profiles.is_empty());
    assert_eq!(report.unavailable.len(), 1);
    assert_eq!(report.unavailable[0].kind.as_deref(), Some("sqlite"));
}

#[test]
fn profile_ssh_deserialization_defaults_omissions_and_rejects_unknown_fields() {
    let mut value = serde_json::to_value(postgres_profile()).unwrap();
    value.as_object_mut().unwrap().remove("ssh");
    let profile: ConnectionProfile = serde_json::from_value(value.clone()).unwrap();
    assert!(profile.ssh.is_none());

    value["ssh"] = serde_json::to_value(ssh_options()).unwrap();
    let profile: ConnectionProfile = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(profile.ssh, Some(ssh_options()));

    value["ssh"]["unknown"] = serde_json::Value::Bool(true);
    assert!(serde_json::from_value::<ConnectionProfile>(value).is_err());
}

#[test]
fn ssh_changes_invalidate_the_discovery_fingerprint() {
    let mut profile = postgres_profile();
    let baseline = DiscoveryFingerprint::for_profile(&profile, false, 0);
    profile.ssh = Some(ssh_options());
    let with_ssh = DiscoveryFingerprint::for_profile(&profile, false, 0);
    assert_ne!(baseline, with_ssh);

    profile.ssh.as_mut().unwrap().user = Some("bob".into());
    assert_ne!(
        with_ssh,
        DiscoveryFingerprint::for_profile(&profile, false, 0)
    );
}

#[test]
fn profile_draft_round_trips_ssh_and_clears_it_for_unsupported_drivers() {
    let mut draft = ProfileDraft::new(DatabaseKind::Postgres);
    assert!(draft.visible_fields().contains(&ProfileField::SshEnabled));
    assert!(draft.visible_fields().contains(&ProfileField::SshHost));
    draft.name.set("tunneled");
    draft.database.set("app");
    draft.ssh_enabled = true;
    draft.ssh_host.set("bastion.example.com");
    draft.ssh_user.set("alice");
    draft.ssh_port.set("2222");
    draft.ssh_identity_file.set("/home/alice/.ssh/id_ed25519");
    draft.ssh_config_file.set("/home/alice/.ssh/config");
    draft
        .ssh_known_hosts_file
        .set("/home/alice/.ssh/known_hosts");
    draft.ssh_proxy_jump.set("alice@jump.example.com:2200");
    draft.ssh_host.set("-oProxyCommand=bad");
    assert_eq!(
        draft.validate(&[]).unwrap_err().field,
        ProfileField::SshHost
    );
    draft.ssh_host.set("bastion.example.com");

    let profile = draft.validate(&[]).unwrap().profile;
    assert_eq!(profile.ssh, Some(ssh_options_without_second_hop()));

    let mut edited = ProfileDraft::edit(&profile, false);
    assert!(edited.ssh_enabled);
    assert_eq!(edited.ssh_host.value(), "bastion.example.com");
    assert_eq!(edited.ssh_port.value(), "2222");
    edited.ssh_enabled = false;
    assert!(edited.validate(&[]).unwrap().profile.ssh.is_none());

    let mut manager = ProfileManagerState::new(false);
    manager.start_new(DatabaseKind::Postgres);
    {
        let draft = manager.draft.as_mut().unwrap();
        draft.ssh_enabled = true;
        draft.ssh_host.set("bastion.example.com");
        draft.ssh_user.set("alice");
    }
    manager.select_driver(DatabaseKind::BigQuery);
    let draft = manager.draft.as_ref().unwrap();
    assert!(!draft.ssh_enabled);
    assert!(draft.ssh_host.value().is_empty());
    assert!(draft.ssh_user.value().is_empty());
    assert!(!draft.visible_fields().contains(&ProfileField::SshHost));
}

fn ssh_options_without_second_hop() -> SshOptions {
    SshOptions {
        proxy_jump: Some("alice@jump.example.com:2200".into()),
        ..ssh_options()
    }
}

fn valid_ssh_draft() -> ProfileDraft {
    let mut draft = ProfileDraft::new(DatabaseKind::Postgres);
    draft.name.set("tunneled");
    draft.database.set("app");
    draft.ssh_enabled = true;
    draft.ssh_host.set("bastion.example.com");
    draft.ssh_user.set("alice");
    draft.ssh_port.set("2222");
    draft.ssh_identity_file.set("/home/alice/.ssh/id_ed25519");
    draft.ssh_config_file.set("/home/alice/.ssh/config");
    draft
        .ssh_known_hosts_file
        .set("/home/alice/.ssh/known_hosts");
    draft.ssh_proxy_jump.set("alice@jump.example.com:2200");
    draft
}

fn replace_draft_url(draft: &mut ProfileDraft, url: &str) {
    draft.move_home(ProfileField::Url);
    for _ in 0..draft.url_display().chars().count() {
        draft.delete(ProfileField::Url);
    }
    draft.paste(ProfileField::Url, url);
}

#[test]
fn profile_draft_rejects_hostile_ssh_values_without_trimming_them() {
    let mut draft = valid_ssh_draft();
    assert_eq!(
        draft.validate(&[]).unwrap().profile.ssh,
        Some(ssh_options_without_second_hop())
    );

    draft.ssh_host.set("bastion.example.com\n");
    assert_eq!(
        draft.validate(&[]).unwrap_err().field,
        ProfileField::SshHost
    );
    draft.ssh_host.set("bastion.example.com");

    draft.ssh_user.set("alice\n");
    assert_eq!(
        draft.validate(&[]).unwrap_err().field,
        ProfileField::SshUser
    );
    draft.ssh_user.set("alice");

    draft.ssh_port.set(" 2222");
    assert_eq!(
        draft.validate(&[]).unwrap_err().field,
        ProfileField::SshPort
    );
    draft.ssh_port.set("2222");

    draft.ssh_proxy_jump.set("alice@jump.example.com\n");
    assert_eq!(
        draft.validate(&[]).unwrap_err().field,
        ProfileField::SshProxyJump
    );
    draft.ssh_proxy_jump.set("alice@jump.example.com:2200");

    draft.ssh_identity_file.set("/home/alice/.ssh/id_ed25519\n");
    assert_eq!(
        draft.validate(&[]).unwrap_err().field,
        ProfileField::SshIdentityFile
    );
    draft.ssh_identity_file.set("/home/alice/.ssh/id_ed25519");

    draft.ssh_config_file.set("-oProxyCommand=bad");
    assert_eq!(
        draft.validate(&[]).unwrap_err().field,
        ProfileField::SshConfigFile
    );
}

#[test]
fn selecting_an_unsupported_driver_from_the_url_clears_ssh_draft_fields() {
    let mut draft = valid_ssh_draft();
    replace_draft_url(&mut draft, "sqlite::memory:");

    draft.commit_url().unwrap();

    assert_eq!(draft.kind, DatabaseKind::Sqlite);
    assert!(!draft.ssh_enabled);
    assert!(draft.ssh_host.value().is_empty());
    assert!(draft.ssh_user.value().is_empty());
    assert!(draft.ssh_port.value().is_empty());
    assert!(draft.ssh_identity_file.value().is_empty());
    assert!(draft.ssh_config_file.value().is_empty());
    assert!(draft.ssh_proxy_jump.value().is_empty());
    assert!(draft.ssh_known_hosts_file.value().is_empty());
}

#[test]
fn toggling_ssh_invalidates_catalog_discovery() {
    let mut manager = ProfileManagerState::new(false);
    manager.start_new(DatabaseKind::Postgres);
    let fingerprint = DiscoveryFingerprint::for_profile(&postgres_profile(), false, 0);
    manager
        .draft
        .as_mut()
        .unwrap()
        .begin_catalog_discovery(fingerprint);
    manager.selected_field = ProfileField::SshEnabled;

    manager.toggle();

    let draft = manager.draft.as_ref().unwrap();
    assert!(draft.ssh_enabled);
    assert!(draft.discovery_fingerprint.is_none());
}

#[test]
fn persistence_rejects_ssh_for_unsupported_drivers() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("connections.toml");
    let mut profile = import_connection_url("sqlite::memory:", Some("sqlite"))
        .unwrap()
        .profile;
    profile.ssh = Some(ssh_options());

    assert!(matches!(
        ProfileStore::new(path).save(vec![profile]),
        Err(PersistenceError::InvalidSshConfiguration { .. })
    ));
}

#[test]
fn ssh_config_defaults_and_bounds_come_from_the_embedded_configuration() {
    assert_eq!(
        AppConfig::from_toml(DEFAULT_CONFIG_TOML)
            .unwrap()
            .connections
            .ssh,
        SshConfig::default()
    );
    assert_eq!(
        SshConfig::default(),
        SshConfig {
            connect_timeout_seconds: 30,
            server_alive_interval_seconds: 30,
            server_alive_count_max: 3,
        }
    );
    for (from, to) in [
        (
            "connect_timeout_seconds = 30",
            "connect_timeout_seconds = 0",
        ),
        (
            "connect_timeout_seconds = 30",
            "connect_timeout_seconds = 301",
        ),
        (
            "server_alive_interval_seconds = 30",
            "server_alive_interval_seconds = 3601",
        ),
        ("server_alive_count_max = 3", "server_alive_count_max = 11"),
    ] {
        let contents = DEFAULT_CONFIG_TOML.replace(from, to);
        assert!(matches!(
            AppConfig::from_toml(&contents),
            Err(ConfigError::InvalidSshSettings)
        ));
    }
}

#[test]
fn cli_parses_ssh_flags_and_rejects_no_ssh_conflicts() {
    let cli = Cli::try_parse_from([
        "lazydb",
        "connections",
        "add",
        "--name",
        "tunneled",
        "--url",
        "postgresql://localhost/app",
        "--ssh-host",
        "bastion",
        "--ssh-user",
        "alice",
        "--ssh-port",
        "2222",
        "--ssh-identity-file",
        "/tmp/id",
        "--ssh-config-file",
        "/tmp/config",
        "--ssh-proxy-jump",
        "jump",
        "--ssh-known-hosts-file",
        "/tmp/known_hosts",
    ])
    .unwrap();
    let Some(lazydb::cli::Command::Connections { command, .. }) = cli.command else {
        panic!("connections add did not parse");
    };
    let ConnectionsCommand::Add {
        ssh_host,
        ssh_user,
        ssh_port,
        ssh_identity_file,
        ssh_config_file,
        ssh_proxy_jump,
        ssh_known_hosts_file,
        ..
    } = command
    else {
        panic!("expected connections add");
    };
    assert_eq!(ssh_host.as_deref(), Some("bastion"));
    assert_eq!(ssh_user.as_deref(), Some("alice"));
    assert_eq!(ssh_port, Some(2222));
    assert_eq!(ssh_identity_file.as_deref(), Some(Path::new("/tmp/id")));
    assert_eq!(ssh_config_file.as_deref(), Some(Path::new("/tmp/config")));
    assert_eq!(ssh_proxy_jump.as_deref(), Some("jump"));
    assert_eq!(
        ssh_known_hosts_file.as_deref(),
        Some(Path::new("/tmp/known_hosts"))
    );

    assert!(
        Cli::try_parse_from([
            "lazydb",
            "connections",
            "add",
            "--name",
            "conflict",
            "--url",
            "postgresql://localhost/app",
            "--no-ssh",
            "--ssh-host",
            "bastion",
        ])
        .is_err()
    );
}

#[test]
fn cli_add_preserves_updates_and_clears_ssh_on_upsert() {
    let temp = tempdir().unwrap();
    let app = temp.path().join("app");
    let config = app.join("connections.toml");
    let project = temp.path().join("project");
    fs::create_dir_all(&app).unwrap();
    fs::create_dir_all(project.join(".git")).unwrap();

    let base_args = [
        "add",
        "--name",
        "tunneled",
        "--url",
        "postgresql://localhost:5432/app",
        "--scope",
        "global",
        "--json",
    ];
    let mut initial = base_args.to_vec();
    initial.extend([
        "--ssh-host",
        "bastion.example.com",
        "--ssh-user",
        "alice",
        "--ssh-port",
        "2222",
        "--ssh-identity-file",
        "/tmp/id_ed25519",
        "--ssh-config-file",
        "/tmp/ssh_config",
        "--ssh-proxy-jump",
        "jump.example.com",
        "--ssh-known-hosts-file",
        "/tmp/known_hosts",
    ]);
    let output = run_connection_add(&config, &project, &initial);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["data"]["ssh"]["host"], "bastion.example.com");
    assert_eq!(body["data"]["ssh"]["port"], 2222);
    assert!(body["data"]["ssh"].get("identity_file").is_none());

    let mut upsert_args = base_args.to_vec();
    upsert_args.push("--upsert");
    let output = run_connection_add(&config, &project, &upsert_args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["data"]["changed"], false);
    let saved = ProfileStore::new(config.clone()).load().unwrap();
    assert_eq!(
        saved.profiles[0]
            .ssh
            .as_ref()
            .unwrap()
            .identity_file
            .as_deref(),
        Some(Path::new("/tmp/id_ed25519"))
    );

    let mut port_args = base_args.to_vec();
    port_args.extend(["--upsert", "--ssh-port", "2200"]);
    let output = run_connection_add(&config, &project, &port_args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let saved = ProfileStore::new(config.clone()).load().unwrap();
    let ssh = saved.profiles[0].ssh.as_ref().unwrap();
    assert_eq!(ssh.host, "bastion.example.com");
    assert_eq!(ssh.user.as_deref(), Some("alice"));
    assert_eq!(ssh.port, Some(2200));

    for (option, value) in [
        ("--ssh-user", "alice\n"),
        ("--ssh-proxy-jump", "jump\n"),
        ("--ssh-port", "0"),
    ] {
        let mut invalid_args = base_args.to_vec();
        invalid_args.extend(["--upsert", option, value]);
        let output = run_connection_add(&config, &project, &invalid_args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "accepted {option} {value:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let body: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(body["error"]["code"], "invalid_ssh_configuration");
        let saved = ProfileStore::new(config.clone()).load().unwrap();
        let ssh = saved.profiles[0].ssh.as_ref().unwrap();
        assert_eq!(ssh.user.as_deref(), Some("alice"));
        assert_eq!(ssh.proxy_jump.as_deref(), Some("jump.example.com"));
        assert_eq!(ssh.port, Some(2200));
    }

    let mut clear_args = base_args.to_vec();
    clear_args.extend(["--upsert", "--no-ssh"]);
    let output = run_connection_add(&config, &project, &clear_args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        ProfileStore::new(config).load().unwrap().profiles[0]
            .ssh
            .is_none()
    );
}

#[test]
fn cli_add_rejects_ssh_for_an_unsupported_driver() {
    let temp = tempdir().unwrap();
    let app = temp.path().join("app");
    let config = app.join("connections.toml");
    let project = temp.path().join("project");
    fs::create_dir_all(&app).unwrap();
    fs::create_dir_all(project.join(".git")).unwrap();

    let output = run_connection_add(
        &config,
        &project,
        &[
            "add",
            "--name",
            "sqlite-tunnel",
            "--url",
            "sqlite::memory:",
            "--scope",
            "global",
            "--ssh-host",
            "bastion",
            "--json",
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["error"]["code"], "invalid_ssh_configuration");
}
