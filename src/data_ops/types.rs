use std::{path::PathBuf, sync::Arc};

use chrono::{DateTime, Utc};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::profile::{ConnectionProfile, DatabaseKind};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataOperation {
    Backup,
    Restore,
    Copy,
}

impl DataOperation {
    pub const fn writes_database(self) -> bool {
        matches!(self, Self::Restore | Self::Copy)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Backup => "Backup",
            Self::Restore => "Restore",
            Self::Copy => "Copy",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DataEndpoint {
    pub profile_id: Uuid,
    pub database: String,
}

impl DataEndpoint {
    pub fn from_profile(profile: &ConnectionProfile) -> Self {
        Self {
            profile_id: profile.id,
            database: if profile.kind == DatabaseKind::Sqlite {
                profile
                    .sqlite_path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else {
                profile.database.clone().unwrap_or_default()
            },
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct NativeToolPaths {
    pub dump: Option<PathBuf>,
    pub restore: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DataJobSpec {
    pub operation: DataOperation,
    pub source: Option<DataEndpoint>,
    pub destination: Option<DataEndpoint>,
    pub artifact: PathBuf,
    #[serde(default)]
    pub tools: NativeToolPaths,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativeToolVersion {
    pub name: String,
    pub path: PathBuf,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativePlan {
    pub engine: DatabaseKind,
    pub steps: Vec<String>,
    pub warnings: Vec<String>,
    pub tools: Vec<NativeToolVersion>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DataJobPlan {
    pub id: Uuid,
    pub spec: DataJobSpec,
    pub native: NativePlan,
    pub profile_fingerprint: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataJobStatus {
    Queued,
    Preflight,
    Running,
    Verifying,
    Succeeded,
    SucceededWithWarnings,
    Failed,
    Cancelled,
    Interrupted,
}

impl DataJobStatus {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded
                | Self::SucceededWithWarnings
                | Self::Failed
                | Self::Cancelled
                | Self::Interrupted
        )
    }

    pub const fn is_success(self) -> bool {
        matches!(self, Self::Succeeded | Self::SucceededWithWarnings)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DataJobProgress {
    pub status: DataJobStatus,
    pub message: String,
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_backup: Option<NativeOutcome>,
}

pub type ProgressSink = Arc<dyn Fn(DataJobProgress) + Send + Sync>;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativeOutcome {
    pub verification: Vec<String>,
    pub warnings: Vec<String>,
    pub artifact: Option<PathBuf>,
    pub bytes: Option<u64>,
    pub sha256: Option<String>,
    pub target_created: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DataJobRecord {
    pub schema_version: u16,
    pub id: Uuid,
    pub plan: DataJobPlan,
    pub status: DataJobStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message: String,
    pub bytes: Option<u64>,
    pub log: Vec<String>,
    pub outcome: Option<NativeOutcome>,
    pub error: Option<String>,
    pub target_may_be_partial: bool,
}

pub struct ResolvedEndpoint {
    pub profile: ConnectionProfile,
    pub database: String,
    pub password: Option<SecretString>,
}

pub const fn supports_native_data_operations(kind: DatabaseKind) -> bool {
    matches!(
        kind,
        DatabaseKind::Postgres | DatabaseKind::MySql | DatabaseKind::MariaDb | DatabaseKind::Sqlite
    )
}
