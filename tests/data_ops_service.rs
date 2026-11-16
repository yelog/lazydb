use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use lazydb::{
    agent::policy::WritePolicy,
    connections::DatabaseConnector,
    data_ops::{
        DataEndpoint, DataJobRecord, DataJobService, DataJobSpec, DataJobStatus, DataJobStore,
        DataOperation, NativeToolPaths,
        service::{ArtifactManifest, hash_file, manifest_path},
    },
    persistence::{
        credentials::CredentialResolver, local_credentials::LocalCredentialStore,
        secrets::NativeSecretStore,
    },
    profile::{ConnectionProfile, Environment, import_connection_url},
};
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use tokio_util::sync::CancellationToken;

fn service(root: &Path, profiles: Vec<ConnectionProfile>) -> DataJobService {
    DataJobService::new(
        profiles,
        Arc::new(CredentialResolver::new(
            Arc::new(NativeSecretStore),
            LocalCredentialStore::new(root.join("credential.key"), "test-data-jobs"),
        )),
        DatabaseConnector::new(),
        DataJobStore::new(root.join("jobs")),
    )
}

async fn source_database(root: &Path) -> ConnectionProfile {
    let path = root.join("source.sqlite");
    let mut connection = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::raw_sql(
        "CREATE TABLE records (id INTEGER PRIMARY KEY, value TEXT, binary_value BLOB);
         WITH RECURSIVE numbers(value) AS (
           VALUES(1) UNION ALL SELECT value + 1 FROM numbers WHERE value < 12001
         )
         INSERT INTO records SELECT value, 'hello 世界', X'00FF1020' FROM numbers;
         INSERT INTO records VALUES (12002, NULL, NULL);
         INSERT INTO records VALUES (12003, '', X'');
         CREATE INDEX records_value ON records(value);",
    )
    .execute(&mut connection)
    .await
    .unwrap();
    connection.close().await.unwrap();
    let mut profile =
        import_connection_url(&format!("sqlite://{}", path.display()), Some("source"))
            .unwrap()
            .profile;
    profile.read_only = false;
    profile
}

fn backup(profile: &ConnectionProfile, root: &Path) -> DataJobSpec {
    DataJobSpec {
        operation: DataOperation::Backup,
        source: Some(DataEndpoint::from_profile(profile)),
        destination: None,
        artifact: root.join("snapshot.sqlite"),
        tools: NativeToolPaths::default(),
    }
}

fn restore(profile: &ConnectionProfile, root: &Path) -> DataJobSpec {
    DataJobSpec {
        operation: DataOperation::Restore,
        source: None,
        destination: Some(DataEndpoint {
            profile_id: profile.id,
            database: root.join("restored.sqlite").to_str().unwrap().to_owned(),
        }),
        artifact: root.join("snapshot.sqlite"),
        tools: NativeToolPaths::default(),
    }
}

async fn execute(service: &DataJobService, spec: DataJobSpec) -> DataJobRecord {
    let token = CancellationToken::new();
    let plan = service
        .plan(spec, WritePolicy::NonProduction, token.clone())
        .await
        .unwrap();
    let record = service
        .run(plan, WritePolicy::NonProduction, token, Arc::new(|_| {}))
        .await
        .unwrap();
    assert!(record.status.is_success(), "{record:#?}");
    record
}

#[tokio::test]
async fn backup_restore_and_history_preserve_all_rows_and_values() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let profile = source_database(root).await;
    let jobs = service(root, vec![profile.clone()]);
    let spec = backup(&profile, root);
    let token = CancellationToken::new();
    let plan = jobs
        .plan(spec.clone(), WritePolicy::Deny, token.clone())
        .await
        .unwrap();
    assert!(!spec.artifact.exists());
    assert!(jobs.store().list().unwrap().is_empty());
    let saved_plan = plan.clone();
    let completed = jobs
        .run(plan, WritePolicy::Deny, token, Arc::new(|_| {}))
        .await
        .unwrap();
    assert!(completed.status.is_success(), "{completed:#?}");
    assert!(manifest_path(&spec.artifact).unwrap().exists());
    assert!(completed.outcome.as_ref().unwrap().sha256.is_some());
    assert!(
        jobs.run(
            saved_plan,
            WritePolicy::Deny,
            CancellationToken::new(),
            Arc::new(|_| {})
        )
        .await
        .is_err()
    );

    let restored = execute(&jobs, restore(&profile, root)).await;
    let mut connection = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(root.join("restored.sqlite"))
            .read_only(true),
    )
    .await
    .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM records")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(count, 12003);
    let values: (String, Vec<u8>) =
        sqlx::query_as("SELECT value, binary_value FROM records WHERE id = 1")
            .fetch_one(&mut connection)
            .await
            .unwrap();
    assert_eq!(values, ("hello 世界".into(), vec![0, 255, 16, 32]));
    let null_value: Option<String> =
        sqlx::query_scalar("SELECT value FROM records WHERE id = 12002")
            .fetch_one(&mut connection)
            .await
            .unwrap();
    assert_eq!(null_value, None);
    let empty: String = sqlx::query_scalar("SELECT value FROM records WHERE id = 12003")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(empty, "");
    connection.close().await.unwrap();
    assert_eq!(jobs.store().list().unwrap().len(), 2);
    assert_eq!(jobs.store().load(restored.id).unwrap(), restored);
    assert!(
        jobs.plan(
            restore(&profile, root),
            WritePolicy::All,
            CancellationToken::new()
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn changed_connection_invalidates_reviewed_plan_before_backup() {
    let directory = tempfile::tempdir().unwrap();
    let profile = source_database(directory.path()).await;
    let jobs = service(directory.path(), vec![profile.clone()]);
    let spec = backup(&profile, directory.path());
    let plan = jobs
        .plan(spec.clone(), WritePolicy::Deny, CancellationToken::new())
        .await
        .unwrap();
    let mut changed = profile;
    changed.name = "renamed after review".into();
    let changed_jobs = service(directory.path(), vec![changed]);
    let result = changed_jobs
        .run(
            plan,
            WritePolicy::Deny,
            CancellationToken::new(),
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
    assert_eq!(result.status, DataJobStatus::Failed);
    assert!(result.message.contains("changed after review"));
    assert!(!spec.artifact.exists());
}

#[tokio::test]
async fn corrupted_archive_is_rejected_before_target_creation() {
    use std::io::Write;
    let directory = tempfile::tempdir().unwrap();
    let profile = source_database(directory.path()).await;
    let jobs = service(directory.path(), vec![profile.clone()]);
    execute(&jobs, backup(&profile, directory.path())).await;
    let spec = restore(&profile, directory.path());
    let plan = jobs
        .plan(spec.clone(), WritePolicy::All, CancellationToken::new())
        .await
        .unwrap();
    std::fs::OpenOptions::new()
        .append(true)
        .open(&spec.artifact)
        .unwrap()
        .write_all(b"changed after planning")
        .unwrap();
    let record = jobs
        .run(
            plan,
            WritePolicy::All,
            CancellationToken::new(),
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
    assert_eq!(record.status, DataJobStatus::Failed);
    assert!(record.message.contains("checksum"));
    assert!(!record.target_may_be_partial);
    assert!(!directory.path().join("restored.sqlite").exists());
}

#[tokio::test]
async fn read_only_and_production_policies_apply_before_native_preflight() {
    let directory = tempfile::tempdir().unwrap();
    let mut profile = source_database(directory.path()).await;
    profile.read_only = true;
    let jobs = service(directory.path(), vec![profile.clone()]);
    let error = jobs
        .plan(
            restore(&profile, directory.path()),
            WritePolicy::All,
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("read-only"));
    let backup_plan = jobs
        .plan(
            backup(&profile, directory.path()),
            WritePolicy::Deny,
            CancellationToken::new(),
        )
        .await;
    assert!(backup_plan.is_ok(), "{backup_plan:?}");

    profile.read_only = false;
    profile.environment = Environment::Production;
    let jobs = service(directory.path(), vec![profile.clone()]);
    let error = jobs
        .plan(
            restore(&profile, directory.path()),
            WritePolicy::NonProduction,
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("production"));
}

#[tokio::test]
async fn cancellation_before_execution_leaves_no_archive_or_target() {
    let directory = tempfile::tempdir().unwrap();
    let profile = source_database(directory.path()).await;
    let jobs = service(directory.path(), vec![profile.clone()]);
    let spec = backup(&profile, directory.path());
    let plan = jobs
        .plan(spec.clone(), WritePolicy::Deny, CancellationToken::new())
        .await
        .unwrap();
    let token = CancellationToken::new();
    token.cancel();
    let result = jobs
        .run(plan, WritePolicy::Deny, token, Arc::new(|_| {}))
        .await
        .unwrap();
    assert_eq!(result.status, DataJobStatus::Cancelled);
    assert!(!spec.artifact.exists());
    assert!(!result.target_may_be_partial);
}

#[tokio::test]
async fn copy_reads_a_read_only_source_keeps_its_archive_and_refuses_existing_outputs() {
    let directory = tempfile::tempdir().unwrap();
    let mut profile = source_database(directory.path()).await;
    let mut destination = profile.clone();
    destination.id = uuid::Uuid::new_v4();
    destination.name = "destination".into();
    profile.read_only = true;
    let jobs = service(directory.path(), vec![profile.clone(), destination.clone()]);
    let mut spec = backup(&profile, directory.path());
    spec.operation = DataOperation::Copy;
    spec.destination = restore(&destination, directory.path()).destination;
    let completed = execute(&jobs, spec.clone()).await;
    assert!(completed.outcome.unwrap().target_created);
    assert!(spec.artifact.exists());
    assert!(directory.path().join("restored.sqlite").exists());
    assert!(
        jobs.plan(spec, WritePolicy::All, CancellationToken::new())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn failed_copy_preserves_verified_backup_metadata_before_restoring() {
    let directory = tempfile::tempdir().unwrap();
    let profile = source_database(directory.path()).await;
    let jobs = service(directory.path(), vec![profile.clone()]);
    let mut spec = backup(&profile, directory.path());
    spec.operation = DataOperation::Copy;
    spec.destination = restore(&profile, directory.path()).destination;
    let token = CancellationToken::new();
    let plan = jobs
        .plan(spec.clone(), WritePolicy::All, token.clone())
        .await
        .unwrap();
    let target = directory.path().join("restored.sqlite");
    let racing_target = target.clone();
    let metadata_path = manifest_path(&spec.artifact).unwrap();
    let checkpoint_metadata = metadata_path.clone();
    let checkpoint_seen = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&checkpoint_seen);
    let completed = jobs
        .run(
            plan,
            WritePolicy::All,
            token,
            Arc::new(move |record| {
                if !record.status.is_terminal()
                    && record.outcome.is_some()
                    && !seen.swap(true, Ordering::SeqCst)
                {
                    assert!(checkpoint_metadata.exists());
                    std::fs::write(&racing_target, b"another process owns this target").unwrap();
                }
            }),
        )
        .await
        .unwrap();
    assert!(checkpoint_seen.load(Ordering::SeqCst));
    assert_eq!(completed.status, DataJobStatus::Failed);
    let retained = completed.outcome.as_ref().unwrap();
    let expected_artifact = spec.artifact.canonicalize().unwrap();
    assert_eq!(retained.artifact.as_ref(), Some(&expected_artifact));
    assert!(!retained.target_created);
    let manifest: ArtifactManifest =
        serde_json::from_slice(&std::fs::read(metadata_path).unwrap()).unwrap();
    let (bytes, sha256) = hash_file(&spec.artifact, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(retained.bytes, Some(bytes));
    assert_eq!(retained.sha256.as_ref(), Some(&sha256));
    assert_eq!(manifest.bytes, bytes);
    assert_eq!(manifest.sha256, sha256);
    assert_eq!(
        std::fs::read(target).unwrap(),
        b"another process owns this target"
    );
    assert_eq!(jobs.store().load(completed.id).unwrap(), completed);
}

#[tokio::test]
async fn backup_metadata_publication_failure_stops_copy_before_destination_writes() {
    let directory = tempfile::tempdir().unwrap();
    let profile = source_database(directory.path()).await;
    let jobs = service(directory.path(), vec![profile.clone()]);
    let mut spec = backup(&profile, directory.path());
    spec.operation = DataOperation::Copy;
    spec.destination = restore(&profile, directory.path()).destination;
    let token = CancellationToken::new();
    let plan = jobs
        .plan(spec.clone(), WritePolicy::All, token.clone())
        .await
        .unwrap();
    let metadata_path = manifest_path(&spec.artifact).unwrap();
    let racing_metadata = metadata_path.clone();
    let completed = jobs
        .run(
            plan,
            WritePolicy::All,
            token,
            Arc::new(move |record| {
                if record.message == "Creating SQLite VACUUM INTO snapshot" {
                    std::fs::write(&racing_metadata, b"another process owns this metadata")
                        .unwrap();
                }
            }),
        )
        .await
        .unwrap();
    assert_eq!(completed.status, DataJobStatus::Failed);
    assert!(completed.message.contains("metadata"));
    assert!(!directory.path().join("restored.sqlite").exists());
    assert_eq!(
        std::fs::read(metadata_path).unwrap(),
        b"another process owns this metadata"
    );
    let retained = completed.outcome.as_ref().unwrap();
    assert!(!retained.target_created);
    let (bytes, sha256) = hash_file(&spec.artifact, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(retained.bytes, Some(bytes));
    assert_eq!(retained.sha256.as_ref(), Some(&sha256));
    assert_eq!(jobs.store().load(completed.id).unwrap(), completed);
}
