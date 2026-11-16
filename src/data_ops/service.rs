use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, bail};
use chrono::Utc;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    agent::policy::{WriteCapability, WritePolicy, write_capability},
    connections::DatabaseConnector,
    persistence::credentials::CredentialResolver,
    profile::{CatalogSelection, ConnectionProfile, DatabaseKind},
    security::sanitize_terminal_text,
};

use super::{
    DataEndpoint, DataJobPlan, DataJobProgress, DataJobRecord, DataJobSpec, DataJobStatus,
    DataJobStore, DataOperation, NativeOutcome, NativeToolVersion, ProgressSink, ResolvedEndpoint,
    native, supports_native_data_operations,
};

const MAX_LOG_ENTRIES: usize = 200;
const MAX_LOG_MESSAGE_CHARS: usize = 2_000;

#[derive(Clone)]
pub struct DataJobService {
    profiles: Arc<Vec<ConnectionProfile>>,
    credentials: Arc<CredentialResolver>,
    connector: DatabaseConnector,
    store: DataJobStore,
    passwords: Arc<HashMap<Uuid, SecretString>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ArtifactManifest {
    pub schema_version: u16,
    pub engine: DatabaseKind,
    pub source: Option<DataEndpoint>,
    pub created_at: chrono::DateTime<Utc>,
    pub bytes: u64,
    pub sha256: String,
    pub tools: Vec<NativeToolVersion>,
    pub verification: Vec<String>,
    pub warnings: Vec<String>,
}

impl DataJobService {
    pub fn new(
        profiles: Vec<ConnectionProfile>,
        credentials: Arc<CredentialResolver>,
        connector: DatabaseConnector,
        store: DataJobStore,
    ) -> Self {
        Self {
            profiles: Arc::new(profiles),
            credentials,
            connector,
            store,
            passwords: Arc::new(HashMap::new()),
        }
    }

    pub fn with_passwords(mut self, passwords: HashMap<Uuid, SecretString>) -> Self {
        self.passwords = Arc::new(passwords);
        self
    }

    pub fn store(&self) -> &DataJobStore {
        &self.store
    }

    pub fn profiles(&self) -> &[ConnectionProfile] {
        &self.profiles
    }

    pub fn can_view(&self, spec: &DataJobSpec) -> bool {
        spec.source
            .iter()
            .chain(spec.destination.iter())
            .all(|endpoint| {
                self.profiles
                    .iter()
                    .any(|profile| profile.id == endpoint.profile_id)
            })
    }

    pub async fn plan(
        &self,
        mut spec: DataJobSpec,
        policy: WritePolicy,
        cancellation: CancellationToken,
    ) -> anyhow::Result<DataJobPlan> {
        if cancellation.is_cancelled() {
            bail!("Data-task planning was cancelled.");
        }
        self.validate(&spec, policy)?;
        self.normalize(&mut spec)?;
        let (source, destination) = self.resolve(&spec).await?;
        let secrets = endpoint_secrets(source.as_ref(), destination.as_ref());
        let native = tokio::select! {
            biased;
            () = cancellation.cancelled() => bail!("Data-task planning was cancelled."),
            result = native::plan(
                &spec,
                source.as_ref(),
                destination.as_ref(),
                &self.connector,
                &cancellation,
            ) => result.map_err(|error| anyhow::anyhow!(redact(&format!("{error:#}"), &secrets)))?,
        };
        let input_digest = self
            .input_digest(&spec, native.engine, &cancellation)
            .await?;
        Ok(DataJobPlan {
            id: Uuid::new_v4(),
            profile_fingerprint: self.fingerprint(&spec, input_digest.as_deref())?,
            spec,
            native,
            created_at: Utc::now(),
        })
    }

    pub async fn run(
        &self,
        plan: DataJobPlan,
        policy: WritePolicy,
        cancellation: CancellationToken,
        on_update: Arc<dyn Fn(DataJobRecord) + Send + Sync>,
    ) -> anyhow::Result<DataJobRecord> {
        let _job_lock = self.store.lock_job(plan.id)?;
        let record = Arc::new(Mutex::new(DataJobRecord {
            schema_version: 1,
            id: plan.id,
            plan: plan.clone(),
            status: DataJobStatus::Queued,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            message: "Waiting for preflight.".into(),
            bytes: None,
            log: Vec::new(),
            outcome: None,
            error: None,
            target_may_be_partial: false,
        }));
        self.store.create(&snapshot(&record)?)?;
        on_update(snapshot(&record)?);
        let mut target_started = false;
        let result = async {
            self.publish(
                &record,
                DataJobProgress {
                    status: DataJobStatus::Preflight,
                    message: "Rechecking the reviewed plan and target.".into(),
                    bytes: None,
                    completed_backup: None,
                },
                &on_update,
            )?;
            if cancellation.is_cancelled() {
                bail!("Data task was cancelled before execution.");
            }
            self.validate(&plan.spec, policy)?;
            let mut normalized = plan.spec.clone();
            self.normalize(&mut normalized)?;
            if normalized != plan.spec {
                bail!("A path changed after review. Create a new plan.");
            }
            let input_digest = self
                .input_digest(&plan.spec, plan.native.engine, &cancellation)
                .await?;
            if self.fingerprint(&plan.spec, input_digest.as_deref())?
                != plan.profile_fingerprint
            {
                bail!("The connection, artifact, or operation changed after review. Create a new plan.");
            }
            let mut resource_keys = self.resource_keys(&plan.spec)?;
            resource_keys.sort();
            resource_keys.dedup();
            let _resource_locks = resource_keys
                .iter()
                .map(|key| self.store.lock_resource(key))
                .collect::<anyhow::Result<Vec<_>>>()?;
            let (source, destination) = self.resolve(&plan.spec).await?;
            let secrets = endpoint_secrets(source.as_ref(), destination.as_ref());
            if cancellation.is_cancelled() {
                bail!("Data task was cancelled before execution.");
            }
            self.publish(
                &record,
                DataJobProgress {
                    status: DataJobStatus::Running,
                    message: format!("{} started.", plan.spec.operation.label()),
                    bytes: None,
                    completed_backup: None,
                },
                &on_update,
            )?;
            target_started = plan.spec.operation.writes_database();
            let execution_cancellation = cancellation.child_token();
            let persistence_failure = Arc::new(Mutex::new(None::<String>));
            let failure_for_progress = Arc::clone(&persistence_failure);
            let record_for_progress = Arc::clone(&record);
            let service_for_progress = self.clone();
            let update_for_progress = Arc::clone(&on_update);
            let cancel_for_progress = execution_cancellation.clone();
            let progress_secrets = secrets.clone();
            let progress: ProgressSink = Arc::new(move |mut progress| {
                if progress.status.is_terminal() {
                    return;
                }
                progress.message = redact(&progress.message, &progress_secrets);
                if let Some(outcome) = &mut progress.completed_backup {
                    outcome.warnings = outcome
                        .warnings
                        .iter()
                        .map(|warning| redact(warning, &progress_secrets))
                        .collect();
                }
                if let Err(error) = service_for_progress.publish(
                    &record_for_progress,
                    progress,
                    &update_for_progress,
                ) {
                    if let Ok(mut failure) = failure_for_progress.lock() {
                        *failure = Some(error.to_string());
                    }
                    cancel_for_progress.cancel();
                }
            });
            let outcome = native::execute(
                &plan.spec,
                &plan.native,
                source.as_ref(),
                destination.as_ref(),
                &self.connector,
                &execution_cancellation,
                progress,
            )
            .await
            .map_err(|error| anyhow::anyhow!(redact(&format!("{error:#}"), &secrets)));
            if let Some(error) = persistence_failure
                .lock()
                .map_err(|_| anyhow::anyhow!("Task progress state is unavailable."))?
                .take()
            {
                bail!("Task progress could not be persisted: {error}");
            }
            let mut outcome = outcome?;
            outcome.warnings = outcome
                .warnings
                .iter()
                .map(|warning| redact(warning, &secrets))
                .collect();
            for warning in &plan.native.warnings {
                let warning = redact(warning, &secrets);
                if !outcome.warnings.contains(&warning) {
                    outcome.warnings.push(warning);
                }
            }
            if matches!(plan.spec.operation, DataOperation::Backup | DataOperation::Copy)
                && snapshot(&record)?.outcome.is_none()
            {
                bail!("The native provider did not publish verified backup metadata.");
            }
            Ok::<_, anyhow::Error>(outcome)
        }
        .await;
        let mut final_record = snapshot(&record)?;
        final_record.updated_at = Utc::now();
        match result {
            Ok(outcome) => {
                final_record.status = if outcome.warnings.is_empty() {
                    DataJobStatus::Succeeded
                } else {
                    DataJobStatus::SucceededWithWarnings
                };
                final_record.message = format!("{} completed.", plan.spec.operation.label());
                final_record.bytes = outcome.bytes;
                final_record.outcome = Some(outcome);
            }
            Err(error) => {
                final_record.status = if cancellation.is_cancelled() {
                    DataJobStatus::Cancelled
                } else {
                    DataJobStatus::Failed
                };
                final_record.message = truncate(&sanitize_terminal_text(&format!("{error:#}")));
                final_record.error = Some(final_record.message.clone());
                final_record.target_may_be_partial = target_started;
            }
        }
        let final_message = final_record.message.clone();
        append_log(&mut final_record, final_message);
        self.store.save(&final_record)?;
        on_update(final_record.clone());
        Ok(final_record)
    }

    fn validate(&self, spec: &DataJobSpec, policy: WritePolicy) -> anyhow::Result<()> {
        let shape_valid = match spec.operation {
            DataOperation::Backup => spec.source.is_some() && spec.destination.is_none(),
            DataOperation::Restore => spec.source.is_none() && spec.destination.is_some(),
            DataOperation::Copy => spec.source.is_some() && spec.destination.is_some(),
        };
        if !shape_valid {
            bail!("The operation has an invalid source/destination combination.");
        }
        if spec.artifact.as_os_str().is_empty() {
            bail!("An archive path is required.");
        }
        let mut engine = None;
        for endpoint in spec.source.iter().chain(spec.destination.iter()) {
            if endpoint.database.trim().is_empty()
                || endpoint.database.chars().any(char::is_control)
            {
                bail!("A database name or file path without control characters is required.");
            }
            let profile = self.profile(endpoint.profile_id)?;
            if !supports_native_data_operations(profile.kind) {
                bail!("Native data operations are not supported for this database engine.");
            }
            if engine.is_some_and(|kind| kind != profile.kind) {
                bail!("Native copies require matching database engines.");
            }
            engine = Some(profile.kind);
        }
        if let Some(source) = &spec.source {
            let profile = self.profile(source.profile_id)?;
            if profile.kind == DatabaseKind::Sqlite {
                let selected = profile
                    .sqlite_path
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("The SQLite profile has no file path."))?;
                if selected.canonicalize()? != Path::new(&source.database).canonicalize()? {
                    bail!("The SQLite source must be the file configured in the selected profile.");
                }
            } else if !allows_whole_database(profile, &source.database) {
                bail!(
                    "Full-database backup requires the database and all of its schemas to be visible in this profile."
                );
            }
        }
        if let Some(destination) = &spec.destination {
            let profile = self.profile(destination.profile_id)?;
            if let WriteCapability::Denied(error) = write_capability(profile, policy) {
                bail!("{error}");
            }
        }
        Ok(())
    }

    fn normalize(&self, spec: &mut DataJobSpec) -> anyhow::Result<()> {
        spec.artifact = if spec.operation == DataOperation::Restore {
            spec.artifact
                .canonicalize()
                .context("Cannot locate the input archive")?
        } else {
            new_file_path(&spec.artifact)?
        };
        if let Some(endpoint) = &mut spec.source
            && self.profile(endpoint.profile_id)?.kind == DatabaseKind::Sqlite
        {
            endpoint.database = Path::new(&endpoint.database)
                .canonicalize()?
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("SQLite data-operation paths must be valid UTF-8."))?
                .to_owned();
        }
        if let Some(endpoint) = &mut spec.destination
            && self.profile(endpoint.profile_id)?.kind == DatabaseKind::Sqlite
        {
            endpoint.database = new_file_path(Path::new(&endpoint.database))?
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("SQLite data-operation paths must be valid UTF-8."))?
                .to_owned();
        }
        Ok(())
    }

    fn profile(&self, id: Uuid) -> anyhow::Result<&ConnectionProfile> {
        self.profiles
            .iter()
            .find(|profile| profile.id == id)
            .ok_or_else(|| anyhow::anyhow!("The task references an unavailable connection."))
    }

    async fn resolve(
        &self,
        spec: &DataJobSpec,
    ) -> anyhow::Result<(Option<ResolvedEndpoint>, Option<ResolvedEndpoint>)> {
        let source = match &spec.source {
            Some(endpoint) => Some(self.resolve_endpoint(endpoint).await?),
            None => None,
        };
        let destination = match &spec.destination {
            Some(endpoint) => Some(self.resolve_endpoint(endpoint).await?),
            None => None,
        };
        Ok((source, destination))
    }

    async fn resolve_endpoint(&self, endpoint: &DataEndpoint) -> anyhow::Result<ResolvedEndpoint> {
        let profile = self.profile(endpoint.profile_id)?.clone();
        let password = match self.passwords.get(&profile.id) {
            Some(password) => Some(password.clone()),
            None => self.credentials.resolve_headless(&profile).await?,
        };
        Ok(ResolvedEndpoint {
            profile,
            database: endpoint.database.clone(),
            password,
        })
    }

    fn fingerprint(
        &self,
        spec: &DataJobSpec,
        input_digest: Option<&str>,
    ) -> anyhow::Result<String> {
        let profiles = spec
            .source
            .iter()
            .chain(spec.destination.iter())
            .map(|endpoint| self.profile(endpoint.profile_id))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let bytes = serde_json::to_vec(&(spec, profiles, input_digest))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    fn resource_keys(&self, spec: &DataJobSpec) -> anyhow::Result<Vec<String>> {
        let mut keys = Vec::new();
        if spec.operation != DataOperation::Restore {
            keys.push(format!("artifact:{}", spec.artifact.display()));
        }
        if let Some(destination) = &spec.destination {
            let profile = self.profile(destination.profile_id)?;
            let key = serde_json::to_string(&(
                profile.kind,
                profile
                    .host
                    .as_deref()
                    .unwrap_or("localhost")
                    .to_lowercase(),
                profile
                    .port
                    .unwrap_or(if profile.kind == DatabaseKind::Postgres {
                        5432
                    } else {
                        3306
                    }),
                &profile.ssh,
                &destination.database,
            ))?;
            keys.push(format!("destination:{key}"));
        }
        Ok(keys)
    }

    fn publish(
        &self,
        record: &Arc<Mutex<DataJobRecord>>,
        progress: DataJobProgress,
        on_update: &Arc<dyn Fn(DataJobRecord) + Send + Sync>,
    ) -> anyhow::Result<()> {
        let mut record = record
            .lock()
            .map_err(|_| anyhow::anyhow!("Task state is unavailable."))?;
        record.status = progress.status;
        record.message = truncate(&sanitize_terminal_text(&progress.message));
        record.updated_at = Utc::now();
        if progress.bytes.is_some() {
            record.bytes = progress.bytes;
        }
        if let Some(outcome) = progress.completed_backup {
            if !matches!(
                record.plan.spec.operation,
                DataOperation::Backup | DataOperation::Copy
            ) || record.outcome.is_some()
                || outcome.target_created
            {
                bail!("The native provider reported an invalid backup checkpoint.");
            }
            record.outcome = Some(outcome.clone());
            self.write_manifest(&record.plan, &outcome)?;
        }
        let message = record.message.clone();
        append_log(&mut record, message);
        self.store.save(&record)?;
        let update = record.clone();
        drop(record);
        on_update(update);
        Ok(())
    }

    async fn input_digest(
        &self,
        spec: &DataJobSpec,
        engine: DatabaseKind,
        cancellation: &CancellationToken,
    ) -> anyhow::Result<Option<String>> {
        if spec.operation != DataOperation::Restore {
            if manifest_path(&spec.artifact)?.try_exists()? {
                bail!("The backup metadata path already exists. Choose a new archive path.");
            }
            return Ok(None);
        }
        let (bytes, digest) = hash_file(&spec.artifact, cancellation).await?;
        let manifest_path = manifest_path(&spec.artifact)?;
        if manifest_path.try_exists()? {
            let manifest: ArtifactManifest = serde_json::from_slice(&super::store::read_metadata(
                &manifest_path,
                1024 * 1024,
                "Backup metadata",
            )?)
            .context("Backup metadata is invalid")?;
            if manifest.schema_version != 1 || manifest.engine != engine {
                bail!("Backup metadata is not compatible with the selected database engine.");
            }
            if manifest.bytes != bytes || manifest.sha256 != digest {
                bail!("The backup checksum does not match its metadata. Restore was not started.");
            }
        }
        Ok(Some(digest))
    }

    fn write_manifest(&self, plan: &DataJobPlan, outcome: &NativeOutcome) -> anyhow::Result<()> {
        let artifact = outcome
            .artifact
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("The backup provider did not return its artifact."))?;
        if artifact != &plan.spec.artifact {
            bail!("The backup provider returned an unexpected artifact path.");
        }
        let manifest = ArtifactManifest {
            schema_version: 1,
            engine: plan.native.engine,
            source: plan.spec.source.clone(),
            created_at: Utc::now(),
            bytes: outcome
                .bytes
                .ok_or_else(|| anyhow::anyhow!("Backup size was not verified."))?,
            sha256: outcome
                .sha256
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Backup checksum was not verified."))?,
            tools: plan.native.tools.clone(),
            verification: outcome.verification.clone(),
            warnings: outcome.warnings.clone(),
        };
        let path = manifest_path(artifact)?;
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Archive has no parent directory."))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist_noclobber(&path)
            .context("Cannot publish backup metadata; the native archive is retained")?;
        super::store::sync_directory(parent)?;
        Ok(())
    }
}

fn snapshot(record: &Arc<Mutex<DataJobRecord>>) -> anyhow::Result<DataJobRecord> {
    record
        .lock()
        .map(|record| record.clone())
        .map_err(|_| anyhow::anyhow!("Task state is unavailable."))
}

fn append_log(record: &mut DataJobRecord, message: String) {
    if record.log.last() != Some(&message) {
        record.log.push(message);
        if record.log.len() > MAX_LOG_ENTRIES {
            record.log.remove(0);
        }
    }
}

fn endpoint_secrets(
    source: Option<&ResolvedEndpoint>,
    destination: Option<&ResolvedEndpoint>,
) -> Vec<String> {
    source
        .into_iter()
        .chain(destination)
        .filter_map(|endpoint| {
            endpoint
                .password
                .as_ref()
                .map(|password| password.expose_secret().to_owned())
        })
        .filter(|password| !password.is_empty())
        .collect()
}

fn redact(message: &str, secrets: &[String]) -> String {
    let mut message = message.to_owned();
    for secret in secrets {
        message = message.replace(secret, "[REDACTED]");
    }
    truncate(&sanitize_terminal_text(&message))
}

fn truncate(message: &str) -> String {
    message.chars().take(MAX_LOG_MESSAGE_CHARS).collect()
}

fn allows_whole_database(profile: &ConnectionProfile, database: &str) -> bool {
    match &profile.catalog_scope.databases {
        CatalogSelection::All => true,
        CatalogSelection::Selected(databases) => databases.iter().any(|scope| {
            scope.name == database
                && (matches!(scope.schemas, CatalogSelection::All)
                    || matches!(
                        (&profile.kind, &scope.schemas),
                        (
                            DatabaseKind::MySql | DatabaseKind::MariaDb,
                            CatalogSelection::Selected(schemas)
                        ) if schemas.len() == 1 && schemas[0] == database
                    ))
        }),
    }
}

pub fn new_file_path(path: &Path) -> anyhow::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let name = absolute
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("A file name is required."))?;
    let parent = absolute
        .parent()
        .ok_or_else(|| anyhow::anyhow!("A parent directory is required."))?;
    Ok(parent
        .canonicalize()
        .context("The destination directory must already exist")?
        .join(name))
}

pub fn manifest_path(artifact: &Path) -> anyhow::Result<PathBuf> {
    let mut name = artifact
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("An archive file name is required."))?
        .to_os_string();
    name.push(".lazydb.json");
    Ok(artifact.with_file_name(name))
}

pub async fn hash_file(
    path: &Path,
    cancellation: &CancellationToken,
) -> anyhow::Result<(u64, String)> {
    if !tokio::fs::metadata(path).await?.is_file() {
        bail!("The archive must be a regular file.");
    }
    let mut file = tokio::fs::File::open(path).await?;
    if !file.metadata().await?.is_file() {
        bail!("The archive must be a regular file.");
    }
    let mut digest = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = tokio::select! {
            biased;
            () = cancellation.cancelled() => bail!("Archive verification was cancelled."),
            count = file.read(&mut buffer) => count?,
        };
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        bytes += count as u64;
    }
    Ok((bytes, format!("{:x}", digest.finalize())))
}
