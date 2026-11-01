use uuid::Uuid;

use crate::{
    db::{
        catalog::{CatalogEntry, NamespaceModel},
        catalog_mutation::{
            CatalogMutationAnchor, CatalogMutationCapabilities, CatalogMutationMode,
        },
    },
    profile::DatabaseKind,
};

use super::explorer::{ExplorerMutationIntent, ExplorerNodeId, resolve_mutation_intent};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExplorerActionAvailability {
    Available(ExplorerMutationIntent),
    Unavailable(String),
    NotApplicable,
}

pub struct ExplorerActionContext<'a> {
    pub database_kind: DatabaseKind,
    pub connected: bool,
    pub read_only: bool,
    pub namespace_model: NamespaceModel,
    pub capabilities: &'a CatalogMutationCapabilities,
    pub selected_entry: Option<&'a CatalogEntry>,
}

impl<'a> ExplorerActionContext<'a> {
    pub fn resolve(
        &self,
        selected: Option<&ExplorerNodeId>,
        mode: CatalogMutationMode,
    ) -> ExplorerActionAvailability {
        let Some(selected) = selected else {
            return ExplorerActionAvailability::NotApplicable;
        };

        if let ExplorerNodeId::Profile(profile_id) = selected {
            return self.resolve_profile(*profile_id, mode);
        }
        if matches!(
            selected,
            ExplorerNodeId::RedisDatabase { .. } | ExplorerNodeId::Mongo { .. }
        ) {
            return ExplorerActionAvailability::NotApplicable;
        }
        if !self.connected {
            return ExplorerActionAvailability::Unavailable(
                "Connect this connection first".to_owned(),
            );
        }
        if self.read_only {
            return ExplorerActionAvailability::Unavailable("Read-only connection".to_owned());
        }

        let Some(intent) =
            resolve_mutation_intent(Some(selected), mode == CatalogMutationMode::Edit)
        else {
            return ExplorerActionAvailability::NotApplicable;
        };
        match (&mode, &intent) {
            (CatalogMutationMode::Create, ExplorerMutationIntent::Create(anchor)) => {
                self.resolve_create(anchor)
            }
            (CatalogMutationMode::Edit, ExplorerMutationIntent::Edit(anchor)) => {
                self.resolve_edit(anchor)
            }
            _ => ExplorerActionAvailability::NotApplicable,
        }
    }

    fn resolve_profile(
        &self,
        profile_id: Uuid,
        mode: CatalogMutationMode,
    ) -> ExplorerActionAvailability {
        if mode == CatalogMutationMode::Edit {
            return ExplorerActionAvailability::Available(ExplorerMutationIntent::EditProfile(
                profile_id,
            ));
        }
        if !self.connected {
            return ExplorerActionAvailability::Unavailable(
                "Connect this connection first".to_owned(),
            );
        }
        if self.read_only {
            return ExplorerActionAvailability::Unavailable("Read-only connection".to_owned());
        }
        self.resolve_create(&CatalogMutationAnchor::Profile { profile_id })
    }

    fn resolve_create(&self, anchor: &CatalogMutationAnchor) -> ExplorerActionAvailability {
        match self.capabilities.create_options_for_namespace(
            anchor,
            self.selected_entry,
            self.namespace_model,
        ) {
            Ok(options) if !options.is_empty() => ExplorerActionAvailability::Available(
                ExplorerMutationIntent::Create(anchor.clone()),
            ),
            Ok(_) => ExplorerActionAvailability::Unavailable(format!(
                "Catalog creation is not supported for {}",
                self.database_kind.label()
            )),
            Err(error) => ExplorerActionAvailability::Unavailable(error.to_string()),
        }
    }

    fn resolve_edit(&self, anchor: &CatalogMutationAnchor) -> ExplorerActionAvailability {
        match self.capabilities.can_edit(anchor, self.selected_entry) {
            Ok(true) => {
                ExplorerActionAvailability::Available(ExplorerMutationIntent::Edit(anchor.clone()))
            }
            Ok(false) => ExplorerActionAvailability::Unavailable(format!(
                "Catalog editing is not supported for {}",
                self.database_kind.label()
            )),
            Err(error) => ExplorerActionAvailability::Unavailable(error.to_string()),
        }
    }
}

trait DatabaseKindLabel {
    fn label(self) -> &'static str;
}

impl DatabaseKindLabel for DatabaseKind {
    fn label(self) -> &'static str {
        match self {
            DatabaseKind::Postgres => "PostgreSQL",
            DatabaseKind::CockroachDb => "CockroachDB",
            DatabaseKind::MySql => "MySQL",
            DatabaseKind::MariaDb => "MariaDB",
            DatabaseKind::TiDb => "TiDB",
            DatabaseKind::Oracle => "Oracle",
            DatabaseKind::Sqlite => "SQLite",
            DatabaseKind::DuckDb => "DuckDB",
            DatabaseKind::SqlServer => "SQL Server",
            DatabaseKind::Redis => "Redis",
            DatabaseKind::Valkey => "Valkey",
            DatabaseKind::MongoDb => "MongoDB",
            DatabaseKind::ClickHouse => "ClickHouse",
            DatabaseKind::Snowflake => "Snowflake",
            DatabaseKind::Elasticsearch => "Elasticsearch",
        }
    }
}
