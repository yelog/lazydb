use std::collections::{HashMap, HashSet};

use uuid::Uuid;

pub use crate::identity::ConnectionIdentity;

use crate::db::catalog::{CatalogSearchHit, CatalogSearchPage, search_text_matches};
use crate::db::catalog_mutation::{
    CatalogMutationCapabilities, CatalogOwnerContext, CatalogOwnerContextRequest,
};
use crate::db::{
    ServerInfo,
    catalog::{CatalogEntry, CatalogId, CatalogKind, CatalogNode, CatalogTarget, OptionalMetadata},
};
use crate::help::HelpState;
use crate::model::execution_target::ExecutionTarget;
use crate::model::explorer::{
    ExplorerConnectionStatus, ExplorerNodeAlignment, ExplorerNodeId, ExplorerNodeTarget,
    ExplorerScrollAmount, ExplorerTreeState, ProfilePlacement, ProfileProvenance, StatusRowKind,
};
use crate::model::tab::{ConsoleRecord, WorkspaceTab};
use crate::model::text_input::TextInput;
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PaneSizePreferences {
    pub explorer_width: Option<u16>,
    pub editor_height: Option<u16>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PaneLayoutMetrics {
    pub explorer_width: Option<u16>,
    pub editor_height: Option<u16>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneSplit {
    ExplorerWidth,
    EditorHeight,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneResize {
    pub split: PaneSplit,
    pub delta: i32,
}

pub fn pane_resize(focus: Focus, operator: char, count: u32) -> Option<PaneResize> {
    let step = i32::try_from(count).ok()?;
    if step == 0 {
        return None;
    }

    let (split, direction) = match (focus, operator) {
        (Focus::Explorer, '>') => (PaneSplit::ExplorerWidth, 1),
        (Focus::Explorer, '<') => (PaneSplit::ExplorerWidth, -1),
        (Focus::Editor, '+') => (PaneSplit::EditorHeight, 1),
        (Focus::Editor, '-') => (PaneSplit::EditorHeight, -1),
        (Focus::Editor, '>') => (PaneSplit::ExplorerWidth, -1),
        (Focus::Editor, '<') => (PaneSplit::ExplorerWidth, 1),
        (Focus::Results, '+') => (PaneSplit::EditorHeight, -1),
        (Focus::Results, '-') => (PaneSplit::EditorHeight, 1),
        (Focus::Results, '>') => (PaneSplit::ExplorerWidth, -1),
        (Focus::Results, '<') => (PaneSplit::ExplorerWidth, 1),
        _ => return None,
    };

    Some(PaneResize {
        split,
        delta: step * direction,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Overlay {
    Update(crate::model::update::UpdateOverlayState),
    Help(HelpState),
    NotificationHistory(crate::model::notification::NotificationHistoryState),
    RecordView(crate::model::record_view::RecordViewState),
    ProfileManager,
    CatalogEditor,
    ProfileAccess {
        profile_id: Uuid,
        selected: usize,
        options: Vec<ProfileAccessOption>,
    },
    ProfileGroup(crate::model::profile_group::ProfileGroupOverlay),
    ExplorerAdd(crate::model::explorer_add::ExplorerAddMenu),
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
    RelationTransactionConfirm {
        tab_id: Uuid,
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
    PageSizeSelector {
        relation: bool,
        selected: usize,
    },
    DeleteConsole {
        console_id: Uuid,
    },
    SqlEditorList(crate::model::sql_editor_list::SqlEditorListState),
    CatalogDropConfirm {
        plan: Box<crate::db::catalog_drop::CatalogDropPlan>,
        input: crate::model::text_input::TextInput,
        busy: bool,
        error: Option<String>,
    },
    CatalogEditorDestructiveConfirm {
        plan: Box<crate::db::catalog_mutation::CatalogMutationPlan>,
        input: crate::model::text_input::TextInput,
    },
    CatalogEditorDiscardConfirm {
        focus: CatalogEditorDiscardFocus,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogEditorDiscardFocus {
    KeepEditing,
    DiscardChanges,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileAccessOption {
    pub label: String,
    pub change: crate::action::ProfileAccessChange,
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
pub enum CatalogOwnerContextState {
    #[default]
    NotRequested,
    Loading(CatalogOwnerContextRequest),
    Loaded {
        connection: ConnectionIdentity,
        context: CatalogOwnerContext,
    },
    Failed {
        connection: ConnectionIdentity,
        message: String,
    },
}

impl CatalogOwnerContextState {
    pub fn context_for(&self, connection: ConnectionIdentity) -> Option<&CatalogOwnerContext> {
        match self {
            Self::Loaded {
                connection: loaded,
                context,
            } if *loaded == connection => Some(context),
            _ => None,
        }
    }

    pub fn is_loading_for(&self, connection: ConnectionIdentity) -> bool {
        matches!(self, Self::Loading(request) if request.connection == connection)
    }

    pub fn begin(&mut self, request: CatalogOwnerContextRequest) -> bool {
        if self.is_loading_for(request.connection) || self.context_for(request.connection).is_some()
        {
            return false;
        }
        *self = Self::Loading(request);
        true
    }

    pub fn finish(
        &mut self,
        request: &CatalogOwnerContextRequest,
        context: CatalogOwnerContext,
    ) -> bool {
        if !matches!(self, Self::Loading(active) if active == request) {
            return false;
        }
        *self = Self::Loaded {
            connection: request.connection,
            context,
        };
        true
    }

    pub fn fail(&mut self, request: &CatalogOwnerContextRequest, message: String) -> bool {
        if !matches!(self, Self::Loading(active) if active == request) {
            return false;
        }
        *self = Self::Failed {
            connection: request.connection,
            message,
        };
        true
    }
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
    pub mutation_capabilities: CatalogMutationCapabilities,
    pub owner_context: CatalogOwnerContextState,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ConnectionWorkspace {
    pub tabs: Vec<WorkspaceTab>,
    pub sql_editors: Vec<ConsoleRecord>,
    pub sql: Vec<(Uuid, String)>,
    pub active_tab_id: Option<Uuid>,
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
    pub placement: Option<ProfilePlacement>,
    pub connection_status: Option<ExplorerConnectionStatus>,
    pub endpoint: Option<String>,
    pub expandable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisibleExplorerViewport {
    pub pinned: Vec<VisibleCatalogNode>,
    pub rows: Vec<VisibleCatalogNode>,
    pub hidden_ancestor_count: usize,
    pub show_ancestor_indicator: bool,
    pub body_height: usize,
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
    pub find: Option<ExplorerFindState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorerSearchPhase {
    Editing,
    Confirmed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExplorerFindRow {
    pub id: ExplorerNodeId,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExplorerFindState {
    pub phase: ExplorerSearchPhase,
    pub query: TextInput,
    pub rows: Vec<ExplorerFindRow>,
    pub matches: Vec<ExplorerNodeId>,
    pub current: usize,
    pub original_selected: Option<ExplorerNodeId>,
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
    pub phase: ExplorerSearchPhase,
    pub connection: Option<ConnectionIdentity>,
    pub session_id: u64,
    pub query: TextInput,
    pub generation: u64,
    pub lifecycle: ExplorerSearchLifecycle,
    pub hits: Vec<CatalogSearchHit>,
    pub selected: usize,
    pub scroll: usize,
    pub truncated: bool,
    pub total_count: Option<usize>,
    pub located: Option<crate::db::catalog::CatalogId>,
    pub rows: Vec<ExplorerCatalogSearchRow>,
    pub match_rows: Vec<usize>,
    pub frontend_rows: Vec<crate::model::explorer::VisibleExplorerNode>,
    pub frontend_match_rows: Vec<usize>,
    pub original_selected: Option<ExplorerNodeId>,
    pub original_scroll: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExplorerCatalogSearchRow {
    pub id: ExplorerNodeId,
    pub depth: usize,
    pub label: String,
    pub kind: Option<CatalogKind>,
    pub is_match: bool,
    pub hit_index: Option<usize>,
}

impl ExplorerSearchState {
    fn new(connection: Option<ConnectionIdentity>, session_id: u64) -> Self {
        Self {
            phase: ExplorerSearchPhase::Editing,
            connection,
            session_id,
            query: TextInput::default(),
            generation: 0,
            lifecycle: ExplorerSearchLifecycle::Idle,
            hits: Vec::new(),
            selected: 0,
            scroll: 0,
            truncated: false,
            total_count: None,
            located: None,
            rows: Vec::new(),
            match_rows: Vec::new(),
            frontend_rows: Vec::new(),
            frontend_match_rows: Vec::new(),
            original_selected: None,
            original_scroll: 0,
        }
    }
}

impl ExplorerState {
    pub fn invalidate_catalog_target(&mut self, profile_id: Uuid, target: &CatalogTarget) {
        if let Some(state) = self.normalized.profiles.get_mut(&profile_id) {
            state.invalidate_catalog_target(target);
        }
    }

    pub fn apply_catalog_selection(&mut self, id: ExplorerNodeId) {
        self.normalized.selected = Some(id);
        self.normalized.ensure_selected_visible();
        self.sync_selected_index();
    }

    pub fn connection_changed(&mut self) {
        self.catalog_generation = self.catalog_generation.saturating_add(1);
        self.nodes.clear();
        self.expanded.clear();
        self.completion_index = CompletionIndex::default();
        self.selected = 0;
        self.scroll = 0;
        self.active_profile = None;
        self.search = None;
        self.find = None;
    }

    pub fn open_find(&mut self) {
        self.find = Some(ExplorerFindState {
            phase: ExplorerSearchPhase::Editing,
            query: TextInput::default(),
            rows: self
                .visible()
                .into_iter()
                .map(|row| ExplorerFindRow {
                    id: row.id,
                    label: row.label,
                })
                .collect(),
            matches: Vec::new(),
            current: 0,
            original_selected: self.normalized.selected.clone(),
        });
    }

    pub fn edit_find(&mut self, edit: impl FnOnce(&mut String)) -> bool {
        self.edit_find_input(|query| {
            query.edit_string(edit);
        })
    }

    pub fn edit_find_input(&mut self, edit: impl FnOnce(&mut TextInput)) -> bool {
        let Some(find) = self.find.as_mut() else {
            return false;
        };
        let before_value = find.query.value().to_owned();
        let before_cursor = find.query.cursor();
        edit(&mut find.query);
        if before_value == find.query.value() && before_cursor == find.query.cursor() {
            return false;
        }
        find.matches = if find.query.value().trim().is_empty() {
            Vec::new()
        } else {
            find.rows
                .iter()
                .filter(|row| search_text_matches(&row.label, find.query.value()))
                .map(|row| row.id.clone())
                .collect()
        };
        find.current = if find.matches.is_empty() {
            0
        } else {
            let start = find
                .original_selected
                .as_ref()
                .and_then(|id| find.rows.iter().position(|row| &row.id == id))
                .unwrap_or(find.rows.len().saturating_sub(1));
            find.matches
                .iter()
                .position(|id| {
                    find.rows
                        .iter()
                        .position(|row| &row.id == id)
                        .is_some_and(|position| position >= start)
                })
                .unwrap_or(0)
        };
        true
    }

    pub fn find_match_position(&self) -> (usize, usize) {
        let Some(find) = self.find.as_ref() else {
            return (0, 0);
        };
        if find.matches.is_empty() {
            (0, 0)
        } else {
            (find.current + 1, find.matches.len())
        }
    }

    pub fn confirm_find(&mut self) -> bool {
        let Some(find) = self.find.as_mut() else {
            return false;
        };
        find.phase = ExplorerSearchPhase::Confirmed;
        self.select_find_match()
    }

    pub fn move_find_match(&mut self, delta: isize) -> bool {
        let Some(find) = self.find.as_mut() else {
            return false;
        };
        if find.matches.is_empty() {
            return false;
        }
        let len = find.matches.len();
        let offset = delta.unsigned_abs() % len;
        find.current = if delta.is_negative() {
            (find.current + len - offset) % len
        } else {
            (find.current + offset) % len
        };
        self.select_find_match()
    }

    pub fn close_find(&mut self, restore_original: bool) {
        let Some(find) = self.find.take() else {
            return;
        };
        if restore_original && let Some(selected) = find.original_selected {
            self.normalized.selected = Some(selected);
        }
    }

    fn select_find_match(&mut self) -> bool {
        let Some(find) = self.find.as_ref() else {
            return false;
        };
        let Some(id) = find.matches.get(find.current) else {
            return false;
        };
        if !self.normalized.visible().iter().any(|row| &row.id == id) {
            return false;
        }
        self.normalized.selected = Some(id.clone());
        self.normalized
            .align_selected(ExplorerNodeAlignment::Middle);
        self.sync_selected_index();
        true
    }

    pub fn open_search(&mut self, connection: Option<ConnectionIdentity>, session_id: u64) {
        let mut search = ExplorerSearchState::new(connection, session_id);
        search.original_selected = self.normalized.selected.clone();
        search.original_scroll = self.normalized.scroll;
        self.search = Some(search);
    }

    pub fn edit_search(&mut self, edit: impl FnOnce(&mut String)) -> Option<u64> {
        self.edit_search_input(|query| {
            query.edit_string(edit);
        })
    }

    pub fn edit_search_input(&mut self, edit: impl FnOnce(&mut TextInput)) -> Option<u64> {
        let search = self.search.as_mut()?;
        let before_value = search.query.value().to_owned();
        let before_cursor = search.query.cursor();
        edit(&mut search.query);
        if before_value == search.query.value() && before_cursor == search.query.cursor() {
            return None;
        }
        search.generation = search.generation.saturating_add(1);
        search.selected = 0;
        search.scroll = 0;
        search.total_count = None;
        search.truncated = false;
        search.located = None;
        search.frontend_rows.clear();
        search.frontend_match_rows.clear();
        search.phase = ExplorerSearchPhase::Editing;
        search.lifecycle = if search.query.value().trim().is_empty() {
            search.hits.clear();
            ExplorerSearchLifecycle::Idle
        } else {
            ExplorerSearchLifecycle::Loading
        };
        Some(search.generation)
    }

    pub fn refresh_frontend_search(&mut self) {
        let Some(search) = self.search.as_ref() else {
            return;
        };
        if search.query.value().trim().is_empty() {
            let Some(search) = self.search.as_mut() else {
                return;
            };
            search.frontend_rows.clear();
            search.frontend_match_rows.clear();
            search.lifecycle = ExplorerSearchLifecycle::Idle;
            return;
        }
        let profile_id = search.connection.map(|connection| connection.profile_id);
        let query = search.query.value().to_owned();
        let (rows, matches) = self.normalized.filtered_search_rows(profile_id, &query);
        let selected_id = search
            .frontend_rows
            .get(search.selected)
            .map(|row| row.id.clone());
        let Some(search) = self.search.as_mut() else {
            return;
        };
        search.frontend_rows = rows;
        search.frontend_match_rows = matches;
        search.selected = selected_id
            .and_then(|id| search.frontend_rows.iter().position(|row| row.id == id))
            .unwrap_or_else(|| search.frontend_match_rows.first().copied().unwrap_or(0));
        search.scroll = search
            .scroll
            .min(search.frontend_rows.len().saturating_sub(1));
        search.lifecycle = ExplorerSearchLifecycle::Ready;
    }

    pub fn accept_search_page(&mut self, page: CatalogSearchPage) -> bool {
        let Some(search) = self.search.as_mut().filter(|search| {
            search.connection == Some(page.connection)
                && search.session_id == page.session_id
                && search.generation == page.generation
        }) else {
            return false;
        };
        let hits = page.hits;
        search.rows = catalog_search_rows(&hits);
        search.hits = hits;
        search.match_rows = search
            .rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| row.is_match.then_some(index))
            .collect();
        search.total_count = page.total_count;
        search.truncated = page.truncated;
        search.selected = search.selected.min(search.rows.len().saturating_sub(1));
        search.lifecycle = ExplorerSearchLifecycle::Ready;
        true
    }

    pub fn move_search(&mut self, delta: isize) {
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let rows = if search.frontend_rows.is_empty() {
            search.rows.len()
        } else {
            search.frontend_rows.len()
        };
        search.selected = search
            .selected
            .saturating_add_signed(delta)
            .min(rows.saturating_sub(1));
    }

    pub fn move_search_match(&mut self, delta: isize) -> bool {
        let Some(search) = self.search.as_mut() else {
            return false;
        };
        let match_rows = if search.frontend_match_rows.is_empty() {
            &search.match_rows
        } else {
            &search.frontend_match_rows
        };
        if match_rows.is_empty() {
            return false;
        }
        let current = match_rows
            .iter()
            .position(|row| *row == search.selected)
            .unwrap_or(0);
        let len = match_rows.len();
        let offset = delta.unsigned_abs() % len;
        let next = if delta.is_negative() {
            (current + len - offset) % len
        } else {
            (current + offset) % len
        };
        search.selected = match_rows[next];
        true
    }

    pub fn confirm_search(&mut self) -> bool {
        let Some(search) = self.search.as_mut() else {
            return false;
        };
        search.phase = ExplorerSearchPhase::Confirmed;
        let match_rows = if search.frontend_match_rows.is_empty() {
            &search.match_rows
        } else {
            &search.frontend_match_rows
        };
        match_rows
            .first()
            .copied()
            .map(|row| {
                search.selected = row;
            })
            .is_some()
    }

    pub fn locate_search_hit(&mut self) -> Result<bool, String> {
        let Some(search) = self.search.as_ref() else {
            return Ok(false);
        };
        if !search.frontend_rows.is_empty() {
            if !search.frontend_match_rows.contains(&search.selected) {
                return Ok(false);
            }
            let Some(ExplorerNodeId::Catalog(id)) =
                search.frontend_rows.get(search.selected).map(|row| &row.id)
            else {
                return Ok(false);
            };
            if !self
                .normalized
                .reveal_node(ExplorerNodeId::Catalog(id.clone()))
            {
                return Ok(false);
            }
            self.normalized
                .align_selected(ExplorerNodeAlignment::Middle);
            self.sync_selected_index();
            self.search = None;
            return Ok(true);
        }
        let Some(hit_index) = search
            .rows
            .get(search.selected)
            .and_then(|row| row.hit_index)
        else {
            return Ok(false);
        };
        let Some(hit) = search.hits.get(hit_index).cloned() else {
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
        self.normalized.ensure_selected_visible();
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
        self.normalized.ensure_selected_visible();
        self.scroll = self
            .normalized
            .scroll
            .min(self.normalized.visible().len().saturating_sub(1));
        self.normalized.scroll = self.scroll;
    }

    pub fn visible(&self) -> Vec<VisibleCatalogNode> {
        self.visible_rows(self.normalized.visible())
    }

    pub fn visible_search(&self) -> Vec<VisibleCatalogNode> {
        let rows = self
            .explorer_search_rows()
            .map_or_else(Vec::new, |rows| rows.to_vec());
        self.visible_rows(rows)
    }

    pub fn viewport(&self, height: usize) -> VisibleExplorerViewport {
        let viewport = self.normalized.viewport(height);
        VisibleExplorerViewport {
            pinned: self.visible_rows(viewport.pinned),
            rows: self.visible_rows(viewport.rows),
            hidden_ancestor_count: viewport.hidden_ancestor_count,
            show_ancestor_indicator: viewport.show_ancestor_indicator,
            body_height: viewport.body_height,
        }
    }

    fn visible_rows(
        &self,
        rows: Vec<crate::model::explorer::VisibleExplorerNode>,
    ) -> Vec<VisibleCatalogNode> {
        rows.into_iter()
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
                    placement,
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
                            None,
                            true,
                        )
                    }
                    ExplorerNodeId::ConnectionGroup { group_id, region } => (
                        self.normalized
                            .groups
                            .iter()
                            .find(|group| group.id == *group_id)
                            .map(|group| group.name.clone())
                            .unwrap_or_else(|| "Missing group".to_owned()),
                        Some(
                            self.normalized
                                .profiles
                                .values()
                                .filter(|profile| {
                                    profile.group_id == Some(*group_id)
                                        && match region {
                                            crate::model::explorer::ProfileRegion::Primary => {
                                                matches!(
                                                    profile.placement,
                                                    ProfilePlacement::CurrentProject
                                                        | ProfilePlacement::Global
                                                )
                                            }
                                            crate::model::explorer::ProfileRegion::Others => {
                                                profile.placement == ProfilePlacement::OtherProject
                                            }
                                        }
                                })
                                .count()
                                .to_string(),
                        ),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        true,
                    ),
                    ExplorerNodeId::Status { owner, kind } => (
                        status_label(*kind).to_owned(),
                        profile.and_then(|profile| profile.load_errors.get(owner).cloned()),
                        None,
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
                                Some(profile.placement),
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
                        None,
                        false,
                    ),
                    ExplorerNodeId::Others => (
                        "others".to_owned(),
                        Some({
                            let count = self
                                .normalized
                                .profiles
                                .values()
                                .filter(|profile| {
                                    profile.placement == ProfilePlacement::OtherProject
                                })
                                .count();
                            let active = self.active_profile.is_some_and(|profile_id| {
                                self.normalized
                                    .profiles
                                    .get(&profile_id)
                                    .is_some_and(|profile| {
                                        profile.placement == ProfilePlacement::OtherProject
                                    })
                            });
                            if active {
                                format!("{count} · 1 ACTIVE")
                            } else {
                                count.to_string()
                            }
                        }),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        true,
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
                    placement,
                }
            })
            .collect()
    }

    fn explorer_search_rows(&self) -> Option<&[crate::model::explorer::VisibleExplorerNode]> {
        self.search
            .as_ref()
            .map(|search| search.frontend_rows.as_slice())
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
        let viewport_height = self.normalized.viewport_height;
        self.normalized.move_selection(delta, viewport_height);
        self.sync_selected_index();
    }

    pub fn set_viewport_height(&mut self, height: usize) {
        self.normalized.set_viewport_height(height);
        self.sync_selected_index();
    }

    pub fn select_target(&mut self, target: ExplorerNodeTarget) {
        self.normalized.select_target(target);
        self.sync_selected_index();
    }

    pub fn scroll_nodes(&mut self, direction: isize, amount: ExplorerScrollAmount) {
        self.normalized.scroll_nodes(direction, amount);
        self.sync_selected_index();
    }

    pub fn align_selected(&mut self, alignment: ExplorerNodeAlignment) {
        self.normalized.align_selected(alignment);
        self.sync_selected_index();
    }

    pub fn select_id(&mut self, id: ExplorerNodeId) -> bool {
        let rows = self.visible();
        let Some(index) = rows.iter().position(|row| row.id == id) else {
            return false;
        };
        self.selected = index;
        self.normalized.selected = Some(id);
        self.normalized.ensure_selected_visible();
        self.scroll = self.normalized.scroll;
        true
    }

    pub fn remove_dropped_subtree(
        &mut self,
        root: &CatalogId,
    ) -> Result<Vec<CatalogId>, crate::model::explorer::ExplorerTreeError> {
        let removed = self.normalized.remove_dropped_subtree(root)?;
        self.sync_selected_index();
        Ok(removed)
    }

    pub fn toggle_selected(&mut self) -> bool {
        let Some(id) = self.selected_id().cloned() else {
            return false;
        };
        if !matches!(
            id,
            ExplorerNodeId::ConnectionGroup { .. }
                | ExplorerNodeId::Others
                | ExplorerNodeId::Profile(_)
                | ExplorerNodeId::Catalog(_)
                | ExplorerNodeId::Group { .. }
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

fn catalog_search_rows(hits: &[CatalogSearchHit]) -> Vec<ExplorerCatalogSearchRow> {
    let mut rows = Vec::new();
    let mut indexes = HashMap::new();
    for (hit_index, hit) in hits.iter().enumerate() {
        let profile_id = hit.entry.id.profile_id();
        let profile_id_node = ExplorerNodeId::Profile(profile_id);
        append_search_row(
            &mut rows,
            &mut indexes,
            ExplorerCatalogSearchRow {
                id: profile_id_node,
                depth: 0,
                label: "Profile".to_owned(),
                kind: None,
                is_match: false,
                hit_index: None,
            },
        );
        for (depth, entry) in hit
            .ancestors
            .iter()
            .chain(std::iter::once(&hit.entry))
            .enumerate()
        {
            if entry.id == hit.entry.id
                && let Some(group) = entry.parent_id.as_ref().and_then(|parent| {
                    (parent.kind == CatalogKind::Schema)
                        .then(|| search_object_group(entry.kind))
                        .flatten()
                })
            {
                append_search_row(
                    &mut rows,
                    &mut indexes,
                    ExplorerCatalogSearchRow {
                        id: ExplorerNodeId::Group {
                            parent: entry.parent_id.clone().unwrap(),
                            group,
                        },
                        depth: depth + 1,
                        label: format!("{group:?}"),
                        kind: None,
                        is_match: false,
                        hit_index: None,
                    },
                );
            }
            let id = ExplorerNodeId::Catalog(entry.id.clone());
            append_search_row(
                &mut rows,
                &mut indexes,
                ExplorerCatalogSearchRow {
                    id,
                    depth: depth + 1,
                    label: entry.qualified_name.object.clone(),
                    kind: Some(entry.kind),
                    is_match: entry.id == hit.entry.id,
                    hit_index: (entry.id == hit.entry.id).then_some(hit_index),
                },
            );
        }
    }
    rows
}

fn append_search_row(
    rows: &mut Vec<ExplorerCatalogSearchRow>,
    indexes: &mut HashMap<ExplorerNodeId, usize>,
    row: ExplorerCatalogSearchRow,
) {
    if let Some(index) = indexes.get(&row.id).copied() {
        rows[index].is_match |= row.is_match;
        if row.hit_index.is_some() {
            rows[index].hit_index = row.hit_index;
        }
        return;
    }
    indexes.insert(row.id.clone(), rows.len());
    rows.push(row);
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
            let mut detail = column.native_type.clone();
            if let OptionalMetadata::Supported(Some(default)) = &column.default_expression {
                if !detail.is_empty() {
                    detail.push(' ');
                }
                detail.push_str(&format!("DEFAULT {default}"));
            }
            (!detail.is_empty()).then_some(detail)
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

#[cfg(test)]
mod tests {
    use super::{
        CatalogOwnerContextState, Focus, PaneResize, PaneSplit, entry_detail, pane_resize,
    };
    use crate::db::catalog::{
        CatalogEntry, CatalogId, CatalogKind, CatalogMetadata, ColumnMetadata, OptionalMetadata,
    };
    use crate::db::catalog_mutation::{
        CatalogOwnerChoice, CatalogOwnerContext, CatalogOwnerContextRequest,
    };
    use crate::model::execution_target::ExecutionTarget;
    use crate::model::workspace::ConnectionIdentity;
    use uuid::Uuid;

    #[test]
    fn owner_context_requires_an_exact_request_and_deduplicates_loading() {
        let identity = ConnectionIdentity {
            profile_id: Uuid::from_u128(1),
            generation: 2,
        };
        let request = CatalogOwnerContextRequest {
            connection: identity,
            request_id: 7,
            target: ExecutionTarget {
                profile_id: identity.profile_id,
                database: "app".into(),
                schema: None,
            },
        };
        let mut state = CatalogOwnerContextState::default();
        assert!(state.begin(request.clone()));
        assert!(!state.begin(request.clone()));
        assert!(!state.finish(
            &CatalogOwnerContextRequest {
                request_id: 8,
                ..request.clone()
            },
            CatalogOwnerContext {
                current_user: "alice".into(),
                choices: vec![CatalogOwnerChoice {
                    name: "alice".into(),
                    can_login: true,
                    selectable: true,
                    is_current: true,
                }],
            },
        ));
        assert!(state.fail(&request, "failed".into()));
    }

    #[test]
    fn pane_resize_maps_focused_operations_to_split_deltas() {
        assert_eq!(
            pane_resize(Focus::Explorer, '>', 3),
            Some(PaneResize {
                split: PaneSplit::ExplorerWidth,
                delta: 3,
            })
        );
        assert_eq!(
            pane_resize(Focus::Editor, '+', 2),
            Some(PaneResize {
                split: PaneSplit::EditorHeight,
                delta: 2,
            })
        );
        assert_eq!(
            pane_resize(Focus::Results, '+', 2),
            Some(PaneResize {
                split: PaneSplit::EditorHeight,
                delta: -2,
            })
        );
        assert_eq!(
            pane_resize(Focus::Editor, '>', 2),
            Some(PaneResize {
                split: PaneSplit::ExplorerWidth,
                delta: -2,
            })
        );
    }

    #[test]
    fn pane_resize_rejects_unsupported_or_zero_operations() {
        assert_eq!(pane_resize(Focus::Explorer, '+', 1), None);
        assert_eq!(pane_resize(Focus::Editor, '+', 0), None);
    }

    fn column_detail(
        native_type: &str,
        nullable: bool,
        default: OptionalMetadata<String>,
    ) -> Option<String> {
        let id = CatalogId::new(Uuid::nil(), CatalogKind::Column, ["1"]);
        let mut metadata = ColumnMetadata::new(1, native_type, nullable);
        metadata.default_expression = default;
        entry_detail(&CatalogEntry {
            id,
            parent_id: None,
            kind: CatalogKind::Column,
            native_kind: native_type.to_owned(),
            qualified_name: crate::db::catalog::QualifiedName {
                database: None,
                schema: None,
                object: "column".to_owned(),
            },
            comment: OptionalMetadata::Unsupported,
            metadata: CatalogMetadata::Column(metadata),
            expandable: false,
            relation_id: None,
        })
    }

    #[test]
    fn column_detail_shows_type_and_default_without_nullability_flag() {
        assert_eq!(
            column_detail(
                "INTEGER",
                false,
                OptionalMetadata::Supported(Some("0".to_owned()))
            ),
            Some("INTEGER DEFAULT 0".to_owned())
        );
    }

    #[test]
    fn column_detail_shows_type_without_default() {
        assert_eq!(
            column_detail("TEXT", false, OptionalMetadata::Supported(None)),
            Some("TEXT".to_owned())
        );
    }

    #[test]
    fn column_detail_preserves_parentheses_in_default_expression() {
        assert_eq!(
            column_detail(
                "INTEGER",
                true,
                OptionalMetadata::Supported(Some("(nextval('seq'))".to_owned()))
            ),
            Some("INTEGER DEFAULT (nextval('seq'))".to_owned())
        );
    }
}
