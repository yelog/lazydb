use std::collections::{HashMap, HashSet};

use thiserror::Error;
use uuid::Uuid;

use crate::db::catalog::{
    CatalogCompleteness, CatalogCount, CatalogCursor, CatalogEntry, CatalogId, CatalogKind,
    CatalogRequest, CatalogSearchHit, CatalogTarget, ObjectGroup,
};
use crate::profile::DatabaseKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileProvenance {
    Saved,
    Session,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ExplorerOwnerId {
    Profile(Uuid),
    Catalog(CatalogId),
    Group {
        parent: CatalogId,
        group: ObjectGroup,
    },
}

impl ExplorerOwnerId {
    pub const fn profile_id(&self) -> Uuid {
        match self {
            Self::Profile(profile_id) => *profile_id,
            Self::Catalog(id) | Self::Group { parent: id, .. } => id.profile_id(),
        }
    }

    pub fn node_id(&self) -> ExplorerNodeId {
        match self {
            Self::Profile(profile_id) => ExplorerNodeId::Profile(*profile_id),
            Self::Catalog(id) => ExplorerNodeId::Catalog(id.clone()),
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
    Profile(Uuid),
    Catalog(CatalogId),
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
}

impl ExplorerNodeId {
    pub const fn profile_id(&self) -> Option<Uuid> {
        match self {
            Self::Profile(profile_id) => Some(*profile_id),
            Self::Catalog(id) => Some(id.profile_id()),
            Self::Group { parent, .. } => Some(parent.profile_id()),
            Self::Status { owner, .. }
            | Self::LoadMore { parent: owner, .. }
            | Self::Empty { owner } => Some(owner.profile_id()),
            Self::EmptyProfiles => None,
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
    pub status: ExplorerConnectionStatus,
    pub catalog: CatalogTree,
    pub catalog_epoch: u64,
    pub next_request_id: u64,
    pub load_states: HashMap<ExplorerOwnerId, ExplorerLoadState>,
    pub pending_requests: HashMap<ExplorerOwnerId, CatalogRequest>,
    pub previous_load_states: HashMap<ExplorerOwnerId, ExplorerLoadState>,
    pub load_errors: HashMap<ExplorerOwnerId, String>,
    pub last_error: Option<String>,
    pub expand_after_connect: bool,
}

impl ExplorerProfileState {
    pub fn new(
        profile_id: Uuid,
        display_name: String,
        kind: DatabaseKind,
        endpoint: String,
        provenance: ProfileProvenance,
    ) -> Self {
        Self {
            display_name,
            kind,
            endpoint,
            provenance,
            status: ExplorerConnectionStatus::Offline,
            catalog: CatalogTree::new(profile_id),
            catalog_epoch: 0,
            next_request_id: 1,
            load_states: HashMap::new(),
            pending_requests: HashMap::new(),
            previous_load_states: HashMap::new(),
            load_errors: HashMap::new(),
            last_error: None,
            expand_after_connect: false,
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
        let query = query.to_lowercase();
        let mut included = HashSet::new();
        let mut matches = HashSet::new();
        for entry in profile.catalog.entries().values() {
            if entry.kind.is_relation_child()
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
                .join(".")
                .to_lowercase();
            if !entry.qualified_name.object.to_lowercase().contains(&query)
                && !path.contains(&query)
            {
                continue;
            }
            matches.insert(ExplorerNodeId::Catalog(entry.id.clone()));
            included.insert(ExplorerNodeId::Catalog(entry.id.clone()));
            let mut current = entry.id.clone();
            while let Some(parent) = profile.catalog.parent(&current).cloned() {
                included.insert(ExplorerNodeId::Catalog(parent.clone()));
                if parent.kind == CatalogKind::Schema {
                    if let Some(group) = search_group(entry.kind) {
                        included.insert(ExplorerNodeId::Group {
                            parent: parent.clone(),
                            group,
                        });
                    }
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
        if let Some(profile) = self.profiles.get_mut(&profile_id) {
            profile.display_name = display_name;
            profile.kind = kind;
            profile.endpoint = endpoint;
            profile.provenance = provenance;
            return;
        }
        if self.profiles.contains_key(&profile_id) {
            return;
        }
        self.profiles.insert(
            profile_id,
            ExplorerProfileState::new(profile_id, display_name, kind, endpoint, provenance),
        );
        self.profile_order.push(profile_id);
        self.expanded.insert(ExplorerNodeId::Profile(profile_id));
        if matches!(self.selected, None | Some(ExplorerNodeId::EmptyProfiles)) {
            self.selected = Some(ExplorerNodeId::Profile(profile_id));
        }
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
        if rows.is_empty() || height == 0 {
            return ExplorerViewport {
                pinned: Vec::new(),
                rows: Vec::new(),
                hidden_ancestor_count: 0,
                show_ancestor_indicator: false,
                body_height: 0,
            };
        }

        let indexes = rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.id.clone(), index))
            .collect::<HashMap<_, _>>();
        let ancestors = self
            .selected_ancestors()
            .into_iter()
            .filter_map(|id| indexes.get(&id).copied().map(|index| (index, id)))
            .filter(|(index, _)| *index < self.scroll)
            .map(|(_, id)| id)
            .collect::<Vec<_>>();
        let show_ancestor_indicator = height >= 3 && ancestors.len() > height.saturating_sub(2);
        let pinned_capacity = height
            .saturating_sub(1)
            .saturating_sub(usize::from(show_ancestor_indicator));
        let hidden_ancestor_count = ancestors.len().saturating_sub(pinned_capacity);
        let pinned = ancestors
            .into_iter()
            .skip(hidden_ancestor_count)
            .filter_map(|id| indexes.get(&id).and_then(|index| rows.get(*index)).cloned())
            .collect::<Vec<_>>();
        let indicator_height = usize::from(show_ancestor_indicator);
        let body_height = height
            .saturating_sub(pinned.len())
            .saturating_sub(indicator_height)
            .max(1)
            .min(height);
        let rows = rows
            .into_iter()
            .skip(self.scroll)
            .take(body_height)
            .collect();

        ExplorerViewport {
            pinned,
            rows,
            hidden_ancestor_count,
            show_ancestor_indicator,
            body_height,
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
        projection.rows
    }

    #[doc(hidden)]
    pub fn visible_with_visit_count(&self) -> (Vec<VisibleExplorerNode>, usize) {
        let mut projection = Projection::new(self);
        for profile_id in &self.profile_order {
            let Some(profile) = self.profiles.get(profile_id) else {
                continue;
            };
            let profile_node = ExplorerNodeId::Profile(*profile_id);
            projection.push(profile_node.clone(), 0);
            if !self.expanded.contains(&profile_node) {
                continue;
            }
            let roots = profile.catalog.roots();
            for root in roots {
                projection.append_catalog(profile, root, 1);
            }
            projection.append_state_rows(
                profile,
                &ExplorerOwnerId::Profile(*profile_id),
                1,
                roots.len(),
            );
        }
        if projection.rows.is_empty() {
            projection.push(ExplorerNodeId::EmptyProfiles, 0);
        }
        (projection.rows, projection.visited_catalog_entries)
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
            ExplorerNodeId::Group { parent, group } => self
                .profiles
                .get(&parent.profile_id())
                .is_some_and(|profile| profile.catalog.group_state(parent, *group).is_some()),
            ExplorerNodeId::EmptyProfiles
            | ExplorerNodeId::Status { .. }
            | ExplorerNodeId::LoadMore { .. }
            | ExplorerNodeId::Empty { .. } => false,
        };
        expandable && self.expanded.insert(selected)
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
        true
    }

    pub fn set_viewport_height(&mut self, viewport_height: usize) {
        self.viewport_height = viewport_height;
        self.ensure_selected_visible();
    }

    pub fn move_selection(&mut self, delta: isize, viewport_height: usize) {
        self.viewport_height = viewport_height;
        let rows = self.visible();
        if rows.is_empty() {
            self.selected = None;
            self.scroll = 0;
            return;
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
        self.update_scroll(selected_index, rows.len());
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
        let body_height = self.body_height_for_scroll(&rows, self.scroll);
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
        self.update_scroll(selected_index, rows.len());
    }

    pub fn scroll_nodes(&mut self, direction: isize, amount: ExplorerScrollAmount) {
        let rows = self.visible();
        if rows.is_empty() || self.viewport_height == 0 || direction == 0 {
            return;
        }
        let step = match amount {
            ExplorerScrollAmount::HalfPage => {
                (self.body_height_for_scroll(&rows, self.scroll) / 2).max(1)
            }
            ExplorerScrollAmount::Page => self.body_height_for_scroll(&rows, self.scroll),
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
        self.update_scroll(selected_index, rows.len());
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
        let body_height = self.body_height_for_scroll(&rows, self.scroll);
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

    pub fn ensure_selected_visible(&mut self) {
        let rows = self.visible();
        let Some(selected) = self.selected.as_ref() else {
            self.scroll = 0;
            return;
        };
        let Some(selected_index) = rows.iter().position(|row| &row.id == selected) else {
            return;
        };
        self.update_scroll(selected_index, rows.len());
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

    fn update_scroll(&mut self, selected_index: usize, row_count: usize) {
        if self.viewport_height == 0 {
            self.scroll = selected_index;
            return;
        }
        let rows = self.visible();
        for _ in 0..=self.selected_ancestors().len() {
            let body_height = self.body_height_for_scroll(&rows, self.scroll);
            if selected_index < self.scroll {
                self.scroll = selected_index;
            } else if selected_index >= self.scroll.saturating_add(body_height) {
                self.scroll = selected_index + 1 - body_height;
            }
            self.scroll = self.scroll.min(row_count.saturating_sub(body_height));
        }
    }

    fn body_height_for_scroll(&self, rows: &[VisibleExplorerNode], scroll: usize) -> usize {
        if self.viewport_height == 0 || rows.is_empty() {
            return 0;
        }
        let indexes = rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.id.clone(), index))
            .collect::<HashMap<_, _>>();
        let pinned = self
            .selected_ancestors()
            .into_iter()
            .filter_map(|id| indexes.get(&id).copied())
            .filter(|index| *index < scroll)
            .count();
        self.viewport_height.saturating_sub(pinned).max(1)
    }

    fn visible_parent(&self, node: &ExplorerNodeId) -> Option<ExplorerNodeId> {
        match node {
            ExplorerNodeId::EmptyProfiles | ExplorerNodeId::Profile(_) => None,
            ExplorerNodeId::Catalog(id) => {
                let profile = self.profiles.get(&id.profile_id())?;
                let entry = profile.catalog.get(id)?;
                let Some(parent) = entry.parent_id.as_ref() else {
                    return Some(ExplorerNodeId::Profile(id.profile_id()));
                };
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
            ExplorerNodeId::Group { parent, .. } => Some(ExplorerNodeId::Catalog(parent.clone())),
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
            ExplorerNodeId::Catalog(id) => self
                .profiles
                .get(&id.profile_id())
                .is_some_and(|profile| profile.catalog.get(id).is_some()),
            ExplorerNodeId::Group { parent, group } => self
                .profiles
                .get(&parent.profile_id())
                .is_some_and(|profile| profile.catalog.group_state(parent, *group).is_some()),
            ExplorerNodeId::Status { owner, kind } => self
                .load_state(owner)
                .is_some_and(|state| status_kind(state) == Some(*kind)),
            ExplorerNodeId::LoadMore { parent, cursor } => {
                self.load_state(parent).and_then(load_state_cursor) == Some(cursor)
            }
            ExplorerNodeId::Empty { owner } => self.load_state(owner).is_some_and(|state| {
                matches!(state, ExplorerLoadState::Loaded { next_cursor: None })
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
            ExplorerOwnerId::Group { parent, group } => {
                Some(profile.catalog.group_children(parent, *group).len())
            }
        }
    }

    fn selection_fallback_chain(&self) -> Vec<ExplorerNodeId> {
        let Some(selected) = self.selected.as_ref() else {
            return Vec::new();
        };
        let mut chain = vec![selected.clone()];
        match selected {
            ExplorerNodeId::Catalog(id) => self.append_catalog_ancestors(id, &mut chain),
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
                ExplorerOwnerId::Group { parent, group } => {
                    chain.push(ExplorerNodeId::Group {
                        parent: parent.clone(),
                        group: *group,
                    });
                    self.append_catalog_ancestors(parent, &mut chain)
                }
            },
            ExplorerNodeId::EmptyProfiles | ExplorerNodeId::Profile(_) => {}
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

    fn reconcile_after_catalog_change(&mut self, fallback: Vec<ExplorerNodeId>) {
        self.retain_existing_expansion();
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
                self.append_catalog(profile, child, depth + 1);
            }
            self.append_state_rows(
                profile,
                &ExplorerOwnerId::Catalog(id.clone()),
                depth + 1,
                children.len(),
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
        if let Some(kind) = status_kind(state) {
            self.push(
                ExplorerNodeId::Status {
                    owner: owner.clone(),
                    kind,
                },
                depth,
            );
        }
        let cursor = load_state_cursor(state);
        if matches!(state, ExplorerLoadState::Loaded { .. }) && child_count == 0 && cursor.is_none()
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
        | CatalogKind::Function
        | CatalogKind::Procedure
        | CatalogKind::Sequence
        | CatalogKind::Type => parent == CatalogKind::Schema,
        CatalogKind::Trigger => parent == CatalogKind::Schema || parent.is_relation(),
        CatalogKind::Column
        | CatalogKind::Index
        | CatalogKind::PrimaryKey
        | CatalogKind::UniqueConstraint
        | CatalogKind::ForeignKey
        | CatalogKind::CheckConstraint => parent.is_relation(),
    }
}

fn object_group(kind: CatalogKind) -> Option<ObjectGroup> {
    match kind {
        CatalogKind::Table => Some(ObjectGroup::Tables),
        CatalogKind::View => Some(ObjectGroup::Views),
        CatalogKind::MaterializedView => Some(ObjectGroup::MaterializedViews),
        CatalogKind::Sequence => Some(ObjectGroup::Sequences),
        CatalogKind::Function => Some(ObjectGroup::Functions),
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
        | CatalogKind::CheckConstraint => None,
    }
}
