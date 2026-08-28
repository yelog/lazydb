use std::collections::HashSet;

use uuid::Uuid;

pub use crate::identity::ConnectionIdentity;

use crate::db::catalog::{CatalogSearchHit, CatalogSearchPage};
use crate::db::{
    ServerInfo,
    catalog::{CatalogEntry, CatalogId, CatalogKind, CatalogNode, OptionalMetadata},
};
use crate::help::HelpState;
use crate::model::execution_target::ExecutionTarget;
use crate::model::explorer::{
    ExplorerConnectionStatus, ExplorerNodeId, ExplorerTreeState, ProfileProvenance, StatusRowKind,
};
use crate::model::transaction::{
    CancellationIntent, DeferredTransactionPrompt, TransactionExitChoice,
};
use crate::profile::DatabaseKind;
use crate::sql::CompletionIndex;
use crate::sql::ExecutionDraft;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Focus {
    Explorer,
    #[default]
    Editor,
    Results,
}

impl Focus {
    pub fn next(self) -> Self {
        match self {
            Self::Explorer => Self::Editor,
            Self::Editor => Self::Results,
            Self::Results => Self::Explorer,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Explorer => Self::Results,
            Self::Editor => Self::Explorer,
            Self::Results => Self::Editor,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Overlay {
    Help(HelpState),
    ProfileManager,
    Message {
        title: String,
        body: String,
    },
    SubstituteConfirm {
        remaining: usize,
    },
    ExecutionConfirm {
        draft: ExecutionDraft,
        focus: ExecutionConfirmFocus,
    },
    ManualCancelConfirm {
        intent: CancellationIntent,
        focus: ManualCancelFocus,
    },
    TransactionExitConfirm {
        prompt: DeferredTransactionPrompt,
        choice: TransactionExitChoice,
    },
    ClearTransactionOutcome {
        console_id: Uuid,
        connection: ConnectionIdentity,
        transaction_generation: u64,
    },
    TargetSelector {
        candidates: Vec<ExecutionTarget>,
        selected: usize,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExecutionConfirmFocus {
    #[default]
    Cancel,
    Execute,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ManualCancelFocus {
    #[default]
    KeepRunning,
    CancelQueryAndRollback,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum QueryStatus {
    #[default]
    Idle,
    Running,
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConnectionStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Failed,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConnectionState {
    pub profile_id: Option<Uuid>,
    pub generation: u64,
    pub pending_profile_id: Option<Uuid>,
    pub pending_generation: Option<u64>,
    pub target: Option<ExecutionTarget>,
    pub pending_target: Option<ExecutionTarget>,
    pub status: ConnectionStatus,
    pub server: Option<ServerInfo>,
    pub error: Option<String>,
}

impl ConnectionState {
    pub fn active_identity(&self) -> Option<ConnectionIdentity> {
        Some(ConnectionIdentity {
            profile_id: self.profile_id?,
            generation: self.generation,
        })
    }

    pub fn pending_identity(&self) -> Option<ConnectionIdentity> {
        Some(ConnectionIdentity {
            profile_id: self.pending_profile_id?,
            generation: self.pending_generation?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisibleCatalogNode {
    pub id: ExplorerNodeId,
    pub depth: usize,
    pub label: String,
    pub metadata: Option<String>,
    pub comment: Option<String>,
    pub kind: Option<CatalogKind>,
    pub profile_kind: Option<DatabaseKind>,
    pub provenance: Option<ProfileProvenance>,
    pub connection_status: Option<ExplorerConnectionStatus>,
    pub endpoint: Option<String>,
    pub expandable: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ExplorerState {
    pub normalized: ExplorerTreeState,
    pub nodes: Vec<CatalogNode>,
    pub expanded: HashSet<CatalogId>,
    pub selected: usize,
    pub scroll: usize,
    pub catalog_generation: u64,
    pub completion_index: CompletionIndex,
    pub active_profile: Option<Uuid>,
    pub search: Option<ExplorerSearchState>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExplorerSearchLifecycle {
    Idle,
    Loading,
    Ready,
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExplorerSearchState {
    pub connection: Option<ConnectionIdentity>,
    pub session_id: u64,
    pub query: String,
    pub generation: u64,
    pub lifecycle: ExplorerSearchLifecycle,
    pub hits: Vec<CatalogSearchHit>,
    pub selected: usize,
    pub scroll: usize,
    pub truncated: bool,
    pub total_count: Option<usize>,
    pub located: Option<crate::db::catalog::CatalogId>,
}

impl ExplorerSearchState {
    fn new(connection: Option<ConnectionIdentity>, session_id: u64) -> Self {
        Self {
            connection,
            session_id,
            query: String::new(),
            generation: 0,
            lifecycle: ExplorerSearchLifecycle::Idle,
            hits: Vec::new(),
            selected: 0,
            scroll: 0,
            truncated: false,
            total_count: None,
            located: None,
        }
    }
}

impl ExplorerState {
    pub fn connection_changed(&mut self) {
        self.catalog_generation = self.catalog_generation.saturating_add(1);
        self.nodes.clear();
        self.expanded.clear();
        self.completion_index = CompletionIndex::default();
        self.selected = 0;
        self.scroll = 0;
        self.active_profile = None;
        self.search = None;
    }

    pub fn open_search(&mut self, connection: Option<ConnectionIdentity>, session_id: u64) {
        self.search = Some(ExplorerSearchState::new(connection, session_id));
    }

    pub fn edit_search(&mut self, edit: impl FnOnce(&mut String)) -> Option<u64> {
        let search = self.search.as_mut()?;
        edit(&mut search.query);
        search.generation = search.generation.saturating_add(1);
        search.selected = 0;
        search.scroll = 0;
        search.total_count = None;
        search.truncated = false;
        search.located = None;
        search.lifecycle = if search.query.trim().is_empty() {
            search.hits.clear();
            ExplorerSearchLifecycle::Idle
        } else {
            ExplorerSearchLifecycle::Loading
        };
        Some(search.generation)
    }

    pub fn accept_search_page(&mut self, page: CatalogSearchPage) -> bool {
        let Some(search) = self.search.as_mut().filter(|search| {
            search.connection == Some(page.connection)
                && search.session_id == page.session_id
                && search.generation == page.generation
        }) else {
            return false;
        };
        search.hits = page.hits;
        search.total_count = page.total_count;
        search.truncated = page.truncated;
        search.selected = search.selected.min(search.hits.len().saturating_sub(1));
        search.lifecycle = ExplorerSearchLifecycle::Ready;
        true
    }

    pub fn move_search(&mut self, delta: isize) {
        let Some(search) = self.search.as_mut() else {
            return;
        };
        search.selected = search
            .selected
            .saturating_add_signed(delta)
            .min(search.hits.len().saturating_sub(1));
    }

    pub fn locate_search_hit(&mut self) -> Result<bool, String> {
        let Some(search) = self.search.as_ref() else {
            return Ok(false);
        };
        let Some(hit) = search.hits.get(search.selected).cloned() else {
            return Ok(false);
        };
        let profile_id = hit.entry.id.profile_id();
        let profile = self
            .normalized
            .profiles
            .get_mut(&profile_id)
            .ok_or_else(|| "search result profile is unavailable".to_owned())?;
        profile
            .catalog
            .merge_search_hit(&hit)
            .map_err(|error| error.to_string())?;
        self.normalized
            .expanded
            .insert(ExplorerNodeId::Profile(profile_id));
        for entry in hit.ancestors.iter().chain(std::iter::once(&hit.entry)) {
            if let Some(parent) = entry.parent_id.as_ref()
                && parent.kind == CatalogKind::Schema
                && let Some(group) = search_object_group(entry.kind)
            {
                self.normalized.expanded.insert(ExplorerNodeId::Group {
                    parent: parent.clone(),
                    group,
                });
            }
            if entry.expandable || entry.id != hit.entry.id {
                self.normalized
                    .expanded
                    .insert(ExplorerNodeId::Catalog(entry.id.clone()));
            }
        }
        self.normalized.selected = Some(ExplorerNodeId::Catalog(hit.entry.id.clone()));
        self.normalized.ensure_selected_visible(8);
        self.sync_selected_index();
        if let Some(search) = self.search.as_mut() {
            search.located = Some(hit.entry.id);
        }
        Ok(true)
    }

    pub fn rebuild_projection(&mut self, profile_id: Uuid) {
        let Some(profile) = self.normalized.profiles.get(&profile_id) else {
            self.nodes.clear();
            return;
        };
        self.active_profile = Some(profile_id);
        self.nodes = profile
            .catalog
            .entries()
            .values()
            .map(project_entry)
            .collect();
        self.nodes.sort_by(|left, right| {
            left.id
                .native_path
                .cmp(&right.id.native_path)
                .then_with(|| left.name.cmp(&right.name))
        });
        self.expanded = self
            .normalized
            .expanded
            .iter()
            .filter_map(|node| match node {
                ExplorerNodeId::Catalog(id) => Some(id.clone()),
                _ => None,
            })
            .collect();
        self.selected = self
            .normalized
            .selected_visible_index_for_profile(profile_id)
            .unwrap_or(0);
        self.normalized.ensure_selected_visible(8);
        self.scroll = self
            .normalized
            .scroll
            .min(self.normalized.visible().len().saturating_sub(1));
        self.normalized.scroll = self.scroll;
    }

    pub fn visible(&self) -> Vec<VisibleCatalogNode> {
        self.normalized
            .visible()
            .into_iter()
            .map(|row| {
                let profile = row
                    .id
                    .profile_id()
                    .and_then(|profile_id| self.normalized.profiles.get(&profile_id));
                let (
                    label,
                    metadata,
                    comment,
                    kind,
                    profile_kind,
                    provenance,
                    connection_status,
                    endpoint,
                    expandable,
                ) = match &row.id {
                    ExplorerNodeId::Catalog(id) => profile
                        .and_then(|profile| profile.catalog.get(id))
                        .map_or_else(
                            || {
                                (
                                    "Missing object".to_owned(),
                                    None,
                                    None,
                                    None,
                                    None,
                                    None,
                                    None,
                                    None,
                                    false,
                                )
                            },
                            |entry| {
                                (
                                    entry_label(entry),
                                    entry_detail(entry),
                                    entry_comment(entry),
                                    Some(entry.kind),
                                    None,
                                    None,
                                    None,
                                    None,
                                    entry.expandable,
                                )
                            },
                        ),
                    ExplorerNodeId::Group { parent, group } => {
                        let metadata = profile
                            .and_then(|profile| profile.catalog.group_state(parent, *group))
                            .and_then(|state| catalog_count_label(state.count));
                        (
                            group_label(*group).to_owned(),
                            metadata,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            true,
                        )
                    }
                    ExplorerNodeId::Status { owner, kind } => (
                        status_label(*kind).to_owned(),
                        profile.and_then(|profile| profile.load_errors.get(owner).cloned()),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        false,
                    ),
                    ExplorerNodeId::LoadMore { .. } => (
                        "Load more...".to_owned(),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        false,
                    ),
                    ExplorerNodeId::Empty { .. } => (
                        "No objects".to_owned(),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        false,
                    ),
                    ExplorerNodeId::Profile(_profile_id) => profile.map_or_else(
                        || {
                            (
                                "Missing profile".to_owned(),
                                None,
                                None,
                                None,
                                None,
                                None,
                                None,
                                None,
                                false,
                            )
                        },
                        |profile| {
                            (
                                profile.display_name.clone(),
                                None,
                                None,
                                None,
                                Some(profile.kind),
                                Some(profile.provenance),
                                Some(profile.status),
                                Some(profile.endpoint.clone()),
                                true,
                            )
                        },
                    ),
                    ExplorerNodeId::EmptyProfiles => (
                        "No profiles".to_owned(),
                        None,
                        Some("NEW".to_owned()),
                        None,
                        None,
                        None,
                        None,
                        None,
                        false,
                    ),
                };
                VisibleCatalogNode {
                    id: row.id,
                    depth: row.depth,
                    label,
                    metadata,
                    comment,
                    kind,
                    profile_kind,
                    provenance,
                    connection_status,
                    endpoint,
                    expandable,
                }
            })
            .collect()
    }

    pub fn selected_id(&self) -> Option<&ExplorerNodeId> {
        self.normalized.selected.as_ref()
    }

    pub fn selected_entry(&self) -> Option<&CatalogEntry> {
        let ExplorerNodeId::Catalog(id) = self.selected_id()? else {
            return None;
        };
        self.normalized
            .profiles
            .get(&id.profile_id())?
            .catalog
            .get(id)
    }

    pub fn move_selection(&mut self, delta: isize) {
        let rows = self.visible();
        if rows.is_empty() {
            self.selected = 0;
            return;
        }
        let current = self
            .normalized
            .selected
            .as_ref()
            .and_then(|selected| rows.iter().position(|row| &row.id == selected))
            .unwrap_or(0);
        self.selected = current
            .saturating_add_signed(delta)
            .min(rows.len().saturating_sub(1));
        self.normalized.selected = Some(rows[self.selected].id.clone());
        self.normalized.ensure_selected_visible(8);
        self.scroll = self.normalized.scroll;
    }

    pub fn select_id(&mut self, id: ExplorerNodeId) -> bool {
        let rows = self.visible();
        let Some(index) = rows.iter().position(|row| row.id == id) else {
            return false;
        };
        self.selected = index;
        self.normalized.selected = Some(id);
        self.normalized.ensure_selected_visible(8);
        self.scroll = self.normalized.scroll;
        true
    }

    pub fn toggle_selected(&mut self) -> bool {
        let Some(id) = self.selected_id().cloned() else {
            return false;
        };
        if !matches!(
            id,
            ExplorerNodeId::Profile(_) | ExplorerNodeId::Catalog(_) | ExplorerNodeId::Group { .. }
        ) {
            return false;
        }
        if !self.normalized.expanded.remove(&id) {
            self.normalized.expanded.insert(id);
            true
        } else {
            false
        }
    }

    pub fn sync_selected_index(&mut self) {
        let Some(selected) = self.normalized.selected.as_ref() else {
            return;
        };
        self.selected = self
            .visible()
            .iter()
            .position(|row| &row.id == selected)
            .unwrap_or(0);
        self.scroll = self.normalized.scroll;
    }
}

fn search_object_group(kind: CatalogKind) -> Option<crate::db::catalog::ObjectGroup> {
    use crate::db::catalog::ObjectGroup;
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

impl ExplorerTreeState {
    fn selected_visible_index_for_profile(&self, profile_id: Uuid) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        self.visible_profile(profile_id)
            .iter()
            .position(|row| &row.id == selected)
    }
}

fn project_entry(entry: &CatalogEntry) -> CatalogNode {
    CatalogNode::new(
        entry.id.clone(),
        entry.parent_id.clone(),
        entry.qualified_name.object.clone(),
        entry.native_kind.clone(),
        match &entry.comment {
            OptionalMetadata::Supported(comment) => comment.clone(),
            OptionalMetadata::Unsupported => None,
        },
        entry.expandable,
    )
}

fn group_label(group: crate::db::catalog::ObjectGroup) -> &'static str {
    use crate::db::catalog::ObjectGroup;
    match group {
        ObjectGroup::Tables => "Tables",
        ObjectGroup::Views => "Views",
        ObjectGroup::MaterializedViews => "Materialized views",
        ObjectGroup::Sequences => "Sequences",
        ObjectGroup::Functions => "Functions",
        ObjectGroup::Procedures => "Procedures",
        ObjectGroup::Types => "Types",
        ObjectGroup::Triggers => "Triggers",
    }
}

fn status_label(kind: StatusRowKind) -> &'static str {
    match kind {
        StatusRowKind::Loading => "Loading...",
        StatusRowKind::Retry => "Retry",
        StatusRowKind::Stale => "Stale - retry",
        StatusRowKind::PermissionDenied => "Permission denied - retry",
    }
}

fn entry_detail(entry: &CatalogEntry) -> Option<String> {
    use crate::db::catalog::{CatalogMetadata, ConstraintMetadata};
    match &entry.metadata {
        CatalogMetadata::Column(column) => {
            let mut flags = Vec::new();
            if !column.nullable {
                flags.push("NOT NULL".to_owned());
            }
            if let OptionalMetadata::Supported(Some(default)) = &column.default_expression {
                flags.push(format!("DEFAULT {default}"));
            }
            (!flags.is_empty()).then(|| flags.join(" "))
        }
        CatalogMetadata::Index(index) => Some(format!(
            "{} ({})",
            if index.unique { "UNIQUE" } else { "INDEX" },
            index.columns.join(", ")
        )),
        CatalogMetadata::Constraint(constraint) => Some(match constraint {
            ConstraintMetadata::PrimaryKey { columns } => {
                format!("PRIMARY KEY ({})", columns.join(", "))
            }
            ConstraintMetadata::Unique { columns } => format!("UNIQUE ({})", columns.join(", ")),
            ConstraintMetadata::ForeignKey {
                columns,
                referenced_relation,
                ..
            } => format!(
                "FOREIGN KEY ({}) -> {}",
                columns.join(", "),
                referenced_relation.object
            ),
            ConstraintMetadata::Check { expression } => format!("CHECK ({expression})"),
        }),
        CatalogMetadata::None => None,
    }
}

fn entry_label(entry: &CatalogEntry) -> String {
    entry.qualified_name.object.clone()
}

fn entry_comment(entry: &CatalogEntry) -> Option<String> {
    match &entry.comment {
        OptionalMetadata::Supported(comment) => comment.clone(),
        OptionalMetadata::Unsupported => None,
    }
}

fn catalog_count_label(count: crate::db::catalog::CatalogCount) -> Option<String> {
    match count {
        crate::db::catalog::CatalogCount::Exact(value) => Some(value.to_string()),
        crate::db::catalog::CatalogCount::AtLeast(value) => Some(format!("{value}+")),
        crate::db::catalog::CatalogCount::Unknown => None,
    }
}
