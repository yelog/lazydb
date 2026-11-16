use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use futures_util::TryStreamExt;
use sqlx::{AssertSqlSafe, Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use tokio_util::sync::CancellationToken;

use crate::{
    data_ops::{
        DataJobSpec, DataJobStatus, DataOperation, NativeOutcome, NativePlan, ProgressSink,
        ResolvedEndpoint,
    },
    profile::DatabaseKind,
};

use super::common::{self, PinnedInput, StagedArtifact};

const ROWID_WARNING: &str = "SQLite VACUUM INTO may change implicit ROWIDs in tables without an explicit INTEGER PRIMARY KEY; logical contents are preserved, not physical page or ROWID identity.";

fn source_path(endpoint: &ResolvedEndpoint) -> Result<PathBuf> {
    ensure!(
        endpoint.profile.ssh.is_none(),
        "SQLite does not support SSH routes"
    );
    ensure!(
        endpoint.database != ":memory:",
        "native SQLite operations require an existing on-disk database"
    );
    common::input_path(Path::new(&endpoint.database))
}

fn destination_path(endpoint: &ResolvedEndpoint) -> Result<PathBuf> {
    ensure!(
        endpoint.profile.ssh.is_none(),
        "SQLite does not support SSH routes"
    );
    common::sqlite_output(Path::new(&endpoint.database))
}

async fn connection(path: &Path, cancellation: &CancellationToken) -> Result<SqliteConnection> {
    common::check_cancel(cancellation)?;
    common::signature(path, b"SQLite format 3\0")?;
    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .create_if_missing(false)
        .busy_timeout(std::time::Duration::from_millis(250));
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .context("cannot open SQLite database read-only")?;
    let token = cancellation.clone();
    connection
        .lock_handle()
        .await?
        .set_progress_handler(1000, move || !token.is_cancelled());
    common::check_cancel(cancellation)?;
    Ok(connection)
}

async fn integrity(path: &Path, cancellation: &CancellationToken) -> Result<()> {
    let mut connection = connection(path, cancellation).await?;
    let result = async {
        let mut rows =
            sqlx::query_scalar::<_, String>("PRAGMA integrity_check").fetch(&mut connection);
        let mut count = 0;
        while let Some(row) = rows.try_next().await? {
            common::check_cancel(cancellation)?;
            ensure!(row == "ok", "SQLite integrity_check failed: {row}");
            count += 1;
        }
        ensure!(count == 1, "SQLite integrity_check returned no result");
        Ok::<_, anyhow::Error>(())
    }
    .await;
    connection.close().await?;
    common::check_cancel(cancellation)?;
    result
}

pub(super) async fn plan(
    spec: &DataJobSpec,
    source: Option<&ResolvedEndpoint>,
    destination: Option<&ResolvedEndpoint>,
    cancellation: &CancellationToken,
) -> Result<NativePlan> {
    let artifact = if spec.operation == DataOperation::Restore {
        common::input_path(&spec.artifact)?
    } else {
        common::output_path(&spec.artifact)?
    };
    if let Some(source) = source {
        let source = source_path(source)?;
        ensure!(
            source != artifact,
            "artifact and source must be different files"
        );
        integrity(&source, cancellation).await?;
        let mut connection = connection(&source, cancellation).await?;
        let version: String = sqlx::query_scalar("SELECT sqlite_version()")
            .fetch_one(&mut connection)
            .await?;
        connection.close().await?;
        ensure!(
            common::version_number(&version)? >= (3, 27, 0),
            "SQLite 3.27 or newer is required for VACUUM INTO"
        );
    }
    if let Some(destination) = destination {
        let destination = destination_path(destination)?;
        ensure!(
            destination != artifact,
            "artifact and destination must be different files"
        );
        if let Some(source) = source {
            ensure!(
                source_path(source)? != destination,
                "source and destination must be different files"
            );
        }
    }
    if spec.operation == DataOperation::Restore {
        integrity(&artifact, cancellation).await?;
    }
    let steps = match spec.operation {
        DataOperation::Backup => vec![
            "Snapshot the complete SQLite main database with VACUUM INTO a private partial file"
                .into(),
            "Run SQLite integrity_check, hash the snapshot, and publish without overwriting".into(),
        ],
        DataOperation::Restore => vec![
            "Pin and validate the SQLite snapshot with integrity_check".into(),
            "Stream the snapshot to a new file, verify integrity, and publish exclusively".into(),
        ],
        DataOperation::Copy => vec![
            "Create, verify, hash, and exclusively publish a complete SQLite snapshot".into(),
            "Stream the completed snapshot to a new destination and verify SQLite integrity".into(),
            "Keep the completed backup artifact if destination restoration fails".into(),
        ],
    };
    let mut warnings = vec![
        "Only the main on-disk SQLite database is included; attached databases and temporary objects are excluded.".into(),
        "SQLite integrity_check verifies structural integrity, not application-level data parity.".into(),
    ];
    if spec.operation != DataOperation::Restore {
        warnings.push(ROWID_WARNING.into());
    }
    Ok(NativePlan {
        engine: DatabaseKind::Sqlite,
        steps,
        warnings,
        tools: Vec::new(),
    })
}

pub(super) async fn execute(
    spec: &DataJobSpec,
    plan: &NativePlan,
    source: Option<&ResolvedEndpoint>,
    destination: Option<&ResolvedEndpoint>,
    pinned: Option<&PinnedInput>,
    cancellation: &CancellationToken,
    progress: &ProgressSink,
) -> Result<NativeOutcome> {
    let mut outcome = NativeOutcome {
        warnings: plan.warnings.clone(),
        ..NativeOutcome::default()
    };
    let artifact = if spec.operation != DataOperation::Restore {
        let source = source_path(source.context("SQLite backup source is missing")?)?;
        let output = common::output_path(&spec.artifact)?;
        let stage = StagedArtifact::new(&output)?;
        let partial = stage.path().to_path_buf();
        let snapshot = async {
            common::progress(
                progress,
                DataJobStatus::Running,
                "Creating SQLite VACUUM INTO snapshot",
                None,
            );
            let mut connection = connection(&source, cancellation).await?;
            let statement = "VACUUM main INTO ?";
            let result = sqlx::query(AssertSqlSafe(statement))
                .bind(common::path_text(stage.path())?)
                .execute(&mut connection)
                .await;
            connection.close().await?;
            common::check_cancel(cancellation)?;
            result.context("SQLite VACUUM INTO failed")?;
            common::progress(
                progress,
                DataJobStatus::Verifying,
                "Checking SQLite snapshot integrity",
                None,
            );
            integrity(stage.path(), cancellation).await?;
            let (bytes, digest) = common::fingerprint(stage.path(), cancellation).await?;
            common::check_cancel(cancellation)?;
            stage.publish(&output)?;
            Ok::<_, anyhow::Error>((bytes, digest))
        }
        .await
        .with_context(|| {
            format!(
                "SQLite snapshot failed; incomplete artifact retained at {}",
                partial.display()
            )
        })?;
        outcome.bytes = Some(snapshot.0);
        outcome.sha256 = Some(snapshot.1);
        outcome.artifact = Some(output.clone());
        outcome
            .verification
            .push("SQLite backup PRAGMA integrity_check returned ok".into());
        common::backup_completed(progress, &outcome);
        output
    } else {
        let input = pinned.context("SQLite restore input was not pinned")?;
        integrity(input.path(), cancellation).await?;
        outcome.bytes = Some(input.bytes);
        outcome.sha256 = Some(input.sha256.clone());
        outcome.artifact = Some(spec.artifact.clone());
        input.path().to_path_buf()
    };
    if spec.operation.writes_database() {
        common::check_cancel(cancellation)?;
        let destination = destination.context("SQLite destination is missing")?;
        let output = destination_path(destination)?;
        let stage = StagedArtifact::new(&output)?;
        let partial = stage.path().to_path_buf();
        let restore = async {
            common::progress(
                progress,
                DataJobStatus::Running,
                "Streaming SQLite snapshot into a new file",
                outcome.bytes,
            );
            let copied = common::copy_file(&artifact, stage.writer()?, cancellation).await?;
            ensure!(
                Some(copied.0) == outcome.bytes && Some(&copied.1) == outcome.sha256.as_ref(),
                "SQLite input changed while restoring"
            );
            common::progress(
                progress,
                DataJobStatus::Verifying,
                "Checking new SQLite database integrity",
                outcome.bytes,
            );
            integrity(stage.path(), cancellation).await?;
            common::check_cancel(cancellation)?;
            common::sqlite_output(&output)?;
            stage.publish(&output)?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        restore.with_context(|| format!(
            "SQLite restore failed; partial file retained at {}; completed input artifact kept at {}",
            partial.display(), spec.artifact.display()
        ))?;
        outcome.target_created = true;
        outcome.verification.push("New SQLite database PRAGMA integrity_check returned ok; copied bytes match the snapshot SHA-256".into());
    }
    common::check_cancel(cancellation)?;
    Ok(outcome)
}
