use super::App;
use crate::{
    action::Action,
    db::{
        catalog::CatalogKind,
        mongodb::types::{MongoExplorerNodeId, MongoNamespace},
    },
    model::{explorer::ExplorerNodeId, tab::WorkspaceTab},
    profile::DatabaseKind,
};
use uuid::Uuid;

impl App {
    pub fn set_explorer_auto_reveal_active_tab(&mut self, enabled: bool) {
        self.explorer_auto_reveal_active_tab = enabled;
        self.pending_explorer_tab_reveal = None;
    }

    pub(super) fn explorer_reveal_requested_tab(&self, action: &Action) -> Option<Uuid> {
        let index = match action {
            Action::NextTab if !self.tabs.is_empty() => (self.active_tab + 1) % self.tabs.len(),
            Action::PreviousTab if !self.tabs.is_empty() => self
                .active_tab
                .checked_sub(1)
                .unwrap_or(self.tabs.len() - 1),
            Action::ActivateTab(index) => *index,
            Action::ActivateSqlEditor(id) => return Some(*id),
            _ => return None,
        };
        self.tabs.get(index).map(WorkspaceTab::id)
    }

    pub(super) fn synchronize_explorer_with_active_tab(
        &mut self,
        previous_tab: Option<Uuid>,
        requested_tab: Option<Uuid>,
        metadata_arrived: bool,
        explorer_navigation: bool,
    ) {
        if !self.explorer_auto_reveal_active_tab {
            return;
        }
        let active_tab = self.active_tab_id();
        if active_tab != previous_tab || (active_tab.is_some() && requested_tab == active_tab) {
            self.reveal_active_tab_in_explorer();
        } else if explorer_navigation {
            self.pending_explorer_tab_reveal = None;
        } else if metadata_arrived
            && active_tab.is_some()
            && self.pending_explorer_tab_reveal == active_tab
        {
            self.reveal_active_tab_in_explorer();
        }
    }

    fn reveal_active_tab_in_explorer(&mut self) {
        self.pending_explorer_tab_reveal = None;
        let Some((node, exact)) = self.active_tab_explorer_node() else {
            return;
        };
        let profile_id = node.profile_id();
        let revealed = self.explorer.reveal_node(node.clone());
        if revealed && exact {
            return;
        }
        if !revealed {
            let fallback = match node {
                ExplorerNodeId::Mongo {
                    profile_id,
                    node: MongoExplorerNodeId::Collection(namespace),
                } => Some(ExplorerNodeId::Mongo {
                    profile_id,
                    node: MongoExplorerNodeId::Database(namespace.database),
                }),
                ExplorerNodeId::Principal { entry } => Some(ExplorerNodeId::PrincipalGroup {
                    profile_id: entry.profile_id,
                }),
                ExplorerNodeId::Catalog(_) => {
                    if let Some(WorkspaceTab::Relation(tab)) = self.tabs.get(self.active_tab) {
                        tab.descriptor
                            .qualified_name
                            .database
                            .as_deref()
                            .map(|database| {
                                self.explorer_namespace_node(
                                    tab.descriptor.key.profile_id,
                                    database,
                                    tab.descriptor.qualified_name.schema.as_deref(),
                                )
                                .0
                            })
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if !fallback.is_some_and(|node| self.explorer.reveal_node(node))
                && let Some(profile_id) = profile_id
            {
                self.explorer
                    .reveal_node(ExplorerNodeId::Profile(profile_id));
            }
        }
        self.pending_explorer_tab_reveal = self.active_tab_id();
    }

    fn active_tab_explorer_node(&self) -> Option<(ExplorerNodeId, bool)> {
        let node = match self.tabs.get(self.active_tab)? {
            WorkspaceTab::Sql(tab) => {
                let target = tab.execution_target.as_ref()?;
                return Some(self.explorer_namespace_node(
                    target.profile_id,
                    self.target_catalog_database(target),
                    target.schema.as_deref(),
                ));
            }
            WorkspaceTab::Relation(tab) => {
                let profile_id = tab.descriptor.key.profile_id;
                if self.profiles.iter().any(|profile| {
                    profile.id == profile_id && profile.kind == DatabaseKind::MongoDb
                }) {
                    ExplorerNodeId::Mongo {
                        profile_id,
                        node: MongoExplorerNodeId::Collection(MongoNamespace {
                            database: tab.descriptor.qualified_name.database.clone()?,
                            collection: Some(tab.descriptor.qualified_name.object.clone()),
                        }),
                    }
                } else {
                    ExplorerNodeId::Catalog(tab.descriptor.key.object_id.clone())
                }
            }
            WorkspaceTab::Dashboard(tab) => {
                let profile_id = tab.profile_id?;
                match tab.redis_database {
                    Some(database) => ExplorerNodeId::RedisDatabase {
                        profile_id,
                        database,
                    },
                    None => ExplorerNodeId::Profile(profile_id),
                }
            }
            WorkspaceTab::RedisBrowser(tab) => ExplorerNodeId::RedisDatabase {
                profile_id: tab.target.profile_id,
                database: tab.target.database,
            },
            WorkspaceTab::MongoBrowser(tab) => ExplorerNodeId::Mongo {
                profile_id: tab.profile_id,
                node: MongoExplorerNodeId::Collection(tab.namespace.clone()),
            },
            WorkspaceTab::MongoConsole(tab) => ExplorerNodeId::Mongo {
                profile_id: tab.profile_id,
                node: MongoExplorerNodeId::Database(tab.database.clone()),
            },
            WorkspaceTab::MongoDashboard(tab) => ExplorerNodeId::Mongo {
                profile_id: tab.profile_id,
                node: MongoExplorerNodeId::Database(tab.database.clone()),
            },
            WorkspaceTab::PrincipalDdl(tab) => ExplorerNodeId::Principal {
                entry: tab.entry.id.clone(),
            },
        };
        Some((node, true))
    }

    fn explorer_namespace_node(
        &self,
        profile_id: Uuid,
        database: &str,
        schema: Option<&str>,
    ) -> (ExplorerNodeId, bool) {
        let profile_node = ExplorerNodeId::Profile(profile_id);
        let Some(profile) = self.explorer.normalized.profiles.get(&profile_id) else {
            return (profile_node, false);
        };
        let catalog = &profile.catalog;
        let database_entry = catalog
            .roots()
            .iter()
            .filter_map(|id| catalog.get(id))
            .find(|entry| {
                entry.kind == CatalogKind::Database
                    && (entry.qualified_name.database.as_deref() == Some(database)
                        || entry.qualified_name.object == database)
            });
        let Some(database_entry) = database_entry else {
            return (profile_node, false);
        };
        if let Some(schema) = schema {
            let schema_entry = catalog
                .children(&database_entry.id)
                .iter()
                .filter_map(|id| catalog.get(id))
                .find(|entry| {
                    entry.kind == CatalogKind::Schema
                        && (entry.qualified_name.schema.as_deref() == Some(schema)
                            || entry.qualified_name.object == schema)
                });
            if let Some(schema_entry) = schema_entry {
                return (ExplorerNodeId::Catalog(schema_entry.id.clone()), true);
            }
        }
        (
            ExplorerNodeId::Catalog(database_entry.id.clone()),
            schema.is_none(),
        )
    }
}
