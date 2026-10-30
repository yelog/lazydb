use std::collections::{HashMap, HashSet};

use thiserror::Error;
use uuid::Uuid;

use crate::db::catalog::{
    CatalogCompleteness, CatalogCount, CatalogCursor, CatalogEntry, CatalogId, CatalogKind,
    CatalogRequest, CatalogSearchHit, CatalogTarget, ObjectGroup, search_text_matches,
};
use crate::db::catalog_mutation::CatalogMutationAnchor;
use crate::db::principal::PrincipalEntry;
use crate::profile::{ConnectionGroup, DatabaseKind};

#[cfg(test)]
std::thread_local! {
    static PROJECTION_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Test-only read of how many full-tree projections ran on this thread.
#[cfg(test)]
pub(crate) fn take_projection_calls() -> usize {
    PROJECTION_CALLS.with(|calls| calls.replace(0))
}

fn group_label(group: ObjectGroup) -> &'static str {
    match group {
        ObjectGroup::Tables => "Tables",
        ObjectGroup::Views => "Views",
        ObjectGroup::MaterializedViews => "Materialized views",
        ObjectGroup::Dictionaries => "Dictionaries",
        ObjectGroup::UserDefinedFunctions => "User defined functions",
        ObjectGroup::Sequences => "Sequences",
        ObjectGroup::Functions => "Functions",
        ObjectGroup::Procedures => "Procedures",
        ObjectGroup::Types => "Types",
        ObjectGroup::Triggers => "Triggers",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileProvenance {
    Saved,
    Session,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProfilePlacement {
    #[default]
    CurrentProject,
    Global,
    OtherProject,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ProfileRegion {
    Primary,
    Others,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ExplorerOwnerId {
    Profile(Uuid),
    Catalog(CatalogId),
    Projections(CatalogId),
    Group {
        parent: CatalogId,
        group: ObjectGroup,
    },
}

impl ExplorerOwnerId {
    pub const fn profile_id(&self) -> Uuid {
        match self {
            Self::Profile(profile_id) => *profile_id,
            Self::Catalog(id) | Self::Projections(id) | Self::Group { parent: id, .. } => {
                id.profile_id()
            }
        }
    }

    pub fn node_id(&self) -> ExplorerNodeId {
        match self {
            Self::Profile(profile_id) => ExplorerNodeId::Profile(*profile_id),
            Self::Catalog(id) => ExplorerNodeId::Catalog(id.clone()),
            Self::Projections(id) => ExplorerNodeId::ProjectionGroup(id.clone()),
            Self::Group { parent, group } => ExplorerNodeId::Group {
                parent: parent.clone(),
                group: *group,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ExplorerNodeId {
    EmptyProfiles,
    Others,
    Profile(Uuid),
    RedisDatabase {
        profile_id: Uuid,
        database: u32,
    },
    Mongo {
        profile_id: Uuid,
        node: crate::db::mongodb::types::MongoExplorerNodeId,
    },
    ConnectionGroup {
        group_id: Uuid,
        region: ProfileRegion,
    },
    Catalog(CatalogId),
    ProjectionGroup(CatalogId),
    Group {
        parent: CatalogId,
        group: ObjectGroup,
    },
    Status {
        owner: ExplorerOwnerId,
        kind: StatusRowKind,
    },
    LoadMore {
        parent: ExplorerOwnerId,
        cursor: CatalogCursor,
    },
    Empty {
        owner: ExplorerOwnerId,
    },
    PrincipalGroup {
        profile_id: Uuid,
    },
    Principal {
        entry: crate::db::principal::PrincipalId,
    },
    /// Non-selectable informational row shown under the `Users & Roles`
    /// group when there is no principal list to show (unsupported database,
    /// empty result, or a load failure).
    PrincipalNotice {
        profile_id: Uuid,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExplorerMutationIntent {
    EditProfile(Uuid),
    Create(CatalogMutationAnchor),
    Edit(CatalogMutationAnchor),
}

pub fn resolve_mutation_intent(
    selected: Option<&ExplorerNodeId>,
    edit: bool,
) -> Option<ExplorerMutationIntent> {
    match selected? {
        ExplorerNodeId::Profile(profile_id) => {
            if edit {
                Some(ExplorerMutationIntent::EditProfile(*profile_id))
            } else {
                Some(ExplorerMutationIntent::Create(
                    CatalogMutationAnchor::Profile {
                        profile_id: *profile_id,
                    },
                ))
            }
        }
        ExplorerNodeId::Catalog(id) => Some(if edit {
            ExplorerMutationIntent::Edit(CatalogMutationAnchor::Catalog(id.clone()))
        } else {
            ExplorerMutationIntent::Create(CatalogMutationAnchor::Catalog(id.clone()))
        }),
        ExplorerNodeId::Group { parent, group } if !edit => Some(ExplorerMutationIntent::Create(
            CatalogMutationAnchor::Group {
                schema: parent.clone(),
                group: *group,
            },
        )),
        ExplorerNodeId::ConnectionGroup { .. }
        | ExplorerNodeId::RedisDatabase { .. }
        | ExplorerNodeId::Mongo { .. } => None,
        ExplorerNodeId::PrincipalGroup { .. } | ExplorerNodeId::Principal { .. } => None,
        ExplorerNodeId::PrincipalNotice { .. } => None,
        ExplorerNodeId::EmptyProfiles
        | ExplorerNodeId::Others
        | ExplorerNodeId::Status { .. }
        | ExplorerNodeId::LoadMore { .. }
        | ExplorerNodeId::Empty { .. }
        | ExplorerNodeId::ProjectionGroup(_)
        | ExplorerNodeId::Group { .. } => None,
    }
}

impl ExplorerNodeId {
    pub const fn profile_id(&self) -> Option<Uuid> {
        match self {
            Self::Profile(profile_id) => Some(*profile_id),
            Self::Catalog(id) => Some(id.profile_id()),
            Self::ProjectionGroup(id) => Some(id.profile_id()),
            Self::Group { parent, .. } => Some(parent.profile_id()),
            Self::Status { owner, .. }
            | Self::LoadMore { parent: owner, .. }
            | Self::Empty { owner } => Some(owner.profile_id()),
            Self::EmptyProfiles | Self::Others | Self::ConnectionGroup { .. } => None,
            Self::RedisDatabase { profile_id, .. } => Some(*profile_id),
            Self::Mongo { profile_id, .. } => Some(*profile_id),
            Self::PrincipalGroup { profile_id } => Some(*profile_id),
            Self::Principal { entry } => Some(entry.profile_id),
            Self::PrincipalNotice { profile_id } => Some(*profile_id),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StatusRowKind {
    Loading,
    Retry,
    Stale,
    PermissionDenied,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExplorerConnectionStatus {
    #[default]
    Offline,
    Linking,
    Online,
    Syncing,
    Failed,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum ExplorerLoadState {
    #[default]
    NotLoaded,
    Loading {
        request_id: u64,
    },
    Loaded {
        next_cursor: Option<CatalogCursor>,
    },
    Stale {
        next_cursor: Option<CatalogCursor>,
    },
    Failed {
        request_id: u64,
    },
    PermissionDenied {
        request_id: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogGroupState {
    pub count: CatalogCount,
    pub completeness: CatalogCompleteness,
}

impl Default for CatalogGroupState {
    fn default() -> Self {
        Self {
            count: CatalogCount::Unknown,
            completeness: CatalogCompleteness::Partial,
        }
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CatalogTreeError {
    #[error("catalog profile {found} does not match tree profile {expected}")]
    ProfileMismatch { expected: Uuid, found: Uuid },
    #[error("duplicate catalog ID {id:?}")]
    DuplicateId { id: CatalogId },
    #[error("catalog entry {id:?} declares kind {entry_kind:?} inconsistent with its ID")]
    KindMismatch {
        id: CatalogId,
        entry_kind: CatalogKind,
    },
    #[error("catalog entry {id:?} is missing parent {parent:?}")]
    MissingParent { id: CatalogId, parent: CatalogId },
    #[error("catalog entry {id:?} has invalid parent {parent:?}")]
    InvalidParent { id: CatalogId, parent: CatalogId },
    #[error("catalog root {id:?} must be a database")]
    InvalidRoot { id: CatalogId },
    #[error("catalog owner {id:?} does not exist")]
    MissingOwner { id: CatalogId },
    #[error("catalog group parent {parent:?} must be a schema")]
    InvalidGroupParent { parent: CatalogId },
    #[error("catalog group {group:?} already exists for schema {parent:?}")]
    DuplicateGroup {
        parent: CatalogId,
        group: ObjectGroup,
    },
    #[error("catalog entry {id:?} is outside replacement owner {owner:?}")]
    EntryOutsideOwner {
        id: CatalogId,
        owner: ExplorerOwnerId,
    },
}

#[derive(Clone, Debug)]
pub struct CatalogTree {
    profile_id: Uuid,
    entries: HashMap<CatalogId, CatalogEntry>,
    children: HashMap<CatalogId, Vec<CatalogId>>,
    roots: Vec<CatalogId>,
    group_states: HashMap<(CatalogId, ObjectGroup), CatalogGroupState>,
    group_order: HashMap<CatalogId, Vec<ObjectGroup>>,
    group_children: HashMap<(CatalogId, ObjectGroup), Vec<CatalogId>>,
    search_injected: HashSet<CatalogId>,
}

impl CatalogTree {
    pub fn new(profile_id: Uuid) -> Self {
        Self {
            profile_id,
            entries: HashMap::new(),
            children: HashMap::new(),
            roots: Vec::new(),
            group_states: HashMap::new(),
            group_order: HashMap::new(),
            group_children: HashMap::new(),
            search_injected: HashSet::new(),
        }
    }

    pub const fn profile_id(&self) -> Uuid {
        self.profile_id
    }

    pub fn entries(&self) -> &HashMap<CatalogId, CatalogEntry> {
        &self.entries
    }

    pub fn schemas_in_database(&self, database: &str) -> Vec<CatalogId> {
        self.entries
            .values()
            .filter(|entry| {
                entry.kind == CatalogKind::Schema
                    && entry.qualified_name.database.as_deref() == Some(database)
            })
            .map(|entry| entry.id.clone())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, id: &CatalogId) -> Option<&CatalogEntry> {
        self.entries.get(id)
    }

    pub fn parent(&self, id: &CatalogId) -> Option<&CatalogId> {
        self.entries.get(id)?.parent_id.as_ref()
    }

    pub fn children(&self, parent: &CatalogId) -> &[CatalogId] {
        self.children.get(parent).map_or(&[], Vec::as_slice)
    }

    pub fn roots(&self) -> &[CatalogId] {
        &self.roots
    }

    pub fn group_state(
        &self,
        schema: &CatalogId,
        group: ObjectGroup,
    ) -> Option<&CatalogGroupState> {
        self.group_states.get(&(schema.clone(), group))
    }

    pub fn groups(&self, schema: &CatalogId) -> &[ObjectGroup] {
        self.group_order.get(schema).map_or(&[], Vec::as_slice)
    }

    pub fn group_children(&self, schema: &CatalogId, group: ObjectGroup) -> &[CatalogId] {
        self.group_children
            .get(&(schema.clone(), group))
            .map_or(&[], Vec::as_slice)
    }

    pub fn owning_relation_id(&self, id: &CatalogId) -> Option<&CatalogId> {
        let entry = self.entries.get(id)?;
        if entry.id.profile_id() != self.profile_id {
            return None;
        }
        if entry.kind.is_relation() {
            return Some(&entry.id);
        }
        if entry.kind == CatalogKind::Projection {
            let table_id = entry.parent_id.as_ref()?;
            let table = self.entries.get(table_id)?;
            return (table.kind == CatalogKind::Table && table.id.profile_id() == self.profile_id)
                .then_some(&table.id);
        }
        if !entry.kind.is_relation_child() {
            return None;
        }
        let declared_relation = entry.owning_relation_id().cloned();
        let mut current = id;
        let mut visited = HashSet::new();
        loop {
            if !visited.insert(current.clone()) {
                return None;
            }
            let current_entry = self.entries.get(current)?;
            if current_entry.kind.is_relation() {
                return (current_entry.id.profile_id() == self.profile_id
                    && declared_relation
                        .as_ref()
                        .is_none_or(|id| id == &current_entry.id))
                .then_some(&current_entry.id);
            }
            if current_entry.kind == CatalogKind::Schema {
                let relation_id = declared_relation.as_ref()?;
                let relation = self.entries.get(relation_id)?;
                return (relation.kind.is_relation()
                    && relation.id.profile_id() == self.profile_id)
                    .then_some(&relation.id);
            }
            if current_entry.kind == CatalogKind::Table {
                return Some(&current_entry.id);
            }
            if !current_entry.kind.is_relation_child() {
                return None;
            }
            let parent = current_entry.parent_id.as_ref()?;
            if parent.profile_id() != self.profile_id {
                return None;
            }
            current = parent;
        }
    }

    pub fn owning_relation(&self, id: &CatalogId) -> Option<&CatalogEntry> {
        self.entries.get(self.owning_relation_id(id)?)
    }

    pub fn set_group_state(
        &mut self,
        schema: &CatalogId,
        group: ObjectGroup,
        state: CatalogGroupState,
    ) -> Result<(), CatalogTreeError> {
        self.validate_profile(schema.profile_id())?;
        let Some(entry) = self.entries.get(schema) else {
            return Err(CatalogTreeError::MissingOwner { id: schema.clone() });
        };
        if entry.kind != CatalogKind::Schema {
            return Err(CatalogTreeError::InvalidGroupParent {
                parent: schema.clone(),
            });
        }
        let key = (schema.clone(), group);
        if !self.group_states.contains_key(&key) {
            self.group_order
                .entry(schema.clone())
                .or_default()
                .push(group);
        }
        self.group_states.insert(key, state);
        Ok(())
    }

    pub fn replace_group_states(
        &mut self,
        schema: &CatalogId,
        states: Vec<(ObjectGroup, CatalogGroupState)>,
    ) -> Result<(), CatalogTreeError> {
        self.validate_profile(schema.profile_id())?;
        let Some(entry) = self.entries.get(schema) else {
            return Err(CatalogTreeError::MissingOwner { id: schema.clone() });
        };
        if entry.kind != CatalogKind::Schema {
            return Err(CatalogTreeError::InvalidGroupParent {
                parent: schema.clone(),
            });
        }
        self.group_order.insert(
            schema.clone(),
            states.iter().map(|(group, _)| *group).collect(),
        );
        self.group_states.retain(|(parent, _), _| parent != schema);
        for (group, state) in states {
            self.group_states.insert((schema.clone(), group), state);
        }
        Ok(())
    }

    pub fn reconcile_group_states(
        &mut self,
        schema: &CatalogId,
        states: Vec<(ObjectGroup, CatalogGroupState)>,
    ) -> Result<Vec<CatalogId>, CatalogTreeError> {
        self.validate_profile(schema.profile_id())?;
        let Some(entry) = self.entries.get(schema) else {
            return Err(CatalogTreeError::MissingOwner { id: schema.clone() });
        };
        if entry.kind != CatalogKind::Schema {
            return Err(CatalogTreeError::InvalidGroupParent {
                parent: schema.clone(),
            });
        }
        let mut seen = HashSet::with_capacity(states.len());
        for (group, _) in &states {
            if !seen.insert(*group) {
                return Err(CatalogTreeError::DuplicateGroup {
                    parent: schema.clone(),
                    group: *group,
                });
            }
        }
        let removed_groups = self
            .groups(schema)
            .iter()
            .filter(|group| !seen.contains(group))
            .copied()
            .collect::<Vec<_>>();
        let removed_roots = removed_groups
            .iter()
            .flat_map(|group| self.group_children(schema, *group).iter().cloned())
            .collect::<Vec<_>>();
        let removed = self.collect_subtrees(&removed_roots);
        let removed_set = removed.iter().cloned().collect::<HashSet<_>>();
        self.remove_ids(&removed, &removed_set);

        self.group_order.insert(
            schema.clone(),
            states.iter().map(|(group, _)| *group).collect(),
        );
        let retained_groups = seen;
        self.group_children
            .retain(|(parent, group), _| parent != schema || retained_groups.contains(group));
        self.group_states.retain(|(parent, _), _| parent != schema);
        for (group, state) in states {
            self.group_states.insert((schema.clone(), group), state);
        }
        Ok(removed)
    }

    pub fn append_group_states(
        &mut self,
        schema: &CatalogId,
        states: Vec<(ObjectGroup, CatalogGroupState)>,
    ) -> Result<(), CatalogTreeError> {
        self.validate_profile(schema.profile_id())?;
        let Some(entry) = self.entries.get(schema) else {
            return Err(CatalogTreeError::MissingOwner { id: schema.clone() });
        };
        if entry.kind != CatalogKind::Schema {
            return Err(CatalogTreeError::InvalidGroupParent {
                parent: schema.clone(),
            });
        }
        if let Some((group, _)) = states
            .iter()
            .find(|(group, _)| self.group_states.contains_key(&(schema.clone(), *group)))
        {
            return Err(CatalogTreeError::DuplicateGroup {
                parent: schema.clone(),
                group: *group,
            });
        }
        self.group_order
            .entry(schema.clone())
            .or_default()
            .extend(states.iter().map(|(group, _)| *group));
        for (group, state) in states {
            self.group_states.insert((schema.clone(), group), state);
        }
        Ok(())
    }

    pub fn insert(&mut self, entry: CatalogEntry) -> Result<(), CatalogTreeError> {
        self.insert_subtree(vec![entry])
    }

    pub fn insert_subtree(&mut self, entries: Vec<CatalogEntry>) -> Result<(), CatalogTreeError> {
        self.validate_batch(&entries, &HashSet::new())?;
        self.insert_validated(entries);
        Ok(())
    }

    pub fn merge_search_hit(&mut self, hit: &CatalogSearchHit) -> Result<(), CatalogTreeError> {
        let entries = hit
            .ancestors
            .iter()
            .chain(std::iter::once(&hit.entry))
            .filter(|entry| !self.entries.contains_key(&entry.id))
            .cloned()
            .collect::<Vec<_>>();
        if !entries.is_empty() {
            let injected = entries
                .iter()
                .map(|entry| entry.id.clone())
                .collect::<Vec<_>>();
            self.insert_subtree(entries)?;
            self.search_injected.extend(injected);
        }
        for entry in hit.ancestors.iter().chain(std::iter::once(&hit.entry)) {
            if let Some(existing) = self.entries.get_mut(&entry.id) {
                *existing = entry.clone();
            }
        }
        for entry in hit.ancestors.iter().chain(std::iter::once(&hit.entry)) {
            if let Some(group) = object_group(entry.kind)
                && let Some(schema) = entry.parent_id.as_ref()
                && self.group_state(schema, group).is_none()
            {
                self.set_group_state(schema, group, CatalogGroupState::default())?;
            }
        }
        Ok(())
    }

    pub fn replace_subtree(
        &mut self,
        parent: &CatalogId,
        entries: Vec<CatalogEntry>,
    ) -> Result<Vec<CatalogId>, CatalogTreeError> {
        self.replace_page(&ExplorerOwnerId::Catalog(parent.clone()), entries)
    }

    pub fn replace_projections(
        &mut self,
        table: &CatalogId,
        entries: Vec<CatalogEntry>,
    ) -> Result<Vec<CatalogId>, CatalogTreeError> {
        self.replace_page(&ExplorerOwnerId::Projections(table.clone()), entries)
    }

    pub fn replace_roots(
        &mut self,
        entries: Vec<CatalogEntry>,
    ) -> Result<Vec<CatalogId>, CatalogTreeError> {
        self.replace_page(&ExplorerOwnerId::Profile(self.profile_id), entries)
    }

    pub fn replace_page(
        &mut self,
        owner: &ExplorerOwnerId,
        entries: Vec<CatalogEntry>,
    ) -> Result<Vec<CatalogId>, CatalogTreeError> {
        self.validate_profile(owner.profile_id())?;
        self.validate_owner(owner)?;

        let removal_roots = match owner {
            ExplorerOwnerId::Profile(_) => self.roots.clone(),
            ExplorerOwnerId::Catalog(parent) => self.children(parent).to_vec(),
            ExplorerOwnerId::Projections(parent) => self
                .children(parent)
                .iter()
                .filter(|id| {
                    self.entries
                        .get(*id)
                        .is_some_and(|entry| entry.kind == CatalogKind::Projection)
                })
                .cloned()
                .collect(),
            ExplorerOwnerId::Group { parent, group } => {
                self.group_children(parent, *group).to_vec()
            }
        };
        let removed = self.collect_subtrees(&removal_roots);
        let removed_set: HashSet<_> = removed.iter().cloned().collect();
        self.validate_batch(&entries, &removed_set)?;
        self.validate_replacement_scope(owner, &entries)?;

        self.remove_ids(&removed, &removed_set);
        if let ExplorerOwnerId::Group { parent, group } = owner
            && self.group_state(parent, *group).is_none()
        {
            self.set_group_state(parent, *group, CatalogGroupState::default())?;
        }
        self.insert_validated(entries);
        Ok(removed)
    }

    /// Reconcile a complete owner snapshot while retaining subtrees whose IDs
    /// are unchanged. Background refreshes use this to keep stable catalog
    /// entries and their loaded descendants available while pages are fetched.
    pub fn reconcile_page(
        &mut self,
        owner: &ExplorerOwnerId,
        entries: Vec<CatalogEntry>,
    ) -> Result<Vec<CatalogId>, CatalogTreeError> {
        self.validate_profile(owner.profile_id())?;
        self.validate_owner(owner)?;
        self.validate_replacement_scope(owner, &entries)?;

        let mut incoming = HashSet::with_capacity(entries.len());
        for entry in &entries {
            self.validate_profile(entry.id.profile_id())?;
            if entry.kind != entry.id.kind {
                return Err(CatalogTreeError::KindMismatch {
                    id: entry.id.clone(),
                    entry_kind: entry.kind,
                });
            }
            if !incoming.insert(entry.id.clone()) {
                return Err(CatalogTreeError::DuplicateId {
                    id: entry.id.clone(),
                });
            }
            if let Some(existing) = self.entries.get(&entry.id)
                && existing.parent_id != entry.parent_id
            {
                return Err(CatalogTreeError::EntryOutsideOwner {
                    id: entry.id.clone(),
                    owner: owner.clone(),
                });
            }
        }

        let current_children = match owner {
            ExplorerOwnerId::Profile(_) => self.roots.clone(),
            ExplorerOwnerId::Catalog(parent) => self
                .children(parent)
                .iter()
                .filter(|id| {
                    parent.kind != CatalogKind::Table
                        || !self
                            .entries
                            .get(*id)
                            .is_some_and(|entry| entry.kind == CatalogKind::Projection)
                })
                .cloned()
                .collect(),
            ExplorerOwnerId::Projections(parent) => self
                .children(parent)
                .iter()
                .filter(|id| {
                    self.entries
                        .get(*id)
                        .is_some_and(|entry| entry.kind == CatalogKind::Projection)
                })
                .cloned()
                .collect(),
            ExplorerOwnerId::Group { parent, group } => {
                self.group_children(parent, *group).to_vec()
            }
        };
        let removal_roots = current_children
            .iter()
            .filter(|id| !incoming.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        let removed = self.collect_subtrees(&removal_roots);
        let removed_set = removed.iter().cloned().collect::<HashSet<_>>();
        let new_entries = entries
            .iter()
            .filter(|entry| !self.entries.contains_key(&entry.id))
            .cloned()
            .collect::<Vec<_>>();
        self.validate_batch(&new_entries, &removed_set)?;

        self.remove_ids(&removed, &removed_set);
        for entry in &entries {
            if self.entries.contains_key(&entry.id) {
                self.entries.insert(entry.id.clone(), entry.clone());
            }
        }
        self.insert_validated(new_entries);

        let mut ordered_ids = entries
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        match owner {
            ExplorerOwnerId::Profile(_) => self.roots = ordered_ids,
            ExplorerOwnerId::Catalog(parent) => {
                if parent.kind == CatalogKind::Table {
                    ordered_ids.extend(
                        self.children(parent)
                            .iter()
                            .filter(|id| {
                                self.entries
                                    .get(*id)
                                    .is_some_and(|entry| entry.kind == CatalogKind::Projection)
                            })
                            .cloned(),
                    );
                }
                self.children.insert(parent.clone(), ordered_ids);
            }
            ExplorerOwnerId::Projections(parent) => {
                let children = self.children.entry(parent.clone()).or_default();
                children.retain(|id| {
                    self.entries
                        .get(id)
                        .is_some_and(|entry| entry.kind != CatalogKind::Projection)
                });
                children.extend(ordered_ids);
            }
            ExplorerOwnerId::Group { parent, group } => {
                self.group_children
                    .insert((parent.clone(), *group), ordered_ids);
            }
        }
        Ok(removed)
    }

    pub fn append_page(
        &mut self,
        owner: &ExplorerOwnerId,
        entries: Vec<CatalogEntry>,
    ) -> Result<(), CatalogTreeError> {
        self.validate_profile(owner.profile_id())?;
        self.validate_owner(owner)?;
        let injected_roots = entries
            .iter()
            .filter(|entry| self.search_injected.contains(&entry.id))
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();
        let removed = self.collect_subtrees(&injected_roots);
        let removed_set = removed.iter().cloned().collect::<HashSet<_>>();
        self.validate_batch(&entries, &removed_set)?;
        self.validate_replacement_scope(owner, &entries)?;
        self.remove_ids(&removed, &removed_set);
        self.insert_validated(entries);
        Ok(())
    }

    pub fn append_projections(
        &mut self,
        table: &CatalogId,
        entries: Vec<CatalogEntry>,
    ) -> Result<(), CatalogTreeError> {
        self.append_page(&ExplorerOwnerId::Projections(table.clone()), entries)
    }

    pub fn is_projection_table(&self, table: &CatalogId) -> bool {
        self.entries
            .get(table)
            .is_some_and(|entry| entry.kind == CatalogKind::Table)
    }

    pub fn remove_subtree(&mut self, root: &CatalogId) -> Result<Vec<CatalogId>, CatalogTreeError> {
        self.validate_profile(root.profile_id())?;
        if !self.entries.contains_key(root) {
            return Ok(Vec::new());
        }
        let removed = self.collect_subtrees(std::slice::from_ref(root));
        let removed_set: HashSet<_> = removed.iter().cloned().collect();
        self.remove_ids(&removed, &removed_set);
        Ok(removed)
    }

    pub fn replace_entry(
        &mut self,
        old: &CatalogId,
        entry: CatalogEntry,
    ) -> Result<Vec<CatalogId>, CatalogTreeError> {
        self.validate_profile(entry.id.profile_id())?;
        let removed = self.collect_subtrees(std::slice::from_ref(old));
        let removed_set = removed.iter().cloned().collect::<HashSet<_>>();
        self.validate_batch(std::slice::from_ref(&entry), &removed_set)?;
        self.remove_ids(&removed, &removed_set);
        self.insert_validated(vec![entry]);
        Ok(removed)
    }

    pub fn remove_projection(
        &mut self,
        projection: &CatalogId,
    ) -> Result<Vec<CatalogId>, CatalogTreeError> {
        if projection.kind != CatalogKind::Projection {
            return Ok(Vec::new());
        }
        self.remove_subtree(projection)
    }

    fn validate_profile(&self, found: Uuid) -> Result<(), CatalogTreeError> {
        if found == self.profile_id {
            Ok(())
        } else {
            Err(CatalogTreeError::ProfileMismatch {
                expected: self.profile_id,
                found,
            })
        }
    }

    fn validate_owner(&self, owner: &ExplorerOwnerId) -> Result<(), CatalogTreeError> {
        match owner {
            ExplorerOwnerId::Profile(_) => Ok(()),
            ExplorerOwnerId::Catalog(id) => {
                if self.entries.contains_key(id) {
                    Ok(())
                } else {
                    Err(CatalogTreeError::MissingOwner { id: id.clone() })
                }
            }
            ExplorerOwnerId::Projections(id) => {
                if id.kind == CatalogKind::Table
                    && self
                        .entries
                        .get(id)
                        .is_some_and(|entry| entry.kind == CatalogKind::Table)
                {
                    Ok(())
                } else {
                    Err(CatalogTreeError::MissingOwner { id: id.clone() })
                }
            }
            ExplorerOwnerId::Group { parent, .. } => {
                let Some(entry) = self.entries.get(parent) else {
                    return Err(CatalogTreeError::MissingOwner { id: parent.clone() });
                };
                if entry.kind == CatalogKind::Schema {
                    Ok(())
                } else {
                    Err(CatalogTreeError::InvalidGroupParent {
                        parent: parent.clone(),
                    })
                }
            }
        }
    }

    fn validate_batch(
        &self,
        entries: &[CatalogEntry],
        removable: &HashSet<CatalogId>,
    ) -> Result<(), CatalogTreeError> {
        let mut batch_ids = HashSet::with_capacity(entries.len());
        for entry in entries {
            self.validate_profile(entry.id.profile_id())?;
            if entry.kind != entry.id.kind {
                return Err(CatalogTreeError::KindMismatch {
                    id: entry.id.clone(),
                    entry_kind: entry.kind,
                });
            }
            if !batch_ids.insert(entry.id.clone())
                || (self.entries.contains_key(&entry.id) && !removable.contains(&entry.id))
            {
                return Err(CatalogTreeError::DuplicateId {
                    id: entry.id.clone(),
                });
            }
            if let Some(relation_id) = entry.owning_relation_id() {
                self.validate_profile(relation_id.profile_id())?;
            }
        }

        for entry in entries {
            let Some(parent) = entry.parent_id.as_ref() else {
                if entry.kind == CatalogKind::Database {
                    continue;
                }
                return Err(CatalogTreeError::InvalidRoot {
                    id: entry.id.clone(),
                });
            };
            self.validate_profile(parent.profile_id())?;
            if !batch_ids.contains(parent)
                && (!self.entries.contains_key(parent) || removable.contains(parent))
            {
                return Err(CatalogTreeError::MissingParent {
                    id: entry.id.clone(),
                    parent: parent.clone(),
                });
            }
            if !valid_parent(entry.kind, parent.kind) {
                return Err(CatalogTreeError::InvalidParent {
                    id: entry.id.clone(),
                    parent: parent.clone(),
                });
            }
        }
        Ok(())
    }

    fn validate_replacement_scope(
        &self,
        owner: &ExplorerOwnerId,
        entries: &[CatalogEntry],
    ) -> Result<(), CatalogTreeError> {
        let batch_ids: HashSet<_> = entries.iter().map(|entry| &entry.id).collect();
        for entry in entries {
            if entry
                .parent_id
                .as_ref()
                .is_some_and(|parent| batch_ids.contains(parent))
            {
                continue;
            }
            let belongs = match owner {
                ExplorerOwnerId::Profile(_) => entry.parent_id.is_none(),
                ExplorerOwnerId::Catalog(parent) => entry.parent_id.as_ref() == Some(parent),
                ExplorerOwnerId::Projections(parent) => {
                    entry.kind == CatalogKind::Projection
                        && entry.parent_id.as_ref() == Some(parent)
                        && entry.relation_id.as_ref() == Some(parent)
                        && entry.id.native_path.len() == parent.native_path.len() + 1
                        && entry.id.native_path.starts_with(&parent.native_path)
                }
                ExplorerOwnerId::Group { parent, group } => {
                    entry.parent_id.as_ref() == Some(parent) && group.contains_kind(entry.kind)
                }
            };
            if !belongs {
                return Err(CatalogTreeError::EntryOutsideOwner {
                    id: entry.id.clone(),
                    owner: owner.clone(),
                });
            }
        }
        Ok(())
    }

    fn insert_validated(&mut self, entries: Vec<CatalogEntry>) {
        for entry in &entries {
            self.entries.insert(entry.id.clone(), entry.clone());
        }
        for entry in entries {
            let id = entry.id.clone();
            match entry.parent_id.as_ref() {
                Some(parent) => {
                    self.children
                        .entry(parent.clone())
                        .or_default()
                        .push(id.clone());
                    if parent.kind == CatalogKind::Schema
                        && let Some(group) = object_group(entry.kind)
                    {
                        self.group_children
                            .entry((parent.clone(), group))
                            .or_default()
                            .push(id);
                    }
                }
                None => self.roots.push(id),
            }
        }
    }

    fn collect_subtrees(&self, roots: &[CatalogId]) -> Vec<CatalogId> {
        let mut removed = Vec::new();
        let mut stack: Vec<_> = roots.iter().rev().cloned().collect();
        while let Some(id) = stack.pop() {
            if !self.entries.contains_key(&id) {
                continue;
            }
            removed.push(id.clone());
            if let Some(children) = self.children.get(&id) {
                stack.extend(children.iter().rev().cloned());
            }
        }
        removed
    }

    fn remove_ids(&mut self, removed: &[CatalogId], removed_set: &HashSet<CatalogId>) {
        self.search_injected.retain(|id| !removed_set.contains(id));
        self.roots.retain(|id| !removed_set.contains(id));
        let mut retained_parents = HashSet::new();
        let mut retained_group_parents = HashSet::new();
        for id in removed {
            let Some(entry) = self.entries.get(id) else {
                continue;
            };
            let Some(parent) = entry.parent_id.as_ref() else {
                continue;
            };
            if !removed_set.contains(parent) {
                retained_parents.insert(parent.clone());
                if parent.kind == CatalogKind::Schema
                    && let Some(group) = object_group(entry.kind)
                {
                    retained_group_parents.insert((parent.clone(), group));
                }
            }
        }
        for parent in retained_parents {
            if let Some(children) = self.children.get_mut(&parent) {
                children.retain(|id| !removed_set.contains(id));
            }
        }
        for key in retained_group_parents {
            if let Some(children) = self.group_children.get_mut(&key) {
                children.retain(|id| !removed_set.contains(id));
            }
        }

        for id in removed {
            self.entries.remove(id);
            self.children.remove(id);
            self.group_order.remove(id);
        }
        self.group_states
            .retain(|(schema, _), _| !removed_set.contains(schema));
        self.group_children
            .retain(|(schema, _), _| !removed_set.contains(schema));
    }
}

#[derive(Clone, Debug)]
pub struct ExplorerProfileState {
    pub display_name: String,
    pub kind: DatabaseKind,
    pub endpoint: String,
    pub provenance: ProfileProvenance,
    pub placement: ProfilePlacement,
    pub group_id: Option<Uuid>,
    pub status: ExplorerConnectionStatus,
    pub catalog: CatalogTree,
    pub catalog_epoch: u64,
    pub next_request_id: u64,
    pub load_states: HashMap<ExplorerOwnerId, ExplorerLoadState>,
    pub pending_requests: HashMap<ExplorerOwnerId, CatalogRequest>,
    pub pending_identity_requests: HashMap<u64, CatalogId>,
    pub previous_load_states: HashMap<ExplorerOwnerId, ExplorerLoadState>,
    pub refresh_staged_entries: HashMap<ExplorerOwnerId, Vec<CatalogEntry>>,
    pub refresh_staged_groups: HashMap<ExplorerOwnerId, Vec<(ObjectGroup, CatalogGroupState)>>,
    pub refresh_shallow_owners: HashSet<ExplorerOwnerId>,
    pub load_errors: HashMap<ExplorerOwnerId, String>,
    pub last_error: Option<String>,
    pub expand_after_connect: bool,
    pub unavailable_reason: Option<String>,
    pub redis_databases: Vec<crate::db::redis::discovery::RedisDatabaseInfo>,
    pub mongodb_databases: Vec<String>,
    pub mongodb_collections: HashMap<String, Vec<crate::db::mongodb::types::MongoCollectionInfo>>,
    pub mongodb_indexes: HashMap<
        crate::db::mongodb::types::MongoNamespace,
        Vec<crate::db::mongodb::types::MongoIndexInfo>,
    >,
    pub mongodb_error: Option<String>,
    pub mongodb_collection_errors: HashMap<String, String>,
    pub mongodb_index_errors: HashMap<crate::db::mongodb::types::MongoNamespace, String>,
    pub redis_databases_partial: bool,
    pub redis_databases_error: Option<String>,
    pub principals: Vec<PrincipalEntry>,
    pub principals_loaded: bool,
    pub principals_complete: bool,
    pub principals_error: Option<String>,
    pub principals_unsupported: Option<String>,
    pub principals_pending: Option<u64>,
}

impl ExplorerProfileState {
    pub fn new(
        profile_id: Uuid,
        display_name: String,
        kind: DatabaseKind,
        endpoint: String,
        provenance: ProfileProvenance,
        placement: ProfilePlacement,
    ) -> Self {
        Self {
            display_name,
            kind,
            endpoint,
            provenance,
            placement,
            group_id: None,
            status: ExplorerConnectionStatus::Offline,
            catalog: CatalogTree::new(profile_id),
            catalog_epoch: 0,
            next_request_id: 1,
            load_states: HashMap::new(),
            pending_requests: HashMap::new(),
            pending_identity_requests: HashMap::new(),
            previous_load_states: HashMap::new(),
            refresh_staged_entries: HashMap::new(),
            refresh_staged_groups: HashMap::new(),
            refresh_shallow_owners: HashSet::new(),
            load_errors: HashMap::new(),
            last_error: None,
            expand_after_connect: false,
            unavailable_reason: None,
            redis_databases: Vec::new(),
            mongodb_databases: Vec::new(),
            mongodb_collections: HashMap::new(),
            mongodb_indexes: HashMap::new(),
            mongodb_error: None,
            mongodb_collection_errors: HashMap::new(),
            mongodb_index_errors: HashMap::new(),
            redis_databases_partial: false,
            redis_databases_error: None,
            principals: Vec::new(),
            principals_loaded: false,
            principals_complete: false,
            principals_error: None,
            principals_unsupported: None,
            principals_pending: None,
        }
    }

    pub fn allocate_request_id(&mut self) -> Option<u64> {
        let next = self.next_request_id.checked_add(1)?;
        let request_id = self.next_request_id;
        self.next_request_id = next;
        Some(request_id)
    }

    pub fn advance_catalog_epoch(&mut self) -> Option<u64> {
        let next = self.catalog_epoch.checked_add(1)?;
        self.catalog_epoch = next;
        Some(next)
    }

    pub fn set_redis_databases(
        &mut self,
        discovery: crate::db::redis::discovery::RedisDatabaseDiscovery,
    ) {
        self.redis_databases = discovery.databases;
        self.redis_databases_partial = matches!(
            discovery.completeness,
            crate::db::redis::discovery::RedisDiscoveryCompleteness::Partial
        );
        self.redis_databases_error =
            (!discovery.warnings.is_empty()).then(|| discovery.warnings.join("; "));
    }

    pub fn set_principals(&mut self, page: crate::db::principal::PrincipalPage) {
        self.principals = page.entries;
        self.principals_loaded = true;
        self.principals_complete = page.complete;
        self.principals_error = None;
    }

    pub fn invalidate_catalog_target(&mut self, target: &CatalogTarget) {
        let owner = owner_for_target(self.catalog.profile_id(), target);
        let previous = self
            .previous_load_states
            .get(&owner)
            .or_else(|| self.load_states.get(&owner));
        let next_cursor = match previous {
            Some(ExplorerLoadState::Loaded { next_cursor })
            | Some(ExplorerLoadState::Stale { next_cursor }) => next_cursor.clone(),
            _ => None,
        };
        self.pending_requests.remove(&owner);
        self.previous_load_states.remove(&owner);
        self.refresh_staged_entries.remove(&owner);
        self.refresh_staged_groups.remove(&owner);
        self.refresh_shallow_owners.remove(&owner);
        self.refresh_shallow_owners.remove(&owner);
        self.load_errors.remove(&owner);
        self.load_states
            .insert(owner, ExplorerLoadState::Stale { next_cursor });
        if let CatalogTarget::RelationChildren { relation } = target
            && relation.kind == CatalogKind::Table
            && self.catalog.is_projection_table(relation)
        {
            let projection_owner = ExplorerOwnerId::Projections(relation.clone());
            let projection_previous = self
                .previous_load_states
                .get(&projection_owner)
                .or_else(|| self.load_states.get(&projection_owner));
            let projection_cursor = match projection_previous {
                Some(ExplorerLoadState::Loaded { next_cursor })
                | Some(ExplorerLoadState::Stale { next_cursor }) => next_cursor.clone(),
                _ => None,
            };
            self.pending_requests.remove(&projection_owner);
            self.previous_load_states.remove(&projection_owner);
            self.refresh_staged_entries.remove(&projection_owner);
            self.refresh_staged_groups.remove(&projection_owner);
            self.refresh_shallow_owners.remove(&projection_owner);
            self.refresh_shallow_owners.remove(&projection_owner);
            self.load_errors.remove(&projection_owner);
            self.load_states.insert(
                projection_owner,
                ExplorerLoadState::Stale {
                    next_cursor: projection_cursor,
                },
            );
        }
    }

    pub fn has_catalog_snapshot(&self, owner: &ExplorerOwnerId) -> bool {
        if matches!(
            self.previous_load_states
                .get(owner)
                .or_else(|| self.load_states.get(owner)),
            Some(ExplorerLoadState::Loaded { .. } | ExplorerLoadState::Stale { .. })
        ) {
            return true;
        }
        match owner {
            ExplorerOwnerId::Profile(_) => !self.catalog.roots().is_empty(),
            ExplorerOwnerId::Catalog(id) => {
                !self.catalog.children(id).is_empty() || !self.catalog.groups(id).is_empty()
            }
            ExplorerOwnerId::Projections(id) => self.catalog.children(id).iter().any(|child| {
                self.catalog
                    .get(child)
                    .is_some_and(|entry| entry.kind == CatalogKind::Projection)
            }),
            ExplorerOwnerId::Group { parent, group } => {
                !self.catalog.group_children(parent, *group).is_empty()
            }
        }
    }

    pub fn remove_catalog_state_for(&mut self, removed: &HashSet<CatalogId>) {
        let references_removed = |owner: &ExplorerOwnerId| match owner {
            ExplorerOwnerId::Profile(_) => false,
            ExplorerOwnerId::Catalog(id) | ExplorerOwnerId::Projections(id) => removed.contains(id),
            ExplorerOwnerId::Group { parent, .. } => removed.contains(parent),
        };
        self.load_states
            .retain(|owner, _| !references_removed(owner));
        self.pending_requests
            .retain(|owner, _| !references_removed(owner));
        self.previous_load_states
            .retain(|owner, _| !references_removed(owner));
        self.refresh_staged_entries
            .retain(|owner, _| !references_removed(owner));
        self.refresh_staged_groups
            .retain(|owner, _| !references_removed(owner));
        self.refresh_shallow_owners
            .retain(|owner| !references_removed(owner));
        self.refresh_shallow_owners
            .retain(|owner| !references_removed(owner));
        self.load_errors
            .retain(|owner, _| !references_removed(owner));
        self.pending_identity_requests
            .retain(|_, id| !removed.contains(id));
    }

    pub fn remove_catalog_group_state_for(
        &mut self,
        schema: &CatalogId,
        removed: &HashSet<ObjectGroup>,
    ) {
        let removed_owner = |owner: &ExplorerOwnerId| {
            matches!(
                owner,
                ExplorerOwnerId::Group { parent, group }
                    if parent == schema && removed.contains(group)
            )
        };
        self.load_states.retain(|owner, _| !removed_owner(owner));
        self.pending_requests
            .retain(|owner, _| !removed_owner(owner));
        self.previous_load_states
            .retain(|owner, _| !removed_owner(owner));
        self.refresh_staged_entries
            .retain(|owner, _| !removed_owner(owner));
        self.refresh_staged_groups
            .retain(|owner, _| !removed_owner(owner));
        self.refresh_shallow_owners
            .retain(|owner| !removed_owner(owner));
        self.refresh_shallow_owners
            .retain(|owner| !removed_owner(owner));
        self.load_errors.retain(|owner, _| !removed_owner(owner));
    }

    pub fn invalidate_database_objects(&mut self, database: &str) {
        let schemas = self.catalog.schemas_in_database(database);
        let targets = schemas
            .iter()
            .flat_map(|schema| {
                let mut targets = vec![CatalogTarget::Groups {
                    schema: schema.clone(),
                }];
                targets.extend(self.catalog.groups(schema).iter().map(|group| {
                    CatalogTarget::Objects {
                        schema: schema.clone(),
                        group: *group,
                    }
                }));
                targets
            })
            .collect::<Vec<_>>();
        for target in targets {
            self.invalidate_catalog_target(&target);
        }
    }

    pub fn recover_pending_requests(&mut self) -> Vec<CatalogTarget> {
        let pending = std::mem::take(&mut self.pending_requests);
        let mut targets = Vec::with_capacity(pending.len());
        for (owner, request) in pending {
            let previous = self
                .previous_load_states
                .remove(&owner)
                .unwrap_or(ExplorerLoadState::NotLoaded);
            self.refresh_staged_entries.remove(&owner);
            self.refresh_staged_groups.remove(&owner);
            self.refresh_shallow_owners.remove(&owner);
            self.refresh_shallow_owners.remove(&owner);
            self.load_states.insert(owner, previous);
            targets.push(request.key.target);
        }
        self.previous_load_states.clear();
        targets
    }
}

pub fn owner_for_target(profile_id: Uuid, target: &CatalogTarget) -> ExplorerOwnerId {
    match target {
        CatalogTarget::Databases => ExplorerOwnerId::Profile(profile_id),
        CatalogTarget::Schemas { database } => ExplorerOwnerId::Catalog(database.clone()),
        CatalogTarget::Groups { schema } => ExplorerOwnerId::Catalog(schema.clone()),
        CatalogTarget::Objects { schema, group } => ExplorerOwnerId::Group {
            parent: schema.clone(),
            group: *group,
        },
        CatalogTarget::ProjectionChildren { table } => ExplorerOwnerId::Projections(table.clone()),
        CatalogTarget::RelationChildren { relation } => ExplorerOwnerId::Catalog(relation.clone()),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisibleExplorerNode {
    pub id: ExplorerNodeId,
    pub depth: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExplorerViewport {
    pub pinned: Vec<VisibleExplorerNode>,
    pub rows: Vec<VisibleExplorerNode>,
    pub hidden_ancestor_count: usize,
    pub show_ancestor_indicator: bool,
    pub body_height: usize,
    /// Total projected logical rows, independent of the viewport window.
    pub total_rows: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorerNodeTarget {
    First,
    Last,
    ViewTop,
    ViewMiddle,
    ViewBottom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorerScrollAmount {
    Lines(usize),
    HalfPage,
    Page,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorerNodeAlignment {
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ExplorerTreeError {
    #[error("Explorer profile {profile_id} does not exist")]
    ProfileNotFound { profile_id: Uuid },
    #[error(transparent)]
    Catalog(#[from] CatalogTreeError),
}

#[derive(Clone, Debug)]
pub struct ExplorerTreeState {
    pub groups: Vec<ConnectionGroup>,
    pub profile_order: Vec<Uuid>,
    pub profiles: HashMap<Uuid, ExplorerProfileState>,
    pub selected: Option<ExplorerNodeId>,
    pub expanded: HashSet<ExplorerNodeId>,
    pub scroll: usize,
    pub viewport_height: usize,
}

impl Default for ExplorerTreeState {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            profile_order: Vec::new(),
            profiles: HashMap::new(),
            selected: Some(ExplorerNodeId::EmptyProfiles),
            expanded: HashSet::new(),
            scroll: 0,
            viewport_height: 0,
        }
    }
}

fn search_group(kind: CatalogKind) -> Option<ObjectGroup> {
    match kind {
        CatalogKind::Table => Some(ObjectGroup::Tables),
        CatalogKind::View => Some(ObjectGroup::Views),
        CatalogKind::MaterializedView => Some(ObjectGroup::MaterializedViews),
        CatalogKind::Dictionary => Some(ObjectGroup::Dictionaries),
        CatalogKind::Sequence => Some(ObjectGroup::Sequences),
        CatalogKind::Function => Some(ObjectGroup::Functions),
        CatalogKind::Procedure => Some(ObjectGroup::Procedures),
        CatalogKind::Type => Some(ObjectGroup::Types),
        CatalogKind::Trigger => Some(ObjectGroup::Triggers),
        _ => None,
    }
}

fn append_filtered_search_rows(
    profile: &ExplorerProfileState,
    id: &CatalogId,
    included: &HashSet<ExplorerNodeId>,
    depth: usize,
    rows: &mut Vec<VisibleExplorerNode>,
) {
    let node = ExplorerNodeId::Catalog(id.clone());
    if !included.contains(&node) {
        return;
    }
    rows.push(VisibleExplorerNode { id: node, depth });
    let Some(entry) = profile.catalog.get(id) else {
        return;
    };
    if entry.kind == CatalogKind::Table && profile.kind == DatabaseKind::ClickHouse {
        let projection_node = ExplorerNodeId::ProjectionGroup(id.clone());
        if included.contains(&projection_node) {
            rows.push(VisibleExplorerNode {
                id: projection_node,
                depth: depth + 1,
            });
        }
    }
    if entry.kind.is_relation() {
        return;
    }
    if entry.kind == CatalogKind::Schema {
        for group in profile.catalog.groups(id) {
            let group_node = ExplorerNodeId::Group {
                parent: id.clone(),
                group: *group,
            };
            if !included.contains(&group_node) {
                continue;
            }
            rows.push(VisibleExplorerNode {
                id: group_node,
                depth: depth + 1,
            });
            for child in profile.catalog.group_children(id, *group) {
                append_filtered_search_rows(profile, child, included, depth + 2, rows);
            }
        }
    } else {
        for child in profile.catalog.children(id) {
            append_filtered_search_rows(profile, child, included, depth + 1, rows);
        }
    }
}

impl ExplorerTreeState {
    pub fn selected_primary_name(&self) -> Option<String> {
        let selected = self.selected.as_ref()?;
        match selected {
            ExplorerNodeId::Profile(profile_id) => self
                .profiles
                .get(profile_id)
                .map(|profile| profile.display_name.clone()),
            ExplorerNodeId::Catalog(id) => self
                .profiles
                .get(&id.profile_id())
                .and_then(|profile| profile.catalog.get(id))
                .map(|entry| entry.qualified_name.object.clone()),
            ExplorerNodeId::Group { group, .. } => Some(group_label(*group).to_owned()),
            ExplorerNodeId::PrincipalGroup { .. } => Some("Users & Roles".to_owned()),
            ExplorerNodeId::Mongo { node, .. } => Some(match node {
                crate::db::mongodb::types::MongoExplorerNodeId::Database(name) => name.clone(),
                crate::db::mongodb::types::MongoExplorerNodeId::Monitor(database) => {
                    format!("Monitor · {database}")
                }
                crate::db::mongodb::types::MongoExplorerNodeId::Users(database) => {
                    format!("Users & Roles · {database}")
                }
                crate::db::mongodb::types::MongoExplorerNodeId::Collection(namespace) => {
                    namespace.collection.clone().unwrap_or_default()
                }
                crate::db::mongodb::types::MongoExplorerNodeId::Index { name, .. } => name.clone(),
                crate::db::mongodb::types::MongoExplorerNodeId::RetryCollections(database) => {
                    format!("Retry collections for {database}")
                }
                crate::db::mongodb::types::MongoExplorerNodeId::RetryIndexes(namespace) => format!(
                    "Retry indexes for {}",
                    namespace.collection.as_deref().unwrap_or_default()
                ),
            }),
            ExplorerNodeId::Principal { entry } => self
                .profiles
                .get(&entry.profile_id)
                .and_then(|profile| {
                    profile
                        .principals
                        .iter()
                        .find(|principal| principal.id == *entry)
                })
                .map(|principal| principal.name.clone()),
            ExplorerNodeId::ProjectionGroup(_) => Some("Projections".to_owned()),
            ExplorerNodeId::EmptyProfiles
            | ExplorerNodeId::Others
            | ExplorerNodeId::Status { .. }
            | ExplorerNodeId::LoadMore { .. }
            | ExplorerNodeId::Empty { .. }
            | ExplorerNodeId::ConnectionGroup { .. }
            | ExplorerNodeId::PrincipalNotice { .. }
            | ExplorerNodeId::RedisDatabase { .. } => None,
        }
    }

    pub fn filtered_search_rows(
        &self,
        profile_id: Option<Uuid>,
        query: &str,
    ) -> (Vec<VisibleExplorerNode>, Vec<usize>) {
        let Some(profile_id) = profile_id else {
            return (Vec::new(), Vec::new());
        };
        let Some(profile) = self.profiles.get(&profile_id) else {
            return (Vec::new(), Vec::new());
        };
        let mut included = HashSet::new();
        let mut matches = HashSet::new();
        for entry in profile.catalog.entries().values() {
            if (entry.kind.is_relation_child() && entry.kind != CatalogKind::Projection)
                || entry
                    .relation_id
                    .as_ref()
                    .is_some_and(|relation| relation != &entry.id)
            {
                continue;
            }
            let path = entry
                .qualified_name
                .database
                .iter()
                .chain(entry.qualified_name.schema.iter())
                .chain(std::iter::once(&entry.qualified_name.object))
                .cloned()
                .collect::<Vec<_>>()
                .join(".");
            if !search_text_matches(&entry.qualified_name.object, query)
                && !search_text_matches(&path, query)
            {
                continue;
            }
            matches.insert(ExplorerNodeId::Catalog(entry.id.clone()));
            included.insert(ExplorerNodeId::Catalog(entry.id.clone()));
            if entry.kind == CatalogKind::Projection
                && let Some(table) = entry.parent_id.as_ref()
            {
                included.insert(ExplorerNodeId::ProjectionGroup(table.clone()));
            }
            let mut current = entry.id.clone();
            if entry.kind == CatalogKind::Projection
                && let Some(table) = entry.parent_id.as_ref()
            {
                included.insert(ExplorerNodeId::ProjectionGroup(table.clone()));
            }
            while let Some(parent) = profile.catalog.parent(&current).cloned() {
                included.insert(ExplorerNodeId::Catalog(parent.clone()));
                if parent.kind == CatalogKind::Schema
                    && let Some(group) = search_group(entry.kind)
                {
                    included.insert(ExplorerNodeId::Group {
                        parent: parent.clone(),
                        group,
                    });
                }
                current = parent;
            }
            included.insert(ExplorerNodeId::Profile(profile_id));
        }
        let mut rows = Vec::new();
        if included.contains(&ExplorerNodeId::Profile(profile_id)) {
            rows.push(VisibleExplorerNode {
                id: ExplorerNodeId::Profile(profile_id),
                depth: 0,
            });
            for root in profile.catalog.roots() {
                append_filtered_search_rows(profile, root, &included, 1, &mut rows);
            }
        }
        let match_rows = rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| matches.contains(&row.id).then_some(index))
            .collect();
        (rows, match_rows)
    }

    pub fn add_profile(&mut self, profile_id: Uuid) {
        self.add_profile_with_metadata(
            profile_id,
            String::new(),
            DatabaseKind::Sqlite,
            String::new(),
            ProfileProvenance::Saved,
        );
    }

    pub fn add_profile_with_metadata(
        &mut self,
        profile_id: Uuid,
        display_name: String,
        kind: DatabaseKind,
        endpoint: String,
        provenance: ProfileProvenance,
    ) {
        self.add_profile_with_placement(
            profile_id,
            display_name,
            kind,
            endpoint,
            provenance,
            ProfilePlacement::CurrentProject,
        );
    }

    pub fn add_profile_with_placement(
        &mut self,
        profile_id: Uuid,
        display_name: String,
        kind: DatabaseKind,
        endpoint: String,
        provenance: ProfileProvenance,
        placement: ProfilePlacement,
    ) {
        self.add_profile_with_group(
            profile_id,
            display_name,
            kind,
            endpoint,
            provenance,
            placement,
            None,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_profile_with_group(
        &mut self,
        profile_id: Uuid,
        display_name: String,
        kind: DatabaseKind,
        endpoint: String,
        provenance: ProfileProvenance,
        placement: ProfilePlacement,
        group_id: Option<Uuid>,
    ) {
        if let Some(profile) = self.profiles.get_mut(&profile_id) {
            profile.display_name = display_name;
            profile.kind = kind;
            profile.endpoint = endpoint;
            profile.provenance = provenance;
            profile.placement = placement;
            profile.group_id = group_id;
            return;
        }
        if self.profiles.contains_key(&profile_id) {
            return;
        }
        let mut state = ExplorerProfileState::new(
            profile_id,
            display_name,
            kind,
            endpoint,
            provenance,
            placement,
        );
        state.group_id = group_id;
        self.profiles.insert(profile_id, state);
        self.profile_order.push(profile_id);
        self.expanded.insert(ExplorerNodeId::Profile(profile_id));
        if matches!(self.selected, None | Some(ExplorerNodeId::EmptyProfiles)) {
            self.selected = Some(ExplorerNodeId::Profile(profile_id));
        }
    }

    pub fn add_unavailable_profile(
        &mut self,
        profile_id: Uuid,
        display_name: String,
        kind: String,
        reason: String,
        placement: ProfilePlacement,
    ) {
        let mut state = ExplorerProfileState::new(
            profile_id,
            display_name,
            DatabaseKind::Sqlite,
            kind,
            ProfileProvenance::Saved,
            placement,
        );
        state.unavailable_reason = Some(reason);
        state.status = ExplorerConnectionStatus::Failed;
        state.group_id = None;
        self.profiles.insert(profile_id, state);
        self.profile_order.push(profile_id);
        if matches!(self.selected, None | Some(ExplorerNodeId::EmptyProfiles)) {
            self.selected = Some(ExplorerNodeId::Profile(profile_id));
        }
    }

    pub fn sync_organization(
        &mut self,
        groups: Vec<ConnectionGroup>,
        profile_order: Vec<Uuid>,
        profile_groups: &HashMap<Uuid, Option<Uuid>>,
    ) {
        self.groups = groups;
        self.profile_order = profile_order;
        for profile in self.profiles.values_mut() {
            profile.group_id = profile_groups
                .get(&profile.catalog.profile_id())
                .copied()
                .flatten();
        }
        self.retain_existing_expansion();
        self.reconcile_after_catalog_change(self.selection_fallback_chain());
    }

    pub fn remove_profile(&mut self, profile_id: Uuid) -> Option<ExplorerProfileState> {
        let removed_index = self.profile_order.iter().position(|id| *id == profile_id);
        let removed = self.profiles.remove(&profile_id)?;
        self.profile_order.retain(|id| *id != profile_id);
        self.retain_existing_expansion();
        if self
            .selected
            .as_ref()
            .is_some_and(|selected| !self.node_exists(selected))
        {
            self.selected = removed_index
                .and_then(|index| {
                    self.profile_order
                        .get(index.min(self.profile_order.len().saturating_sub(1)))
                })
                .copied()
                .map(ExplorerNodeId::Profile)
                .or(Some(ExplorerNodeId::EmptyProfiles));
        }
        self.scroll = 0;
        Some(removed)
    }

    pub fn visible(&self) -> Vec<VisibleExplorerNode> {
        self.visible_with_visit_count().0
    }

    pub fn viewport(&self, height: usize) -> ExplorerViewport {
        let rows = self.visible();
        self.viewport_for_rows(&rows, height)
    }

    /// Build a viewport from rows the caller already projected.
    fn viewport_for_rows(&self, rows: &[VisibleExplorerNode], height: usize) -> ExplorerViewport {
        if rows.is_empty() || height == 0 {
            return ExplorerViewport {
                pinned: Vec::new(),
                rows: Vec::new(),
                hidden_ancestor_count: 0,
                show_ancestor_indicator: false,
                body_height: 0,
                total_rows: rows.len(),
            };
        }

        let ancestors_with_index = self
            .selected_ancestors()
            .into_iter()
            .filter_map(|id| {
                rows.iter()
                    .position(|row| row.id == id)
                    .filter(|index| *index < self.scroll)
                    .map(|index| (index, id))
            })
            .collect::<Vec<_>>();
        let show_ancestor_indicator =
            height >= 3 && ancestors_with_index.len() > height.saturating_sub(2);
        let pinned_capacity = height
            .saturating_sub(1)
            .saturating_sub(usize::from(show_ancestor_indicator));
        let hidden_ancestor_count = ancestors_with_index.len().saturating_sub(pinned_capacity);
        let pinned = ancestors_with_index
            .into_iter()
            .skip(hidden_ancestor_count)
            .filter_map(|(index, _)| rows.get(index).cloned())
            .collect::<Vec<_>>();
        let indicator_height = usize::from(show_ancestor_indicator);
        let body_height = height
            .saturating_sub(pinned.len())
            .saturating_sub(indicator_height)
            .max(1)
            .min(height);
        let body = rows
            .iter()
            .skip(self.scroll)
            .take(body_height)
            .cloned()
            .collect();

        ExplorerViewport {
            pinned,
            rows: body,
            hidden_ancestor_count,
            show_ancestor_indicator,
            body_height,
            total_rows: rows.len(),
        }
    }

    pub fn visible_profile(&self, profile_id: Uuid) -> Vec<VisibleExplorerNode> {
        let Some(profile) = self.profiles.get(&profile_id) else {
            return Vec::new();
        };
        let mut projection = Projection::new(self);
        for root in profile.catalog.roots() {
            projection.append_catalog(profile, root, 0);
        }
        projection.append_state_rows(
            profile,
            &ExplorerOwnerId::Profile(profile_id),
            0,
            profile.catalog.roots().len(),
        );
        if self.principal_group_is_visible(profile_id) {
            projection.push(ExplorerNodeId::PrincipalGroup { profile_id }, 0);
        }
        projection.rows
    }

    fn principal_group_is_visible(&self, profile_id: Uuid) -> bool {
        self.expanded.contains(&ExplorerNodeId::Profile(profile_id))
            && self.profiles.get(&profile_id).is_some_and(|profile| {
                !profile.kind.is_redis_compatible()
                    && matches!(
                        profile.status,
                        ExplorerConnectionStatus::Online | ExplorerConnectionStatus::Syncing
                    )
            })
    }

    #[doc(hidden)]
    pub fn visible_with_visit_count(&self) -> (Vec<VisibleExplorerNode>, usize) {
        #[cfg(test)]
        PROJECTION_CALLS.with(|calls| calls.set(calls.get().saturating_add(1)));
        let mut projection = Projection::new(self);
        self.append_region(&mut projection, ProfileRegion::Primary, 0);
        let other_profiles = self
            .profile_order
            .iter()
            .filter(|profile_id| {
                self.profiles
                    .get(profile_id)
                    .is_some_and(|profile| profile.placement == ProfilePlacement::OtherProject)
            })
            .copied()
            .collect::<Vec<_>>();
        if !other_profiles.is_empty() {
            let others = ExplorerNodeId::Others;
            projection.push(others.clone(), 0);
            if self.expanded.contains(&others) {
                self.append_region(&mut projection, ProfileRegion::Others, 1);
            }
        }
        if projection.rows.is_empty() {
            projection.push(ExplorerNodeId::EmptyProfiles, 0);
        }
        (projection.rows, projection.visited_catalog_entries)
    }

    fn append_region(&self, projection: &mut Projection<'_>, region: ProfileRegion, depth: usize) {
        let placements = match region {
            ProfileRegion::Primary => [ProfilePlacement::CurrentProject, ProfilePlacement::Global],
            ProfileRegion::Others => [
                ProfilePlacement::OtherProject,
                ProfilePlacement::OtherProject,
            ],
        };
        for group in &self.groups {
            let members = self
                .profile_order
                .iter()
                .copied()
                .filter(|id| {
                    self.profiles.get(id).is_some_and(|profile| {
                        profile.group_id == Some(group.id)
                            && placements.contains(&profile.placement)
                    })
                })
                .collect::<Vec<_>>();
            if members.is_empty() && region == ProfileRegion::Others {
                continue;
            }
            let node = ExplorerNodeId::ConnectionGroup {
                group_id: group.id,
                region,
            };
            projection.push(node.clone(), depth);
            if self.expanded.contains(&node) {
                for id in members {
                    self.append_profile(projection, id, depth + 1);
                }
            }
        }
        for id in &self.profile_order {
            if let Some(profile) = self.profiles.get(id)
                && profile.group_id.is_none()
                && placements.contains(&profile.placement)
            {
                self.append_profile(projection, *id, depth);
            }
        }
    }

    fn append_profile(&self, projection: &mut Projection<'_>, profile_id: Uuid, depth: usize) {
        let Some(profile) = self.profiles.get(&profile_id) else {
            return;
        };
        let profile_node = ExplorerNodeId::Profile(profile_id);
        projection.push(profile_node.clone(), depth);
        if !self.expanded.contains(&profile_node) {
            return;
        }
        let child_depth = depth + 1;
        if profile.kind.is_redis_compatible() {
            for database in &profile.redis_databases {
                projection.push(
                    ExplorerNodeId::RedisDatabase {
                        profile_id,
                        database: database.database,
                    },
                    child_depth,
                );
            }
            if profile.redis_databases.is_empty() && profile.redis_databases_error.is_some() {
                projection.push(
                    ExplorerNodeId::Status {
                        owner: ExplorerOwnerId::Profile(profile_id),
                        kind: StatusRowKind::Retry,
                    },
                    child_depth,
                );
            } else if profile.redis_databases.is_empty()
                && profile.status == ExplorerConnectionStatus::Syncing
            {
                projection.push(
                    ExplorerNodeId::Status {
                        owner: ExplorerOwnerId::Profile(profile_id),
                        kind: StatusRowKind::Loading,
                    },
                    child_depth,
                );
            }
            return;
        }
        if profile.kind == DatabaseKind::MongoDb {
            for database in &profile.mongodb_databases {
                let database_node = ExplorerNodeId::Mongo {
                    profile_id,
                    node: crate::db::mongodb::types::MongoExplorerNodeId::Database(
                        database.clone(),
                    ),
                };
                projection.push(database_node.clone(), child_depth);
                // Monitor and Users & Roles are reached from the database row
                // with `m` and `u`; listing them here buried the collections.
                if self.expanded.contains(&database_node) {
                    if let Some(collections) = profile.mongodb_collections.get(database) {
                        for collection in collections {
                            let collection_node = ExplorerNodeId::Mongo {
                                profile_id,
                                node: crate::db::mongodb::types::MongoExplorerNodeId::Collection(
                                    collection.namespace.clone(),
                                ),
                            };
                            projection.push(collection_node.clone(), child_depth + 1);
                            if self.expanded.contains(&collection_node) {
                                if let Some(indexes) =
                                    profile.mongodb_indexes.get(&collection.namespace)
                                {
                                    for index in indexes {
                                        projection.push(ExplorerNodeId::Mongo { profile_id, node: crate::db::mongodb::types::MongoExplorerNodeId::Index { namespace: collection.namespace.clone(), name: index.name.clone() } }, child_depth + 2);
                                    }
                                } else if profile
                                    .mongodb_index_errors
                                    .contains_key(&collection.namespace)
                                {
                                    projection.push(ExplorerNodeId::Mongo { profile_id, node: crate::db::mongodb::types::MongoExplorerNodeId::RetryIndexes(collection.namespace.clone()) }, child_depth + 2);
                                } else {
                                    projection.push(
                                        ExplorerNodeId::Status {
                                            owner: ExplorerOwnerId::Profile(profile_id),
                                            kind: StatusRowKind::Loading,
                                        },
                                        child_depth + 2,
                                    );
                                }
                            }
                        }
                    } else if profile.mongodb_collection_errors.contains_key(database) {
                        projection.push(ExplorerNodeId::Mongo { profile_id, node: crate::db::mongodb::types::MongoExplorerNodeId::RetryCollections(database.clone()) }, child_depth + 1);
                    } else {
                        projection.push(
                            ExplorerNodeId::Status {
                                owner: ExplorerOwnerId::Profile(profile_id),
                                kind: if profile.mongodb_error.is_some() {
                                    StatusRowKind::Retry
                                } else {
                                    StatusRowKind::Loading
                                },
                            },
                            child_depth + 1,
                        );
                    }
                }
            }
            return;
        }
        let roots = profile.catalog.roots();
        for root in roots {
            projection.append_catalog(profile, root, child_depth);
        }
        projection.append_state_rows(
            profile,
            &ExplorerOwnerId::Profile(profile_id),
            child_depth,
            roots.len(),
        );
        if !self.principal_group_is_visible(profile_id) {
            return;
        }
        let principal_group = ExplorerNodeId::PrincipalGroup { profile_id };
        projection.push(principal_group.clone(), child_depth);
        if self.expanded.contains(&principal_group) {
            if profile.principals_error.is_some()
                || profile.principals_unsupported.is_some()
                || (profile.principals_loaded
                    && (!profile.principals_complete || profile.principals.is_empty()))
            {
                projection.push(
                    ExplorerNodeId::PrincipalNotice { profile_id },
                    child_depth + 1,
                );
            }
            for principal in &profile.principals {
                projection.push(
                    ExplorerNodeId::Principal {
                        entry: principal.id.clone(),
                    },
                    child_depth + 1,
                );
            }
        }
    }

    pub fn select(&mut self, id: ExplorerNodeId) -> bool {
        if !self.node_exists(&id) {
            return false;
        }
        self.selected = Some(id);
        true
    }

    pub fn reveal_node(&mut self, id: ExplorerNodeId) -> bool {
        if !self.node_exists(&id) {
            return false;
        }
        let mut parent = self.visible_parent(&id);
        while let Some(node) = parent {
            self.expanded.insert(node.clone());
            parent = self.visible_parent(&node);
        }
        self.selected = Some(id);
        true
    }

    pub fn expand(&mut self) -> bool {
        let Some(selected) = self.selected.clone() else {
            return false;
        };
        let expandable = match &selected {
            ExplorerNodeId::Profile(profile_id) => self.profiles.contains_key(profile_id),
            ExplorerNodeId::Catalog(id) => self
                .profiles
                .get(&id.profile_id())
                .and_then(|profile| profile.catalog.get(id))
                .is_some_and(|entry| entry.expandable),
            ExplorerNodeId::ProjectionGroup(table) => self
                .profiles
                .get(&table.profile_id())
                .is_some_and(|profile| {
                    profile.kind == DatabaseKind::ClickHouse
                        && profile.catalog.is_projection_table(table)
                }),
            ExplorerNodeId::Group { parent, group } => self
                .profiles
                .get(&parent.profile_id())
                .is_some_and(|profile| profile.catalog.group_state(parent, *group).is_some()),
            ExplorerNodeId::ConnectionGroup { group_id, region } => {
                self.groups.iter().any(|group| {
                    group.id == *group_id && self.group_exists_in_region(*group_id, *region)
                })
            }
            ExplorerNodeId::Others => true,
            ExplorerNodeId::PrincipalGroup { profile_id } => self.profiles.contains_key(profile_id),
            ExplorerNodeId::Mongo { profile_id, node } => {
                self.mongo_node_is_expandable(*profile_id, node)
            }
            ExplorerNodeId::EmptyProfiles
            | ExplorerNodeId::Status { .. }
            | ExplorerNodeId::LoadMore { .. }
            | ExplorerNodeId::Empty { .. }
            | ExplorerNodeId::Principal { .. }
            | ExplorerNodeId::PrincipalNotice { .. }
            | ExplorerNodeId::RedisDatabase { .. } => false,
        };
        expandable && self.expanded.insert(selected)
    }

    /// Whether a MongoDB node can be expanded into children.
    ///
    /// Only databases and loaded collections have children: databases hold
    /// collections and collections hold indexes. Action rows (monitor, users,
    /// retry) and index rows are leaves. The Explorer projection uses this to
    /// decide whether a row advertises an expand affordance, so it has to agree
    /// with [`Self::expand`].
    pub fn mongo_node_is_expandable(
        &self,
        profile_id: Uuid,
        node: &crate::db::mongodb::types::MongoExplorerNodeId,
    ) -> bool {
        use crate::db::mongodb::types::MongoExplorerNodeId;
        let Some(profile) = self.profiles.get(&profile_id) else {
            return false;
        };
        if profile.kind != DatabaseKind::MongoDb {
            return false;
        }
        match node {
            MongoExplorerNodeId::Database(_) => true,
            MongoExplorerNodeId::Collection(namespace) => profile
                .mongodb_collections
                .get(&namespace.database)
                .is_some_and(|collections| {
                    collections
                        .iter()
                        .any(|collection| collection.namespace == *namespace)
                }),
            MongoExplorerNodeId::Index { .. }
            | MongoExplorerNodeId::Monitor(_)
            | MongoExplorerNodeId::Users(_)
            | MongoExplorerNodeId::RetryCollections(_)
            | MongoExplorerNodeId::RetryIndexes(_) => false,
        }
    }

    fn group_exists_in_region(&self, group_id: Uuid, region: ProfileRegion) -> bool {
        self.profile_order.iter().any(|profile_id| {
            self.profiles.get(profile_id).is_some_and(|profile| {
                profile.group_id == Some(group_id)
                    && match region {
                        ProfileRegion::Primary => matches!(
                            profile.placement,
                            ProfilePlacement::CurrentProject | ProfilePlacement::Global
                        ),
                        ProfileRegion::Others => {
                            profile.placement == ProfilePlacement::OtherProject
                        }
                    }
            })
        }) || (region == ProfileRegion::Primary
            && self.groups.iter().any(|group| group.id == group_id))
    }

    pub fn collapse(&mut self) -> bool {
        self.selected
            .as_ref()
            .is_some_and(|selected| self.expanded.remove(selected))
    }

    pub fn move_to_parent(&mut self) -> bool {
        let Some(selected) = self.selected.as_ref() else {
            return false;
        };
        let Some(parent) = self.visible_parent(selected) else {
            return false;
        };
        self.selected = Some(parent);
        self.ensure_selected_visible();
        true
    }

    pub fn set_viewport_height(&mut self, viewport_height: usize) {
        self.viewport_height = viewport_height;
        self.ensure_selected_visible();
    }

    pub fn move_selection(&mut self, delta: isize, viewport_height: usize) {
        let _ = self.move_selection_index(delta, viewport_height);
    }

    /// Move the selection and return the new row index so callers can sync a cached
    /// index without projecting the tree again.
    #[doc(hidden)]
    pub fn move_selection_index(&mut self, delta: isize, viewport_height: usize) -> Option<usize> {
        self.viewport_height = viewport_height;
        let rows = self.visible();
        if rows.is_empty() {
            self.selected = None;
            self.scroll = 0;
            return None;
        }
        let current = self
            .selected
            .as_ref()
            .and_then(|selected| rows.iter().position(|row| &row.id == selected))
            .unwrap_or(0);
        let selected_index = current
            .saturating_add_signed(delta)
            .min(rows.len().saturating_sub(1));
        self.selected = Some(rows[selected_index].id.clone());
        self.update_scroll(&rows, selected_index);
        Some(selected_index)
    }

    pub fn selected_visible_index(&self) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        self.visible().iter().position(|row| &row.id == selected)
    }

    pub fn select_target(&mut self, target: ExplorerNodeTarget) {
        let rows = self.visible();
        if rows.is_empty() || self.viewport_height == 0 {
            return;
        }
        let ancestor_indexes = self.ancestor_row_indexes(&rows);
        let body_height = self.body_height_for_scroll(rows.len(), &ancestor_indexes, self.scroll);
        let first = self.scroll.min(rows.len() - 1);
        let last = first.saturating_add(body_height - 1).min(rows.len() - 1);
        let selected_index = match target {
            ExplorerNodeTarget::First => 0,
            ExplorerNodeTarget::Last => rows.len() - 1,
            ExplorerNodeTarget::ViewTop => first,
            ExplorerNodeTarget::ViewMiddle => first + (last - first) / 2,
            ExplorerNodeTarget::ViewBottom => last,
        };
        self.selected = Some(rows[selected_index].id.clone());
        self.update_scroll(&rows, selected_index);
    }

    pub fn scroll_nodes(&mut self, direction: isize, amount: ExplorerScrollAmount) {
        let rows = self.visible();
        if rows.is_empty() || self.viewport_height == 0 || direction == 0 {
            return;
        }
        let ancestor_indexes = self.ancestor_row_indexes(&rows);
        let step = match amount {
            ExplorerScrollAmount::Lines(lines) => {
                self.scroll_lines(&rows, direction, lines);
                return;
            }
            ExplorerScrollAmount::HalfPage => {
                (self.body_height_for_scroll(rows.len(), &ancestor_indexes, self.scroll) / 2).max(1)
            }
            ExplorerScrollAmount::Page => {
                self.body_height_for_scroll(rows.len(), &ancestor_indexes, self.scroll)
            }
        };
        let delta = if direction.is_negative() {
            -(step.min(isize::MAX as usize) as isize)
        } else {
            step.min(isize::MAX as usize) as isize
        };
        let current = self
            .selected
            .as_ref()
            .and_then(|selected| rows.iter().position(|row| &row.id == selected))
            .unwrap_or(0);
        let selected_index = current.saturating_add_signed(delta).min(rows.len() - 1);
        self.selected = Some(rows[selected_index].id.clone());
        self.scroll = self.scroll.saturating_add_signed(delta);
        self.update_scroll(&rows, selected_index);
    }

    pub fn set_scroll_offset(&mut self, offset: usize) {
        let rows = self.visible();
        if rows.is_empty() || self.viewport_height == 0 {
            self.scroll = 0;
            return;
        }
        self.scroll = offset.min(rows.len().saturating_sub(1));
        let viewport = self.viewport_for_rows(&rows, self.viewport_height);
        let pinned = viewport
            .pinned
            .iter()
            .map(|row| &row.id)
            .collect::<HashSet<_>>();
        let Some(selected_index) = self
            .selected
            .as_ref()
            .and_then(|selected| rows.iter().position(|row| &row.id == selected))
        else {
            return;
        };
        if pinned.contains(&rows[selected_index].id) {
            return;
        }
        let body_height = viewport.body_height.max(1);
        if selected_index < self.scroll || selected_index >= self.scroll.saturating_add(body_height)
        {
            let visible_index = selected_index
                .clamp(self.scroll, self.scroll.saturating_add(body_height - 1))
                .min(rows.len() - 1);
            self.selected = Some(rows[visible_index].id.clone());
        }
    }

    fn scroll_lines(&mut self, rows: &[VisibleExplorerNode], direction: isize, lines: usize) {
        let Some(selected_index) = self
            .selected
            .as_ref()
            .and_then(|selected| rows.iter().position(|row| &row.id == selected))
        else {
            return;
        };
        let delta = if direction.is_negative() {
            -(lines.min(isize::MAX as usize) as isize)
        } else {
            lines.min(isize::MAX as usize) as isize
        };
        let requested = self.scroll.saturating_add_signed(delta);

        for _ in 0..=self.selected_ancestors().len().saturating_add(1) {
            let ancestor_indexes = self.ancestor_row_indexes(rows);
            let body_height = self
                .body_height_for_scroll(rows.len(), &ancestor_indexes, requested)
                .min(rows.len());
            let scroll = requested.min(rows.len().saturating_sub(body_height));
            let last = scroll
                .saturating_add(body_height.saturating_sub(1))
                .min(rows.len() - 1);
            let clamped = selected_index.clamp(scroll, last);
            let stable = self.scroll == scroll && self.selected.as_ref() == Some(&rows[clamped].id);
            self.scroll = scroll;
            self.selected = Some(rows[clamped].id.clone());
            if stable {
                break;
            }
        }
    }

    pub fn align_selected(&mut self, alignment: ExplorerNodeAlignment) {
        let rows = self.visible();
        let Some(selected_index) = self
            .selected
            .as_ref()
            .and_then(|selected| rows.iter().position(|row| &row.id == selected))
        else {
            return;
        };
        if self.viewport_height == 0 {
            return;
        }
        if alignment == ExplorerNodeAlignment::Middle {
            self.align_selected_middle(&rows, selected_index);
            return;
        }
        let ancestor_indexes = self.ancestor_row_indexes(&rows);
        let body_height = self.body_height_for_scroll(rows.len(), &ancestor_indexes, self.scroll);
        let screen_row = match alignment {
            ExplorerNodeAlignment::Top => 0,
            ExplorerNodeAlignment::Middle => (body_height - 1) / 2,
            ExplorerNodeAlignment::Bottom => body_height - 1,
        };
        self.scroll = selected_index.saturating_sub(screen_row).min(
            rows.len()
                .saturating_sub(self.viewport_height.min(rows.len())),
        );
    }

    fn align_selected_middle(&mut self, rows: &[VisibleExplorerNode], selected_index: usize) {
        let max_scroll = rows.len().saturating_sub(1);
        let ancestor_indexes = self.ancestor_row_indexes(rows);
        let mut candidates = HashSet::from([
            0,
            max_scroll,
            selected_index.saturating_sub(self.viewport_height.saturating_sub(1) / 2),
        ]);
        for ancestor in self.selected_ancestors() {
            let Some(index) = rows.iter().position(|row| row.id == ancestor) else {
                continue;
            };
            for scroll in [index.saturating_sub(1), index, index.saturating_add(1)] {
                let body_height =
                    self.body_height_for_scroll(rows.len(), &ancestor_indexes, scroll);
                candidates
                    .insert(selected_index.saturating_sub((body_height.saturating_sub(1)) / 2));
            }
        }
        let mut best = None;
        let ideal = selected_index.saturating_sub(self.viewport_height.saturating_sub(1) / 2);
        for scroll in candidates {
            let scroll = scroll.min(max_scroll);
            let body_height = self.body_height_for_scroll(rows.len(), &ancestor_indexes, scroll);
            if selected_index < scroll || selected_index >= scroll.saturating_add(body_height) {
                continue;
            }
            let screen_row = selected_index - scroll;
            let distance = screen_row.abs_diff((body_height - 1) / 2);
            let candidate = (distance, scroll.abs_diff(ideal), scroll);
            if best.is_none_or(|current| candidate < current) {
                best = Some(candidate);
            }
        }
        if let Some((_, _, scroll)) = best {
            self.scroll = scroll;
        }
    }

    pub fn ensure_selected_visible(&mut self) {
        let rows = self.visible();
        let Some(selected) = self.selected.as_ref() else {
            self.scroll = 0;
            return;
        };
        let Some(selected_index) = rows.iter().position(|row| &row.id == selected) else {
            return;
        };
        self.update_scroll(&rows, selected_index);
    }

    pub fn replace_page(
        &mut self,
        owner: ExplorerOwnerId,
        entries: Vec<CatalogEntry>,
    ) -> Result<Vec<CatalogId>, ExplorerTreeError> {
        let fallback = self.selection_fallback_chain();
        let profile_id = owner.profile_id();
        let Some(profile) = self.profiles.get_mut(&profile_id) else {
            return Err(ExplorerTreeError::ProfileNotFound { profile_id });
        };
        let new_namespace_ids = entries
            .iter()
            .filter(|entry| matches!(entry.kind, CatalogKind::Database | CatalogKind::Schema))
            .filter(|entry| !profile.catalog.entries().contains_key(&entry.id))
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();
        let removed = profile.catalog.replace_page(&owner, entries)?;
        for id in new_namespace_ids {
            self.expanded.insert(ExplorerNodeId::Catalog(id));
        }
        self.reconcile_after_catalog_change(fallback);
        Ok(removed)
    }

    pub fn reconcile_catalog_page(
        &mut self,
        owner: ExplorerOwnerId,
        entries: Vec<CatalogEntry>,
    ) -> Result<Vec<CatalogId>, ExplorerTreeError> {
        let fallback = self.selection_fallback_chain();
        let profile_id = owner.profile_id();
        let Some(profile) = self.profiles.get_mut(&profile_id) else {
            return Err(ExplorerTreeError::ProfileNotFound { profile_id });
        };
        let new_namespace_ids = entries
            .iter()
            .filter(|entry| matches!(entry.kind, CatalogKind::Database | CatalogKind::Schema))
            .filter(|entry| !profile.catalog.entries().contains_key(&entry.id))
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();
        let removed = profile.catalog.reconcile_page(&owner, entries)?;
        for id in new_namespace_ids {
            self.expanded.insert(ExplorerNodeId::Catalog(id));
        }
        self.reconcile_after_catalog_change(fallback);
        Ok(removed)
    }

    pub fn reconcile_catalog_groups(
        &mut self,
        schema: &CatalogId,
        states: Vec<(ObjectGroup, CatalogGroupState)>,
    ) -> Result<Vec<CatalogId>, ExplorerTreeError> {
        let fallback = self.selection_fallback_chain();
        let Some(profile) = self.profiles.get_mut(&schema.profile_id()) else {
            return Err(ExplorerTreeError::ProfileNotFound {
                profile_id: schema.profile_id(),
            });
        };
        let removed = profile.catalog.reconcile_group_states(schema, states)?;
        self.reconcile_after_catalog_change(fallback);
        Ok(removed)
    }

    pub fn replace_subtree(
        &mut self,
        parent: &CatalogId,
        entries: Vec<CatalogEntry>,
    ) -> Result<Vec<CatalogId>, ExplorerTreeError> {
        self.replace_page(ExplorerOwnerId::Catalog(parent.clone()), entries)
    }

    pub fn remove_subtree(
        &mut self,
        root: &CatalogId,
    ) -> Result<Vec<CatalogId>, ExplorerTreeError> {
        let fallback = self.selection_fallback_chain();
        let profile_id = root.profile_id();
        let Some(profile) = self.profiles.get_mut(&profile_id) else {
            return Err(ExplorerTreeError::ProfileNotFound { profile_id });
        };
        let removed = profile.catalog.remove_subtree(root)?;
        self.reconcile_after_catalog_change(fallback);
        Ok(removed)
    }

    /// Remove a dropped object and prefer its former parent for selection.
    pub fn remove_dropped_subtree(
        &mut self,
        root: &CatalogId,
    ) -> Result<Vec<CatalogId>, ExplorerTreeError> {
        let parent = self
            .profiles
            .get(&root.profile_id())
            .and_then(|profile| profile.catalog.parent(root).cloned());
        let removed = self.remove_subtree(root)?;
        if let Some(parent) = parent {
            let _ = self.select(ExplorerNodeId::Catalog(parent));
        }
        self.ensure_selected_visible();
        Ok(removed)
    }

    fn update_scroll(&mut self, rows: &[VisibleExplorerNode], selected_index: usize) {
        if self.viewport_height == 0 {
            self.scroll = selected_index;
            return;
        }
        let ancestors = self.selected_ancestors();
        let ancestor_indexes = ancestor_row_indexes(rows, &ancestors);
        for _ in 0..=ancestors.len() {
            let body_height =
                self.body_height_for_scroll(rows.len(), &ancestor_indexes, self.scroll);
            if selected_index < self.scroll {
                self.scroll = selected_index;
            } else if selected_index >= self.scroll.saturating_add(body_height) {
                self.scroll = selected_index + 1 - body_height;
            }
            self.scroll = self.scroll.min(rows.len().saturating_sub(body_height));
        }
    }

    fn ancestor_row_indexes(&self, rows: &[VisibleExplorerNode]) -> Vec<usize> {
        ancestor_row_indexes(rows, &self.selected_ancestors())
    }

    fn body_height_for_scroll(
        &self,
        rows_len: usize,
        ancestor_indexes: &[usize],
        scroll: usize,
    ) -> usize {
        if self.viewport_height == 0 || rows_len == 0 {
            return 0;
        }
        let pinned = ancestor_indexes
            .iter()
            .filter(|index| **index < scroll)
            .count();
        self.viewport_height.saturating_sub(pinned).max(1)
    }

    fn visible_parent(&self, node: &ExplorerNodeId) -> Option<ExplorerNodeId> {
        match node {
            ExplorerNodeId::EmptyProfiles | ExplorerNodeId::Others => None,
            ExplorerNodeId::Profile(profile_id) => {
                let profile = self.profiles.get(profile_id)?;
                (profile.placement == ProfilePlacement::OtherProject)
                    .then_some(ExplorerNodeId::Others)
            }
            ExplorerNodeId::Catalog(id) => {
                let profile = self.profiles.get(&id.profile_id())?;
                let entry = profile.catalog.get(id)?;
                let Some(parent) = entry.parent_id.as_ref() else {
                    return Some(ExplorerNodeId::Profile(id.profile_id()));
                };
                if entry.kind == CatalogKind::Projection {
                    return Some(ExplorerNodeId::ProjectionGroup(parent.clone()));
                }
                if parent.kind == CatalogKind::Schema
                    && let Some(group) = object_group(entry.kind)
                    && profile.catalog.group_state(parent, group).is_some()
                {
                    return Some(ExplorerNodeId::Group {
                        parent: parent.clone(),
                        group,
                    });
                }
                Some(ExplorerNodeId::Catalog(parent.clone()))
            }
            ExplorerNodeId::ProjectionGroup(table) => Some(ExplorerNodeId::Catalog(table.clone())),
            ExplorerNodeId::Group { parent, .. } => Some(ExplorerNodeId::Catalog(parent.clone())),
            ExplorerNodeId::RedisDatabase { profile_id, .. } => {
                Some(ExplorerNodeId::Profile(*profile_id))
            }
            ExplorerNodeId::Mongo { profile_id, node } => match node {
                crate::db::mongodb::types::MongoExplorerNodeId::Database(_) => {
                    Some(ExplorerNodeId::Profile(*profile_id))
                }
                crate::db::mongodb::types::MongoExplorerNodeId::Monitor(database)
                | crate::db::mongodb::types::MongoExplorerNodeId::Users(database) => {
                    Some(ExplorerNodeId::Mongo {
                        profile_id: *profile_id,
                        node: crate::db::mongodb::types::MongoExplorerNodeId::Database(
                            database.clone(),
                        ),
                    })
                }
                crate::db::mongodb::types::MongoExplorerNodeId::Collection(namespace) => {
                    Some(ExplorerNodeId::Mongo {
                        profile_id: *profile_id,
                        node: crate::db::mongodb::types::MongoExplorerNodeId::Database(
                            namespace.database.clone(),
                        ),
                    })
                }
                crate::db::mongodb::types::MongoExplorerNodeId::Index { namespace, .. } => {
                    Some(ExplorerNodeId::Mongo {
                        profile_id: *profile_id,
                        node: crate::db::mongodb::types::MongoExplorerNodeId::Collection(
                            namespace.clone(),
                        ),
                    })
                }
                crate::db::mongodb::types::MongoExplorerNodeId::RetryCollections(database) => {
                    Some(ExplorerNodeId::Mongo {
                        profile_id: *profile_id,
                        node: crate::db::mongodb::types::MongoExplorerNodeId::Database(
                            database.clone(),
                        ),
                    })
                }
                crate::db::mongodb::types::MongoExplorerNodeId::RetryIndexes(namespace) => {
                    Some(ExplorerNodeId::Mongo {
                        profile_id: *profile_id,
                        node: crate::db::mongodb::types::MongoExplorerNodeId::Collection(
                            namespace.clone(),
                        ),
                    })
                }
            },
            ExplorerNodeId::PrincipalGroup { profile_id } => {
                Some(ExplorerNodeId::Profile(*profile_id))
            }
            ExplorerNodeId::Principal { entry } => Some(ExplorerNodeId::PrincipalGroup {
                profile_id: entry.profile_id,
            }),
            ExplorerNodeId::PrincipalNotice { profile_id } => {
                Some(ExplorerNodeId::PrincipalGroup {
                    profile_id: *profile_id,
                })
            }
            ExplorerNodeId::ConnectionGroup { region, .. } => Some(match region {
                ProfileRegion::Primary => ExplorerNodeId::EmptyProfiles,
                ProfileRegion::Others => ExplorerNodeId::Others,
            }),
            ExplorerNodeId::Status { owner, .. } | ExplorerNodeId::Empty { owner } => {
                Some(owner.node_id())
            }
            ExplorerNodeId::LoadMore { parent, .. } => Some(parent.node_id()),
        }
    }

    fn selected_ancestors(&self) -> Vec<ExplorerNodeId> {
        let mut ancestors = Vec::new();
        let mut current = self
            .selected
            .as_ref()
            .and_then(|id| self.visible_parent(id));
        while let Some(id) = current {
            current = self.visible_parent(&id);
            ancestors.push(id);
        }
        ancestors.reverse();
        ancestors
    }

    fn node_exists(&self, node: &ExplorerNodeId) -> bool {
        match node {
            ExplorerNodeId::EmptyProfiles => self
                .profile_order
                .iter()
                .all(|profile_id| !self.profiles.contains_key(profile_id)),
            ExplorerNodeId::Profile(profile_id) => {
                self.profile_order.contains(profile_id) && self.profiles.contains_key(profile_id)
            }
            ExplorerNodeId::Others => self.profile_order.iter().any(|profile_id| {
                self.profiles
                    .get(profile_id)
                    .is_some_and(|profile| profile.placement == ProfilePlacement::OtherProject)
            }),
            ExplorerNodeId::Catalog(id) => self
                .profiles
                .get(&id.profile_id())
                .is_some_and(|profile| profile.catalog.get(id).is_some()),
            ExplorerNodeId::ProjectionGroup(table) => self
                .profiles
                .get(&table.profile_id())
                .is_some_and(|profile| {
                    profile.kind == DatabaseKind::ClickHouse
                        && profile.catalog.is_projection_table(table)
                }),
            ExplorerNodeId::Group { parent, group } => self
                .profiles
                .get(&parent.profile_id())
                .is_some_and(|profile| profile.catalog.group_state(parent, *group).is_some()),
            ExplorerNodeId::RedisDatabase {
                profile_id,
                database,
            } => self.profiles.get(profile_id).is_some_and(|profile| {
                profile.kind.is_redis_compatible()
                    && profile
                        .redis_databases
                        .iter()
                        .any(|item| item.database == *database)
            }),
            ExplorerNodeId::PrincipalGroup { profile_id } => self.profiles.contains_key(profile_id),
            ExplorerNodeId::Principal { entry } => {
                self.profiles.get(&entry.profile_id).is_some_and(|profile| {
                    profile
                        .principals
                        .iter()
                        .any(|principal| principal.id == *entry)
                })
            }
            ExplorerNodeId::PrincipalNotice { profile_id } => {
                self.profiles.contains_key(profile_id)
            }
            ExplorerNodeId::Mongo { profile_id, node } => self
                .profiles
                .get(profile_id)
                .is_some_and(|profile| match node {
                    crate::db::mongodb::types::MongoExplorerNodeId::Database(database) => {
                        profile.mongodb_databases.contains(database)
                    }
                    crate::db::mongodb::types::MongoExplorerNodeId::Collection(namespace) => {
                        profile
                            .mongodb_collections
                            .get(&namespace.database)
                            .is_some_and(|collections| {
                                collections
                                    .iter()
                                    .any(|collection| collection.namespace == *namespace)
                            })
                    }
                    crate::db::mongodb::types::MongoExplorerNodeId::Index { namespace, name } => {
                        profile
                            .mongodb_indexes
                            .get(namespace)
                            .is_some_and(|indexes| indexes.iter().any(|index| index.name == *name))
                    }
                    crate::db::mongodb::types::MongoExplorerNodeId::RetryCollections(database) => {
                        profile.mongodb_collection_errors.contains_key(database)
                    }
                    crate::db::mongodb::types::MongoExplorerNodeId::RetryIndexes(namespace) => {
                        profile.mongodb_index_errors.contains_key(namespace)
                    }
                    crate::db::mongodb::types::MongoExplorerNodeId::Monitor(database)
                    | crate::db::mongodb::types::MongoExplorerNodeId::Users(database) => {
                        profile.mongodb_databases.contains(database)
                    }
                }),
            ExplorerNodeId::ConnectionGroup { group_id, region } => {
                self.groups.iter().any(|group| {
                    group.id == *group_id && self.group_exists_in_region(*group_id, *region)
                })
            }
            ExplorerNodeId::Status { owner, kind } => self
                .load_state(owner)
                .is_some_and(|state| status_kind(state) == Some(*kind)),
            ExplorerNodeId::LoadMore { parent, cursor } => {
                self.load_state(parent).and_then(load_state_cursor) == Some(cursor)
            }
            ExplorerNodeId::Empty { owner } => self.load_state(owner).is_some_and(|state| {
                (matches!(state, ExplorerLoadState::Loaded { next_cursor: None })
                    || matches!(
                        state,
                        ExplorerLoadState::Loading { .. }
                            if self.profiles.get(&owner.profile_id()).is_some_and(|profile| {
                                matches!(
                                    profile.previous_load_states.get(owner),
                                    Some(
                                        ExplorerLoadState::Loaded { next_cursor: None }
                                            | ExplorerLoadState::Stale { next_cursor: None }
                                    )
                                )
                            })
                    ))
                    && self.owner_child_count(owner) == Some(0)
            }),
        }
    }

    fn load_state(&self, owner: &ExplorerOwnerId) -> Option<&ExplorerLoadState> {
        let profile = self.profiles.get(&owner.profile_id())?;
        if !self.owner_exists(owner) {
            return None;
        }
        profile.load_states.get(owner)
    }

    fn owner_exists(&self, owner: &ExplorerOwnerId) -> bool {
        match owner {
            ExplorerOwnerId::Profile(profile_id) => self.profiles.contains_key(profile_id),
            ExplorerOwnerId::Catalog(id) => self
                .profiles
                .get(&id.profile_id())
                .is_some_and(|profile| profile.catalog.get(id).is_some()),
            ExplorerOwnerId::Projections(id) => {
                self.profiles.get(&id.profile_id()).is_some_and(|profile| {
                    profile.catalog.get(id).is_some() && id.kind == CatalogKind::Table
                })
            }
            ExplorerOwnerId::Group { parent, group } => self
                .profiles
                .get(&parent.profile_id())
                .is_some_and(|profile| profile.catalog.group_state(parent, *group).is_some()),
        }
    }

    fn owner_child_count(&self, owner: &ExplorerOwnerId) -> Option<usize> {
        let profile = self.profiles.get(&owner.profile_id())?;
        match owner {
            ExplorerOwnerId::Profile(_) => Some(profile.catalog.roots().len()),
            ExplorerOwnerId::Catalog(id) => {
                let entry = profile.catalog.get(id)?;
                if entry.kind == CatalogKind::Schema {
                    Some(profile.catalog.groups(id).len())
                } else {
                    Some(profile.catalog.children(id).len())
                }
            }
            ExplorerOwnerId::Projections(id) => Some(
                profile
                    .catalog
                    .children(id)
                    .iter()
                    .filter(|child| {
                        profile
                            .catalog
                            .get(child)
                            .is_some_and(|entry| entry.kind == CatalogKind::Projection)
                    })
                    .count(),
            ),
            ExplorerOwnerId::Group { parent, group } => {
                Some(profile.catalog.group_children(parent, *group).len())
            }
        }
    }

    pub(crate) fn selection_fallback_chain(&self) -> Vec<ExplorerNodeId> {
        let Some(selected) = self.selected.as_ref() else {
            return Vec::new();
        };
        let mut chain = vec![selected.clone()];
        match selected {
            ExplorerNodeId::Catalog(id) => self.append_catalog_ancestors(id, &mut chain),
            ExplorerNodeId::ProjectionGroup(table) => {
                self.append_catalog_ancestors(table, &mut chain)
            }
            ExplorerNodeId::Group { parent, .. } => {
                self.append_catalog_ancestors(parent, &mut chain)
            }
            ExplorerNodeId::Status { owner, .. }
            | ExplorerNodeId::LoadMore { parent: owner, .. }
            | ExplorerNodeId::Empty { owner } => match owner {
                ExplorerOwnerId::Profile(profile_id) => {
                    chain.push(ExplorerNodeId::Profile(*profile_id));
                }
                ExplorerOwnerId::Catalog(id) => self.append_catalog_ancestors(id, &mut chain),
                ExplorerOwnerId::Projections(id) => {
                    chain.push(ExplorerNodeId::ProjectionGroup(id.clone()));
                    self.append_catalog_ancestors(id, &mut chain)
                }
                ExplorerOwnerId::Group { parent, group } => {
                    chain.push(ExplorerNodeId::Group {
                        parent: parent.clone(),
                        group: *group,
                    });
                    self.append_catalog_ancestors(parent, &mut chain)
                }
            },
            ExplorerNodeId::EmptyProfiles
            | ExplorerNodeId::Others
            | ExplorerNodeId::ConnectionGroup { .. }
            | ExplorerNodeId::RedisDatabase { .. } => {}
            ExplorerNodeId::Mongo { profile_id, node } => {
                let parent = match node {
                    crate::db::mongodb::types::MongoExplorerNodeId::Database(_) => {
                        ExplorerNodeId::Profile(*profile_id)
                    }
                    crate::db::mongodb::types::MongoExplorerNodeId::Collection(namespace) => {
                        ExplorerNodeId::Mongo {
                            profile_id: *profile_id,
                            node: crate::db::mongodb::types::MongoExplorerNodeId::Database(
                                namespace.database.clone(),
                            ),
                        }
                    }
                    crate::db::mongodb::types::MongoExplorerNodeId::Index { namespace, .. } => {
                        ExplorerNodeId::Mongo {
                            profile_id: *profile_id,
                            node: crate::db::mongodb::types::MongoExplorerNodeId::Collection(
                                namespace.clone(),
                            ),
                        }
                    }
                    crate::db::mongodb::types::MongoExplorerNodeId::RetryCollections(database) => {
                        ExplorerNodeId::Mongo {
                            profile_id: *profile_id,
                            node: crate::db::mongodb::types::MongoExplorerNodeId::Database(
                                database.clone(),
                            ),
                        }
                    }
                    crate::db::mongodb::types::MongoExplorerNodeId::RetryIndexes(namespace) => {
                        ExplorerNodeId::Mongo {
                            profile_id: *profile_id,
                            node: crate::db::mongodb::types::MongoExplorerNodeId::Collection(
                                namespace.clone(),
                            ),
                        }
                    }
                    crate::db::mongodb::types::MongoExplorerNodeId::Monitor(database)
                    | crate::db::mongodb::types::MongoExplorerNodeId::Users(database) => {
                        ExplorerNodeId::Mongo {
                            profile_id: *profile_id,
                            node: crate::db::mongodb::types::MongoExplorerNodeId::Database(
                                database.clone(),
                            ),
                        }
                    }
                };
                chain.push(parent);
            }
            ExplorerNodeId::PrincipalGroup { profile_id } => {
                chain.push(ExplorerNodeId::Profile(*profile_id));
            }
            ExplorerNodeId::Principal { entry } => {
                chain.push(ExplorerNodeId::PrincipalGroup {
                    profile_id: entry.profile_id,
                });
                chain.push(ExplorerNodeId::Profile(entry.profile_id));
            }
            ExplorerNodeId::PrincipalNotice { profile_id } => {
                chain.push(ExplorerNodeId::PrincipalGroup {
                    profile_id: *profile_id,
                });
                chain.push(ExplorerNodeId::Profile(*profile_id));
            }
            ExplorerNodeId::Profile(profile_id) => {
                if let Some(profile) = self.profiles.get(profile_id)
                    && let Some(group_id) = profile.group_id
                {
                    chain.push(ExplorerNodeId::ConnectionGroup {
                        group_id,
                        region: match profile.placement {
                            ProfilePlacement::OtherProject => ProfileRegion::Others,
                            ProfilePlacement::CurrentProject | ProfilePlacement::Global => {
                                ProfileRegion::Primary
                            }
                        },
                    });
                }
            }
        }
        chain
    }

    fn append_catalog_ancestors(&self, id: &CatalogId, chain: &mut Vec<ExplorerNodeId>) {
        let Some(profile) = self.profiles.get(&id.profile_id()) else {
            return;
        };
        let mut current = id;
        if chain.last() != Some(&ExplorerNodeId::Catalog(current.clone())) {
            chain.push(ExplorerNodeId::Catalog(current.clone()));
        }
        while let Some(parent) = profile.catalog.parent(current) {
            chain.push(ExplorerNodeId::Catalog(parent.clone()));
            current = parent;
        }
        chain.push(ExplorerNodeId::Profile(id.profile_id()));
    }

    pub(crate) fn reconcile_after_catalog_change(&mut self, fallback: Vec<ExplorerNodeId>) {
        self.retain_existing_expansion();
        let visible = self.visible();
        if visible
            .iter()
            .any(|row| self.selected.as_ref() == Some(&row.id))
        {
            return;
        }
        if let Some(ExplorerNodeId::Profile(profile_id)) = self.selected.as_ref()
            && let Some(profile) = self.profiles.get(profile_id)
            && let Some(group_id) = profile.group_id
        {
            let group = ExplorerNodeId::ConnectionGroup {
                group_id,
                region: match profile.placement {
                    ProfilePlacement::OtherProject => ProfileRegion::Others,
                    ProfilePlacement::CurrentProject | ProfilePlacement::Global => {
                        ProfileRegion::Primary
                    }
                },
            };
            if visible.iter().any(|row| row.id == group) {
                self.selected = Some(group);
                return;
            }
        }
        if self
            .selected
            .as_ref()
            .is_some_and(|selected| self.node_exists(selected))
        {
            return;
        }
        self.selected = fallback
            .into_iter()
            .find(|candidate| self.node_exists(candidate))
            .or_else(|| {
                self.profile_order
                    .iter()
                    .copied()
                    .find(|profile_id| self.profiles.contains_key(profile_id))
                    .map(ExplorerNodeId::Profile)
            })
            .or(Some(ExplorerNodeId::EmptyProfiles));
    }

    fn retain_existing_expansion(&mut self) {
        let expanded = std::mem::take(&mut self.expanded);
        self.expanded = expanded
            .into_iter()
            .filter(|id| self.node_exists(id))
            .collect();
    }
}

struct Projection<'a> {
    explorer: &'a ExplorerTreeState,
    rows: Vec<VisibleExplorerNode>,
    visited_catalog_entries: usize,
}

impl<'a> Projection<'a> {
    fn new(explorer: &'a ExplorerTreeState) -> Self {
        Self {
            explorer,
            rows: Vec::new(),
            visited_catalog_entries: 0,
        }
    }

    fn push(&mut self, id: ExplorerNodeId, depth: usize) {
        self.rows.push(VisibleExplorerNode { id, depth });
    }

    fn append_catalog(&mut self, profile: &ExplorerProfileState, id: &CatalogId, depth: usize) {
        let Some(entry) = profile.catalog.get(id) else {
            return;
        };
        self.visited_catalog_entries += 1;
        let node = ExplorerNodeId::Catalog(id.clone());
        self.push(node.clone(), depth);
        if !entry.expandable || !self.explorer.expanded.contains(&node) {
            return;
        }

        if entry.kind == CatalogKind::Schema {
            let groups = profile.catalog.groups(id);
            for group in groups {
                self.append_group(profile, id, *group, depth + 1);
            }
            self.append_state_rows(
                profile,
                &ExplorerOwnerId::Catalog(id.clone()),
                depth + 1,
                groups.len(),
            );
        } else {
            let children = profile.catalog.children(id);
            for child in children {
                if entry.kind == CatalogKind::Table
                    && profile.kind == DatabaseKind::ClickHouse
                    && profile
                        .catalog
                        .get(child)
                        .is_some_and(|entry| entry.kind == CatalogKind::Projection)
                {
                    continue;
                }
                self.append_catalog(profile, child, depth + 1);
            }
            if entry.kind == CatalogKind::Table && profile.kind == DatabaseKind::ClickHouse {
                let projection_node = ExplorerNodeId::ProjectionGroup(id.clone());
                self.push(projection_node.clone(), depth + 1);
                if self.explorer.expanded.contains(&projection_node) {
                    let projection_children = profile
                        .catalog
                        .children(id)
                        .iter()
                        .filter(|child| {
                            profile
                                .catalog
                                .get(child)
                                .is_some_and(|entry| entry.kind == CatalogKind::Projection)
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    self.append_state_rows(
                        profile,
                        &ExplorerOwnerId::Projections(id.clone()),
                        depth + 2,
                        projection_children.len(),
                    );
                    for child in projection_children {
                        self.append_catalog(profile, &child, depth + 2);
                    }
                }
            }
            self.append_state_rows(
                profile,
                &ExplorerOwnerId::Catalog(id.clone()),
                depth + 1,
                if entry.kind == CatalogKind::Table && profile.kind == DatabaseKind::ClickHouse {
                    children
                        .iter()
                        .filter(|child| {
                            profile
                                .catalog
                                .get(child)
                                .is_some_and(|entry| entry.kind != CatalogKind::Projection)
                        })
                        .count()
                } else {
                    children.len()
                },
            );
        }
    }

    fn append_group(
        &mut self,
        profile: &ExplorerProfileState,
        schema: &CatalogId,
        group: ObjectGroup,
        depth: usize,
    ) {
        let node = ExplorerNodeId::Group {
            parent: schema.clone(),
            group,
        };
        self.push(node.clone(), depth);
        if !self.explorer.expanded.contains(&node) {
            return;
        }
        let children = profile.catalog.group_children(schema, group);
        for child in children {
            self.append_catalog(profile, child, depth + 1);
        }
        self.append_state_rows(
            profile,
            &ExplorerOwnerId::Group {
                parent: schema.clone(),
                group,
            },
            depth + 1,
            children.len(),
        );
    }

    fn append_state_rows(
        &mut self,
        profile: &ExplorerProfileState,
        owner: &ExplorerOwnerId,
        depth: usize,
        child_count: usize,
    ) {
        let Some(state) = profile.load_states.get(owner) else {
            return;
        };
        let refreshing_snapshot = matches!(state, ExplorerLoadState::Loading { .. })
            && (profile.refresh_staged_entries.contains_key(owner)
                || profile.refresh_staged_groups.contains_key(owner));
        if !refreshing_snapshot && let Some(kind) = status_kind(state) {
            self.push(
                ExplorerNodeId::Status {
                    owner: owner.clone(),
                    kind,
                },
                depth,
            );
        }
        let cursor = if !refreshing_snapshot {
            load_state_cursor(state)
        } else {
            None
        };
        let previous_empty_snapshot = refreshing_snapshot
            && matches!(
                profile.previous_load_states.get(owner),
                Some(
                    ExplorerLoadState::Loaded { next_cursor: None }
                        | ExplorerLoadState::Stale { next_cursor: None }
                )
            );
        if (matches!(state, ExplorerLoadState::Loaded { .. }) || previous_empty_snapshot)
            && child_count == 0
            && cursor.is_none()
        {
            self.push(
                ExplorerNodeId::Empty {
                    owner: owner.clone(),
                },
                depth,
            );
        }
        if let Some(cursor) = cursor {
            self.push(
                ExplorerNodeId::LoadMore {
                    parent: owner.clone(),
                    cursor: cursor.clone(),
                },
                depth,
            );
        }
    }
}

fn status_kind(state: &ExplorerLoadState) -> Option<StatusRowKind> {
    match state {
        ExplorerLoadState::Loading { .. } => Some(StatusRowKind::Loading),
        ExplorerLoadState::Stale { .. } => Some(StatusRowKind::Stale),
        ExplorerLoadState::Failed { .. } => Some(StatusRowKind::Retry),
        ExplorerLoadState::PermissionDenied { .. } => Some(StatusRowKind::PermissionDenied),
        ExplorerLoadState::NotLoaded | ExplorerLoadState::Loaded { .. } => None,
    }
}

fn load_state_cursor(state: &ExplorerLoadState) -> Option<&CatalogCursor> {
    match state {
        ExplorerLoadState::Loaded { next_cursor } | ExplorerLoadState::Stale { next_cursor } => {
            next_cursor.as_ref()
        }
        ExplorerLoadState::NotLoaded
        | ExplorerLoadState::Loading { .. }
        | ExplorerLoadState::Failed { .. }
        | ExplorerLoadState::PermissionDenied { .. } => None,
    }
}

fn valid_parent(child: CatalogKind, parent: CatalogKind) -> bool {
    match child {
        CatalogKind::Database => false,
        CatalogKind::Schema => parent == CatalogKind::Database,
        CatalogKind::Table
        | CatalogKind::View
        | CatalogKind::MaterializedView
        | CatalogKind::Dictionary
        | CatalogKind::Function
        | CatalogKind::Procedure
        | CatalogKind::Sequence
        | CatalogKind::Type => parent == CatalogKind::Schema,
        CatalogKind::UserDefinedFunction => parent == CatalogKind::Schema,
        CatalogKind::Trigger => parent == CatalogKind::Schema || parent.is_relation(),
        CatalogKind::Column
        | CatalogKind::Index
        | CatalogKind::PrimaryKey
        | CatalogKind::UniqueConstraint
        | CatalogKind::ForeignKey
        | CatalogKind::CheckConstraint => parent.is_relation(),
        CatalogKind::Projection => parent == CatalogKind::Table,
    }
}

fn object_group(kind: CatalogKind) -> Option<ObjectGroup> {
    match kind {
        CatalogKind::Table => Some(ObjectGroup::Tables),
        CatalogKind::View => Some(ObjectGroup::Views),
        CatalogKind::MaterializedView => Some(ObjectGroup::MaterializedViews),
        CatalogKind::Dictionary => Some(ObjectGroup::Dictionaries),
        CatalogKind::Sequence => Some(ObjectGroup::Sequences),
        CatalogKind::Function => Some(ObjectGroup::Functions),
        CatalogKind::UserDefinedFunction => Some(ObjectGroup::UserDefinedFunctions),
        CatalogKind::Procedure => Some(ObjectGroup::Procedures),
        CatalogKind::Type => Some(ObjectGroup::Types),
        CatalogKind::Trigger => Some(ObjectGroup::Triggers),
        CatalogKind::Database
        | CatalogKind::Schema
        | CatalogKind::Column
        | CatalogKind::Index
        | CatalogKind::PrimaryKey
        | CatalogKind::UniqueConstraint
        | CatalogKind::ForeignKey
        | CatalogKind::CheckConstraint
        | CatalogKind::Projection => None,
    }
}

/// Row indexes of the given ancestor ids, in ancestor order.
///
/// Comparing borrowed ids avoids cloning every `CatalogId` into a map while the
/// body-height loop runs for each navigation step.
fn ancestor_row_indexes(rows: &[VisibleExplorerNode], ancestors: &[ExplorerNodeId]) -> Vec<usize> {
    if ancestors.is_empty() {
        return Vec::new();
    }
    let mut indexes = Vec::with_capacity(ancestors.len());
    for ancestor in ancestors {
        if let Some(index) = rows.iter().position(|row| row.id == *ancestor) {
            indexes.push(index);
        }
    }
    indexes
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::catalog::{OptionalMetadata, QualifiedName};

    #[test]
    fn mongodb_collection_and_index_nodes_project_as_native_namespaces() {
        let profile_id = Uuid::new_v4();
        let namespace = crate::db::mongodb::types::MongoNamespace::collection("app", "orders.v2");
        let mut tree = ExplorerTreeState::default();
        tree.add_profile(profile_id);
        let profile = tree.profiles.get_mut(&profile_id).unwrap();
        profile.kind = DatabaseKind::MongoDb;
        profile.mongodb_databases = vec!["app".into()];
        profile.mongodb_collections.insert(
            "app".into(),
            vec![crate::db::mongodb::types::MongoCollectionInfo {
                namespace: namespace.clone(),
                collection_type: crate::db::mongodb::types::MongoCollectionType::Collection,
                options_ejson: None,
            }],
        );
        profile.mongodb_indexes.insert(
            namespace.clone(),
            vec![crate::db::mongodb::types::MongoIndexInfo {
                name: "_id_".into(),
                keys: mongodb::bson::doc! {"_id": 1},
                unique: true,
                ttl_seconds: None,
                raw: mongodb::bson::doc! {},
                key_description: vec![("_id".into(), mongodb::bson::Bson::Int32(1))],
            }],
        );
        tree.expanded.insert(ExplorerNodeId::Profile(profile_id));
        tree.expanded.insert(ExplorerNodeId::Mongo {
            profile_id,
            node: crate::db::mongodb::types::MongoExplorerNodeId::Database("app".into()),
        });
        tree.expanded.insert(ExplorerNodeId::Mongo {
            profile_id,
            node: crate::db::mongodb::types::MongoExplorerNodeId::Collection(namespace.clone()),
        });
        let visible = tree.visible();
        assert!(visible.iter().any(|node| node.id
            == ExplorerNodeId::Mongo {
                profile_id,
                node: crate::db::mongodb::types::MongoExplorerNodeId::Collection(namespace.clone())
            }));
        assert!(visible.iter().any(|node| node.id
            == ExplorerNodeId::Mongo {
                profile_id,
                node: crate::db::mongodb::types::MongoExplorerNodeId::Index {
                    namespace: namespace.clone(),
                    name: "_id_".into()
                }
            }));
    }

    #[test]
    fn mongodb_discovery_error_projects_a_targeted_retry_row() {
        let profile_id = Uuid::new_v4();
        let mut tree = ExplorerTreeState::default();
        tree.add_profile(profile_id);
        let profile = tree.profiles.get_mut(&profile_id).unwrap();
        profile.kind = DatabaseKind::MongoDb;
        profile.mongodb_databases = vec!["app".into()];
        profile
            .mongodb_collection_errors
            .insert("app".into(), "denied".into());
        tree.expanded.insert(ExplorerNodeId::Profile(profile_id));
        tree.expanded.insert(ExplorerNodeId::Mongo {
            profile_id,
            node: crate::db::mongodb::types::MongoExplorerNodeId::Database("app".into()),
        });
        let retry = ExplorerNodeId::Mongo {
            profile_id,
            node: crate::db::mongodb::types::MongoExplorerNodeId::RetryCollections("app".into()),
        };
        assert!(tree.visible().iter().any(|row| row.id == retry));
        assert!(tree.node_exists(&retry));
    }

    fn table_entry(profile: Uuid, schema: &CatalogId, index: usize) -> CatalogEntry {
        let name = format!("table_{index:04}");
        CatalogEntry::relation(
            CatalogId::new(profile, CatalogKind::Table, [name.as_str()]),
            schema.clone(),
            QualifiedName {
                database: Some("app".to_owned()),
                schema: Some("public".to_owned()),
                object: name,
            },
            "table",
            OptionalMetadata::Supported(None),
            false,
        )
        .unwrap()
    }

    pub(crate) fn explorer_with_tables(
        table_count: usize,
    ) -> (ExplorerTreeState, Vec<ExplorerNodeId>) {
        let profile = Uuid::from_u128(1);
        let database = CatalogEntry::database(
            CatalogId::new(profile, CatalogKind::Database, ["app"]),
            QualifiedName {
                database: Some("app".to_owned()),
                schema: None,
                object: "app".to_owned(),
            },
            "database",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap();
        let schema = CatalogEntry::schema(
            CatalogId::new(profile, CatalogKind::Schema, ["public"]),
            database.id.clone(),
            QualifiedName {
                database: Some("app".to_owned()),
                schema: Some("public".to_owned()),
                object: "public".to_owned(),
            },
            "schema",
            OptionalMetadata::Supported(None),
            true,
        )
        .unwrap();
        let tables = (0..table_count)
            .map(|index| table_entry(profile, &schema.id, index))
            .collect::<Vec<_>>();
        let table_ids = tables
            .iter()
            .map(|table| ExplorerNodeId::Catalog(table.id.clone()))
            .collect::<Vec<_>>();

        let mut entries = vec![database.clone(), schema.clone()];
        entries.extend(tables);
        let mut tree = CatalogTree::new(profile);
        tree.insert_subtree(entries).unwrap();
        tree.set_group_state(
            &schema.id,
            ObjectGroup::Tables,
            CatalogGroupState {
                count: CatalogCount::Exact(table_count as u64),
                completeness: CatalogCompleteness::Complete,
            },
        )
        .unwrap();

        let mut explorer = ExplorerTreeState::default();
        explorer.add_profile(profile);
        let profile_state = explorer.profiles.get_mut(&profile).unwrap();
        profile_state.catalog = tree;
        profile_state.status = ExplorerConnectionStatus::Online;
        explorer.expanded.extend([
            ExplorerNodeId::Profile(profile),
            ExplorerNodeId::Catalog(database.id.clone()),
            ExplorerNodeId::Catalog(schema.id.clone()),
            ExplorerNodeId::Group {
                parent: schema.id.clone(),
                group: ObjectGroup::Tables,
            },
        ]);
        explorer.viewport_height = 30;
        explorer.selected = Some(table_ids[0].clone());
        (explorer, table_ids)
    }

    #[test]
    fn expanded_table_rows_have_expected_shape() {
        let (explorer, table_ids) = explorer_with_tables(956);
        let rows = explorer.visible();
        // The connection-level `Users & Roles` group is appended last.
        assert_eq!(rows.len(), 961);
        assert_eq!(
            rows[rows.len() - 2].id.clone(),
            table_ids.last().cloned().unwrap()
        );
        assert_eq!(
            rows.last().map(|row| row.id.clone()),
            Some(ExplorerNodeId::PrincipalGroup {
                profile_id: uuid::Uuid::from_u128(1)
            })
        );
    }

    #[test]
    fn one_navigation_step_projects_at_most_once() {
        let (mut explorer, _) = explorer_with_tables(956);
        for delta in [1, 1, -1, 5, -20] {
            let _ = take_projection_calls();
            explorer.move_selection(delta, 30);
            assert!(
                take_projection_calls() <= 1,
                "a single move must not re-project the expanded tree"
            );
        }
    }

    #[test]
    fn viewport_scroll_and_alignment_each_project_once() {
        let (mut explorer, _) = explorer_with_tables(956);

        let _ = take_projection_calls();
        let viewport = explorer.viewport(30);
        assert_eq!(take_projection_calls(), 1);
        assert_eq!(viewport.total_rows, 961);
        assert!(viewport.rows.len() <= 30);

        let _ = take_projection_calls();
        explorer.set_scroll_offset(400);
        assert_eq!(take_projection_calls(), 1);

        let _ = take_projection_calls();
        explorer.ensure_selected_visible();
        assert_eq!(take_projection_calls(), 1);

        let _ = take_projection_calls();
        explorer.align_selected(ExplorerNodeAlignment::Middle);
        assert_eq!(take_projection_calls(), 1);
    }

    #[test]
    fn navigation_over_expanded_tables_keeps_scroll_in_bounds() {
        let (mut explorer, _) = explorer_with_tables(956);
        let total = explorer.visible().len();
        for _ in 0..2_000 {
            explorer.move_selection(1, 30);
            assert!(explorer.scroll < total);
            assert!(explorer.selected.is_some());
        }
        let bottom = explorer.selected.clone();
        for _ in 0..2_000 {
            explorer.move_selection(-1, 30);
        }
        assert_eq!(explorer.scroll, 0);
        assert_ne!(explorer.selected, bottom);
    }
}
