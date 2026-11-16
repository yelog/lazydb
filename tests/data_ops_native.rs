use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use lazydb::{
    connections::DatabaseConnector,
    data_ops::{
        DataEndpoint, DataJobProgress, DataJobSpec, DataJobStatus, DataOperation, NativePlan,
        NativeToolPaths, ProgressSink, ResolvedEndpoint, native,
    },
    profile::import_connection_url,
};
use sqlx::{Connection, Executor, SqliteConnection, sqlite::SqliteConnectOptions};
use tokio_util::sync::CancellationToken;

fn endpoint(path: &Path) -> ResolvedEndpoint {
    let mut profile = import_connection_url("sqlite://:memory:", Some("native-fixture"))
        .unwrap()
        .profile;
    profile.sqlite_path = Some(path.to_path_buf());
    profile.database = None;
    ResolvedEndpoint {
        profile,
        database: path.to_str().unwrap().into(),
        password: None,
    }
}

fn reference(endpoint: &ResolvedEndpoint) -> DataEndpoint {
    DataEndpoint {
        profile_id: endpoint.profile.id,
        database: endpoint.database.clone(),
    }
}

fn spec(
    operation: DataOperation,
    source: Option<&ResolvedEndpoint>,
    destination: Option<&ResolvedEndpoint>,
    artifact: &Path,
) -> DataJobSpec {
    DataJobSpec {
        operation,
        source: source.map(reference),
        destination: destination.map(reference),
        artifact: artifact.into(),
        tools: NativeToolPaths::default(),
    }
}

fn progress() -> (ProgressSink, Arc<Mutex<Vec<DataJobProgress>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&events);
    (
        Arc::new(move |event| captured.lock().unwrap().push(event)),
        events,
    )
}

#[test]
fn native_public_futures_fit_within_worker_thread_stack_budgets() {
    let source = endpoint(Path::new("/not-opened/source.db"));
    let request = spec(
        DataOperation::Backup,
        Some(&source),
        None,
        Path::new("/not-opened/backup.db"),
    );
    let connector = DatabaseConnector::new();
    let token = CancellationToken::new();
    let plan = NativePlan {
        engine: source.profile.kind,
        steps: Vec::new(),
        warnings: Vec::new(),
        tools: Vec::new(),
    };
    let planning = native::plan(&request, Some(&source), None, &connector, &token);
    let (sink, _) = progress();
    let executing = native::execute(
        &request,
        &plan,
        Some(&source),
        None,
        &connector,
        &token,
        sink,
    );
    assert!(std::mem::size_of_val(&planning) < 32 * 1024);
    assert!(std::mem::size_of_val(&executing) < 32 * 1024);
}

async fn seed(path: &Path) {
    let mut connection = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    connection
        .execute(
            "CREATE TABLE items(id INTEGER PRIMARY KEY, label TEXT NOT NULL, payload BLOB);\
         WITH RECURSIVE ids(id) AS (VALUES(1) UNION ALL SELECT id + 1 FROM ids WHERE id < 800) \
         INSERT INTO items SELECT id, 'value-' || id, X'000127ff' FROM ids;\
         CREATE INDEX items_label ON items(label);\
         CREATE VIEW item_names AS SELECT id, label FROM items;",
        )
        .await
        .unwrap();
    connection.close().await.unwrap();
}

async fn count(path: &Path) -> i64 {
    let mut connection = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .create_if_missing(false),
    )
    .await
    .unwrap();
    let count = sqlx::query_scalar("SELECT count(*) FROM item_names")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    let payload: Vec<u8> = sqlx::query_scalar("SELECT payload FROM items WHERE id = 800")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(payload, [0, 1, 39, 255]);
    connection.close().await.unwrap();
    count
}

#[tokio::test]
async fn sqlite_backup_and_restore_round_trip_all_rows_and_report_real_verification() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source 'quoted'.db");
    let restored_path = directory.path().join("restored 'new'.db");
    let artifact = directory.path().join("backup 'complete'.db");
    seed(&source_path).await;
    let source = endpoint(&source_path);
    let destination = endpoint(&restored_path);
    let connector = DatabaseConnector::new();
    let token = CancellationToken::new();
    let backup = spec(DataOperation::Backup, Some(&source), None, &artifact);
    let plan = native::plan(&backup, Some(&source), None, &connector, &token)
        .await
        .unwrap();
    assert!(!artifact.exists());
    let (sink, events) = progress();
    let outcome = native::execute(
        &backup,
        &plan,
        Some(&source),
        None,
        &connector,
        &token,
        sink,
    )
    .await
    .unwrap();
    assert_eq!(
        outcome.bytes,
        Some(std::fs::metadata(&artifact).unwrap().len())
    );
    assert_eq!(outcome.sha256.as_ref().unwrap().len(), 64);
    assert!(!outcome.target_created);
    assert!(
        outcome
            .verification
            .iter()
            .any(|verification| verification.contains("integrity_check"))
    );
    assert!(
        plan.warnings
            .iter()
            .any(|warning| warning.contains("ROWID"))
    );
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|event| event.status == DataJobStatus::Verifying)
    );
    {
        let events = events.lock().unwrap();
        let checkpoints: Vec<_> = events
            .iter()
            .filter(|event| event.completed_backup.is_some())
            .collect();
        assert_eq!(checkpoints.len(), 1);
        assert_eq!(checkpoints[0].status, DataJobStatus::Verifying);
        assert_eq!(
            checkpoints[0].message,
            "Completed backup artifact published"
        );
        assert_eq!(checkpoints[0].bytes, outcome.bytes);
        assert_eq!(checkpoints[0].completed_backup.as_ref(), Some(&outcome));
    }
    assert_eq!(count(&artifact).await, 800);
    let restore = spec(DataOperation::Restore, None, Some(&destination), &artifact);
    let plan = native::plan(&restore, None, Some(&destination), &connector, &token)
        .await
        .unwrap();
    assert!(!restored_path.exists());
    let (sink, events) = progress();
    let restored = native::execute(
        &restore,
        &plan,
        None,
        Some(&destination),
        &connector,
        &token,
        sink,
    )
    .await
    .unwrap();
    assert!(restored.target_created);
    assert_eq!(restored.sha256, outcome.sha256);
    assert_eq!(count(&restored_path).await, 800);
    assert_eq!(count(&source_path).await, 800);
    assert_eq!(connector.active_tunnel_count(), 0);
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .all(|event| event.completed_backup.is_none())
    );
}

#[tokio::test]
async fn sqlite_copy_creates_new_database_and_keeps_complete_backup() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.db");
    seed(&source_path).await;
    let source = endpoint(&source_path);
    let target_path = directory.path().join("target.db");
    let destination = endpoint(&target_path);
    let artifact = directory.path().join("copy-backup.db");
    let copy = spec(
        DataOperation::Copy,
        Some(&source),
        Some(&destination),
        &artifact,
    );
    let connector = DatabaseConnector::new();
    let token = CancellationToken::new();
    let plan = native::plan(&copy, Some(&source), Some(&destination), &connector, &token)
        .await
        .unwrap();
    let (sink, events) = progress();
    let outcome = native::execute(
        &copy,
        &plan,
        Some(&source),
        Some(&destination),
        &connector,
        &token,
        sink,
    )
    .await
    .unwrap();
    assert!(outcome.target_created);
    assert!(artifact.exists());
    assert_eq!(count(&artifact).await, 800);
    assert_eq!(count(&target_path).await, 800);
    let events = events.lock().unwrap();
    let checkpoints: Vec<_> = events
        .iter()
        .enumerate()
        .filter(|(_, event)| event.completed_backup.is_some())
        .collect();
    assert_eq!(checkpoints.len(), 1);
    let (checkpoint_index, event) = checkpoints[0];
    let completed = event.completed_backup.as_ref().unwrap();
    assert_eq!(completed.artifact, outcome.artifact);
    assert_eq!(completed.bytes, outcome.bytes);
    assert_eq!(completed.sha256, outcome.sha256);
    assert_eq!(completed.warnings, outcome.warnings);
    assert!(!completed.target_created);
    assert!(
        completed
            .verification
            .iter()
            .all(|verification| verification.contains("backup"))
    );
    let restore_index = events
        .iter()
        .position(|event| event.message == "Streaming SQLite snapshot into a new file")
        .unwrap();
    assert!(checkpoint_index < restore_index);
}

#[tokio::test]
async fn sqlite_missing_sources_invalid_artifacts_and_existing_outputs_are_refused() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing.db");
    let source = endpoint(&missing);
    let artifact = directory.path().join("backup.db");
    let connector = DatabaseConnector::new();
    let token = CancellationToken::new();
    let backup = spec(DataOperation::Backup, Some(&source), None, &artifact);
    assert!(
        native::plan(&backup, Some(&source), None, &connector, &token)
            .await
            .is_err()
    );
    assert!(!missing.exists());
    seed(&missing).await;
    std::fs::write(&artifact, b"existing output").unwrap();
    assert!(
        native::plan(&backup, Some(&source), None, &connector, &token)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&artifact).unwrap(), b"existing output");
    let target_path = directory.path().join("new.db");
    let destination = endpoint(&target_path);
    let restore = spec(DataOperation::Restore, None, Some(&destination), &artifact);
    assert!(
        native::plan(&restore, None, Some(&destination), &connector, &token)
            .await
            .is_err()
    );
    assert!(!target_path.exists());
}

#[tokio::test]
async fn sqlite_rechecks_target_races_and_restore_input_changes() {
    let directory = tempfile::tempdir().unwrap();
    let artifact = directory.path().join("input.db");
    seed(&artifact).await;
    let target = directory.path().join("target.db");
    let destination = endpoint(&target);
    let restore = spec(DataOperation::Restore, None, Some(&destination), &artifact);
    let connector = DatabaseConnector::new();
    let token = CancellationToken::new();
    let plan = native::plan(&restore, None, Some(&destination), &connector, &token)
        .await
        .unwrap();
    std::fs::write(&target, b"created by another process").unwrap();
    let (sink, _) = progress();
    assert!(
        native::execute(
            &restore,
            &plan,
            None,
            Some(&destination),
            &connector,
            &token,
            sink
        )
        .await
        .is_err()
    );
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"created by another process"
    );
    let other_target = directory.path().join("other-target.db");
    let other_destination = endpoint(&other_target);
    let changed = spec(
        DataOperation::Restore,
        None,
        Some(&other_destination),
        &artifact,
    );
    let plan = native::plan(&changed, None, Some(&other_destination), &connector, &token)
        .await
        .unwrap();
    let mut connection =
        SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&artifact))
            .await
            .unwrap();
    connection
        .execute("UPDATE items SET label = 'changed after review' WHERE id = 1")
        .await
        .unwrap();
    connection.close().await.unwrap();
    let (sink, _) = progress();
    let error = native::execute(
        &changed,
        &plan,
        None,
        Some(&other_destination),
        &connector,
        &token,
        sink,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("changed after review"));
    assert!(!other_target.exists());
}

#[tokio::test]
async fn sqlite_cancellation_never_publishes_a_complete_artifact_or_reports_success() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.db");
    seed(&source_path).await;
    let source = endpoint(&source_path);
    let artifact = directory.path().join("cancelled.db");
    let backup = spec(DataOperation::Backup, Some(&source), None, &artifact);
    let connector = DatabaseConnector::new();
    let token = CancellationToken::new();
    let plan = native::plan(&backup, Some(&source), None, &connector, &token)
        .await
        .unwrap();
    let cancelled = token.clone();
    let sink: ProgressSink = Arc::new(move |event| {
        if event.status == DataJobStatus::Running {
            cancelled.cancel();
        }
    });
    let error = native::execute(
        &backup,
        &plan,
        Some(&source),
        None,
        &connector,
        &token,
        sink,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    assert!(!artifact.exists());
    assert_eq!(count(&source_path).await, 800);
    assert!(std::fs::read_dir(directory.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|extension| extension == "partial")
    }));
    assert!(
        native::plan(&backup, Some(&source), None, &connector, &token)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn sqlite_restore_cancelled_after_copy_keeps_the_input_and_only_a_partial_output() {
    let directory = tempfile::tempdir().unwrap();
    let artifact = directory.path().join("complete.db");
    seed(&artifact).await;
    let input_bytes = std::fs::read(&artifact).unwrap();
    let target = directory.path().join("cancelled-target.db");
    let destination = endpoint(&target);
    let restore = spec(DataOperation::Restore, None, Some(&destination), &artifact);
    let connector = DatabaseConnector::new();
    let token = CancellationToken::new();
    let plan = native::plan(&restore, None, Some(&destination), &connector, &token)
        .await
        .unwrap();
    let cancelled = token.clone();
    let sink: ProgressSink = Arc::new(move |event| {
        if event.message == "Checking new SQLite database integrity" {
            cancelled.cancel();
        }
    });
    let error = native::execute(
        &restore,
        &plan,
        None,
        Some(&destination),
        &connector,
        &token,
        sink,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    assert!(!target.exists());
    assert_eq!(std::fs::read(&artifact).unwrap(), input_bytes);
    assert!(std::fs::read_dir(directory.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|extension| extension == "partial")
    }));
}

#[tokio::test]
async fn sqlite_copy_retains_backup_when_target_publication_loses_a_race() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.db");
    seed(&source_path).await;
    let source = endpoint(&source_path);
    let artifact = directory.path().join("completed-backup.db");
    let target = directory.path().join("target.db");
    let destination = endpoint(&target);
    let copy = spec(
        DataOperation::Copy,
        Some(&source),
        Some(&destination),
        &artifact,
    );
    let connector = DatabaseConnector::new();
    let token = CancellationToken::new();
    let plan = native::plan(&copy, Some(&source), Some(&destination), &connector, &token)
        .await
        .unwrap();
    let racing = target.clone();
    let (capture, events) = progress();
    let sink: ProgressSink = Arc::new(move |event| {
        if event.message == "Streaming SQLite snapshot into a new file" {
            std::fs::write(&racing, b"racing file must survive").unwrap();
        }
        capture(event);
    });
    let error = native::execute(
        &copy,
        &plan,
        Some(&source),
        Some(&destination),
        &connector,
        &token,
        sink,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("partial"));
    assert_eq!(std::fs::read(&target).unwrap(), b"racing file must survive");
    assert_eq!(count(&artifact).await, 800);
    let events = events.lock().unwrap();
    let checkpoints: Vec<_> = events
        .iter()
        .filter_map(|event| event.completed_backup.as_ref())
        .collect();
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(
        checkpoints[0].artifact,
        Some(std::fs::canonicalize(&artifact).unwrap())
    );
    assert_eq!(
        checkpoints[0].bytes,
        Some(std::fs::metadata(&artifact).unwrap().len())
    );
    assert_eq!(checkpoints[0].sha256.as_ref().unwrap().len(), 64);
    assert!(!checkpoints[0].target_created);
}

#[tokio::test]
async fn sqlite_copy_cancelled_at_backup_checkpoint_keeps_backup_without_starting_restore() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.db");
    seed(&source_path).await;
    let source = endpoint(&source_path);
    let artifact = directory.path().join("completed-backup.db");
    let target = directory.path().join("cancelled-target.db");
    let destination = endpoint(&target);
    let copy = spec(
        DataOperation::Copy,
        Some(&source),
        Some(&destination),
        &artifact,
    );
    let connector = DatabaseConnector::new();
    let token = CancellationToken::new();
    let plan = native::plan(&copy, Some(&source), Some(&destination), &connector, &token)
        .await
        .unwrap();
    let cancelled = token.clone();
    let (capture, events) = progress();
    let published = artifact.clone();
    let destination_path = target.clone();
    let sink: ProgressSink = Arc::new(move |event| {
        if event.completed_backup.is_some() {
            assert!(published.exists());
            assert!(!destination_path.exists());
            cancelled.cancel();
        }
        capture(event);
    });
    let error = native::execute(
        &copy,
        &plan,
        Some(&source),
        Some(&destination),
        &connector,
        &token,
        sink,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    assert!(!target.exists());
    assert_eq!(count(&artifact).await, 800);
    let events = events.lock().unwrap();
    let checkpoints: Vec<_> = events
        .iter()
        .filter_map(|event| event.completed_backup.as_ref())
        .collect();
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(
        checkpoints[0].artifact,
        Some(std::fs::canonicalize(&artifact).unwrap())
    );
    assert_eq!(
        checkpoints[0].bytes,
        Some(std::fs::metadata(&artifact).unwrap().len())
    );
    assert_eq!(checkpoints[0].sha256.as_ref().unwrap().len(), 64);
    assert!(!checkpoints[0].target_created);
    assert!(
        events
            .iter()
            .all(|event| event.message != "Streaming SQLite snapshot into a new file")
    );
}

#[tokio::test]
async fn sqlite_destination_sidecars_and_same_source_target_are_refused() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.db");
    seed(&source_path).await;
    let source = endpoint(&source_path);
    let destination = endpoint(&source_path);
    let artifact = directory.path().join("copy.db");
    let copy = spec(
        DataOperation::Copy,
        Some(&source),
        Some(&destination),
        &artifact,
    );
    let connector = DatabaseConnector::new();
    let token = CancellationToken::new();
    assert!(
        native::plan(&copy, Some(&source), Some(&destination), &connector, &token)
            .await
            .is_err()
    );
    let target = directory.path().join("target.db");
    let destination = endpoint(&target);
    std::fs::write(directory.path().join("target.db-wal"), b"existing WAL").unwrap();
    let restore = spec(
        DataOperation::Restore,
        None,
        Some(&destination),
        &source_path,
    );
    assert!(
        native::plan(&restore, None, Some(&destination), &connector, &token)
            .await
            .is_err()
    );
    assert!(!target.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_inputs_and_outputs_are_never_followed() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let real = directory.path().join("real.db");
    seed(&real).await;
    let alias = directory.path().join("alias.db");
    symlink(&real, &alias).unwrap();
    let source = endpoint(&alias);
    let artifact = directory.path().join("backup.db");
    let connector = DatabaseConnector::new();
    let token = CancellationToken::new();
    let backup = spec(DataOperation::Backup, Some(&source), None, &artifact);
    assert!(
        native::plan(&backup, Some(&source), None, &connector, &token)
            .await
            .is_err()
    );
    let source = endpoint(&real);
    symlink(directory.path().join("nonexistent"), &artifact).unwrap();
    let backup = spec(DataOperation::Backup, Some(&source), None, &artifact);
    assert!(
        native::plan(&backup, Some(&source), None, &connector, &token)
            .await
            .is_err()
    );
}
