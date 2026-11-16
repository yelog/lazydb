use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, ensure};
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
    profile::{
        CatalogScope, CatalogSelection, ConnectionProfile, DatabaseKind, SslMode,
        import_connection_url,
    },
};
use secrecy::{ExposeSecret, SecretString};
use sqlx::{
    Connection, MySqlConnection, PgConnection,
    mysql::{MySqlConnectOptions, MySqlSslMode},
    postgres::{PgConnectOptions, PgSslMode},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const ROWS: i64 = 12_003;
const NEXT_ID: i64 = 15_000;
const TEXT: &str = "hello 世界 'quoted' \\ path\nline";
const NUMBER: &str = "12345678901234567890.123456789";

struct Fixture {
    engine: &'static str,
    root: PathBuf,
    profile: ConnectionProfile,
    password: SecretString,
    tools: NativeToolPaths,
    database_prefix: String,
}

impl Fixture {
    fn load(engine: &'static str, kind: DatabaseKind) -> Result<Option<Self>> {
        let prefix = format!("LAZYDB_DATA_OPS_{}", engine.to_ascii_uppercase());
        let url_name = format!("{prefix}_URL");
        let url = match std::env::var(&url_name) {
            Ok(value) => value,
            Err(std::env::VarError::NotPresent) => {
                eprintln!(
                    "SKIP native {engine} acceptance: {url_name} is absent. \
                     Run bash tests/fixtures/data_ops/run.sh {engine}; \
                     no native {engine} validation ran."
                );
                return Ok(None);
            }
            Err(error) => return Err(error).context("fixture URL must be valid Unicode"),
        };
        let root = PathBuf::from(required_env("LAZYDB_DATA_OPS_FIXTURE_ROOT")?)
            .canonicalize()
            .context("locate the private Docker fixture root")?;
        let id = Uuid::parse_str(std::fs::read_to_string(root.join("fixture-id"))?.trim())?;
        let database_prefix = format!("ldops_{}", id.simple());
        let port: u16 = std::fs::read_to_string(root.join(format!("{engine}.port")))?
            .trim()
            .parse()?;
        let imported = import_connection_url(&url, Some("isolated-data-ops-acceptance"))
            .context("import the test-only connection URL")?;
        ensure!(
            imported.transient_password.is_none(),
            "fixture URLs must not contain credentials; use the fixture password variable"
        );
        let mut profile = imported.profile;
        ensure!(
            profile.kind == kind,
            "fixture URL has the wrong database engine"
        );
        ensure!(
            profile.host.as_deref() == Some("127.0.0.1") && profile.port == Some(port),
            "fixture URL must use its isolated loopback port"
        );
        ensure!(
            profile.database.as_deref() == Some(&format!("{database_prefix}_source")),
            "fixture URL must select this run's uniquely named source"
        );
        let user = if kind == DatabaseKind::Postgres {
            "lazydb_fixture"
        } else {
            "root"
        };
        ensure!(
            profile.user.as_deref() == Some(user),
            "unexpected fixture user"
        );
        ensure!(
            std::env::temp_dir().canonicalize()? == root,
            "TMPDIR must equal the same-path-mounted fixture root"
        );
        profile.ssl_mode = SslMode::Disable;
        profile.read_only = false;
        profile.catalog_scope = CatalogScope {
            databases: CatalogSelection::All,
        };
        let (dump, restore) = match kind {
            DatabaseKind::Postgres => ("pg_dump", "pg_restore"),
            DatabaseKind::MySql => ("mysqldump", "mysql"),
            DatabaseKind::MariaDb => ("mariadb-dump", "mariadb"),
            _ => unreachable!(),
        };
        let tools = NativeToolPaths {
            dump: Some(wrapper(&root, &format!("{prefix}_DUMP"), dump)?),
            restore: Some(wrapper(&root, &format!("{prefix}_RESTORE"), restore)?),
        };
        Ok(Some(Self {
            engine,
            root,
            profile,
            password: required_env(&format!("{prefix}_PASSWORD"))?.into(),
            tools,
            database_prefix,
        }))
    }

    fn endpoint(&self, suffix: &str) -> DataEndpoint {
        DataEndpoint {
            profile_id: self.profile.id,
            database: format!("{}_{suffix}", self.database_prefix),
        }
    }

    async fn postgres(&self, database: &str) -> Result<PgConnection> {
        PgConnection::connect_with(
            &PgConnectOptions::new()
                .host("127.0.0.1")
                .port(self.profile.port.unwrap())
                .username("lazydb_fixture")
                .password(self.password.expose_secret())
                .database(database)
                .ssl_mode(PgSslMode::Disable),
        )
        .await
        .context("connect to isolated PostgreSQL fixture")
    }

    async fn mysql(&self, database: &str) -> Result<MySqlConnection> {
        MySqlConnection::connect_with(
            &MySqlConnectOptions::new()
                .host("127.0.0.1")
                .port(self.profile.port.unwrap())
                .username("root")
                .password(self.password.expose_secret())
                .database(database)
                .ssl_mode(MySqlSslMode::Disabled),
        )
        .await
        .context("connect to isolated MySQL/MariaDB fixture")
    }

    async fn seed(&self) -> Result<()> {
        let source = self.endpoint("source").database;
        if self.profile.kind == DatabaseKind::Postgres {
            let mut connection = self.postgres(&source).await?;
            sqlx::raw_sql(include_str!("fixtures/data_ops/postgres.sql"))
                .execute(&mut connection)
                .await?;
            connection.close().await?;
        } else {
            let mut connection = self.mysql(&source).await?;
            sqlx::raw_sql(include_str!("fixtures/data_ops/mysql.sql"))
                .execute(&mut connection)
                .await?;
            connection.close().await?;
        }
        Ok(())
    }

    async fn row_count(&self, database: &str) -> Result<i64> {
        if self.profile.kind == DatabaseKind::Postgres {
            let mut connection = self.postgres(database).await?;
            let count = sqlx::query_scalar("SELECT COUNT(*) FROM acceptance_records")
                .fetch_one(&mut connection)
                .await?;
            connection.close().await?;
            Ok(count)
        } else {
            let mut connection = self.mysql(database).await?;
            let count = sqlx::query_scalar("SELECT COUNT(*) FROM acceptance_records")
                .fetch_one(&mut connection)
                .await?;
            connection.close().await?;
            Ok(count)
        }
    }

    async fn verify_new_database(&self, database: &str) -> Result<()> {
        if self.profile.kind == DatabaseKind::Postgres {
            let mut connection = self.postgres(database).await?;
            let values: (String, Vec<u8>, String, i64, f64, bool) = sqlx::query_as(
                "SELECT text_value, binary_value, numeric_value::text, integer_value, \
                 float_value, boolean_value FROM acceptance_records WHERE id = 1",
            )
            .fetch_one(&mut connection)
            .await?;
            assert_values(values);
            let nulls: bool = sqlx::query_scalar(
                "SELECT text_value IS NULL AND binary_value IS NULL AND numeric_value IS NULL \
                 AND integer_value IS NULL AND float_value IS NULL AND boolean_value IS NULL \
                 FROM acceptance_records WHERE id = 12002",
            )
            .fetch_one(&mut connection)
            .await?;
            assert!(nulls);
            let empty: (String, Vec<u8>, String) = sqlx::query_as(
                "SELECT text_value, binary_value, numeric_value::text \
                 FROM acceptance_records WHERE id = 12003",
            )
            .fetch_one(&mut connection)
            .await?;
            assert_eq!(empty, (String::new(), Vec::new(), "-0.000000001".into()));
            let types: Vec<String> = sqlx::query_scalar(
                "SELECT data_type FROM information_schema.columns \
                 WHERE table_schema = 'public' AND table_name = 'acceptance_records' \
                 ORDER BY ordinal_position",
            )
            .fetch_all(&mut connection)
            .await?;
            assert_eq!(
                types,
                [
                    "bigint",
                    "text",
                    "bytea",
                    "numeric",
                    "bigint",
                    "double precision",
                    "boolean"
                ]
            );
            let view_count: i64 = sqlx::query_scalar("SELECT total FROM acceptance_summary")
                .fetch_one(&mut connection)
                .await?;
            assert_eq!(view_count, ROWS);
            let note: String = sqlx::query_scalar("SELECT value FROM acceptance_extra.notes")
                .fetch_one(&mut connection)
                .await?;
            assert_eq!(note, "another schema");
            let next: i64 = sqlx::query_scalar(
                "INSERT INTO acceptance_records(text_value) VALUES ('after restore') RETURNING id",
            )
            .fetch_one(&mut connection)
            .await?;
            assert_eq!(next, NEXT_ID);
            connection.close().await?;
        } else {
            let mut connection = self.mysql(database).await?;
            let values: (String, Vec<u8>, String, i64, f64, bool) = sqlx::query_as(
                "SELECT text_value, binary_value, CAST(numeric_value AS CHAR), integer_value, \
                 float_value, boolean_value FROM acceptance_records WHERE id = 1",
            )
            .fetch_one(&mut connection)
            .await?;
            assert_values(values);
            let nulls: i32 = sqlx::query_scalar(
                "SELECT text_value IS NULL AND binary_value IS NULL AND numeric_value IS NULL \
                 AND integer_value IS NULL AND float_value IS NULL AND boolean_value IS NULL \
                 FROM acceptance_records WHERE id = 12002",
            )
            .fetch_one(&mut connection)
            .await?;
            assert_eq!(nulls, 1);
            let empty: (String, Vec<u8>, String) = sqlx::query_as(
                "SELECT text_value, binary_value, CAST(numeric_value AS CHAR) \
                 FROM acceptance_records WHERE id = 12003",
            )
            .fetch_one(&mut connection)
            .await?;
            assert_eq!(empty, (String::new(), Vec::new(), "-0.000000001".into()));
            let types: Vec<String> = sqlx::query_scalar(
                "SELECT DATA_TYPE FROM information_schema.COLUMNS \
                 WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'acceptance_records' \
                 ORDER BY ORDINAL_POSITION",
            )
            .fetch_all(&mut connection)
            .await?;
            assert_eq!(
                types,
                [
                    "bigint", "longtext", "blob", "decimal", "bigint", "double", "tinyint"
                ]
            );
            let note: String = sqlx::query_scalar("SELECT value FROM acceptance_notes")
                .fetch_one(&mut connection)
                .await?;
            assert_eq!(note, "another table");
            let inserted =
                sqlx::query("INSERT INTO acceptance_records(text_value) VALUES ('after restore')")
                    .execute(&mut connection)
                    .await?;
            assert_eq!(inserted.last_insert_id(), NEXT_ID as u64);
            connection.close().await?;
        }
        assert_eq!(self.row_count(database).await?, ROWS + 1);
        Ok(())
    }
}

fn required_env(name: &str) -> Result<String> {
    let value = std::env::var(name).with_context(|| format!("missing fixture variable {name}"))?;
    ensure!(!value.is_empty(), "fixture variable {name} is empty");
    Ok(value)
}

fn wrapper(root: &Path, variable: &str, name: &str) -> Result<PathBuf> {
    let path = PathBuf::from(required_env(variable)?).canonicalize()?;
    ensure!(
        path == root.join("bin").join(name) && path.is_file(),
        "{variable} must name the wrapper inside this fixture"
    );
    Ok(path)
}

fn assert_values(values: (String, Vec<u8>, String, i64, f64, bool)) {
    assert_eq!(
        values,
        (
            TEXT.into(),
            vec![0, 255, 16, 32, 92, 10],
            NUMBER.into(),
            -9_223_372_036_854_775_000,
            1.25,
            true,
        )
    );
}

async fn execute(service: &DataJobService, spec: DataJobSpec) -> Result<DataJobRecord> {
    let operation = spec.operation;
    let token = CancellationToken::new();
    let plan = service
        .plan(spec, WritePolicy::All, token.clone())
        .await
        .with_context(|| format!("{operation:?} planning failed"))?;
    let record = service
        .run(plan, WritePolicy::All, token, Arc::new(|_| {}))
        .await
        .with_context(|| format!("{operation:?} execution failed"))?;
    ensure!(
        record.status.is_success(),
        "{operation:?} ended {:?}: {}",
        record.status,
        record.error.as_deref().unwrap_or(&record.message)
    );
    assert_eq!(service.store().load(record.id)?, record);
    Ok(record)
}

async fn refused(service: &DataJobService, spec: DataJobSpec) -> Result<()> {
    let error = service
        .plan(spec, WritePolicy::All, CancellationToken::new())
        .await
        .expect_err("an existing archive or database must be refused");
    ensure!(
        format!("{error:#}").to_ascii_lowercase().contains("exist"),
        "refusal must identify the existing output: {error:#}"
    );
    Ok(())
}

async fn failed_copy_keeps_reusable_backup(
    fixture: &Fixture,
    service: &DataJobService,
    root: &Path,
) -> Result<()> {
    let spec = DataJobSpec {
        operation: DataOperation::Copy,
        source: Some(fixture.endpoint("source")),
        destination: Some(fixture.endpoint("failed_copy")),
        artifact: root.join("failed-copy.archive"),
        tools: fixture.tools.clone(),
    };
    let plan = service
        .plan(spec.clone(), WritePolicy::All, CancellationToken::new())
        .await?;
    let restore = fixture.tools.restore.as_ref().unwrap();
    let name = restore
        .file_name()
        .and_then(|value| value.to_str())
        .context("fixture restore wrapper has no UTF-8 name")?;
    let marker = restore.with_file_name(format!("fail-{name}"));
    let metadata_path = manifest_path(&spec.artifact)?;
    std::fs::write(
        &marker,
        metadata_path
            .to_str()
            .context("fixture manifest path is not UTF-8")?,
    )?;
    let result = service
        .run(
            plan,
            WritePolicy::All,
            CancellationToken::new(),
            Arc::new(|_| {}),
        )
        .await;
    std::fs::remove_file(marker)?;
    let failed = result?;
    assert_eq!(failed.status, DataJobStatus::Failed);
    ensure!(
        failed
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("Injected fixture restore failure"),
        "copy must reach the injected restore failure after publishing metadata: {}",
        failed.message
    );
    assert!(failed.target_may_be_partial);
    assert_eq!(service.store().load(failed.id)?, failed);
    let outcome = failed
        .outcome
        .as_ref()
        .context("failed copy did not retain its completed backup outcome")?;
    let (bytes, digest) = hash_file(&spec.artifact, &CancellationToken::new()).await?;
    assert!(bytes > 0);
    assert_eq!(outcome.artifact.as_ref(), Some(&spec.artifact));
    assert_eq!(outcome.bytes, Some(bytes));
    assert_eq!(outcome.sha256.as_deref(), Some(digest.as_str()));
    let metadata = std::fs::read(&metadata_path)?;
    let manifest: ArtifactManifest = serde_json::from_slice(&metadata)?;
    assert_eq!(manifest.engine, fixture.profile.kind);
    assert_eq!(manifest.source, spec.source);
    assert_eq!(manifest.bytes, bytes);
    assert_eq!(manifest.sha256, digest);

    let recovery = DataJobSpec {
        operation: DataOperation::Restore,
        source: None,
        destination: Some(fixture.endpoint("recovered")),
        artifact: spec.artifact.clone(),
        tools: fixture.tools.clone(),
    };
    let recovered = execute(service, recovery).await?;
    assert!(recovered.outcome.as_ref().unwrap().target_created);
    fixture
        .verify_new_database(&fixture.endpoint("recovered").database)
        .await?;
    assert_eq!(std::fs::read(metadata_path)?, metadata);
    assert_eq!(
        hash_file(&spec.artifact, &CancellationToken::new()).await?,
        (bytes, digest)
    );
    eprintln!(
        "ACCEPT {}: failed copy retained checkpoint outcome and pre-restore manifest; \
         retained archive restored and row/type/next-ID verified",
        fixture.engine
    );
    Ok(())
}

async fn acceptance(fixture: Fixture) -> Result<()> {
    fixture.seed().await?;
    let directory = tempfile::Builder::new()
        .prefix(&format!("{}-acceptance-", fixture.engine))
        .tempdir_in(&fixture.root)?;
    let root = directory.path();
    let connector = DatabaseConnector::new();
    let service = DataJobService::new(
        vec![fixture.profile.clone()],
        Arc::new(CredentialResolver::new(
            Arc::new(NativeSecretStore),
            LocalCredentialStore::new(root.join("credential.key"), "data-ops-acceptance"),
        )),
        connector.clone(),
        DataJobStore::new(root.join("jobs")),
    )
    .with_passwords(HashMap::from([(
        fixture.profile.id,
        fixture.password.clone(),
    )]));
    assert!(service.store().list()?.is_empty());
    let backup = DataJobSpec {
        operation: DataOperation::Backup,
        source: Some(fixture.endpoint("source")),
        destination: None,
        artifact: root.join("backup.archive"),
        tools: fixture.tools.clone(),
    };
    let backed_up = execute(&service, backup.clone()).await?;
    let outcome = backed_up
        .outcome
        .as_ref()
        .context("backup outcome is missing")?;
    let (bytes, digest) = hash_file(&backup.artifact, &CancellationToken::new()).await?;
    assert!(bytes > 0);
    assert_eq!(outcome.bytes, Some(bytes));
    assert_eq!(outcome.sha256.as_deref(), Some(digest.as_str()));
    let manifest: ArtifactManifest =
        serde_json::from_slice(&std::fs::read(manifest_path(&backup.artifact)?)?)?;
    assert_eq!(manifest.engine, fixture.profile.kind);
    assert_eq!(manifest.bytes, bytes);
    assert_eq!(manifest.sha256, digest);
    eprintln!(
        "ACCEPT {}: native full backup and manifest checksum",
        fixture.engine
    );
    let original_archive = std::fs::read(&backup.artifact)?;
    refused(&service, backup.clone()).await?;
    assert_eq!(std::fs::read(&backup.artifact)?, original_archive);

    let restore = DataJobSpec {
        operation: DataOperation::Restore,
        source: None,
        destination: Some(fixture.endpoint("restored")),
        artifact: backup.artifact.clone(),
        tools: fixture.tools.clone(),
    };
    let restored = execute(&service, restore.clone()).await?;
    assert!(restored.outcome.as_ref().unwrap().target_created);
    fixture
        .verify_new_database(&fixture.endpoint("restored").database)
        .await?;
    refused(&service, restore).await?;
    assert_eq!(
        fixture
            .row_count(&fixture.endpoint("restored").database)
            .await?,
        ROWS + 1
    );
    eprintln!(
        "ACCEPT {}: new restore, row/type parity, next ID {NEXT_ID}",
        fixture.engine
    );

    let copy = DataJobSpec {
        operation: DataOperation::Copy,
        source: Some(fixture.endpoint("source")),
        destination: Some(fixture.endpoint("copied")),
        artifact: root.join("copy.archive"),
        tools: fixture.tools.clone(),
    };
    let copied = execute(&service, copy.clone()).await?;
    assert!(copied.outcome.as_ref().unwrap().target_created);
    assert!(copy.artifact.is_file());
    assert!(manifest_path(&copy.artifact)?.is_file());
    fixture
        .verify_new_database(&fixture.endpoint("copied").database)
        .await?;
    let refusal = DataJobSpec {
        artifact: root.join("refused-copy.archive"),
        ..copy
    };
    refused(&service, refusal.clone()).await?;
    assert!(!refusal.artifact.exists());
    failed_copy_keeps_reusable_backup(&fixture, &service, root).await?;
    assert_eq!(
        fixture
            .row_count(&fixture.endpoint("source").database)
            .await?,
        ROWS
    );
    let records = service.store().list()?;
    assert_eq!(records.len(), 5);
    assert_eq!(
        records
            .iter()
            .filter(|record| record.status.is_success())
            .count(),
        4
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record.status == DataJobStatus::Failed)
            .count(),
        1
    );
    for record in records {
        assert!(record.outcome.is_some());
        ensure!(
            !serde_json::to_string(&record)?.contains(fixture.password.expose_secret()),
            "persisted job metadata must not contain fixture credentials"
        );
    }
    connector.shutdown().await;
    eprintln!(
        "ACCEPT {}: copy verified; {ROWS} original rows, NULL/empty/binary/numeric values, \
         table/column types and sequence state checked in three new databases; \
         3 overwrite refusals; 4 successful jobs and 1 expected restore failure persisted",
        fixture.engine
    );
    Ok(())
}

#[tokio::test]
async fn postgres_native_backup_restore_and_copy() -> Result<()> {
    if let Some(fixture) = Fixture::load("postgres", DatabaseKind::Postgres)? {
        acceptance(fixture).await?;
    }
    Ok(())
}

#[tokio::test]
async fn mysql_native_backup_restore_and_copy() -> Result<()> {
    if let Some(fixture) = Fixture::load("mysql", DatabaseKind::MySql)? {
        acceptance(fixture).await?;
    }
    Ok(())
}

#[tokio::test]
async fn mariadb_native_backup_restore_and_copy() -> Result<()> {
    if let Some(fixture) = Fixture::load("mariadb", DatabaseKind::MariaDb)? {
        acceptance(fixture).await?;
    }
    Ok(())
}
