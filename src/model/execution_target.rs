use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::explorer::ExplorerNodeId;
use crate::profile::{ConnectionProfile, DatabaseKind};

pub fn resolve_default_target(
    selected: Option<&ExplorerNodeId>,
    profiles: &[ConnectionProfile],
    recent: &[ExecutionTarget],
) -> Option<ExecutionTarget> {
    let profile_for = |id| profiles.iter().find(|profile| profile.id == id);
    let selected_target = match selected {
        Some(ExplorerNodeId::Profile(id)) => profile_for(*id).map(ExecutionTarget::from_profile),
        Some(ExplorerNodeId::Catalog(id))
            if matches!(
                id.kind,
                crate::db::catalog::CatalogKind::Database | crate::db::catalog::CatalogKind::Schema
            ) =>
        {
            let profile = profile_for(id.profile_id())?;
            let database = id.native_path.first()?.clone();
            let schema = (id.kind == crate::db::catalog::CatalogKind::Schema)
                .then(|| id.native_path.get(1).cloned())
                .flatten();
            Some(ExecutionTarget {
                profile_id: profile.id,
                database,
                schema,
            })
        }
        _ => None,
    };
    if let Some(target) = selected_target.filter(|target| {
        profile_for(target.profile_id).is_some_and(|profile| target.is_valid(profile))
    }) {
        return Some(target);
    }
    if let Some(target) = recent.iter().find(|target| {
        profile_for(target.profile_id).is_some_and(|profile| target.is_valid(profile))
    }) {
        return Some(target.clone());
    }
    profiles
        .iter()
        .min_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.id.as_bytes().cmp(right.id.as_bytes()))
        })
        .map(ExecutionTarget::from_profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::catalog::{CatalogId, CatalogKind};

    fn profile(id: Uuid, name: &str) -> ConnectionProfile {
        ConnectionProfile {
            id,
            name: name.into(),
            access: crate::profile::ProfileAccess::Global,
            group_id: None,
            kind: DatabaseKind::Postgres,
            url_format: Default::default(),
            host: Some("localhost".into()),
            port: None,
            user: Some("user".into()),
            database: Some("app".into()),
            default_schema: Some("public".into()),
            sqlite_path: None,
            mongodb_options: None,
            pg_options: None,
            snowflake_options: None,
            elasticsearch_options: None,
            duckdb_path: None,
            ssl_mode: Default::default(),
            credential_policy: Default::default(),
            read_only: false,
            environment: Default::default(),
            catalog_scope: crate::profile::CatalogScope {
                databases: crate::profile::CatalogSelection::All,
            },
        }
    }

    #[test]
    fn default_target_resolution_is_table_driven() {
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        let profiles = vec![profile(first, "zulu"), profile(second, "alpha")];
        let mru = ExecutionTarget {
            profile_id: first,
            database: "app".into(),
            schema: Some("public".into()),
        };
        let database = CatalogId::new(second, CatalogKind::Database, ["analytics"]);
        let schema = CatalogId::new(second, CatalogKind::Schema, ["analytics", "reporting"]);
        let cases = [
            (
                "profile node",
                Some(ExplorerNodeId::Profile(first)),
                vec![],
                first,
                "app",
                "public",
            ),
            (
                "database node",
                Some(ExplorerNodeId::Catalog(database)),
                vec![],
                second,
                "analytics",
                "",
            ),
            (
                "schema node",
                Some(ExplorerNodeId::Catalog(schema)),
                vec![],
                second,
                "analytics",
                "reporting",
            ),
            (
                "mru",
                Some(ExplorerNodeId::Others),
                vec![mru],
                first,
                "app",
                "public",
            ),
            (
                "stable profile",
                Some(ExplorerNodeId::Others),
                vec![],
                second,
                "app",
                "public",
            ),
        ];
        for (name, selected, recent, id, database, schema) in cases {
            let target = resolve_default_target(selected.as_ref(), &profiles, &recent)
                .unwrap_or_else(|| panic!("{name} should resolve"));
            assert_eq!(target.profile_id, id, "{name}");
            assert_eq!(target.database, database, "{name}");
            if schema.is_empty() {
                assert_eq!(target.schema, None, "{name}");
            } else {
                assert_eq!(target.schema.as_deref(), Some(schema), "{name}");
            }
        }
        assert!(resolve_default_target(None, &[], &[]).is_none());
        assert!(
            resolve_default_target(
                Some(&ExplorerNodeId::Others),
                &profiles,
                &[ExecutionTarget {
                    profile_id: Uuid::from_u128(9),
                    database: "app".into(),
                    schema: Some("public".into())
                }],
            )
            .is_some()
        );
        assert_eq!(
            resolve_default_target(
                Some(&ExplorerNodeId::Others),
                &profiles,
                &[ExecutionTarget {
                    profile_id: Uuid::from_u128(9),
                    database: "deleted".into(),
                    schema: Some("public".into()),
                }],
            )
            .unwrap()
            .profile_id,
            second
        );
    }
}

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
            .or_else(|| {
                profile
                    .duckdb_path
                    .as_ref()
                    .map(|path| path.display().to_string())
            })
            .or_else(|| (profile.kind == DatabaseKind::DuckDb).then(|| "memory".to_owned()))
            .filter(|database| !database.is_empty())
            .or_else(|| (profile.kind == DatabaseKind::Elasticsearch).then(|| "default".to_owned()))
            .unwrap_or_default();
        let schema = match profile.kind {
            DatabaseKind::MySql
            | DatabaseKind::MariaDb
            | DatabaseKind::TiDb
            | DatabaseKind::ClickHouse
            | DatabaseKind::Elasticsearch => Some(database.clone()),
            DatabaseKind::Sqlite => Some("main".to_owned()),
            DatabaseKind::DuckDb => Some(
                profile
                    .default_schema
                    .clone()
                    .unwrap_or_else(|| "main".into()),
            ),
            DatabaseKind::Snowflake => Some(
                profile
                    .default_schema
                    .clone()
                    .unwrap_or_else(|| "PUBLIC".into()),
            ),
            DatabaseKind::Postgres
            | DatabaseKind::CockroachDb
            | DatabaseKind::SqlServer
            | DatabaseKind::Oracle => profile.default_schema.clone(),
            DatabaseKind::Redis | DatabaseKind::Valkey | DatabaseKind::MongoDb => None,
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
            DatabaseKind::MySql | DatabaseKind::MariaDb | DatabaseKind::TiDb => {
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
            DatabaseKind::DuckDb => {
                profile.database.as_deref() == Some(self.database.as_str())
                    && self.schema.as_deref().is_some_and(|schema| {
                        profile.catalog_scope.allows_schema(&self.database, schema)
                    })
            }
            DatabaseKind::Snowflake => profile
                .catalog_scope
                .allows_schema(&self.database, self.schema.as_deref().unwrap_or("PUBLIC")),
            DatabaseKind::Postgres
            | DatabaseKind::CockroachDb
            | DatabaseKind::SqlServer
            | DatabaseKind::Oracle => self
                .schema
                .as_deref()
                .is_none_or(|schema| profile.catalog_scope.allows_schema(&self.database, schema)),
            DatabaseKind::Redis | DatabaseKind::Valkey | DatabaseKind::MongoDb => {
                self.schema.is_none()
            }
            DatabaseKind::ClickHouse | DatabaseKind::Elasticsearch => {
                self.schema.as_deref() == Some(self.database.as_str())
                    && profile
                        .catalog_scope
                        .allows_schema(&self.database, &self.database)
            }
        }
    }

    pub fn apply_to_profile(&self, profile: &ConnectionProfile) -> Option<ConnectionProfile> {
        if !self.is_valid(profile) {
            return None;
        }
        let mut configured = profile.clone();
        match profile.kind {
            DatabaseKind::Snowflake => {
                configured.database = Some(self.database.clone());
                configured.default_schema =
                    Some(self.schema.clone().unwrap_or_else(|| "PUBLIC".into()));
            }
            DatabaseKind::Postgres
            | DatabaseKind::CockroachDb
            | DatabaseKind::SqlServer
            | DatabaseKind::Oracle => {
                configured.database = Some(self.database.clone());
                configured.default_schema = self.schema.clone();
            }
            DatabaseKind::MySql
            | DatabaseKind::MariaDb
            | DatabaseKind::TiDb
            | DatabaseKind::ClickHouse
            | DatabaseKind::Elasticsearch => {
                configured.database = Some(self.database.clone());
                configured.default_schema = Some(self.database.clone());
            }
            DatabaseKind::Sqlite => {}
            DatabaseKind::DuckDb => configured.default_schema = self.schema.clone(),
            DatabaseKind::Redis | DatabaseKind::Valkey | DatabaseKind::MongoDb => {
                configured.database = Some(self.database.clone());
                configured.default_schema = None;
            }
        }
        Some(configured)
    }
}
