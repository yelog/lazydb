use std::{fs, path::Path, process::Command};

use lazydb::{
    persistence::profiles::ProfileStore,
    profile::{CredentialPolicy, ProfileAccess},
};
use serde_json::Value;
use tempfile::tempdir;

fn run(args: &[&str], config: &Path, project: &Path) -> std::process::Output {
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
fn add_upsert_list_show_and_agent_query_work_across_processes() {
    let temp = tempdir().unwrap();
    let app = temp.path().join("app-config");
    let config = app.join("connections.toml");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&app).unwrap();
    fs::create_dir(project.join(".git")).unwrap();

    let added = run(
        &[
            "add",
            "--name",
            "scratch",
            "--url",
            "sqlite::memory:",
            "--scope",
            "project",
            "--project",
            ".",
            "--json",
        ],
        &config,
        &project,
    );
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let added: Value = serde_json::from_slice(&added.stdout).unwrap();
    assert_eq!(added["ok"], true);
    assert_eq!(added["data"]["changed"], true);
    let id = added["data"]["id"].as_str().unwrap().to_owned();

    let upserted = run(
        &[
            "add",
            "--name",
            "scratch",
            "--url",
            "sqlite::memory:",
            "--scope",
            "project",
            "--project",
            ".",
            "--upsert",
            "--json",
        ],
        &config,
        &project,
    );
    assert!(
        upserted.status.success(),
        "{}",
        String::from_utf8_lossy(&upserted.stderr)
    );
    let upserted: Value = serde_json::from_slice(&upserted.stdout).unwrap();
    assert_eq!(upserted["data"]["id"], id);
    assert_eq!(upserted["data"]["changed"], false);

    let listed = run(&["list", "--project", ".", "--json"], &config, &project);
    assert!(listed.status.success());
    let listed: Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(listed["data"]["connections"].as_array().unwrap().len(), 1);

    let shown = run(
        &["show", "scratch", "--project", ".", "--json"],
        &config,
        &project,
    );
    assert!(shown.status.success());
    let shown: Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(shown["data"]["id"], id);

    let agent = Command::new(env!("CARGO_BIN_EXE_lazydb"))
        .arg("--config")
        .arg(&config)
        .args([
            "agent",
            "query",
            "--project",
            ".",
            "--connection",
            "scratch",
            "--sql",
            "SELECT 1",
        ])
        .env("LAZYDB_CONFIG_HOME", &app)
        .current_dir(&project)
        .output()
        .unwrap();
    assert!(
        agent.status.success(),
        "{}",
        String::from_utf8_lossy(&agent.stderr)
    );
    assert!(String::from_utf8_lossy(&agent.stdout).contains("\"rows\""));

    let profiles = ProfileStore::new(config).load().unwrap();
    assert_eq!(profiles.profiles.len(), 1);
    assert!(matches!(
        profiles.profiles[0].access,
        ProfileAccess::Projects { .. }
    ));
    assert_eq!(
        profiles.profiles[0].credential_policy,
        CredentialPolicy::None
    );
}

#[test]
fn unchanged_password_upsert_is_a_noop_without_reencrypting() {
    let temp = tempdir().unwrap();
    let app = temp.path().join("app-config");
    let project = temp.path().join("project");
    let config = app.join("connections.toml");
    fs::create_dir_all(&app).unwrap();
    fs::create_dir_all(project.join(".git")).unwrap();
    let args = [
        "connections",
        "add",
        "--name",
        "pg",
        "--url",
        "postgresql://app@localhost:5432/app",
        "--scope",
        "global",
        "--password-env",
        "LAZYDB_TEST_UPSERT_PASSWORD",
        "--json",
    ];
    let run_with_upsert = |upsert: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lazydb"));
        command
            .arg("--config")
            .arg(&config)
            .args(args)
            .env("LAZYDB_TEST_UPSERT_PASSWORD", "same-password")
            .env("LAZYDB_CONFIG_HOME", &app)
            .current_dir(&project);
        if upsert {
            command.arg("--upsert");
        }
        command.output().unwrap()
    };
    let first = run_with_upsert(false);
    assert!(first.status.success());
    let first_result: Value = serde_json::from_slice(&first.stdout).unwrap();
    let first_id = first_result["data"]["id"].as_str().unwrap().to_owned();
    let before = fs::read(&config).unwrap();

    let second = run_with_upsert(true);
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_result: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(second_result["data"]["changed"], false);
    assert_eq!(second_result["data"]["id"], first_id);
    assert_eq!(fs::read(&config).unwrap(), before);
}

#[test]
fn password_environment_is_encrypted_and_secrets_are_not_printed() {
    let temp = tempdir().unwrap();
    let app = temp.path().join("app-config");
    let config = app.join("connections.toml");
    let project = temp.path().join("project");
    fs::create_dir_all(&app).unwrap();
    fs::create_dir_all(&project).unwrap();
    fs::create_dir(project.join(".git")).unwrap();
    let secret = "unique-password-sentinel";
    let added = Command::new(env!("CARGO_BIN_EXE_lazydb"))
        .arg("--config")
        .arg(&config)
        .args([
            "connections",
            "add",
            "--name",
            "db",
            "--url",
            "postgresql://alice@127.0.0.1:1/app",
            "--scope",
            "global",
            "--password-env",
            "LAZYDB_TEST_CONNECTION_PASSWORD",
            "--json",
        ])
        .env("LAZYDB_TEST_CONNECTION_PASSWORD", secret)
        .env("LAZYDB_CONFIG_HOME", &app)
        .current_dir(&project)
        .output()
        .unwrap();
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    assert!(!String::from_utf8_lossy(&added.stdout).contains(secret));
    assert!(!String::from_utf8_lossy(&added.stderr).contains(secret));
    let contents = fs::read_to_string(&config).unwrap();
    assert!(!contents.contains(secret));
    assert!(contents.contains("local_encrypted"));
    assert!(app.join("credential.key").exists());
}

#[test]
fn connection_errors_return_json_without_echoing_url_password() {
    let temp = tempdir().unwrap();
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir(project.join(".git")).unwrap();
    let config = temp.path().join("config").join("connections.toml");
    let secret = "cli-url-secret-sentinel";
    let output = Command::new(env!("CARGO_BIN_EXE_lazydb"))
        .arg("--config")
        .arg(&config)
        .args([
            "connections",
            "add",
            "--name",
            "bad",
            "--url",
            &format!("postgresql://alice:{secret}@127.0.0.1:5432/app?unknown=1"),
            "--json",
        ])
        .env("LAZYDB_CONFIG_HOME", temp.path().join("app"))
        .current_dir(&project)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["command"], "connections.add");
    assert!(!String::from_utf8_lossy(&output.stdout).contains(secret));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(secret));
}

#[test]
fn clap_errors_for_connection_commands_keep_json_on_stdout() {
    let temp = tempdir().unwrap();
    let config = temp.path().join("app").join("connections.toml");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir(project.join(".git")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lazydb"))
        .arg("--config")
        .arg(&config)
        .args([
            "connections",
            "add",
            "--name",
            "demo",
            "--url",
            "sqlite::memory:",
            "--json",
            "--unknown-option",
        ])
        .env("LAZYDB_CONFIG_HOME", temp.path().join("app"))
        .current_dir(&project)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["command"], "connections.add");
    assert_eq!(envelope["error"]["code"], "invalid_arguments");
    assert!(output.stderr.is_empty());
}

#[test]
fn legacy_config_directory_key_remains_readable_by_agent() {
    use secrecy::SecretString;
    use uuid::Uuid;

    use lazydb::{
        persistence::{local_credentials::LocalCredentialStore, profiles::ProfileStore},
        profile::{CredentialPolicy, ProfileAccess, import_connection_url},
    };

    let temp = tempdir().unwrap();
    let app = temp.path().join("application");
    let project = temp.path().join("project");
    let config = temp.path().join("custom").join("connections.toml");
    fs::create_dir_all(&app).unwrap();
    fs::create_dir_all(project.join(".git")).unwrap();
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    let sqlite_file = project.join("legacy.sqlite");
    fs::write(&sqlite_file, b"").unwrap();
    let legacy_url = format!("sqlite:{}", sqlite_file.display());
    let mut profile = import_connection_url(&legacy_url, Some("legacy"))
        .unwrap()
        .profile;
    profile.id = Uuid::new_v4();
    profile.access = ProfileAccess::Global;
    let legacy_key = config.parent().unwrap().join("credential.key");
    let legacy_store = LocalCredentialStore::new(legacy_key, "lazydb");
    profile.credential_policy = CredentialPolicy::LocalEncrypted(
        legacy_store
            .encrypt(profile.id, &SecretString::from("legacy-password"))
            .unwrap(),
    );
    ProfileStore::new(config.clone())
        .with_credential_key_path(app.join("credential.key"))
        .save(vec![profile])
        .unwrap();

    let agent = Command::new(env!("CARGO_BIN_EXE_lazydb"))
        .arg("--config")
        .arg(&config)
        .args([
            "agent",
            "query",
            "--project",
            project.to_str().unwrap(),
            "--connection",
            "legacy",
            "--sql",
            "SELECT 1",
        ])
        .env("LAZYDB_CONFIG_HOME", &app)
        .output()
        .unwrap();
    assert!(
        agent.status.success(),
        "{}",
        String::from_utf8_lossy(&agent.stderr)
    );
    assert!(String::from_utf8_lossy(&agent.stdout).contains("\"rows\""));
    assert!(!String::from_utf8_lossy(&agent.stdout).contains("legacy-password"));
}
