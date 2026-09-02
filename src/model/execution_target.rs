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
            DatabaseKind::Postgres | DatabaseKind::SqlServer => profile.default_schema.clone(),
        };
        Self {
            profile_id: profile.id,
            database,
            schema,
        }
    }

    pub fn is_valid(&self, profile: &ConnectionProfile) -> bool {
        if self.profile_id != profile.id
            || self.database.is_empty()
            || !profile.catalog_scope.allows_database(&self.database)
        {
            return false;
        }
        match profile.kind {
            DatabaseKind::MySql => {
                self.schema.as_deref() == Some(self.database.as_str())
                    && profile
                        .catalog_scope
                        .allows_schema(&self.database, &self.database)
            }
            DatabaseKind::Sqlite => {
                profile.database.as_deref() == Some(self.database.as_str())
                    && self.schema.as_deref().is_some_and(|schema| {
                        profile.catalog_scope.allows_schema(&self.database, schema)
                    })
            }
            DatabaseKind::Postgres | DatabaseKind::SqlServer => self
                .schema
                .as_deref()
                .is_none_or(|schema| profile.catalog_scope.allows_schema(&self.database, schema)),
        }
    }

    pub fn apply_to_profile(&self, profile: &ConnectionProfile) -> Option<ConnectionProfile> {
        if !self.is_valid(profile) {
            return None;
        }
        let mut configured = profile.clone();
        match profile.kind {
            DatabaseKind::Postgres | DatabaseKind::SqlServer => {
                configured.database = Some(self.database.clone());
                configured.default_schema = self.schema.clone();
            }
            DatabaseKind::MySql => {
                configured.database = Some(self.database.clone());
                configured.default_schema = Some(self.database.clone());
            }
            DatabaseKind::Sqlite => {}
        }
        Some(configured)
    }
}
