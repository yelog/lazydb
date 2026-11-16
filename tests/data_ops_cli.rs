use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use chrono::Utc;
use clap::Parser;
use lazydb::{
    cli::{Cli, Command as CliCommand},
    data_ops::{
        DataEndpoint, DataJobPlan, DataJobSpec, DataOperation, NativePlan, NativeToolPaths,
        cli::DataCommand,
    },
    persistence::profiles::ProfileStore,
    profile::import_connection_url,
};
use serde_json::Value;
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use uuid::Uuid;

#[test]
fn cli_previews_by_default_and_requires_new_restore_target() {
    let cli = Cli::try_parse_from(["lazydb", "data", "backup", "--output", "backup.dump"]).unwrap();
    let Some(CliCommand::Data(args)) = cli.command else {
        panic!("expected data command");
    };
    let DataCommand::Backup { options, .. } = args.command else {
        panic!("expected backup");
    };
    assert!(!options.execute);
    assert!(Cli::try_parse_from(["lazydb", "data", "restore", "--input", "backup.dump"]).is_err());
}

fn invoke(config: &Path, root: &Path, args: &[&str]) -> std::process::Output {
    let mut stdout = tempfile::NamedTempFile::new_in(root).unwrap();
    let mut stderr = tempfile::NamedTempFile::new_in(root).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_lazydb"))
        .arg("--config")
        .arg(config)
        .arg("data")
        .arg("--json")
        .args(args)
        .env("LAZYDB_CONFIG_HOME", root)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout.as_file().try_clone().unwrap()))
        .stderr(Stdio::from(stderr.as_file().try_clone().unwrap()))
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Data command did not finish within 10 seconds");
        }
        thread::sleep(Duration::from_millis(10));
    };
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    stdout.seek(SeekFrom::Start(0)).unwrap();
    stdout.read_to_end(&mut stdout_bytes).unwrap();
    stderr.seek(SeekFrom::Start(0)).unwrap();
    stderr.read_to_end(&mut stderr_bytes).unwrap();
    std::process::Output {
        status,
        stdout: stdout_bytes,
        stderr: stderr_bytes,
    }
}

fn json(output: std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[tokio::test]
async fn cli_saved_plan_backup_restore_and_rerun_work_across_processes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join(".git")).unwrap();
    let database = root.join("source.sqlite");
    let config = root.join("connections.toml");
    let mut connection = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::raw_sql("CREATE TABLE sample (id INTEGER PRIMARY KEY, value TEXT); INSERT INTO sample VALUES (1, 'hello');")
        .execute(&mut connection).await.unwrap();
    connection.close().await.unwrap();
    let mut profile =
        import_connection_url(&format!("sqlite://{}", database.display()), Some("test"))
            .unwrap()
            .profile;
    profile.read_only = false;
    ProfileStore::new(config.clone()).save(&[profile]).unwrap();

    let planned = json(invoke(
        &config,
        root,
        &[
            "backup",
            "--connection",
            "test",
            "--output",
            "backup.sqlite",
            "--save-plan",
            "plan.json",
        ],
    ));
    assert_eq!(planned["ok"], true);
    assert!(!root.join("backup.sqlite").exists());
    assert!(root.join("plan.json").exists());

    let backed_up = json(invoke(&config, root, &["run", "plan.json", "--execute"]));
    assert_eq!(backed_up["ok"], true);
    let backup_id = backed_up["data"]["id"].as_str().unwrap();
    assert!(root.join("backup.sqlite.lazydb.json").exists());
    let restored = json(invoke(
        &config,
        root,
        &[
            "restore",
            "--connection",
            "test",
            "--database",
            "restored.sqlite",
            "--input",
            "backup.sqlite",
            "--execute",
        ],
    ));
    assert_eq!(restored["ok"], true);
    let mut connection = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(root.join("restored.sqlite"))
            .read_only(true),
    )
    .await
    .unwrap();
    let value: String = sqlx::query_scalar("SELECT value FROM sample WHERE id = 1")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(value, "hello");
    connection.close().await.unwrap();

    let repeated = json(invoke(
        &config,
        root,
        &["rerun", backup_id, "--output", "second.sqlite", "--execute"],
    ));
    assert_ne!(backed_up["data"]["id"], repeated["data"]["id"]);
    let jobs = json(invoke(&config, root, &["jobs"]));
    assert_eq!(jobs["data"].as_array().unwrap().len(), 3);
    let shown = json(invoke(&config, root, &["show", backup_id]));
    assert_eq!(shown["data"]["id"], backup_id);

    let overwrite = invoke(
        &config,
        root,
        &[
            "backup",
            "--connection",
            "test",
            "--output",
            "backup.sqlite",
            "--execute",
        ],
    );
    assert!(!overwrite.status.success());
    let error: Value = serde_json::from_slice(&overwrite.stdout).unwrap();
    assert_eq!(error["ok"], false);
}

#[test]
fn cli_plan_reads_accept_four_mib_and_reject_one_byte_more() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join(".git")).unwrap();
    let config = root.join("connections.toml");
    let profile = import_connection_url("postgres://test@127.0.0.1:1/source", Some("metadata"))
        .unwrap()
        .profile;
    ProfileStore::new(config.clone())
        .save(vec![profile.clone()])
        .unwrap();
    let mut plan = DataJobPlan {
        id: Uuid::new_v4(),
        spec: DataJobSpec {
            operation: DataOperation::Backup,
            source: Some(DataEndpoint::from_profile(&profile)),
            destination: None,
            artifact: root.join("unused.dump"),
            tools: NativeToolPaths::default(),
        },
        native: NativePlan {
            engine: profile.kind,
            steps: vec![String::new()],
            warnings: Vec::new(),
            tools: Vec::new(),
        },
        profile_fingerprint: "metadata-size-test".into(),
        created_at: Utc::now(),
    };
    let limit = 4 * 1024 * 1024;
    let base_length = serde_json::to_vec(&plan).unwrap().len();
    plan.native.steps[0] = "x".repeat(limit - base_length);
    let mut bytes = serde_json::to_vec(&plan).unwrap();
    assert_eq!(bytes.len(), limit);
    let path = root.join("plan.json");
    std::fs::write(&path, &bytes).unwrap();

    let reviewed = json(invoke(&config, root, &["run", "plan.json"]));
    assert_eq!(reviewed["data"]["id"], plan.id.to_string());
    assert_eq!(
        reviewed["data"]["native"]["steps"][0]
            .as_str()
            .unwrap()
            .len(),
        limit - base_length
    );
    assert!(!root.join("unused.dump").exists());
    assert!(!root.join("data-jobs").exists());

    bytes.push(b'\n');
    std::fs::write(&path, &bytes).unwrap();
    let rejected = invoke(&config, root, &["run", "plan.json"]);
    assert_eq!(rejected.status.code(), Some(1));
    let error: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(error["ok"], false);
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("size limit of 4194304 bytes")
    );
}

#[cfg(unix)]
#[test]
fn cli_rejects_fifo_plans_and_records_without_blocking() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir(root.join(".git")).unwrap();
    let config = root.join("connections.toml");
    std::fs::write(&config, "version = 6\nprofiles = []\n").unwrap();
    let plan = root.join("plan.json");
    let history = root.join("data-jobs");
    std::fs::create_dir(&history).unwrap();
    let id = Uuid::new_v4().to_string();
    let record = history.join(format!("{id}.json"));
    for path in [&plan, &record] {
        assert!(Command::new("mkfifo").arg(path).status().unwrap().success());
    }
    for arguments in [
        vec!["run", "plan.json"],
        vec!["jobs"],
        vec!["show", id.as_str()],
    ] {
        let rejected = invoke(&config, root, &arguments);
        assert_eq!(rejected.status.code(), Some(1));
        let error: Value = serde_json::from_slice(&rejected.stdout).unwrap();
        assert_eq!(error["schema_version"], 1);
        assert_eq!(error["ok"], false);
        assert!(
            error["error"]["message"]
                .as_str()
                .unwrap()
                .contains("must be a regular file")
        );
    }
    assert!(!history.join("locks").exists());
}
