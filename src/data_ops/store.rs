use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use chrono::Utc;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{DataJobRecord, DataJobStatus};

const MAX_RECORD_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct DataJobStore {
    directory: PathBuf,
}

pub struct DataJobLock {
    _file: File,
}

impl DataJobStore {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn create(&self, record: &DataJobRecord) -> anyhow::Result<()> {
        self.write(record, false)
    }

    pub fn save(&self, record: &DataJobRecord) -> anyhow::Result<()> {
        self.write(record, true)
    }

    pub fn load(&self, id: Uuid) -> anyhow::Result<DataJobRecord> {
        let record = self.read(id)?;
        if record.status.is_terminal() {
            return Ok(record);
        }
        let Some(_lock) = self.try_job_lock(id)? else {
            return Ok(record);
        };
        let mut record = self.read(id)?;
        if !record.status.is_terminal() {
            record.target_may_be_partial = record.plan.spec.operation.writes_database()
                && matches!(
                    record.status,
                    DataJobStatus::Running | DataJobStatus::Verifying
                );
            record.status = DataJobStatus::Interrupted;
            record.updated_at = Utc::now();
            record.message = "The task's process stopped before recording a final result.".into();
            record.error = Some(record.message.clone());
            self.save(&record)?;
        }
        Ok(record)
    }

    pub fn list(&self) -> anyhow::Result<Vec<DataJobRecord>> {
        if !self.directory.exists() {
            return Ok(Vec::new());
        }
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let Some(id) = path
                .file_stem()
                .and_then(|name| name.to_str())
                .and_then(|name| Uuid::parse_str(name).ok())
            else {
                continue;
            };
            records.push(self.load(id)?);
        }
        records.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        Ok(records)
    }

    pub fn lock_job(&self, id: Uuid) -> anyhow::Result<DataJobLock> {
        self.try_job_lock(id)?
            .ok_or_else(|| anyhow::anyhow!("This data task is already running."))
    }

    pub fn lock_resource(&self, resource: &str) -> anyhow::Result<DataJobLock> {
        let key = format!("resource-{:x}", Sha256::digest(resource.as_bytes()));
        self.try_lock(&key)?
            .ok_or_else(|| anyhow::anyhow!("Another data task is using this target or artifact."))
    }

    fn try_job_lock(&self, id: Uuid) -> anyhow::Result<Option<DataJobLock>> {
        self.try_lock(&format!("job-{id}"))
    }

    fn try_lock(&self, key: &str) -> anyhow::Result<Option<DataJobLock>> {
        let directory = self.directory.join("locks");
        private_directory(&directory)?;
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(directory.join(format!("{key}.lock")))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(DataJobLock { _file: file })),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }

    fn read(&self, id: Uuid) -> anyhow::Result<DataJobRecord> {
        let path = self.directory.join(format!("{id}.json"));
        let bytes = read_metadata(&path, MAX_RECORD_BYTES, &format!("Task {id}"))?;
        let record: DataJobRecord = serde_json::from_slice(&bytes)
            .with_context(|| format!("Task {id} is not a valid task record"))?;
        validate_record(&record)?;
        if record.id != id {
            bail!("Task record identity does not match its filename.");
        }
        Ok(record)
    }

    fn write(&self, record: &DataJobRecord, replace: bool) -> anyhow::Result<()> {
        validate_record(record)?;
        private_directory(&self.directory)?;
        let bytes = serde_json::to_vec_pretty(record)?;
        if bytes.len() as u64 > MAX_RECORD_BYTES {
            bail!("Task record exceeds the task-record size limit.");
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&self.directory)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        let destination = self.directory.join(format!("{}.json", record.id));
        if replace {
            temporary.persist(destination)?;
        } else {
            temporary
                .persist_noclobber(destination)
                .context("This plan was already executed. Create a new plan to run it again.")?;
        }
        sync_directory(&self.directory)?;
        Ok(())
    }
}

pub(crate) fn read_metadata(path: &Path, max_bytes: u64, kind: &str) -> anyhow::Result<Vec<u8>> {
    let limit = max_bytes
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("{kind} has an invalid size limit."))?;
    if !fs::metadata(path)
        .with_context(|| format!("Cannot inspect {kind}"))?
        .is_file()
    {
        bail!("{kind} must be a regular file.");
    }
    let mut options = OpenOptions::new();
    options.read(true);
    let file = options
        .open(path)
        .with_context(|| format!("Cannot open {kind}"))?;
    if !file
        .metadata()
        .with_context(|| format!("Cannot inspect opened {kind}"))?
        .is_file()
    {
        bail!("{kind} must be a regular file.");
    }
    let mut bytes = Vec::new();
    file.take(limit)
        .read_to_end(&mut bytes)
        .with_context(|| format!("Cannot read {kind}"))?;
    if bytes.len() as u64 > max_bytes {
        bail!("{kind} exceeds the size limit of {max_bytes} bytes.");
    }
    Ok(bytes)
}

fn validate_record(record: &DataJobRecord) -> anyhow::Result<()> {
    if record.schema_version != 1 {
        bail!("Unsupported data-task record version.");
    }
    if record.id != record.plan.id {
        bail!("Task and plan identities do not match.");
    }
    Ok(())
}

pub(crate) fn private_directory(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        if !path.is_dir() {
            bail!("Data-task storage path is not a directory.");
        }
        return Ok(());
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}

pub(crate) fn sync_directory(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
