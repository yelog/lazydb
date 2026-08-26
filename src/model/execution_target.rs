use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::profile::{ConnectionProfile, DatabaseKind};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct ExecutionTarget {
    pub profile_id: Uuid,
    pub database: String,
    pub schema: Option<String>,
}

impl ExecutionTarget {
    pub fn from_profile(profile: &ConnectionProfile) -> Self {
        let database = profile
            .database
            .clone()
            .or_else(|| {
                profile
                    .sqlite_path
                    .as_ref()
                    .map(|path| path.display().to_string())
            })
            .unwrap_or_default();
        let schema = match profile.kind {
            DatabaseKind::MySql => Some(database.clone()),
            DatabaseKind::Sqlite => Some("main".to_owned()),
            DatabaseKind::Postgres => profile.default_schema.clone(),
        };
        Self {
            profile_id: profile.id,
            database,
            schema,
        }
    }

    pub fn is_valid(&self, profile: &ConnectionProfile) -> bool {
        self.profile_id == profile.id
            && !self.database.is_empty()
            && match profile.kind {
                DatabaseKind::MySql => self.schema.as_deref() == Some(self.database.as_str()),
                DatabaseKind::Sqlite => self
                    .schema
                    .as_deref()
                    .is_some_and(|schema| matches!(schema, "main" | "temp")),
                DatabaseKind::Postgres => profile
                    .database
                    .as_deref()
                    .is_some_and(|database| database == self.database),
            }
    }
}
