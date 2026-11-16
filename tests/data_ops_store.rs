use chrono::Utc;
use lazydb::{
    data_ops::{
        DataEndpoint, DataJobPlan, DataJobRecord, DataJobSpec, DataJobStatus, DataJobStore,
        DataOperation, NativePlan, NativeToolPaths,
    },
    profile::DatabaseKind,
};
use uuid::Uuid;

fn record(status: DataJobStatus) -> DataJobRecord {
    let id = Uuid::new_v4();
    DataJobRecord {
        schema_version: 1,
        id,
        plan: DataJobPlan {
            id,
            spec: DataJobSpec {
                operation: DataOperation::Restore,
                source: None,
                destination: Some(DataEndpoint {
                    profile_id: Uuid::new_v4(),
                    database: "restored".into(),
                }),
                artifact: "backup.dump".into(),
                tools: NativeToolPaths::default(),
            },
            native: NativePlan {
                engine: DatabaseKind::Postgres,
                steps: Vec::new(),
                warnings: Vec::new(),
                tools: Vec::new(),
            },
            profile_fingerprint: "fingerprint".into(),
            created_at: Utc::now(),
        },
        status,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        message: String::new(),
        bytes: None,
        log: Vec::new(),
        outcome: None,
        error: None,
        target_may_be_partial: false,
    }
}

#[test]
fn live_task_is_not_marked_interrupted_by_another_store() {
    let directory = tempfile::tempdir().unwrap();
    let writer = DataJobStore::new(directory.path().to_owned());
    let reader = DataJobStore::new(directory.path().to_owned());
    let task = record(DataJobStatus::Running);
    let lock = writer.lock_job(task.id).unwrap();
    writer.create(&task).unwrap();

    assert_eq!(reader.load(task.id).unwrap().status, DataJobStatus::Running);
    assert!(reader.lock_job(task.id).is_err());
    drop(lock);

    let recovered = reader.load(task.id).unwrap();
    assert_eq!(recovered.status, DataJobStatus::Interrupted);
    assert!(recovered.target_may_be_partial);
    assert_eq!(
        writer.load(task.id).unwrap().status,
        DataJobStatus::Interrupted
    );
}

#[test]
fn preflight_interruption_does_not_claim_target_was_modified() {
    let directory = tempfile::tempdir().unwrap();
    let store = DataJobStore::new(directory.path().to_owned());
    let task = record(DataJobStatus::Preflight);
    store.create(&task).unwrap();

    let recovered = store.load(task.id).unwrap();
    assert_eq!(recovered.status, DataJobStatus::Interrupted);
    assert!(!recovered.target_may_be_partial);
}

#[test]
fn immutable_task_id_cannot_be_executed_twice() {
    let directory = tempfile::tempdir().unwrap();
    let store = DataJobStore::new(directory.path().to_owned());
    let task = record(DataJobStatus::Succeeded);
    store.create(&task).unwrap();
    assert!(store.create(&task).is_err());
    assert_eq!(store.list().unwrap(), vec![task]);
}

#[test]
fn destination_locks_are_exclusive_until_the_owner_drops() {
    let directory = tempfile::tempdir().unwrap();
    let first = DataJobStore::new(directory.path().to_owned());
    let second = DataJobStore::new(directory.path().to_owned());
    let lock = first
        .lock_resource("postgres:localhost:5432:new_database")
        .unwrap();
    assert!(
        second
            .lock_resource("postgres:localhost:5432:new_database")
            .is_err()
    );
    assert!(
        second
            .lock_resource("postgres:localhost:5432:other_database")
            .is_ok()
    );
    drop(lock);
    assert!(
        second
            .lock_resource("postgres:localhost:5432:new_database")
            .is_ok()
    );
}

#[test]
fn tampered_record_identity_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let store = DataJobStore::new(directory.path().to_owned());
    let task = record(DataJobStatus::Succeeded);
    let different_id = Uuid::new_v4();
    std::fs::write(
        directory.path().join(format!("{different_id}.json")),
        serde_json::to_vec(&task).unwrap(),
    )
    .unwrap();
    assert!(store.load(different_id).is_err());
}

#[test]
fn record_reads_accept_four_mib_and_reject_one_byte_more() {
    let directory = tempfile::tempdir().unwrap();
    let store = DataJobStore::new(directory.path().to_owned());
    let mut task = record(DataJobStatus::Succeeded);
    let limit = 4 * 1024 * 1024;
    let base_length = serde_json::to_vec(&task).unwrap().len();
    task.message = "x".repeat(limit - base_length);
    let mut bytes = serde_json::to_vec(&task).unwrap();
    assert_eq!(bytes.len(), limit);
    let path = directory.path().join(format!("{}.json", task.id));
    std::fs::write(&path, &bytes).unwrap();

    let loaded = store.load(task.id).unwrap();
    assert_eq!(loaded.id, task.id);
    assert_eq!(loaded.message.len(), task.message.len());

    bytes.push(b'\n');
    std::fs::write(&path, &bytes).unwrap();
    let error = store.load(task.id).unwrap_err().to_string();
    assert!(error.contains("size limit of 4194304 bytes"), "{error}");
}

#[test]
fn record_directories_are_rejected_as_non_regular_files() {
    let directory = tempfile::tempdir().unwrap();
    let store = DataJobStore::new(directory.path().to_owned());
    let id = Uuid::new_v4();
    std::fs::create_dir(directory.path().join(format!("{id}.json"))).unwrap();
    let error = store.load(id).unwrap_err().to_string();
    assert!(error.contains("must be a regular file"), "{error}");
}

#[cfg(unix)]
#[test]
fn fifo_records_are_rejected_without_blocking() {
    use std::{
        path::PathBuf,
        process::Command,
        thread,
        time::{Duration, Instant},
    };

    const CHILD_PATH: &str = "LAZYDB_TEST_METADATA_FIFO_RECORD";
    if let Some(path) = std::env::var_os(CHILD_PATH) {
        let path = PathBuf::from(path);
        let id = Uuid::parse_str(path.file_stem().unwrap().to_str().unwrap()).unwrap();
        let store = DataJobStore::new(path.parent().unwrap().to_owned());
        for result in [store.load(id).map(|_| ()), store.list().map(|_| ())] {
            let error = result.unwrap_err().to_string();
            assert!(error.contains("must be a regular file"), "{error}");
        }
        return;
    }

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(format!("{}.json", Uuid::new_v4()));
    assert!(
        Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "fifo_records_are_rejected_without_blocking",
            "--nocapture",
        ])
        .env(CHILD_PATH, &path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("FIFO record reads did not finish within 5 seconds");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
