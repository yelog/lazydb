use std::{
    collections::{BTreeSet, HashMap, HashSet},
    time::{SystemTime, UNIX_EPOCH},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use uuid::Uuid;

use crate::{
    action::{Action, Command},
    cli::ConfirmationPolicy,
    db::{
        ErrorCategory,
        catalog::{
            CatalogMetadata, CatalogPage, CatalogRequest, CatalogRequestKey, CatalogTarget,
            MAX_CATALOG_PAGE_SIZE,
        },
        query::ColumnMeta,
        value::CellValue,
    },
    editor::{EditorEffect, EditorError, EditorWorkspace},
    model::{
        data_query::{
            DataQueryCandidate, DataQueryCapability, DataQueryCompletion, DataQueryInput,
            DataQueryOptions,
        },
        editor::{EditorMode, EditorRenderSnapshot, EditorViewport},
        execution_target::ExecutionTarget,
        explorer::{
            CatalogGroupState, ExplorerConnectionStatus, ExplorerLoadState, ExplorerNodeId,
            ExplorerOwnerId, ProfileProvenance, owner_for_target,
        },
        profile_manager::{
            ProfileCatalogDiscovery, ProfileField, ProfileManagerPage, ProfileManagerState,
            ProfileOperation,
        },
        relation::{
            RelationDescriptor, RelationKey, RelationLoad, RelationRequest, RelationRequestKind,
            RelationSnapshot, RelationTab, RelationView, automatic_relation_column_widths,
        },
        relation_edit::{
            CellEditorState, PendingMutationHistory, RelationEditSession, RelationGridMode,
            RelationMutationHistory,
        },
        tab::{
            CompletionPopup, ConsoleRecord, ConsoleTab, DataGridState, DerivedResultState,
            ExecutionResult, LastExecution, OutputEntry, OutputKind, ResultView, WorkspaceTab,
        },
        transaction::{
            self, DeferredIntent, DeferredIntentQueue, DeferredTransactionPrompt, TransactionEvent,
            TransactionExitChoice, TransactionMode, TransactionState,
        },
        workspace::{
            ConnectionIdentity, ConnectionState, ConnectionStatus, ConnectionWorkspace,
            ExecutionConfirmFocus, ExplorerState, Focus, ManualCancelFocus, Overlay, QueryStatus,
        },
    },
    persistence::workspace::{PersistedConsole, WorkspaceSnapshot},
    profile::{ConnectionProfile, DatabaseKind},
    sql::{self, CompletionScheduleKey, ScopeSource, SqlDialect},
};

fn pending_relation_request<T>(load: &RelationLoad<T>) -> Option<RelationRequest> {
    match load {
        RelationLoad::Loading { request, .. } => Some(request.clone()),
        _ => None,
    }
}

fn cancel_relation_load<T: Clone>(load: &RelationLoad<T>) -> RelationLoad<T> {
    match load {
        RelationLoad::Loading { previous, .. } => RelationLoad::Cancelled {
            previous: previous.clone(),
        },
        other => other.clone(),
    }
}

fn cancel_pending_relation<T: Clone>(load: &mut RelationLoad<T>) -> Option<RelationRequest> {
    let pending = pending_relation_request(load);
    if let RelationLoad::Loading { previous, .. } = load {
        *load = RelationLoad::Cancelled {
            previous: previous.clone(),
        };
    }
    pending
}

fn data_query_identifier(value: &str, cursor: usize) -> Option<(crate::sql::TextRange, String)> {
    let characters = value.chars().collect::<Vec<_>>();
    let cursor = cursor.min(characters.len());
    let mut quote = None;
    let mut index = 0;
    while index < cursor {
        let character = characters[index];
        if let Some(active) = quote {
            if character == active {
                if index + 1 < cursor && characters[index + 1] == active {
                    index += 1;
                } else {
                    quote = None;
                }
            }
        } else if matches!(character, '\'' | '"' | '`') {
            quote = Some(character);
        }
        index += 1;
    }
    if quote.is_some() {
        return None;
    }
    let mut start = cursor;
    while start > 0 && is_data_query_identifier_character(characters[start - 1]) {
        start -= 1;
    }
    let mut end = cursor;
    while end < characters.len() && is_data_query_identifier_character(characters[end]) {
        end += 1;
    }
    (start < cursor).then(|| {
        (
            crate::sql::TextRange::new(start, end),
            characters[start..cursor].iter().collect(),
        )
    })
}

fn is_data_query_identifier_character(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '_' | '-')
}

pub struct App {
    pub profiles: Vec<ConnectionProfile>,
    pub connection: ConnectionState,
    pub active_workspace_profile: Option<Uuid>,
    pub explorer: ExplorerState,
    pub tabs: Vec<WorkspaceTab>,
    pub sql_editors: Vec<ConsoleRecord>,
    pub active_tab: usize,
    pub focus: Focus,
    pub overlay: Option<Overlay>,
    pub profile_manager: Option<ProfileManagerState>,
    pub system_credential_availability: crate::persistence::secrets::SecretStoreAvailability,
    pub should_quit: bool,
    next_console_number: usize,
    connection_request_generation: u64,
    connection_terminal_generation: u64,
    next_search_session: u64,
    editor: EditorWorkspace,
    confirmation_policy: ConfirmationPolicy,
    deferred: DeferredIntentQueue,
    resolving_deferred: Option<DeferredTransactionPrompt>,
    pending_target_console: Option<Uuid>,
    pub sql_editor_list: crate::model::sql_editor_list::SqlEditorListState,
    workspaces: HashMap<Uuid, ConnectionWorkspace>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CatalogRequestIntent {
    Automatic,
    Continuation,
    Explicit,
    Refresh,
    Completion,
}

#[derive(Clone, Copy)]
enum CompletionAfterEdit {
    Schedule,
    Suppress,
}

impl App {
    pub(crate) fn is_active_relation_tab(&self) -> bool {
        matches!(
            self.tabs.get(self.active_tab),
            Some(WorkspaceTab::Relation(_))
        )
    }

    pub fn new(profiles: Vec<ConnectionProfile>) -> Self {
        let persisted = profiles.iter().map(|profile| profile.id).collect();
        Self::with_profiles(profiles, persisted, ConfirmationPolicy::RiskyOnly)
    }

    pub fn with_startup_profiles(
        profiles: Vec<ConnectionProfile>,
        persisted: HashSet<Uuid>,
    ) -> Self {
        Self::with_profiles(profiles, persisted, ConfirmationPolicy::RiskyOnly)
    }

    pub fn with_confirmation_policy(
        profiles: Vec<ConnectionProfile>,
        confirmation_policy: ConfirmationPolicy,
    ) -> Self {
        let persisted = profiles.iter().map(|profile| profile.id).collect();
        Self::with_profiles(profiles, persisted, confirmation_policy)
    }

    pub fn with_startup_profiles_and_confirmation_policy(
        profiles: Vec<ConnectionProfile>,
        persisted: HashSet<Uuid>,
        confirmation_policy: ConfirmationPolicy,
    ) -> Self {
        Self::with_profiles(profiles, persisted, confirmation_policy)
    }

    fn with_profiles(
        profiles: Vec<ConnectionProfile>,
        persisted: HashSet<Uuid>,
        confirmation_policy: ConfirmationPolicy,
    ) -> Self {
        let mut editor = EditorWorkspace::new();
        let mut explorer = ExplorerState::default();
        for profile in &profiles {
            add_explorer_profile(
                &mut explorer,
                profile,
                if persisted.contains(&profile.id) {
                    ProfileProvenance::Saved
                } else {
                    ProfileProvenance::Session
                },
            );
        }
        let (tabs, sql_editors) = if profiles.is_empty() {
            let tab = ConsoleTab::new("console");
            let tab_id = tab.id;
            editor.open_console(tab_id, "");
            (
                vec![WorkspaceTab::Sql(tab)],
                vec![ConsoleRecord {
                    id: tab_id,
                    name: "console".into(),
                    execution_target: None,
                    transaction_mode: TransactionMode::Auto,
                    open: true,
                }],
            )
        } else {
            (Vec::new(), Vec::new())
        };

        Self {
            profiles,
            connection: ConnectionState::default(),
            active_workspace_profile: None,
            explorer,
            tabs,
            sql_editors,
            active_tab: 0,
            focus: Focus::Editor,
            overlay: None,
            profile_manager: None,
            system_credential_availability:
                crate::persistence::secrets::SecretStoreAvailability::Unavailable,
            should_quit: false,
            next_console_number: 2,
            connection_request_generation: 0,
            connection_terminal_generation: 0,
            next_search_session: 0,
            editor,
            confirmation_policy,
            deferred: DeferredIntentQueue::default(),
            resolving_deferred: None,
            pending_target_console: None,
            sql_editor_list: Default::default(),
            workspaces: HashMap::new(),
        }
    }

    fn active_tab_id(&self) -> Option<Uuid> {
        self.tabs.get(self.active_tab).map(WorkspaceTab::id)
    }

    fn take_active_workspace(&mut self) -> Option<(Uuid, ConnectionWorkspace)> {
        let profile_id = self.active_workspace_profile?;
        let active_tab_id = self.active_tab_id();
        let workspace = ConnectionWorkspace {
            tabs: std::mem::take(&mut self.tabs),
            sql_editors: std::mem::take(&mut self.sql_editors),
            active_tab_id,
        };
        self.active_workspace_profile = None;
        self.active_tab = 0;
        Some((profile_id, workspace))
    }

    fn install_workspace(&mut self, profile_id: Uuid, workspace: ConnectionWorkspace) {
        self.tabs = workspace.tabs;
        self.sql_editors = workspace.sql_editors;
        self.active_tab = workspace
            .active_tab_id
            .and_then(|id| self.tabs.iter().position(|tab| tab.id() == id))
            .unwrap_or(0)
            .min(self.tabs.len().saturating_sub(1));
        self.active_workspace_profile = Some(profile_id);
        self.normalize_focus();
    }

    fn empty_workspace_for(
        &mut self,
        _profile_id: Uuid,
        target: ExecutionTarget,
    ) -> ConnectionWorkspace {
        let mut tab = ConsoleTab::new("console");
        tab.execution_target = Some(target.clone());
        let id = tab.id;
        ConnectionWorkspace {
            tabs: vec![WorkspaceTab::Sql(tab)],
            sql_editors: vec![ConsoleRecord {
                id,
                name: "console".into(),
                execution_target: Some(target),
                transaction_mode: TransactionMode::Auto,
                open: true,
            }],
            active_tab_id: Some(id),
        }
    }

    pub fn set_confirmation_policy(&mut self, policy: ConfirmationPolicy) {
        self.confirmation_policy = policy;
    }

    pub fn active_console(&self) -> &ConsoleTab {
        self.tabs[self.active_tab]
            .as_console()
            .expect("active tab is not a SQL console")
    }

    pub fn active_console_mut(&mut self) -> &mut ConsoleTab {
        self.tabs
            .get_mut(self.active_tab)
            .and_then(WorkspaceTab::as_console_mut)
            .expect("active tab is not a SQL console")
    }

    pub fn active_console_opt(&self) -> Option<&ConsoleTab> {
        self.tabs
            .get(self.active_tab)
            .and_then(WorkspaceTab::as_console)
    }

    pub fn active_console_opt_mut(&mut self) -> Option<&mut ConsoleTab> {
        self.tabs
            .get_mut(self.active_tab)
            .and_then(WorkspaceTab::as_console_mut)
    }

    fn normalize_focus(&mut self) {
        if self.active_console_opt().is_none() && self.focus == Focus::Editor {
            self.focus = Focus::Results;
        }
    }

    pub fn active_editor_text(&self) -> Result<String, EditorError> {
        self.active_console_opt()
            .map_or_else(|| Ok(String::new()), |tab| self.editor_text(tab.id))
    }

    pub fn editor_text(&self, tab_id: Uuid) -> Result<String, EditorError> {
        self.editor.text(tab_id)
    }

    pub fn active_editor_revision(&self) -> u64 {
        self.active_console_opt()
            .and_then(|tab| self.editor.revision(tab.id).ok())
            .unwrap_or_default()
    }

    pub fn active_editor_mode(&self) -> EditorMode {
        self.active_console_opt()
            .and_then(|tab| self.editor.mode(tab.id).ok())
            .unwrap_or(EditorMode::Normal)
    }

    pub fn active_editor_viewport(&self) -> Result<EditorViewport, EditorError> {
        self.active_console_opt().map_or_else(
            || Err(EditorError::MissingSession(Uuid::nil())),
            |tab| self.editor.viewport(tab.id),
        )
    }

    pub fn active_editor_render_snapshot(
        &self,
        viewport: EditorViewport,
    ) -> Result<EditorRenderSnapshot, EditorError> {
        let Some(tab) = self.active_console_opt() else {
            return Err(EditorError::MissingSession(Uuid::nil()));
        };
        let text = self.active_editor_text().unwrap_or_default();
        let statement = self
            .editor
            .position(tab.id)
            .ok()
            .and_then(|position| {
                let cursor = cursor_byte(&text, position.line, position.column);
                sql::resolve_scope(&text, cursor, None, self.sql_dialect())
            })
            .map(|scope| match scope.source {
                sql::ScopeSource::Contiguous(range) => range,
                sql::ScopeSource::Block(_) => unreachable!(),
            });
        self.editor.render_snapshot_with_dialect_and_statement(
            tab.id,
            viewport,
            self.sql_dialect(),
            statement,
        )
    }

    pub fn active_profile(&self) -> Option<&ConnectionProfile> {
        let profile_id = self.connection.profile_id?;
        self.profiles
            .iter()
            .find(|profile| profile.id == profile_id)
    }

    pub fn workspace_snapshot(&self) -> WorkspaceSnapshot {
        let consoles = self
            .sql_editors
            .iter()
            .map(|record| {
                let open_tab = self
                    .tabs
                    .iter()
                    .find(|tab| tab.id() == record.id)
                    .and_then(WorkspaceTab::as_console);
                PersistedConsole {
                    id: record.id,
                    name: open_tab.map_or_else(|| record.name.clone(), |tab| tab.name.clone()),
                    sql_file: format!("{}.sql", record.id).into(),
                    target: open_tab
                        .and_then(|tab| tab.execution_target.clone())
                        .or_else(|| record.execution_target.clone()),
                    transaction_mode: open_tab
                        .map_or(record.transaction_mode, |tab| tab.transaction_mode),
                    open: record.open,
                }
            })
            .collect::<Vec<_>>();
        let sql = consoles
            .iter()
            .map(|console| (console.id, self.editor_text(console.id).unwrap_or_default()))
            .collect();
        let active_console = self
            .active_console_opt()
            .map(|tab| tab.id)
            .or_else(|| {
                consoles
                    .iter()
                    .find(|console| console.open)
                    .map(|console| console.id)
            })
            .unwrap_or(Uuid::nil());
        WorkspaceSnapshot {
            active_console,
            consoles,
            sql,
        }
    }

    pub fn restore_workspace(
        &mut self,
        snapshot: WorkspaceSnapshot,
        selected_profile: Option<Uuid>,
    ) {
        if snapshot.consoles.is_empty() {
            return;
        }
        let selected =
            selected_profile.and_then(|id| self.profiles.iter().find(|profile| profile.id == id));
        self.tabs.clear();
        self.sql_editors.clear();
        self.editor = EditorWorkspace::new();
        for persisted in snapshot.consoles {
            let open = persisted.open;
            let mut tab = ConsoleTab::new(persisted.name);
            tab.id = persisted.id;
            tab.transaction_mode = persisted.transaction_mode;
            tab.execution_target = persisted.target.filter(|target| {
                self.profiles
                    .iter()
                    .find(|profile| profile.id == target.profile_id)
                    .is_some_and(|profile| {
                        target.is_valid(profile)
                            && (profile.kind != DatabaseKind::Sqlite
                                || target
                                    .schema
                                    .as_deref()
                                    .is_some_and(|schema| matches!(schema, "main" | "temp")))
                    })
            });
            if tab.execution_target.is_none() {
                tab.execution_target = selected.map(ExecutionTarget::from_profile);
            }
            let text = snapshot
                .sql
                .iter()
                .find(|(id, _)| *id == tab.id)
                .map(|(_, text)| text.as_str())
                .unwrap_or_default();
            self.editor.open_console(tab.id, text);
            self.sql_editors.push(ConsoleRecord {
                id: tab.id,
                name: tab.name.clone(),
                execution_target: tab.execution_target.clone(),
                transaction_mode: tab.transaction_mode,
                open,
            });
            if open {
                self.tabs.push(WorkspaceTab::Sql(tab));
            }
        }
        if self.tabs.is_empty() {
            self.create_sql_editor_named("console".to_owned());
        }
        self.active_tab = self
            .tabs
            .iter()
            .position(|tab| tab.id() == snapshot.active_console)
            .unwrap_or(0);
        self.next_console_number = self.tabs.len().saturating_add(1);
        self.focus = Focus::Editor;
    }

    fn persist_workspace_command(&self) -> Command {
        Command::PersistWorkspace(self.workspace_snapshot())
    }

    fn execute_help_shortcut(&mut self, id: crate::help::HelpShortcutId) -> Vec<Command> {
        let Some(selected) = self.help_selected_id() else {
            return Vec::new();
        };
        if selected != id {
            return Vec::new();
        }
        self.overlay = None;
        use crate::help::HelpShortcutId as Id;
        let editor_key = |code| Action::EditorKey(KeyEvent::new(code, KeyModifiers::NONE));
        let editor_control_key =
            |code| Action::EditorKey(KeyEvent::new(code, KeyModifiers::CONTROL));
        let actions = match id {
            Id::FocusExplorer => vec![Action::Focus(Focus::Explorer)],
            Id::FocusResults => vec![Action::Focus(Focus::Results)],
            Id::FocusEditorFromK | Id::FocusEditorFromL => vec![Action::Focus(Focus::Editor)],
            Id::PreviousTab => vec![Action::PreviousTab],
            Id::NextTab => vec![Action::NextTab],
            Id::NewConsole => vec![Action::NewConsole],
            Id::GotoSqlConsole => vec![Action::GotoSqlConsole],
            Id::CloseTab => vec![Action::CloseActiveTab],
            Id::DeleteConsole => vec![Action::RequestDeleteActiveConsole],
            Id::OpenSqlEditors => vec![Action::OpenSqlEditorList],
            Id::ExplorerMoveDown => vec![Action::ExplorerMove(1)],
            Id::ExplorerMoveUp => vec![Action::ExplorerMove(-1)],
            Id::ExplorerFirst => vec![Action::ExplorerSelectTarget(
                crate::model::explorer::ExplorerNodeTarget::First,
            )],
            Id::ExplorerLast => vec![Action::ExplorerSelectTarget(
                crate::model::explorer::ExplorerNodeTarget::Last,
            )],
            Id::ExplorerViewTop => vec![Action::ExplorerSelectTarget(
                crate::model::explorer::ExplorerNodeTarget::ViewTop,
            )],
            Id::ExplorerViewMiddle => vec![Action::ExplorerSelectTarget(
                crate::model::explorer::ExplorerNodeTarget::ViewMiddle,
            )],
            Id::ExplorerViewBottom => vec![Action::ExplorerSelectTarget(
                crate::model::explorer::ExplorerNodeTarget::ViewBottom,
            )],
            Id::ExplorerHalfPageDown => vec![Action::ExplorerScrollNodes {
                direction: 1,
                amount: crate::model::explorer::ExplorerScrollAmount::HalfPage,
            }],
            Id::ExplorerHalfPageUp => vec![Action::ExplorerScrollNodes {
                direction: -1,
                amount: crate::model::explorer::ExplorerScrollAmount::HalfPage,
            }],
            Id::ExplorerPageDown => vec![Action::ExplorerScrollNodes {
                direction: 1,
                amount: crate::model::explorer::ExplorerScrollAmount::Page,
            }],
            Id::ExplorerPageUp => vec![Action::ExplorerScrollNodes {
                direction: -1,
                amount: crate::model::explorer::ExplorerScrollAmount::Page,
            }],
            Id::ExplorerAlignMiddle => vec![Action::ExplorerAlignSelected(
                crate::model::explorer::ExplorerNodeAlignment::Middle,
            )],
            Id::ExplorerAlignTop => vec![Action::ExplorerAlignSelected(
                crate::model::explorer::ExplorerNodeAlignment::Top,
            )],
            Id::ExplorerAlignBottom => vec![Action::ExplorerAlignSelected(
                crate::model::explorer::ExplorerNodeAlignment::Bottom,
            )],
            Id::ExplorerExpand => vec![Action::ExplorerExpand],
            Id::ExplorerCollapse => vec![Action::ExplorerCollapse],
            Id::ExplorerToggle => vec![Action::ExplorerToggle],
            Id::ExplorerActivate => vec![Action::ExplorerOpenSelected],
            Id::ExplorerNewProfile => vec![Action::ProfileStartNew],
            Id::ExplorerEditProfile => self
                .explorer
                .normalized
                .selected
                .as_ref()
                .and_then(|node| node.profile_id())
                .map(|profile_id| vec![Action::ProfileStartEdit { profile_id }])
                .unwrap_or_default(),
            Id::ExplorerDeleteProfile => self
                .explorer
                .normalized
                .selected
                .as_ref()
                .and_then(|node| node.profile_id())
                .map(|profile_id| vec![Action::ProfileRequestDelete { profile_id }])
                .unwrap_or_default(),
            Id::ExplorerConnect => self
                .explorer
                .normalized
                .selected
                .as_ref()
                .and_then(|node| node.profile_id())
                .map(|profile_id| vec![Action::RequestProfileConnect { profile_id }])
                .unwrap_or_default(),
            Id::ExplorerDisconnect => self
                .explorer
                .normalized
                .selected
                .as_ref()
                .and_then(|node| node.profile_id())
                .map(|profile_id| vec![Action::RequestProfileDisconnect { profile_id }])
                .unwrap_or_default(),
            Id::ExplorerRefresh => vec![Action::ExplorerRefresh],
            Id::ExplorerPreview => vec![Action::OpenSelectedRelation {
                view: RelationView::Data,
            }],
            Id::ExplorerDdl => vec![Action::OpenSelectedRelation {
                view: RelationView::Ddl,
            }],
            Id::EditorInsert => vec![editor_key(KeyCode::Char('i'))],
            Id::EditorNormal => vec![editor_key(KeyCode::Esc)],
            Id::EditorUndo => vec![editor_key(KeyCode::Char('u'))],
            Id::EditorRedo => vec![editor_control_key(KeyCode::Char('r'))],
            Id::EditorRun => vec![Action::RunActiveSql],
            Id::EditorFormat => vec![
                editor_key(KeyCode::Char(' ')),
                editor_key(KeyCode::Char('f')),
            ],
            Id::ToggleTransaction => vec![
                editor_key(KeyCode::Char(' ')),
                editor_key(KeyCode::Char('t')),
                editor_key(KeyCode::Char('t')),
            ],
            Id::CommitTransaction => vec![
                editor_key(KeyCode::Char(' ')),
                editor_key(KeyCode::Char('t')),
                editor_key(KeyCode::Char('c')),
            ],
            Id::RollbackTransaction => vec![
                editor_key(KeyCode::Char(' ')),
                editor_key(KeyCode::Char('t')),
                editor_key(KeyCode::Char('r')),
            ],
            Id::OpenTargetSelector => vec![Action::OpenTargetSelector],
            Id::ResultsMoveLeft => vec![Action::GridMove {
                rows: 0,
                columns: -1,
            }],
            Id::ResultsMoveDown => vec![Action::GridMove {
                rows: 1,
                columns: 0,
            }],
            Id::ResultsMoveUp => vec![Action::GridMove {
                rows: -1,
                columns: 0,
            }],
            Id::ResultsMoveRight => vec![Action::GridMove {
                rows: 0,
                columns: 1,
            }],
            Id::ResultsFirstRow => vec![Action::GridSelectRow(
                crate::model::tab::GridRowTarget::First,
            )],
            Id::ResultsLastRow => vec![Action::GridSelectRow(
                crate::model::tab::GridRowTarget::Last,
            )],
            Id::ResultsViewTop => vec![Action::GridSelectRow(
                crate::model::tab::GridRowTarget::ViewTop,
            )],
            Id::ResultsViewMiddle => vec![Action::GridSelectRow(
                crate::model::tab::GridRowTarget::ViewMiddle,
            )],
            Id::ResultsViewBottom => vec![Action::GridSelectRow(
                crate::model::tab::GridRowTarget::ViewBottom,
            )],
            Id::ResultsHalfPageDown => vec![Action::GridScrollRows {
                direction: 1,
                amount: crate::model::tab::GridScrollAmount::HalfPage,
            }],
            Id::ResultsHalfPageUp => vec![Action::GridScrollRows {
                direction: -1,
                amount: crate::model::tab::GridScrollAmount::HalfPage,
            }],
            Id::ResultsPageDown => vec![Action::GridScrollRows {
                direction: 1,
                amount: crate::model::tab::GridScrollAmount::Page,
            }],
            Id::ResultsPageUp => vec![Action::GridScrollRows {
                direction: -1,
                amount: crate::model::tab::GridScrollAmount::Page,
            }],
            Id::ResultsAlignMiddle => vec![Action::GridAlignSelectedRow(
                crate::model::tab::GridRowAlignment::Middle,
            )],
            Id::ResultsAlignTop => vec![Action::GridAlignSelectedRow(
                crate::model::tab::GridRowAlignment::Top,
            )],
            Id::ResultsAlignBottom => vec![Action::GridAlignSelectedRow(
                crate::model::tab::GridRowAlignment::Bottom,
            )],
            Id::ResultsOpenRecordView => vec![Action::OpenRecordView],
            Id::ResultsToggleView => vec![Action::ToggleResultView],
            Id::RelationWhere => vec![Action::FocusRelationQueryInput(
                crate::model::relation::RelationQueryInput::Where,
            )],
            Id::RelationOrderBy => vec![Action::FocusRelationQueryInput(
                crate::model::relation::RelationQueryInput::OrderBy,
            )],
            Id::RelationApplyInputs => vec![Action::SubmitRelationQuery],
            Id::RelationResizeLeft => vec![Action::ResizeRelationColumn(-1)],
            Id::RelationResizeRight => vec![Action::ResizeRelationColumn(1)],
            Id::RelationResetWidth => vec![Action::ResetRelationColumnWidth],
            Id::RelationRefresh => vec![Action::RefreshActiveRelation],
        };
        actions
            .into_iter()
            .flat_map(|action| self.update(action))
            .collect()
    }

    pub fn help_selected_id(&self) -> Option<crate::help::HelpShortcutId> {
        let relation_data = matches!(
            self.tabs.get(self.active_tab),
            Some(WorkspaceTab::Relation(tab))
                if tab.view == RelationView::Data
        );
        match &self.overlay {
            Some(Overlay::Help(help)) => help.selected_id(relation_data),
            _ => None,
        }
    }

    pub fn update(&mut self, action: Action) -> Vec<Command> {
        if self.active_console_opt().is_none()
            && !(self.is_active_relation_tab()
                && matches!(
                    action,
                    Action::GridMove { .. }
                        | Action::GridSelectRow(_)
                        | Action::GridScrollRows { .. }
                        | Action::GridAlignSelectedRow(_)
                        | Action::GridViewportChanged(_)
                        | Action::GridSelect { .. }
                        | Action::GridResizeColumn(_)
                        | Action::GridResetColumnWidth
                        | Action::GridStartColumnResize { .. }
                        | Action::GridSetColumnWidth { .. }
                        | Action::GridEndColumnResize
                        | Action::GridSetColumnOffset { .. }
                        | Action::OpenRecordView
                        | Action::RecordViewMoveFields(_)
                        | Action::RecordViewJumpFirstField
                        | Action::RecordViewJumpLastField
                        | Action::RecordViewMoveRow(_)
                        | Action::CloseRecordView
                        | Action::RecordViewViewportChanged { .. }
                        | Action::DdlScroll { .. }
                        | Action::DdlScrollToStart
                        | Action::DdlScrollToEnd
                        | Action::SetDdlViewportMetrics { .. }
                        | Action::FocusDataQueryInput(_)
                        | Action::DataQueryInsert(_)
                        | Action::DataQueryBackspace
                        | Action::DataQueryDeletePreviousWord
                        | Action::DataQueryDeleteToStart
                        | Action::DataQueryDelete
                        | Action::DataQueryMoveLeft
                        | Action::DataQueryMoveRight
                        | Action::DataQueryMoveHome
                        | Action::DataQueryMoveEnd
                        | Action::DataQueryClear
                        | Action::DataQueryCompletionNext
                        | Action::DataQueryCompletionPrevious
                        | Action::DataQueryCompletionAccept
                        | Action::DataQueryCompletionDismiss
                        | Action::SubmitDataQuery
                        | Action::CancelDataQueryInput
                        | Action::FocusRelationQueryInput(_)
                        | Action::RelationQueryInsert(_)
                        | Action::RelationQueryBackspace
                        | Action::RelationQueryDelete
                        | Action::RelationQueryMoveLeft
                        | Action::RelationQueryMoveRight
                        | Action::RelationQueryMoveHome
                        | Action::RelationQueryMoveEnd
                        | Action::RelationQueryClear
                        | Action::SubmitRelationQuery
                        | Action::CancelRelationQueryInput
                        | Action::RelationEditCell
                        | Action::RelationEditInsert(_)
                        | Action::RelationEditBackspace
                        | Action::RelationEditDeletePreviousWord
                        | Action::RelationEditDeleteToStart
                        | Action::RelationEditDelete
                        | Action::RelationEditMoveLeft
                        | Action::RelationEditMoveRight
                        | Action::RelationEditMoveHome
                        | Action::RelationEditMoveEnd
                        | Action::RelationEditConfirm
                        | Action::RelationEditCancel
                        | Action::ExplorerFindOpen
                        | Action::ExplorerFindInsert(_)
                        | Action::ExplorerFindBackspace
                        | Action::ExplorerFindClear
                        | Action::ExplorerFindConfirm
                        | Action::ExplorerFindNext
                        | Action::ExplorerFindPrevious
                        | Action::ExplorerFindClose
                        | Action::ExplorerSearchOpen
                        | Action::ExplorerSearchInsert(_)
                        | Action::ExplorerSearchBackspace
                        | Action::ExplorerSearchClear
                        | Action::ExplorerSearchMove(_)
                        | Action::ExplorerSearchNext
                        | Action::ExplorerSearchPrevious
                        | Action::ExplorerSearchLocate
                        | Action::ExplorerSearchClose
                        | Action::ExplorerSearchRetry
                        | Action::CloseActiveTab
                        | Action::RequestDeleteActiveConsole
                        | Action::ConfirmDeleteConsole
                        | Action::CancelDeleteConsole
                        | Action::OpenSqlEditorList
                        | Action::SqlEditorListInsert(_)
                        | Action::SqlEditorListBackspace
                        | Action::SqlEditorListMove(_)
                        | Action::ActivateSqlEditor(_)
                ))
            && matches!(
                action,
                Action::EditorKey(_)
                    | Action::EditorPaste(_)
                    | Action::EditorViewportChanged(_)
                    | Action::EditorScroll { .. }
                    | Action::ReplaceEditor(_)
                    | Action::CompletionDue(_)
                    | Action::RunActiveSql
                    | Action::RunAllSql
                    | Action::CancelActiveQuery
                    | Action::ConfirmManualCancellation
                    | Action::SetTransactionMode(_)
                    | Action::CommitTransaction
                    | Action::RollbackTransaction
                    | Action::ClearTransactionOutcome
                    | Action::GridMove { .. }
                    | Action::GridSelectRow(_)
                    | Action::GridScrollRows { .. }
                    | Action::GridAlignSelectedRow(_)
                    | Action::GridViewportChanged(_)
                    | Action::GridSelect { .. }
                    | Action::GridResizeColumn(_)
                    | Action::GridResetColumnWidth
                    | Action::GridStartColumnResize { .. }
                    | Action::GridSetColumnWidth { .. }
                    | Action::GridEndColumnResize
                    | Action::GridSetColumnOffset { .. }
                    | Action::OpenRecordView
                    | Action::RecordViewMoveFields(_)
                    | Action::RecordViewJumpFirstField
                    | Action::RecordViewJumpLastField
                    | Action::RecordViewMoveRow(_)
                    | Action::CloseRecordView
                    | Action::RecordViewViewportChanged { .. }
                    | Action::CompletionExplicit
                    | Action::CompletionNext
                    | Action::CompletionPrevious
                    | Action::CompletionAccept
                    | Action::CompletionDismiss
                    | Action::ToggleResultView
                    | Action::ConfirmTransactionExitChoice(_)
                    | Action::OpenTargetSelector
                    | Action::MoveTargetSelector(_)
                    | Action::ConfirmTargetSelector
                    | Action::CancelTargetSelector
                    | Action::ConfirmClearTransactionOutcome
                    | Action::CancelClearTransactionOutcome
                    | Action::OpenSqlEditorList
                    | Action::SqlEditorListInsert(_)
                    | Action::SqlEditorListBackspace
                    | Action::SqlEditorListMove(_)
                    | Action::ActivateSqlEditor(_)
                    | Action::RequestDeleteActiveConsole
                    | Action::ConfirmDeleteConsole
                    | Action::CancelDeleteConsole
            )
        {
            return Vec::new();
        }
        match action {
            Action::NewConsole => {
                let name = format!("console_{}", self.next_console_number);
                self.next_console_number += 1;
                let mut tab = ConsoleTab::new(name.clone());
                tab.execution_target = self.active_profile().map(ExecutionTarget::from_profile);
                let id = tab.id;
                self.tabs.push(WorkspaceTab::Sql(tab));
                self.editor.open_console(id, "");
                self.sql_editors.push(ConsoleRecord {
                    id,
                    name: name.clone(),
                    execution_target: self
                        .tabs
                        .last()
                        .and_then(WorkspaceTab::as_console)
                        .and_then(|tab| tab.execution_target.clone()),
                    transaction_mode: TransactionMode::Auto,
                    open: true,
                });
                self.active_tab = self.tabs.len() - 1;
                self.focus = Focus::Editor;
                vec![self.persist_workspace_command()]
            }
            Action::CloseActiveTab => {
                if !self.tabs.is_empty() {
                    let was_console = self.tabs[self.active_tab].as_console().is_some();
                    let id = self.tabs[self.active_tab].id();
                    if self.active_console_opt().is_some() && self.transaction_needs_exit(id) {
                        return self.defer_intent(DeferredIntent::CloseConsole, [id]);
                    }
                    if let Some(tab) = self.tabs[self.active_tab].as_console()
                        && let Some(record) =
                            self.sql_editors.iter_mut().find(|record| record.id == id)
                    {
                        record.name = tab.name.clone();
                        record.execution_target = tab.execution_target.clone();
                        record.transaction_mode = tab.transaction_mode;
                    }
                    let cancel = match self.tabs.get_mut(self.active_tab) {
                        Some(WorkspaceTab::Relation(tab)) => {
                            let requests = [
                                pending_relation_request(&tab.data),
                                pending_relation_request(&tab.ddl),
                            ];
                            tab.data = cancel_relation_load(&tab.data);
                            tab.ddl = cancel_relation_load(&tab.ddl);
                            requests
                                .into_iter()
                                .flatten()
                                .map(Command::CancelRelationRequest)
                                .collect()
                        }
                        _ => Vec::new(),
                    };
                    self.tabs.remove(self.active_tab);
                    if was_console
                        && let Some(record) =
                            self.sql_editors.iter_mut().find(|record| record.id == id)
                    {
                        record.open = false;
                    }
                    self.active_tab = self.active_tab.saturating_sub(1);
                    self.normalize_focus();
                    let mut commands = cancel;
                    if self.tabs.is_empty() {
                        if let Some(replacement_id) = self
                            .sql_editors
                            .iter()
                            .find(|record| record.id != id && !record.open)
                            .map(|record| record.id)
                        {
                            self.open_sql_editor(replacement_id);
                        } else {
                            self.create_sql_editor_named("console".to_owned());
                        }
                        self.active_tab = 0;
                        self.focus = Focus::Editor;
                    }
                    commands.push(self.persist_workspace_command());
                    commands
                } else {
                    Vec::new()
                }
            }
            Action::RequestDeleteActiveConsole => {
                let Some(tab) = self.active_console_opt() else {
                    return Vec::new();
                };
                let id = tab.id;
                if self.transaction_needs_exit(id) {
                    return self.defer_intent(DeferredIntent::DeleteConsole(id), [id]);
                }
                self.overlay = Some(Overlay::DeleteConsole { console_id: id });
                Vec::new()
            }
            Action::ConfirmDeleteConsole => {
                let Some(Overlay::DeleteConsole { console_id }) = self.overlay.take() else {
                    return Vec::new();
                };
                self.delete_console(console_id)
            }
            Action::CancelDeleteConsole => {
                if matches!(self.overlay, Some(Overlay::DeleteConsole { .. })) {
                    self.overlay = None;
                }
                Vec::new()
            }
            Action::OpenSqlEditorList => {
                self.sql_editor_list = Default::default();
                self.overlay = Some(Overlay::SqlEditorList(self.sql_editor_list.clone()));
                Vec::new()
            }
            Action::SqlEditorListInsert(value) => {
                if let Some(Overlay::SqlEditorList(list)) = self.overlay.as_mut() {
                    list.insert(value);
                }
                Vec::new()
            }
            Action::SqlEditorListBackspace => {
                if let Some(Overlay::SqlEditorList(list)) = self.overlay.as_mut() {
                    list.backspace();
                }
                Vec::new()
            }
            Action::SqlEditorListMove(delta) => {
                let count = if let Some(Overlay::SqlEditorList(list)) = self.overlay.as_ref() {
                    self.sql_editors
                        .iter()
                        .filter(|record| {
                            crate::model::sql_editor_list::SqlEditorListState::matches(
                                &record.name,
                                &list.query,
                            )
                        })
                        .count()
                } else {
                    0
                };
                if let Some(Overlay::SqlEditorList(list)) = self.overlay.as_mut() {
                    list.move_selection(delta, count);
                }
                Vec::new()
            }
            Action::ActivateSqlEditor(id) => self.activate_sql_editor(id),
            Action::NextTab => {
                if self.tabs.is_empty() {
                    return Vec::new();
                }
                self.active_tab = (self.active_tab + 1) % self.tabs.len();
                self.normalize_focus();
                Vec::new()
            }
            Action::PreviousTab => {
                if self.tabs.is_empty() {
                    return Vec::new();
                }
                self.active_tab = self
                    .active_tab
                    .checked_sub(1)
                    .unwrap_or(self.tabs.len() - 1);
                self.normalize_focus();
                Vec::new()
            }
            Action::GotoSqlConsole => {
                if let Some(index) = self.tabs.iter().position(|tab| tab.as_console().is_some()) {
                    self.active_tab = index;
                    self.focus = Focus::Editor;
                }
                Vec::new()
            }
            Action::ActivateTab(index) => {
                if index < self.tabs.len() {
                    self.active_tab = index;
                    self.normalize_focus();
                }
                Vec::new()
            }
            Action::FocusNext => {
                self.focus = if self.is_active_relation_tab() {
                    match self.focus {
                        Focus::Explorer => Focus::Results,
                        _ => Focus::Explorer,
                    }
                } else {
                    self.focus.next()
                };
                Vec::new()
            }
            Action::FocusPrevious => {
                self.focus = if self.is_active_relation_tab() {
                    match self.focus {
                        Focus::Explorer => Focus::Results,
                        _ => Focus::Explorer,
                    }
                } else {
                    self.focus.previous()
                };
                Vec::new()
            }
            Action::Focus(focus) => {
                self.focus = focus;
                self.normalize_focus();
                Vec::new()
            }
            Action::ShowHelp => {
                self.overlay = Some(Overlay::Help(crate::help::HelpState::new(self.focus)));
                Vec::new()
            }
            Action::OpenRecordView => {
                if self.focus == Focus::Results {
                    let (rows, columns) = self.active_grid_dimensions();
                    if rows > 0 && columns > 0 {
                        self.overlay = Some(Overlay::RecordView(Default::default()));
                    }
                }
                Vec::new()
            }
            Action::RecordViewMoveFields(delta) => {
                let (_, columns) = self.active_grid_dimensions();
                if let Some(Overlay::RecordView(view)) = self.overlay.as_mut() {
                    view.move_fields(delta, columns, view.visible_fields);
                }
                Vec::new()
            }
            Action::RecordViewJumpFirstField => {
                if let Some(Overlay::RecordView(view)) = self.overlay.as_mut() {
                    view.jump_first();
                }
                Vec::new()
            }
            Action::RecordViewJumpLastField => {
                let (_, columns) = self.active_grid_dimensions();
                if let Some(Overlay::RecordView(view)) = self.overlay.as_mut() {
                    view.jump_last(columns, view.visible_fields);
                }
                Vec::new()
            }
            Action::RecordViewMoveRow(delta) => {
                if matches!(self.overlay, Some(Overlay::RecordView(_))) {
                    self.move_grid(delta, 0);
                    if let Some(Overlay::RecordView(view)) = self.overlay.as_mut() {
                        view.jump_first();
                    }
                }
                Vec::new()
            }
            Action::CloseRecordView => {
                if matches!(self.overlay, Some(Overlay::RecordView(_))) {
                    self.overlay = None;
                }
                Vec::new()
            }
            Action::RecordViewViewportChanged {
                tab_id,
                visible_fields,
            } => {
                let current_tab_id = self.tabs.get(self.active_tab).map(WorkspaceTab::id);
                let (_, columns) = self.active_grid_dimensions();
                if current_tab_id == Some(tab_id)
                    && let Some(Overlay::RecordView(view)) = self.overlay.as_mut()
                {
                    view.clamp(columns, visible_fields);
                }
                Vec::new()
            }
            Action::HelpInsert(character) => {
                if let Some(Overlay::Help(help)) = self.overlay.as_mut() {
                    help.insert(character);
                }
                Vec::new()
            }
            Action::HelpPaste(value) => {
                if let Some(Overlay::Help(help)) = self.overlay.as_mut() {
                    help.paste(&value);
                }
                Vec::new()
            }
            Action::HelpBackspace => {
                if let Some(Overlay::Help(help)) = self.overlay.as_mut() {
                    help.backspace();
                }
                Vec::new()
            }
            Action::HelpClear => {
                if let Some(Overlay::Help(help)) = self.overlay.as_mut() {
                    help.clear();
                }
                Vec::new()
            }
            Action::HelpMove(delta) => {
                let relation_data = matches!(
                    self.tabs.get(self.active_tab),
                    Some(WorkspaceTab::Relation(tab))
                        if tab.view == RelationView::Data
                );
                if let Some(Overlay::Help(help)) = self.overlay.as_mut() {
                    let count =
                        crate::help::filtered_shortcuts(help.context, relation_data, &help.query)
                            .len();
                    help.move_selection(delta, count);
                }
                Vec::new()
            }
            Action::ExecuteHelpShortcut(id) => self.execute_help_shortcut(id),
            Action::DismissOverlay => {
                if matches!(
                    self.overlay,
                    Some(Overlay::DeleteConsole { .. } | Overlay::SqlEditorList(_))
                ) {
                    self.overlay = None;
                    return Vec::new();
                }
                if matches!(self.overlay, Some(Overlay::ExecutionConfirm { .. })) {
                    if let Some(Overlay::ExecutionConfirm { draft, .. }) = self.overlay.take() {
                        self.retain_execution(draft, ExecutionResult::Cancelled);
                    }
                    return Vec::new();
                }
                if matches!(self.overlay, Some(Overlay::SubstituteConfirm { .. })) {
                    self.editor.cancel_substitute();
                    self.overlay = None;
                    return Vec::new();
                }
                if self.overlay == Some(Overlay::ProfileManager) {
                    self.close_profile_manager();
                } else {
                    self.overlay = None;
                }
                Vec::new()
            }
            Action::SubstituteYes
            | Action::SubstituteNo
            | Action::SubstituteAll
            | Action::SubstituteLast
            | Action::SubstituteQuit => {
                let action = action.clone();
                if matches!(action, Action::SubstituteQuit) {
                    self.editor.cancel_substitute();
                    self.overlay = None;
                    return Vec::new();
                }
                self.overlay = None;
                let result = self.editor.substitute_confirm(
                    matches!(
                        action,
                        Action::SubstituteYes | Action::SubstituteAll | Action::SubstituteLast
                    ),
                    matches!(action, Action::SubstituteAll),
                    matches!(action, Action::SubstituteLast),
                );
                if result.is_err() {
                    self.overlay = None;
                }
                self.apply_editor_effects(CompletionAfterEdit::Schedule)
            }
            Action::OpenProfileManager => {
                if self.profile_manager.is_some() {
                    self.overlay = Some(Overlay::ProfileManager);
                    return Vec::new();
                }
                let mut manager = ProfileManagerState::default();
                manager.start_new(DatabaseKind::Postgres);
                manager.set_system_credential_availability(self.system_credential_availability);
                self.profile_manager = Some(manager);
                self.overlay = Some(Overlay::ProfileManager);
                Vec::new()
            }
            Action::SystemCredentialAvailability(availability) => {
                self.system_credential_availability = availability;
                if let Some(manager) = self.profile_manager.as_mut() {
                    manager.set_system_credential_availability(availability);
                }
                Vec::new()
            }
            Action::CloseProfileManager => {
                self.close_profile_manager();
                Vec::new()
            }
            Action::ProfileStartNew => {
                if self
                    .profile_manager
                    .as_ref()
                    .is_some_and(|manager| manager.operation.is_some())
                {
                    self.status_message("Profile operation already in progress");
                    return Vec::new();
                }
                let mut manager = ProfileManagerState::default();
                manager.start_new(DatabaseKind::Postgres);
                manager.set_system_credential_availability(self.system_credential_availability);
                self.profile_manager = Some(manager);
                self.overlay = Some(Overlay::ProfileManager);
                Vec::new()
            }
            Action::ProfileStartEdit { profile_id } => {
                if self
                    .profile_manager
                    .as_ref()
                    .is_some_and(|manager| manager.operation.is_some())
                {
                    self.status_message("Profile operation already in progress");
                    return Vec::new();
                }
                if let Some(profile) = self
                    .profiles
                    .iter()
                    .find(|profile| profile.id == profile_id)
                    .cloned()
                {
                    let mut manager = ProfileManagerState::default();
                    let has_stored_credential =
                        profile.credential_policy.has_persisted_credential();
                    manager.start_edit(&profile, has_stored_credential);
                    manager.set_system_credential_availability(self.system_credential_availability);
                    self.profile_manager = Some(manager);
                    self.overlay = Some(Overlay::ProfileManager);
                }
                Vec::new()
            }
            Action::ProfileRequestDelete { profile_id } => {
                self.request_profile_delete(profile_id);
                Vec::new()
            }
            Action::ProfileConfirmDelete => self.confirm_profile_delete(),
            Action::ProfileCancelDelete => {
                if self
                    .idle_profile_manager_mut(ProfileManagerPage::ConfirmDelete)
                    .is_some()
                {
                    self.profile_manager = None;
                    self.overlay = None;
                }
                Vec::new()
            }
            Action::ProfileFieldNext => {
                if let Some(manager) = self.editable_profile_manager_mut() {
                    manager.move_field(1);
                }
                Vec::new()
            }
            Action::ProfileFieldPrevious => {
                if let Some(manager) = self.editable_profile_manager_mut() {
                    manager.move_field(-1);
                }
                Vec::new()
            }
            Action::ProfileFocusField(field) => {
                if let Some(manager) = self.editable_profile_manager_mut() {
                    manager.focus_field(field);
                }
                Vec::new()
            }
            Action::ProfileInsert(input) => {
                if let Some(manager) = self.editable_profile_manager_mut() {
                    manager.paste(input.value());
                }
                Vec::new()
            }
            Action::ProfilePaste(input) => {
                if let Some(manager) = self.editable_profile_manager_mut() {
                    manager.paste(input.value());
                }
                Vec::new()
            }
            Action::ProfileBackspace => {
                if let Some(manager) = self.editable_profile_manager_mut() {
                    manager.backspace();
                }
                Vec::new()
            }
            Action::ProfileDeleteCharacter => {
                if let Some(manager) = self.editable_profile_manager_mut() {
                    manager.delete();
                }
                Vec::new()
            }
            Action::ProfileMoveLeft => {
                if let Some(manager) = self.editable_profile_manager_mut() {
                    manager.move_cursor_left();
                }
                Vec::new()
            }
            Action::ProfileMoveRight => {
                if let Some(manager) = self.editable_profile_manager_mut() {
                    manager.move_cursor_right();
                }
                Vec::new()
            }
            Action::ProfileMoveHome => {
                if let Some(manager) = self.editable_profile_manager_mut() {
                    manager.move_cursor_home();
                }
                Vec::new()
            }
            Action::ProfileMoveEnd => {
                if let Some(manager) = self.editable_profile_manager_mut() {
                    manager.move_cursor_end();
                }
                Vec::new()
            }
            Action::ProfileCommitUrl => {
                if let Some(manager) = self.editable_profile_manager_mut() {
                    let _ = manager.commit_url();
                }
                Vec::new()
            }
            Action::ProfileCycle(delta) => {
                if let Some(manager) = self.editable_profile_manager_mut() {
                    manager.cycle(delta);
                }
                Vec::new()
            }
            Action::ProfileSelectDriver(kind) => {
                if let Some(manager) = self.editable_profile_manager_mut() {
                    manager.select_driver(kind);
                }
                Vec::new()
            }
            Action::ProfileToggle => {
                if let Some(manager) = self.editable_profile_manager_mut() {
                    manager.toggle();
                }
                Vec::new()
            }
            Action::ProfileToggleField(field) => {
                if let Some(manager) = self.editable_profile_manager_mut()
                    && manager.visible_fields().contains(&field)
                {
                    manager.focus_field(field);
                    manager.toggle();
                }
                Vec::new()
            }
            Action::ProfileOpenScope => self.open_profile_scope(false),
            Action::ProfileRefreshScope => self.open_profile_scope(true),
            Action::ProfileToggleScopeRow(id) => {
                if let Some(manager) = self
                    .profile_manager
                    .as_mut()
                    .filter(|manager| manager.page == ProfileManagerPage::Scope)
                {
                    manager.toggle_scope_row(&id);
                }
                Vec::new()
            }
            Action::ProfileScopeMove(delta) => {
                if let Some(manager) = self
                    .profile_manager
                    .as_mut()
                    .filter(|manager| manager.page == ProfileManagerPage::Scope)
                {
                    manager.move_scope_selection(delta);
                }
                Vec::new()
            }
            Action::ProfileScopeBack => {
                if let Some(manager) = self
                    .profile_manager
                    .as_mut()
                    .filter(|manager| manager.page == ProfileManagerPage::Scope)
                {
                    manager.close_scope_picker();
                }
                Vec::new()
            }
            Action::ProfileTest => self.test_profile_draft(),
            Action::ProfileSave { connect } => self.save_profile_draft(connect),
            Action::ProfileTestSucceeded {
                request_id,
                fingerprint,
                server,
                capabilities,
                discovery,
            } => {
                if let Some(manager) =
                    self.matching_profile_operation(request_id, &[ProfileOperation::Testing])
                {
                    manager.operation = None;
                    let warning = discovery.as_ref().err().cloned();
                    let version = server.version.clone();
                    let database = server.database.clone();
                    let applied = manager.draft.as_mut().is_some_and(|draft| {
                        draft.apply_catalog_discovery(ProfileCatalogDiscovery {
                            fingerprint,
                            server,
                            capabilities,
                            discovery,
                        })
                    });
                    manager.message = Some(if !applied {
                        "Connection test result ignored because the draft changed".into()
                    } else if let Some(warning) = warning {
                        format!(
                            "Connection succeeded: {version} ({database}); catalog discovery warning: {warning}"
                        )
                    } else {
                        format!("Connection succeeded: {version} ({database})")
                    });
                }
                Vec::new()
            }
            Action::ProfileTestFailed {
                request_id,
                message,
            } => {
                if let Some(manager) =
                    self.matching_profile_operation(request_id, &[ProfileOperation::Testing])
                {
                    manager.operation = None;
                    manager.message = Some(message);
                }
                Vec::new()
            }
            Action::ProfileCatalogDiscoverySucceeded {
                request_id,
                fingerprint,
                server,
                capabilities,
                discovery,
            } => {
                if let Some(manager) = self.profile_manager.as_mut()
                    && manager.matches_scope_discovery(request_id, fingerprint)
                {
                    if let Some(draft) = manager.draft.as_mut() {
                        draft.apply_catalog_discovery(ProfileCatalogDiscovery {
                            fingerprint,
                            server,
                            capabilities,
                            discovery: Ok(discovery),
                        });
                    }
                    manager.finish_scope_discovery();
                }
                Vec::new()
            }
            Action::ProfileCatalogDiscoveryFailed {
                request_id,
                fingerprint,
                message,
            } => {
                if let Some(manager) = self.profile_manager.as_mut()
                    && manager.matches_scope_discovery(request_id, fingerprint)
                {
                    manager.fail_scope_discovery(message);
                }
                Vec::new()
            }
            Action::ProfileSaved {
                request_id,
                profile,
                warning,
                change,
                connect,
            } => self.profile_saved(request_id, profile, warning, change, connect),
            Action::ProfileSaveFailed {
                request_id,
                message,
            } => {
                if let Some(manager) = self.matching_profile_operation(
                    request_id,
                    &[
                        ProfileOperation::Saving,
                        ProfileOperation::SavingAndConnecting,
                    ],
                ) {
                    manager.operation = None;
                    manager.message = Some(message);
                }
                Vec::new()
            }
            Action::ProfileDeleted {
                request_id,
                profile_id,
                active_connection,
            } => self.profile_deleted(request_id, profile_id, active_connection),
            Action::ProfileDeleteFailed {
                request_id,
                message,
            } => {
                if let Some(manager) =
                    self.matching_profile_operation(request_id, &[ProfileOperation::Deleting])
                {
                    manager.operation = None;
                    manager.message = Some(message);
                }
                Vec::new()
            }
            Action::CredentialsRequired {
                profile_id,
                generation,
                message,
            } => {
                if !self.pending_connection_matches(profile_id, generation) {
                    return Vec::new();
                }
                let Some(profile) = self
                    .profiles
                    .iter()
                    .find(|profile| profile.id == profile_id)
                    .cloned()
                else {
                    return Vec::new();
                };
                self.connection_terminal_generation =
                    self.connection_terminal_generation.max(generation);
                self.connection.pending_profile_id = None;
                self.connection.pending_generation = None;
                self.connection.pending_target = None;
                self.pending_target_console = None;
                self.connection.status = if self.connection.profile_id.is_some() {
                    ConnectionStatus::Connected
                } else {
                    ConnectionStatus::Failed
                };
                self.connection.error = Some(message.clone());
                if let Some(state) = self.explorer.normalized.profiles.get_mut(&profile_id) {
                    state.status = ExplorerConnectionStatus::Failed;
                    state.last_error = Some(message.clone());
                    state.expand_after_connect = false;
                }
                if let Some(manager) = self.profile_manager.as_mut()
                    && manager
                        .operation
                        .is_some_and(|operation| operation != ProfileOperation::Connecting)
                {
                    manager.message = Some(message);
                    return Vec::new();
                }
                let has_stored_credential = profile.credential_policy.has_persisted_credential();
                let mut manager = ProfileManagerState::default();
                manager.start_edit(&profile, has_stored_credential);
                manager.set_system_credential_availability(self.system_credential_availability);
                manager.selected_field = ProfileField::Password;
                manager.message = Some(message);
                self.profile_manager = Some(manager);
                self.overlay = Some(Overlay::ProfileManager);
                Vec::new()
            }
            Action::DisconnectCompleted { connection } => {
                let pending_matches = self.connection.pending_identity() == Some(connection);
                let active_matches = self.connection.active_identity() == Some(connection);
                if !pending_matches && !active_matches {
                    return Vec::new();
                }
                self.connection_terminal_generation = self
                    .connection_terminal_generation
                    .max(connection.generation);
                if pending_matches {
                    self.connection.pending_profile_id = None;
                    self.connection.pending_generation = None;
                    self.connection.pending_target = None;
                    self.pending_target_console = None;
                }
                if active_matches {
                    self.connection.profile_id = None;
                    self.connection.generation = 0;
                    self.connection.server = None;
                    self.connection.target = None;
                    self.connection.error = None;
                    self.clear_active_catalog(connection.profile_id);
                    self.select_nearest_profile(connection.profile_id);
                }
                self.connection.status = if self.connection.pending_profile_id.is_some() {
                    ConnectionStatus::Connecting
                } else if self.connection.profile_id.is_some() {
                    ConnectionStatus::Connected
                } else {
                    ConnectionStatus::Disconnected
                };
                Vec::new()
            }
            Action::EditorKey(key) => {
                let Some(id) = self.active_console_opt().map(|tab| tab.id) else {
                    return Vec::new();
                };
                self.active_console_opt_mut().unwrap().completion = None;
                if self.editor.key(id, key).is_err() {
                    return Vec::new();
                }
                self.apply_editor_effects(CompletionAfterEdit::Schedule)
            }
            Action::EditorPaste(text) => {
                let Some(id) = self.active_console_opt().map(|tab| tab.id) else {
                    return Vec::new();
                };
                self.active_console_opt_mut().unwrap().completion = None;
                if self.editor.paste(id, &text).is_err() {
                    return Vec::new();
                }
                self.apply_editor_effects(CompletionAfterEdit::Schedule)
            }
            Action::EditorViewportChanged(viewport) => {
                let Some(id) = self.active_console_opt().map(|tab| tab.id) else {
                    return Vec::new();
                };
                let _ = self.editor.set_viewport(id, viewport);
                Vec::new()
            }
            Action::GridViewportChanged(viewport) => {
                self.sync_grid_viewport(viewport);
                Vec::new()
            }
            Action::EditorScroll { rows, columns } => {
                let Some(id) = self.active_console_opt().map(|tab| tab.id) else {
                    return Vec::new();
                };
                let _ = self.editor.scroll(id, rows, columns);
                Vec::new()
            }
            Action::ReplaceEditor(text) => {
                let Some(id) = self.active_console_opt().map(|tab| tab.id) else {
                    return Vec::new();
                };
                let _ = self.editor.set_text(id, &text);
                vec![self.persist_workspace_command()]
            }
            Action::CompletionExplicit => self.complete_now(),
            Action::CompletionDue(key) => {
                if self.completion_key() == Some(key) {
                    self.complete_now()
                } else {
                    Vec::new()
                }
            }
            Action::CompletionNext => {
                if let Some(popup) = &mut self.active_console_mut().completion {
                    popup.selected = (popup.selected + 1) % popup.candidates.len().max(1);
                }
                Vec::new()
            }
            Action::CompletionPrevious => {
                if let Some(popup) = &mut self.active_console_mut().completion {
                    popup.selected = popup
                        .selected
                        .checked_sub(1)
                        .unwrap_or(popup.candidates.len().saturating_sub(1));
                }
                Vec::new()
            }
            Action::CompletionDismiss => {
                self.active_console_mut().completion = None;
                Vec::new()
            }
            Action::CompletionAccept => self.accept_completion(),
            Action::RunActiveSql => self.run_active_sql(false),
            Action::RunAllSql => self.run_active_sql(true),
            Action::ConfirmExecution => self.confirm_execution(),
            Action::CancelExecution => self.cancel_execution(),
            Action::ToggleExecutionConfirmationFocus => {
                if let Some(Overlay::ExecutionConfirm { focus, .. }) = self.overlay.as_mut() {
                    *focus = match *focus {
                        ExecutionConfirmFocus::Cancel => ExecutionConfirmFocus::Execute,
                        ExecutionConfirmFocus::Execute => ExecutionConfirmFocus::Cancel,
                    };
                }
                Vec::new()
            }
            Action::CancelActiveQuery => {
                let active_connection = self.connection.active_identity();
                let tab = self.active_console_mut();
                if tab.query_status != QueryStatus::Running {
                    return Vec::new();
                }
                if tab.transaction_mode == TransactionMode::Manual
                    && tab.transaction_state == TransactionState::Active
                {
                    let intent = transaction::CancellationIntent {
                        console_id: tab.id,
                        query_generation: tab.generation,
                        transaction_generation: tab.transaction_generation,
                        connection: active_connection.unwrap_or(ConnectionIdentity {
                            profile_id: Uuid::nil(),
                            generation: 0,
                        }),
                    };
                    self.overlay = Some(Overlay::ManualCancelConfirm {
                        intent,
                        focus: ManualCancelFocus::KeepRunning,
                    });
                    return Vec::new();
                }
                tab.query_status = QueryStatus::Cancelled;
                tab.output.push(OutputEntry {
                    kind: OutputKind::Cancelled,
                    message: "Query cancellation requested".to_owned(),
                });
                if let Some(last) = tab.last_execution.as_mut() {
                    last.result = ExecutionResult::Cancelled;
                }
                vec![Command::CancelQuery {
                    tab_id: tab.id,
                    generation: tab.generation,
                }]
            }
            Action::ToggleManualCancellationFocus => {
                if let Some(Overlay::ManualCancelConfirm { focus, .. }) = self.overlay.as_mut() {
                    *focus = match *focus {
                        ManualCancelFocus::KeepRunning => ManualCancelFocus::CancelQueryAndRollback,
                        ManualCancelFocus::CancelQueryAndRollback => ManualCancelFocus::KeepRunning,
                    };
                }
                Vec::new()
            }
            Action::CancelManualCancellation => {
                self.overlay = None;
                Vec::new()
            }
            Action::ConfirmTransactionExit => {
                let choice = match self.overlay {
                    Some(Overlay::TransactionExitConfirm { prompt, .. })
                        if self
                            .tabs
                            .iter()
                            .find(|tab| tab.id() == prompt.console_id)
                            .and_then(WorkspaceTab::as_console)
                            .is_some_and(|tab| {
                                tab.transaction_state == TransactionState::OutcomeUnknown
                            }) =>
                    {
                        TransactionExitChoice::Abandon
                    }
                    _ => TransactionExitChoice::Commit,
                };
                self.resolve_transaction_exit(choice)
            }
            Action::ConfirmTransactionExitChoice(choice) => self.resolve_transaction_exit(choice),
            Action::CancelTransactionExit => {
                self.resolve_transaction_exit(TransactionExitChoice::Cancel)
            }
            Action::ToggleTransactionExitChoice => {
                if let Some(Overlay::TransactionExitConfirm { choice, .. }) = self.overlay.as_mut()
                {
                    *choice = match choice {
                        TransactionExitChoice::Commit => TransactionExitChoice::Rollback,
                        TransactionExitChoice::Rollback => TransactionExitChoice::Commit,
                        TransactionExitChoice::Abandon => TransactionExitChoice::Cancel,
                        TransactionExitChoice::Cancel => TransactionExitChoice::Rollback,
                    };
                }
                Vec::new()
            }
            Action::ConfirmClearTransactionOutcome => self.confirm_clear_outcome(),
            Action::CancelClearTransactionOutcome => {
                if matches!(self.overlay, Some(Overlay::ClearTransactionOutcome { .. })) {
                    self.overlay = None;
                }
                Vec::new()
            }
            Action::OpenTargetSelector => {
                let Some(profile) = self.active_profile() else {
                    self.status_message("No active connection; connect before selecting a target");
                    return Vec::new();
                };
                let candidates = self.execution_target_candidates(profile);
                let current = self.active_console().execution_target.as_ref();
                let selected = current
                    .and_then(|target| candidates.iter().position(|candidate| candidate == target))
                    .unwrap_or(0);
                self.overlay = Some(Overlay::TargetSelector {
                    candidates,
                    selected,
                });
                Vec::new()
            }
            Action::MoveTargetSelector(delta) => {
                if let Some(Overlay::TargetSelector {
                    candidates,
                    selected,
                }) = self.overlay.as_mut()
                {
                    let count = candidates.len().max(1) as isize;
                    *selected = (*selected as isize + delta).rem_euclid(count) as usize;
                }
                Vec::new()
            }
            Action::ConfirmTargetSelector => {
                let Some(Overlay::TargetSelector {
                    candidates,
                    selected,
                }) = self.overlay.take()
                else {
                    return Vec::new();
                };
                let Some(target) = candidates.get(selected).cloned() else {
                    return Vec::new();
                };
                let tab = self.active_console();
                if tab.execution_target.as_ref() == Some(&target) {
                    return Vec::new();
                }
                if tab.query_status == QueryStatus::Running {
                    self.status_message("Cannot change target while a query is running");
                    return Vec::new();
                }
                if tab.transaction_mode == TransactionMode::Manual
                    && tab.transaction_state != TransactionState::Idle
                {
                    self.status_message(
                        "Cannot change target while a manual transaction is active",
                    );
                    return Vec::new();
                }
                if self.has_running_query() {
                    self.status_message("Cannot change target while another query is running");
                    return Vec::new();
                }
                if self.tabs.iter().any(|workspace_tab| {
                    workspace_tab.as_console().is_some_and(|console| {
                        console.transaction_mode == TransactionMode::Manual
                            && console.transaction_state != TransactionState::Idle
                    })
                }) {
                    self.status_message(
                        "Cannot change target while another manual transaction is active",
                    );
                    return Vec::new();
                }
                let console_id = tab.id;
                self.pending_target_console = Some(console_id);
                self.request_connection_target(target)
            }
            Action::CancelTargetSelector => {
                self.overlay = None;
                Vec::new()
            }
            Action::ConfirmManualCancellation => {
                let Some(Overlay::ManualCancelConfirm { intent, focus }) = self.overlay.take()
                else {
                    return Vec::new();
                };
                if focus != ManualCancelFocus::CancelQueryAndRollback {
                    return Vec::new();
                }
                let current = self
                    .tabs
                    .iter()
                    .find(|tab| tab.id() == intent.console_id)
                    .and_then(WorkspaceTab::as_console);
                if self.connection.active_identity() != Some(intent.connection)
                    || current.is_none_or(|tab| {
                        tab.generation != intent.query_generation
                            || tab.transaction_generation != intent.transaction_generation
                            || tab.query_status != QueryStatus::Running
                    })
                {
                    self.status_message("Stale cancellation request discarded");
                    return Vec::new();
                }
                let tab = self
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id() == intent.console_id)
                    .and_then(WorkspaceTab::as_console_mut)
                    .unwrap();
                tab.generation = tab.generation.saturating_add(1);
                tab.query_status = QueryStatus::Cancelled;
                tab.output.push(OutputEntry {
                    kind: OutputKind::Cancelled,
                    message: "Cancelling rolls back all uncommitted work in this transaction"
                        .to_owned(),
                });
                vec![Command::CancelManual {
                    connection: intent.connection,
                    tab_id: intent.console_id,
                    query_generation: intent.query_generation,
                    transaction_generation: intent.transaction_generation,
                }]
            }
            Action::SetTransactionMode(mode) => {
                if mode == TransactionMode::Auto
                    && self.transaction_needs_exit(self.active_console().id)
                {
                    return self
                        .defer_intent(DeferredIntent::SetMode(mode), [self.active_console().id]);
                }
                self.set_transaction_mode(mode)
            }
            Action::CommitTransaction => self.transaction_control(true),
            Action::RollbackTransaction => self.transaction_control(false),
            Action::RefreshCatalog => {
                let target = self
                    .selected_catalog_target()
                    .unwrap_or(CatalogTarget::Databases);
                self.start_catalog_request(target, None, CatalogRequestIntent::Refresh)
            }
            Action::ExplorerLoadTarget(target) => {
                self.start_catalog_request(target, None, CatalogRequestIntent::Explicit)
            }
            Action::ExplorerOpenSelected => {
                let opens_relation = matches!(
                    self.explorer.selected_id(),
                    Some(ExplorerNodeId::Catalog(id))
                        if self
                            .explorer
                            .normalized
                            .profiles
                            .get(&id.profile_id())
                            .and_then(|profile| profile.catalog.owning_relation_id(id))
                            .is_some()
                );
                if opens_relation {
                    self.open_selected_relation(RelationView::Data)
                } else {
                    self.primary_explorer_selected()
                }
            }
            Action::OpenSelectedRelation { view } => self.open_selected_relation(view),
            Action::SetRelationView(view) => {
                if let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab) {
                    tab.view = view;
                }
                self.load_active_relation(false)
            }
            Action::RefreshActiveRelation => self.load_active_relation(true),
            Action::CancelActiveRelationRequest => {
                let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab) else {
                    return Vec::new();
                };
                let request = match tab.view {
                    RelationView::Data => cancel_pending_relation(&mut tab.data),
                    RelationView::Ddl => cancel_pending_relation(&mut tab.ddl),
                };
                request
                    .map(Command::CancelRelationRequest)
                    .into_iter()
                    .collect()
            }
            Action::DdlScroll { rows, columns } => {
                if let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab)
                    && tab.view == RelationView::Ddl
                {
                    tab.ddl_viewport.row_offset = tab
                        .ddl_viewport
                        .row_offset
                        .saturating_add_signed(rows)
                        .min(tab.ddl_viewport.max_row_offset());
                    tab.ddl_viewport.column_offset = tab
                        .ddl_viewport
                        .column_offset
                        .saturating_add_signed(columns)
                        .min(tab.ddl_viewport.max_column_offset());
                }
                Vec::new()
            }
            Action::DdlScrollToStart => {
                if let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab)
                    && tab.view == RelationView::Ddl
                {
                    tab.ddl_viewport.row_offset = 0;
                    tab.ddl_viewport.column_offset = 0;
                }
                Vec::new()
            }
            Action::DdlScrollToEnd => {
                if let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab)
                    && tab.view == RelationView::Ddl
                {
                    tab.ddl_viewport.row_offset = tab.ddl_viewport.max_row_offset();
                    tab.ddl_viewport.column_offset = tab.ddl_viewport.max_column_offset();
                }
                Vec::new()
            }
            Action::SetDdlViewportMetrics {
                visible_rows,
                visible_columns,
                total_rows,
                max_line_width,
            } => {
                if let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab)
                    && tab.view == RelationView::Ddl
                {
                    tab.ddl_viewport.visible_rows = visible_rows;
                    tab.ddl_viewport.visible_columns = visible_columns;
                    tab.ddl_viewport.total_rows = total_rows;
                    tab.ddl_viewport.max_line_width = max_line_width;
                    tab.ddl_viewport.clamp();
                }
                Vec::new()
            }
            Action::FocusDataQueryInput(input) => {
                if let Some(query) = self.active_data_query_mut()
                    && matches!(
                        query.capability,
                        DataQueryCapability::Relation | DataQueryCapability::Sql
                    )
                {
                    query.focus = Some(input);
                    query.error = None;
                }
                self.refresh_active_data_query_completion();
                Vec::new()
            }
            Action::DataQueryInsert(character) => {
                self.with_active_data_query(|input| input.insert(character));
                self.refresh_active_data_query_completion();
                Vec::new()
            }
            Action::DataQueryBackspace => {
                self.with_active_data_query(|input| input.backspace());
                self.refresh_active_data_query_completion();
                Vec::new()
            }
            Action::DataQueryDeletePreviousWord => {
                self.with_active_data_query(|input| input.delete_previous_word());
                self.refresh_active_data_query_completion();
                Vec::new()
            }
            Action::DataQueryDeleteToStart => {
                self.with_active_data_query(|input| input.delete_to_start());
                self.refresh_active_data_query_completion();
                Vec::new()
            }
            Action::DataQueryDelete => {
                self.with_active_data_query(|input| input.delete());
                self.refresh_active_data_query_completion();
                Vec::new()
            }
            Action::DataQueryMoveLeft => {
                self.with_active_data_query(|input| input.move_left());
                self.refresh_active_data_query_completion();
                Vec::new()
            }
            Action::DataQueryMoveRight => {
                self.with_active_data_query(|input| input.move_right());
                self.refresh_active_data_query_completion();
                Vec::new()
            }
            Action::DataQueryMoveHome => {
                self.with_active_data_query(|input| input.move_home());
                self.refresh_active_data_query_completion();
                Vec::new()
            }
            Action::DataQueryMoveEnd => {
                self.with_active_data_query(|input| input.move_end());
                self.refresh_active_data_query_completion();
                Vec::new()
            }
            Action::DataQueryClear => {
                self.with_active_data_query(|input| input.set(""));
                self.refresh_active_data_query_completion();
                Vec::new()
            }
            Action::DataQueryCompletionNext => {
                self.move_active_data_query_completion(1);
                Vec::new()
            }
            Action::DataQueryCompletionPrevious => {
                self.move_active_data_query_completion(-1);
                Vec::new()
            }
            Action::DataQueryCompletionAccept => {
                self.accept_active_data_query_completion();
                Vec::new()
            }
            Action::DataQueryCompletionDismiss => {
                if let Some(query) = self.active_data_query_mut() {
                    query.completion = None;
                }
                Vec::new()
            }
            Action::CancelDataQueryInput => {
                self.cancel_active_data_query();
                Vec::new()
            }
            Action::SubmitDataQuery => self.submit_data_query(),
            Action::FocusRelationQueryInput(input) => {
                self.update(Action::FocusDataQueryInput(input))
            }
            Action::RelationQueryInsert(character) => {
                self.update(Action::DataQueryInsert(character))
            }
            Action::RelationQueryBackspace => self.update(Action::DataQueryBackspace),
            Action::RelationQueryDelete => self.update(Action::DataQueryDelete),
            Action::RelationQueryMoveLeft => self.update(Action::DataQueryMoveLeft),
            Action::RelationQueryMoveRight => self.update(Action::DataQueryMoveRight),
            Action::RelationQueryMoveHome => self.update(Action::DataQueryMoveHome),
            Action::RelationQueryMoveEnd => self.update(Action::DataQueryMoveEnd),
            Action::RelationQueryClear => self.update(Action::DataQueryClear),
            Action::CancelRelationQueryInput => self.update(Action::CancelDataQueryInput),
            Action::SubmitRelationQuery => self.update(Action::SubmitDataQuery),
            Action::ResizeRelationColumn(delta) => {
                self.resize_grid_column(delta);
                Vec::new()
            }
            Action::ResetRelationColumnWidth => {
                self.reset_grid_column_width();
                Vec::new()
            }
            Action::StartRelationColumnResize { column, width } => {
                self.set_grid_column_width(column, width);
                Vec::new()
            }
            Action::SetRelationColumnWidth { column, width } => {
                self.set_grid_column_width(column, width);
                Vec::new()
            }
            Action::EndRelationColumnResize => Vec::new(),
            Action::GridResizeColumn(delta) => {
                self.resize_grid_column(delta);
                Vec::new()
            }
            Action::GridResetColumnWidth => {
                self.reset_grid_column_width();
                Vec::new()
            }
            Action::GridStartColumnResize { column, width }
            | Action::GridSetColumnWidth { column, width } => {
                self.set_grid_column_width(column, width);
                Vec::new()
            }
            Action::GridEndColumnResize => Vec::new(),
            Action::GridSetColumnOffset { offset } => {
                self.set_grid_column_offset(offset);
                Vec::new()
            }
            Action::PreviewSelected => self.open_selected_relation(RelationView::Data),
            Action::DdlSelected => self.ddl_selected(),
            Action::RelationSucceeded { request, snapshot } => {
                let commands = self.accept_relation(request, Ok(*snapshot));
                self.refresh_active_data_query_completion();
                commands
            }
            Action::RelationFailed { request, message } => {
                self.accept_relation(request, Err(message))
            }
            Action::RequestProfileConnect { profile_id } => self.request_connection(profile_id),
            Action::RequestConnect(profile_id) => self.request_connection(profile_id),
            Action::RequestProfileDisconnect { profile_id } => {
                self.request_profile_disconnect(profile_id)
            }
            Action::ClearTransactionOutcome => self.request_clear_outcome(),
            Action::ConnectionSucceeded {
                profile_id,
                generation,
                server,
            } => {
                let active_generation = self
                    .connection
                    .profile_id
                    .map(|_| self.connection.generation)
                    .unwrap_or(0);
                if generation <= self.connection_terminal_generation.max(active_generation) {
                    return Vec::new();
                }
                let pending_matches = self.pending_connection_matches(profile_id, generation);
                let target = if pending_matches {
                    self.connection.pending_target.clone().or_else(|| {
                        self.profiles
                            .iter()
                            .find(|profile| profile.id == profile_id)
                            .map(ExecutionTarget::from_profile)
                    })
                } else {
                    self.profiles
                        .iter()
                        .find(|profile| profile.id == profile_id)
                        .map(ExecutionTarget::from_profile)
                };
                let Some(target) = target else {
                    return Vec::new();
                };
                let old_profile_id = self.connection.profile_id;
                self.connection_terminal_generation = generation;
                self.connection.profile_id = Some(profile_id);
                self.connection.generation = generation;
                self.connection.target = Some(target.clone());
                self.connection_request_generation =
                    self.connection_request_generation.max(generation);
                if pending_matches {
                    self.connection.pending_profile_id = None;
                    self.connection.pending_generation = None;
                }
                self.connection.status = if self.connection.pending_profile_id.is_some() {
                    ConnectionStatus::Connecting
                } else {
                    ConnectionStatus::Connected
                };
                self.connection.server = Some(server);
                self.connection.error = None;
                let mut persist_target = false;
                if pending_matches
                    && self.connection.pending_target.as_ref() == Some(&target)
                    && let Some(console_id) = self.pending_target_console.take()
                    && let Some(tab) = self
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.id() == console_id)
                        .and_then(WorkspaceTab::as_console_mut)
                    && tab.execution_target.as_ref() != Some(&target)
                {
                    tab.execution_target = Some(target.clone());
                    persist_target = true;
                }
                if pending_matches {
                    self.connection.pending_target = None;
                }
                let relation_cancellations =
                    self.cancel_relation_requests_for_connection(Some(ConnectionIdentity {
                        profile_id,
                        generation,
                    }));
                self.explorer.connection_changed();
                if let Some(old_profile_id) = old_profile_id.filter(|id| *id != profile_id) {
                    self.clear_profile_catalog(old_profile_id, ExplorerConnectionStatus::Offline);
                }
                let Some(state) = self.explorer.normalized.profiles.get_mut(&profile_id) else {
                    return Vec::new();
                };
                state.status = ExplorerConnectionStatus::Online;
                state.last_error = None;
                let expand_after_connect = state.expand_after_connect;
                state.expand_after_connect = false;
                state.catalog = crate::model::explorer::CatalogTree::new(profile_id);
                state.load_states.clear();
                state.pending_requests.clear();
                state.previous_load_states.clear();
                state.load_errors.clear();
                self.explorer.active_profile = Some(profile_id);
                self.explorer.normalized.selected =
                    Some(crate::model::explorer::ExplorerNodeId::Profile(profile_id));
                if state.advance_catalog_epoch().is_none() {
                    state.last_error = Some("catalog epoch exhausted".to_owned());
                    return Vec::new();
                }
                state.status = ExplorerConnectionStatus::Syncing;
                for tab in &mut self.tabs {
                    let Some(tab) = tab.as_console_mut() else {
                        continue;
                    };
                    let should_default = tab.execution_target.as_ref().is_none_or(|target| {
                        target.profile_id == profile_id
                            && !target.is_valid(
                                self.profiles
                                    .iter()
                                    .find(|profile| profile.id == profile_id)
                                    .expect("connected profile exists"),
                            )
                    });
                    if should_default {
                        tab.execution_target = Some(target.clone());
                        persist_target = true;
                    }
                    if tab.transaction_state == TransactionState::OutcomeUnknown
                        && let Ok(next) = transaction::transition(
                            tab_snapshot(tab),
                            TransactionEvent::ClearOutcome,
                        )
                    {
                        apply_transaction_snapshot(tab, next);
                        tab.output.push(OutputEntry {
                                kind: OutputKind::Info,
                                message: "Transaction outcome cleared after reconnect; the prior operation was not retried".to_owned(),
                            });
                    }
                }
                if pending_matches
                    && let Some(manager) = self.profile_manager.as_mut()
                    && manager.operation == Some(ProfileOperation::Connecting)
                {
                    manager.operation = None;
                    manager.message = Some("Connected".to_owned());
                }
                let commands_for_catalog = self.start_catalog_request(
                    CatalogTarget::Databases,
                    None,
                    CatalogRequestIntent::Automatic,
                );
                if expand_after_connect {
                    self.explorer
                        .normalized
                        .expanded
                        .insert(crate::model::explorer::ExplorerNodeId::Profile(profile_id));
                }
                let mut commands = relation_cancellations;
                commands.extend(commands_for_catalog);
                if persist_target {
                    commands.push(self.persist_workspace_command());
                }
                commands
            }
            Action::ConnectionFailed {
                profile_id,
                generation,
                message,
            } => {
                if self.pending_connection_matches(profile_id, generation) {
                    self.connection_terminal_generation =
                        self.connection_terminal_generation.max(generation);
                    self.connection.pending_profile_id = None;
                    self.connection.pending_generation = None;
                    self.connection.pending_target = None;
                    self.pending_target_console = None;
                    self.connection.status = if self.connection.profile_id.is_some() {
                        ConnectionStatus::Connected
                    } else {
                        ConnectionStatus::Failed
                    };
                    self.connection.error = Some(message.clone());
                    if let Some(state) = self.explorer.normalized.profiles.get_mut(&profile_id) {
                        state.status = ExplorerConnectionStatus::Failed;
                        state.last_error = Some(message.clone());
                        state.expand_after_connect = false;
                    }
                    if let Some(manager) = self.profile_manager.as_mut()
                        && manager.operation == Some(ProfileOperation::Connecting)
                    {
                        manager.operation = None;
                        manager.message = Some(message);
                    }
                }
                Vec::new()
            }
            Action::ConnectionInvalidated {
                connection,
                message,
            } => {
                if self.connection.active_identity() != Some(connection) {
                    return Vec::new();
                }
                self.connection_terminal_generation = self
                    .connection_terminal_generation
                    .max(connection.generation);
                let invalidated_profile_id = connection.profile_id;
                let pending_profile_id = self.connection.pending_profile_id;
                self.connection.profile_id = None;
                self.connection.generation = 0;
                self.connection.server = None;
                self.connection.target = None;
                self.connection.error = Some(message.clone());
                self.connection.status = if pending_profile_id.is_some() {
                    self.connection.pending_profile_id = None;
                    self.connection.pending_generation = None;
                    self.connection.pending_target = None;
                    self.pending_target_console = None;
                    ConnectionStatus::Failed
                } else {
                    ConnectionStatus::Failed
                };
                self.clear_active_catalog(invalidated_profile_id);
                if let Some(state) = self
                    .explorer
                    .normalized
                    .profiles
                    .get_mut(&connection.profile_id)
                {
                    state.status = ExplorerConnectionStatus::Failed;
                    state.last_error = Some(message.clone());
                    state.expand_after_connect = false;
                }
                if let Some(pending_profile_id) = pending_profile_id
                    && let Some(state) = self
                        .explorer
                        .normalized
                        .profiles
                        .get_mut(&pending_profile_id)
                {
                    state.status = ExplorerConnectionStatus::Failed;
                    state.last_error = Some(message.clone());
                    state.expand_after_connect = false;
                }
                self.select_nearest_profile(connection.profile_id);
                for tab in &mut self.tabs {
                    let Some(tab) = tab.as_console_mut() else {
                        continue;
                    };
                    if tab.transaction_state != TransactionState::Idle {
                        tab.transaction_state = TransactionState::OutcomeUnknown;
                        tab.transaction_generation = tab.transaction_generation.saturating_add(1);
                        tab.query_status = QueryStatus::Failed;
                        tab.output.push(OutputEntry {
                            kind: OutputKind::Error,
                            message: message.clone(),
                        });
                    }
                }
                Vec::new()
            }
            Action::CatalogPageLoaded(page) => {
                let commands = self.accept_catalog_page(page);
                self.refresh_active_data_query_completion();
                commands
            }
            Action::CatalogPageFailed {
                key,
                category,
                message,
            } => {
                self.fail_catalog_page(&key, category, message);
                Vec::new()
            }
            Action::CatalogSearchSucceeded(page) => {
                if self.database_command_identity() == Some(page.connection) {
                    let _ = page;
                }
                Vec::new()
            }
            Action::CatalogSearchFailed {
                connection,
                session_id,
                generation,
                message,
            } => {
                if self.database_command_identity() == Some(connection)
                    && let Some(search) = self.explorer.search.as_mut().filter(|search| {
                        search.session_id == session_id && search.generation == generation
                    })
                {
                    search.lifecycle =
                        crate::model::workspace::ExplorerSearchLifecycle::Failed(message);
                }
                Vec::new()
            }
            Action::QueryFinished {
                tab_id,
                generation,
                connection,
                outcome,
            } => {
                let valid = self
                    .tabs
                    .iter()
                    .find(|tab| tab.id() == tab_id)
                    .and_then(WorkspaceTab::as_console)
                    .is_some_and(|tab| tab.generation == generation)
                    && self.connection.active_identity() == Some(connection);
                if !valid {
                    return Vec::new();
                }
                self.finish_query(tab_id, generation, outcome, false);
                Vec::new()
            }
            Action::QueryFailed {
                tab_id,
                generation,
                connection,
                message,
            } => {
                let Some(tab) = self
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id() == tab_id)
                    .and_then(WorkspaceTab::as_console_mut)
                else {
                    return Vec::new();
                };
                if tab.generation != generation
                    || self.connection.active_identity() != Some(connection)
                {
                    return Vec::new();
                }
                tab.query_status = QueryStatus::Failed;
                append_failed_execution_output(tab, generation, message);
                tab.result_view = ResultView::Output;
                if let Some(last) = tab.last_execution.as_mut()
                    && last.draft.query_generation + 1 == generation
                {
                    last.result = ExecutionResult::Failed;
                }
                Vec::new()
            }
            Action::DerivedQueryFinished {
                tab_id,
                source_generation,
                derived_generation,
                connection,
                target,
                outcome,
            } => {
                let Some(tab) = self
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id() == tab_id)
                    .and_then(WorkspaceTab::as_console_mut)
                else {
                    return Vec::new();
                };
                if tab.generation != source_generation
                    || self.connection.active_identity() != Some(connection)
                    || tab.execution_target.as_ref() != Some(&target)
                    || tab
                        .derived
                        .as_ref()
                        .is_none_or(|derived| derived.generation != derived_generation)
                {
                    return Vec::new();
                }
                if let Some(derived) = tab.derived.as_mut() {
                    derived.running = false;
                    derived.error = None;
                    derived.outcome = Some(outcome);
                }
                tab.query.error = None;
                tab.result_view = ResultView::Data;
                Vec::new()
            }
            Action::DerivedQueryFailed {
                tab_id,
                source_generation,
                derived_generation,
                connection,
                target,
                message,
            } => {
                let Some(tab) = self
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id() == tab_id)
                    .and_then(WorkspaceTab::as_console_mut)
                else {
                    return Vec::new();
                };
                if tab.generation != source_generation
                    || self.connection.active_identity() != Some(connection)
                    || tab.execution_target.as_ref() != Some(&target)
                    || tab
                        .derived
                        .as_ref()
                        .is_none_or(|derived| derived.generation != derived_generation)
                {
                    return Vec::new();
                }
                let message = crate::security::sanitize_terminal_text(&message);
                if let Some(derived) = tab.derived.as_mut() {
                    derived.running = false;
                    derived.error = Some(message.clone());
                }
                tab.query.error = Some(message);
                Vec::new()
            }
            Action::ManualStarted {
                tab_id,
                query_generation,
                transaction_generation,
                connection,
            } => {
                if self.manual_matches(
                    tab_id,
                    query_generation,
                    transaction_generation,
                    connection,
                    TransactionState::Starting,
                ) {
                    let tab = self
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.id() == tab_id)
                        .and_then(WorkspaceTab::as_console_mut)
                        .unwrap();
                    if let Ok(next) =
                        transaction::transition(tab_snapshot(tab), TransactionEvent::Started)
                    {
                        apply_transaction_snapshot(tab, next);
                    }
                }
                Vec::new()
            }
            Action::ManualStartFailed {
                tab_id,
                query_generation,
                transaction_generation,
                connection,
                message,
            } => {
                if self.manual_matches(
                    tab_id,
                    query_generation,
                    transaction_generation,
                    connection,
                    TransactionState::Starting,
                ) {
                    let tab = self
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.id() == tab_id)
                        .and_then(WorkspaceTab::as_console_mut)
                        .unwrap();
                    if let Ok(next) =
                        transaction::transition(tab_snapshot(tab), TransactionEvent::StartFailed)
                    {
                        apply_transaction_snapshot(tab, next);
                    }
                    tab.query_status = QueryStatus::Failed;
                    tab.output.push(OutputEntry {
                        kind: OutputKind::Error,
                        message,
                    });
                }
                Vec::new()
            }
            Action::ManualQueryFinished {
                tab_id,
                query_generation,
                transaction_generation,
                connection,
                outcome,
            } => {
                if self.manual_matches(
                    tab_id,
                    query_generation,
                    transaction_generation,
                    connection,
                    TransactionState::Active,
                ) {
                    self.finish_query(tab_id, query_generation, outcome, true);
                }
                Vec::new()
            }
            Action::ManualQueryFailed {
                tab_id,
                query_generation,
                transaction_generation,
                connection,
                message,
            } => {
                if self.manual_matches(
                    tab_id,
                    query_generation,
                    transaction_generation,
                    connection,
                    TransactionState::Active,
                ) {
                    let postgres = self
                        .active_profile()
                        .is_some_and(|profile| profile.kind == DatabaseKind::Postgres);
                    let tab = self
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.id() == tab_id)
                        .and_then(WorkspaceTab::as_console_mut)
                        .unwrap();
                    tab.query_status = QueryStatus::Failed;
                    append_failed_execution_output(tab, query_generation, message);
                    if postgres
                        && let Ok(next) = transaction::transition(
                            tab_snapshot(tab),
                            TransactionEvent::StatementFailed,
                        )
                    {
                        apply_transaction_snapshot(tab, next);
                    }
                } else if self.connection.active_identity() == Some(connection)
                    && let Some(tab) = self
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.id() == tab_id)
                        .and_then(WorkspaceTab::as_console_mut)
                    && tab.transaction_generation == transaction_generation
                    && tab.query_status == QueryStatus::Cancelled
                {
                    let event = if message.contains("acknowledgement was lost") {
                        TransactionEvent::OutcomeUnknown
                    } else {
                        TransactionEvent::RolledBack
                    };
                    if tab.transaction_state == TransactionState::Active
                        && let Ok(next) = transaction::transition(
                            tab_snapshot(tab),
                            if event == TransactionEvent::RolledBack {
                                TransactionEvent::Rollback
                            } else {
                                TransactionEvent::OutcomeUnknown
                            },
                        )
                    {
                        apply_transaction_snapshot(tab, next);
                        if event == TransactionEvent::RolledBack
                            && let Ok(next) = transaction::transition(
                                tab_snapshot(tab),
                                TransactionEvent::RolledBack,
                            )
                        {
                            apply_transaction_snapshot(tab, next);
                        }
                    }
                }
                Vec::new()
            }
            Action::ManualImplicitlyEnded {
                tab_id,
                query_generation,
                transaction_generation,
                connection,
            } => {
                if self.manual_matches(
                    tab_id,
                    query_generation,
                    transaction_generation,
                    connection,
                    TransactionState::Active,
                ) {
                    let tab = self
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.id() == tab_id)
                        .and_then(WorkspaceTab::as_console_mut)
                        .unwrap();
                    if let Ok(next) = transaction::transition(
                        tab_snapshot(tab),
                        TransactionEvent::ImplicitlyEnded,
                    ) {
                        apply_transaction_snapshot(tab, next);
                    }
                    tab.output.push(OutputEntry {
                        kind: OutputKind::Info,
                        message: "Transaction ended implicitly; prior work may have committed"
                            .to_owned(),
                    });
                }
                Vec::new()
            }
            Action::ManualCommitted {
                tab_id,
                query_generation,
                transaction_generation,
                connection,
            } => {
                if self.manual_matches(
                    tab_id,
                    query_generation,
                    transaction_generation,
                    connection,
                    TransactionState::Committing,
                ) {
                    let tab = self
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.id() == tab_id)
                        .and_then(WorkspaceTab::as_console_mut)
                        .unwrap();
                    if let Ok(next) =
                        transaction::transition(tab_snapshot(tab), TransactionEvent::Committed)
                    {
                        apply_transaction_snapshot(tab, next);
                        return self.finish_deferred(tab_id);
                    }
                }
                self.retain_failed_deferred();
                Vec::new()
            }
            Action::ManualCommitFailed {
                tab_id,
                query_generation,
                transaction_generation,
                connection,
                message,
                unknown,
            } => {
                if self.manual_matches(
                    tab_id,
                    query_generation,
                    transaction_generation,
                    connection,
                    TransactionState::Committing,
                ) {
                    let tab = self
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.id() == tab_id)
                        .and_then(WorkspaceTab::as_console_mut)
                        .unwrap();
                    let event = if unknown {
                        TransactionEvent::OutcomeUnknown
                    } else {
                        TransactionEvent::CommitFailed
                    };
                    if let Ok(next) = transaction::transition(tab_snapshot(tab), event) {
                        apply_transaction_snapshot(tab, next);
                    }
                    tab.output.push(OutputEntry {
                        kind: OutputKind::Error,
                        message,
                    });
                    self.retain_failed_deferred();
                }
                Vec::new()
            }
            Action::ManualRolledBack {
                tab_id,
                query_generation,
                transaction_generation,
                connection,
            } => {
                if self.manual_matches(
                    tab_id,
                    query_generation,
                    transaction_generation,
                    connection,
                    TransactionState::RollingBack,
                ) {
                    let tab = self
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.id() == tab_id)
                        .and_then(WorkspaceTab::as_console_mut)
                        .unwrap();
                    if let Ok(next) =
                        transaction::transition(tab_snapshot(tab), TransactionEvent::RolledBack)
                    {
                        apply_transaction_snapshot(tab, next);
                        return self.finish_deferred(tab_id);
                    }
                }
                self.retain_failed_deferred();
                Vec::new()
            }
            Action::ManualRollbackFailed {
                tab_id,
                query_generation,
                transaction_generation,
                connection,
                message,
                unknown,
            } => {
                if self.manual_matches(
                    tab_id,
                    query_generation,
                    transaction_generation,
                    connection,
                    TransactionState::RollingBack,
                ) {
                    let tab = self
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.id() == tab_id)
                        .and_then(WorkspaceTab::as_console_mut)
                        .unwrap();
                    let event = if unknown {
                        TransactionEvent::OutcomeUnknown
                    } else {
                        TransactionEvent::RollbackFailed
                    };
                    if let Ok(next) = transaction::transition(tab_snapshot(tab), event) {
                        apply_transaction_snapshot(tab, next);
                    }
                    tab.output.push(OutputEntry {
                        kind: OutputKind::Error,
                        message,
                    });
                    self.retain_failed_deferred();
                }
                Vec::new()
            }
            Action::ExplorerMove(delta) => {
                self.explorer.move_selection(delta);
                Vec::new()
            }
            Action::ExplorerViewportChanged(height) => {
                self.explorer.set_viewport_height(height);
                Vec::new()
            }
            Action::ExplorerSelectTarget(target) => {
                self.explorer.select_target(target);
                Vec::new()
            }
            Action::ExplorerScrollNodes { direction, amount } => {
                self.explorer.scroll_nodes(direction, amount);
                Vec::new()
            }
            Action::ExplorerAlignSelected(alignment) => {
                self.explorer.align_selected(alignment);
                Vec::new()
            }
            Action::ExplorerFindOpen => {
                if self.focus == Focus::Explorer {
                    let cancel = self.explorer.search.is_some();
                    self.explorer.search = None;
                    self.explorer.open_find();
                    if cancel {
                        return vec![Command::CancelCatalogSearch];
                    }
                }
                Vec::new()
            }
            Action::ExplorerFindInsert(character) => {
                self.explorer.edit_find(|query| query.push(character));
                Vec::new()
            }
            Action::ExplorerFindBackspace => {
                self.explorer.edit_find(|query| {
                    query.pop();
                });
                Vec::new()
            }
            Action::ExplorerFindClear => {
                self.explorer.edit_find(String::clear);
                Vec::new()
            }
            Action::ExplorerFindConfirm => {
                self.explorer.confirm_find();
                Vec::new()
            }
            Action::ExplorerFindNext => {
                self.explorer.move_find_match(1);
                Vec::new()
            }
            Action::ExplorerFindPrevious => {
                self.explorer.move_find_match(-1);
                Vec::new()
            }
            Action::ExplorerFindClose => {
                let restore = self.explorer.find.as_ref().is_some_and(|find| {
                    find.phase == crate::model::workspace::ExplorerSearchPhase::Editing
                });
                self.explorer.close_find(restore);
                Vec::new()
            }
            Action::ExplorerSearchOpen => {
                if self.focus == Focus::Explorer {
                    self.explorer.find = None;
                    self.next_search_session = self.next_search_session.saturating_add(1);
                    self.explorer
                        .open_search(self.database_command_identity(), self.next_search_session);
                }
                Vec::new()
            }
            Action::ExplorerSearchInsert(character) => {
                self.edit_explorer_search(|query| query.push(character))
            }
            Action::ExplorerSearchBackspace => self.edit_explorer_search(|query| {
                query.pop();
            }),
            Action::ExplorerSearchClear => self.edit_explorer_search(String::clear),
            Action::ExplorerSearchMove(delta) => {
                self.explorer.move_search(delta);
                Vec::new()
            }
            Action::ExplorerSearchNext => {
                self.explorer.move_search_match(1);
                Vec::new()
            }
            Action::ExplorerSearchPrevious => {
                self.explorer.move_search_match(-1);
                Vec::new()
            }
            Action::ExplorerSearchLocate => {
                let _ = self.explorer.locate_search_hit();
                Vec::new()
            }
            Action::ExplorerSearchClose => {
                let restore = self.explorer.search.as_ref().is_some_and(|search| {
                    search.phase == crate::model::workspace::ExplorerSearchPhase::Editing
                });
                if let Some(search) = self.explorer.search.take()
                    && restore
                {
                    self.explorer.normalized.selected = search.original_selected;
                    self.explorer.normalized.scroll = search.original_scroll;
                    self.explorer.sync_selected_index();
                }
                Vec::new()
            }
            Action::ExplorerSearchRetry => {
                self.explorer.refresh_frontend_search();
                Vec::new()
            }
            Action::ExplorerSelect(id) => {
                self.explorer.select_id(id);
                self.focus = Focus::Explorer;
                Vec::new()
            }
            Action::GridMove { rows, columns } => {
                self.move_grid(rows, columns);
                Vec::new()
            }
            Action::GridSelectRow(target) => {
                self.with_active_grid(|grid, (row_count, _)| {
                    grid.select_row_target(target, row_count);
                });
                Vec::new()
            }
            Action::GridScrollRows { direction, amount } => {
                self.with_active_grid(|grid, (row_count, _)| {
                    grid.scroll_rows(direction, amount, row_count);
                });
                Vec::new()
            }
            Action::GridAlignSelectedRow(alignment) => {
                self.with_active_grid(|grid, (row_count, _)| {
                    grid.align_selected_row(alignment, row_count);
                });
                Vec::new()
            }
            Action::GridSelect { row, column } => {
                self.focus = Focus::Results;
                self.select_grid(row, column);
                Vec::new()
            }
            Action::ExplorerToggle => self.toggle_explorer_selected(),
            Action::ExplorerExpand => self.expand_explorer_selected(),
            Action::ExplorerCollapse => self.collapse_explorer_selected(),
            Action::ExplorerPrimary => self.primary_explorer_selected(),
            Action::ExplorerRefresh => self.refresh_explorer_selected(),
            Action::ToggleResultView => {
                let tab = self.active_console_mut();
                tab.result_view = match tab.result_view {
                    ResultView::Data => ResultView::Output,
                    ResultView::Output | ResultView::Plan => ResultView::Data,
                };
                Vec::new()
            }
            Action::RelationEditCell => {
                self.relation_edit_cell();
                Vec::new()
            }
            Action::RelationEditInsert(character) => {
                self.relation_edit_insert(character);
                Vec::new()
            }
            Action::RelationEditBackspace => {
                self.relation_edit_input(|input| input.backspace());
                Vec::new()
            }
            Action::RelationEditDeletePreviousWord => {
                self.relation_edit_input(|input| input.delete_previous_word());
                Vec::new()
            }
            Action::RelationEditDeleteToStart => {
                self.relation_edit_input(|input| input.delete_to_start());
                Vec::new()
            }
            Action::RelationEditDelete => {
                self.relation_edit_input(|input| input.delete());
                Vec::new()
            }
            Action::RelationEditMoveLeft => {
                self.relation_edit_input(|input| input.move_left());
                Vec::new()
            }
            Action::RelationEditMoveRight => {
                self.relation_edit_input(|input| input.move_right());
                Vec::new()
            }
            Action::RelationEditMoveHome => {
                self.relation_edit_input(|input| input.move_home());
                Vec::new()
            }
            Action::RelationEditMoveEnd => {
                self.relation_edit_input(|input| input.move_end());
                Vec::new()
            }
            Action::RelationEditConfirm => self.relation_edit_confirm(),
            Action::RelationEditCancel => {
                self.relation_edit_cancel();
                Vec::new()
            }
            Action::RelationVisualLine => {
                self.relation_visual_line();
                Vec::new()
            }
            Action::RelationDeleteCurrent => self.relation_delete_current(),
            Action::RelationDeleteSelected => self.relation_delete_selected(),
            Action::RelationYank => {
                self.relation_yank(false);
                Vec::new()
            }
            Action::RelationYankSelected => {
                self.relation_yank(true);
                Vec::new()
            }
            Action::RelationPaste => self.relation_paste(),
            Action::RelationInsertRow => self.relation_insert_row(),
            Action::RelationUndo => self.relation_undo(),
            Action::RelationRedo => self.relation_redo(),
            Action::RelationCommit => self.relation_commit(true),
            Action::RelationRollback => self.relation_commit(false),
            Action::RelationTransactionStarted {
                tab_id,
                generation,
                connection,
            } => {
                self.relation_transaction_started(tab_id, generation, connection);
                Vec::new()
            }
            Action::RelationTransactionStartFailed {
                tab_id,
                generation,
                connection,
                message,
            } => {
                self.relation_transaction_failed(tab_id, generation, connection, message);
                Vec::new()
            }
            Action::RelationMutationSucceeded { request, result } => {
                self.relation_mutation_result(request, Ok(result));
                Vec::new()
            }
            Action::RelationMutationFailed { request, message } => {
                self.relation_mutation_result(request, Err(message));
                Vec::new()
            }
            Action::RelationCommitted {
                tab_id,
                generation,
                connection,
            } => {
                self.relation_transaction_finished(tab_id, generation, connection, true, None);
                Vec::new()
            }
            Action::RelationCommitFailed {
                tab_id,
                generation,
                connection,
                message,
                unknown,
            } => {
                self.relation_transaction_finished(
                    tab_id,
                    generation,
                    connection,
                    false,
                    Some((message, unknown)),
                );
                Vec::new()
            }
            Action::RelationRolledBack {
                tab_id,
                generation,
                connection,
            } => {
                self.relation_transaction_finished(tab_id, generation, connection, true, None);
                Vec::new()
            }
            Action::RelationRollbackFailed {
                tab_id,
                generation,
                connection,
                message,
                unknown,
            } => {
                self.relation_transaction_finished(
                    tab_id,
                    generation,
                    connection,
                    false,
                    Some((message, unknown)),
                );
                Vec::new()
            }
            Action::Quit => {
                let ids = self
                    .tabs
                    .iter()
                    .filter(|tab| self.transaction_needs_exit(tab.id()))
                    .map(|tab| tab.id())
                    .collect::<Vec<_>>();
                if ids.is_empty() {
                    self.should_quit = true;
                    vec![Command::Quit]
                } else {
                    self.defer_intent(DeferredIntent::Quit, ids)
                }
            }
        }
    }

    fn close_profile_manager(&mut self) {
        let Some(manager) = self.profile_manager.as_ref() else {
            return;
        };
        if manager.operation.is_some() {
            return;
        }
        self.profile_manager = None;
        if self.overlay == Some(Overlay::ProfileManager) {
            self.overlay = None;
        }
    }

    fn transaction_needs_exit(&self, console_id: Uuid) -> bool {
        self.tabs
            .iter()
            .find(|tab| tab.id() == console_id)
            .and_then(WorkspaceTab::as_console)
            .is_some_and(|tab| tab.transaction_state != TransactionState::Idle)
    }

    fn defer_intent<I>(&mut self, intent: DeferredIntent, console_ids: I) -> Vec<Command>
    where
        I: IntoIterator<Item = Uuid>,
    {
        for console_id in console_ids {
            let Some(tab) = self
                .tabs
                .iter()
                .find(|tab| tab.id() == console_id)
                .and_then(WorkspaceTab::as_console)
            else {
                continue;
            };
            self.deferred.push(DeferredTransactionPrompt {
                console_id,
                transaction_generation: tab.transaction_generation,
                intent,
            });
        }
        self.show_next_deferred();
        Vec::new()
    }

    fn show_next_deferred(&mut self) {
        if self.overlay.is_some() {
            return;
        }
        let Some(prompt) = self.deferred.pop() else {
            return;
        };
        let choice = self
            .tabs
            .iter()
            .find(|tab| tab.id() == prompt.console_id)
            .and_then(WorkspaceTab::as_console)
            .filter(|tab| tab.transaction_state == TransactionState::OutcomeUnknown)
            .map_or(TransactionExitChoice::Rollback, |_| {
                TransactionExitChoice::Abandon
            });
        self.overlay = Some(Overlay::TransactionExitConfirm { prompt, choice });
    }

    fn resolve_transaction_exit(&mut self, choice: TransactionExitChoice) -> Vec<Command> {
        let Some(Overlay::TransactionExitConfirm { prompt, .. }) = self.overlay.take() else {
            return Vec::new();
        };
        let Some(tab) = self
            .tabs
            .iter()
            .find(|tab| tab.id() == prompt.console_id)
            .and_then(WorkspaceTab::as_console)
        else {
            self.show_next_deferred();
            return self.replay_deferred(prompt.intent);
        };
        if tab.transaction_generation != prompt.transaction_generation {
            self.status_message("Stale transaction exit prompt discarded");
            self.show_next_deferred();
            return Vec::new();
        }
        if tab.query_status == QueryStatus::Running {
            self.overlay = Some(Overlay::TransactionExitConfirm {
                prompt,
                choice: TransactionExitChoice::Rollback,
            });
            self.status_message("Wait for the query to finish or cancel it before resolving");
            return Vec::new();
        }
        if choice == TransactionExitChoice::Cancel {
            self.show_next_deferred();
            return Vec::new();
        }
        if tab.transaction_state == TransactionState::OutcomeUnknown {
            let console_id = prompt.console_id;
            if let Some(tab) = self
                .tabs
                .iter_mut()
                .find(|tab| tab.id() == console_id)
                .and_then(WorkspaceTab::as_console_mut)
            {
                tab.transaction_state = TransactionState::Idle;
                tab.transaction_generation = tab.transaction_generation.saturating_add(1);
                tab.output.push(OutputEntry {
                    kind: OutputKind::Info,
                    message: "Transaction outcome is unknown; local state abandoned".to_owned(),
                });
            }
            return self.replay_deferred(prompt.intent);
        }
        if choice == TransactionExitChoice::Commit
            && tab.transaction_state == TransactionState::Aborted
        {
            self.overlay = Some(Overlay::TransactionExitConfirm {
                prompt,
                choice: TransactionExitChoice::Rollback,
            });
            self.status_message("COMMIT is unavailable for an aborted transaction");
            return Vec::new();
        }
        let commit = choice == TransactionExitChoice::Commit;
        let Some(connection) = self.database_command_identity() else {
            self.overlay = Some(Overlay::TransactionExitConfirm { prompt, choice });
            return Vec::new();
        };
        let tab = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id() == prompt.console_id)
            .and_then(WorkspaceTab::as_console_mut)
            .unwrap();
        let event = if commit {
            TransactionEvent::Commit
        } else {
            TransactionEvent::Rollback
        };
        let Ok(next) = transaction::transition(tab_snapshot(tab), event) else {
            self.overlay = Some(Overlay::TransactionExitConfirm { prompt, choice });
            return Vec::new();
        };
        tab.generation = tab.generation.saturating_add(1);
        let query_generation = tab.generation;
        let transaction_generation = tab.transaction_generation;
        apply_transaction_snapshot(tab, next);
        self.resolving_deferred = Some(prompt);
        if commit {
            vec![Command::ManualCommit {
                connection,
                tab_id: prompt.console_id,
                query_generation,
                transaction_generation,
            }]
        } else {
            vec![Command::ManualRollback {
                connection,
                tab_id: prompt.console_id,
                query_generation,
                transaction_generation,
            }]
        }
    }

    fn finish_deferred(&mut self, _console_id: Uuid) -> Vec<Command> {
        let Some(prompt) = self.resolving_deferred.take() else {
            return Vec::new();
        };
        if self
            .deferred
            .prompts
            .front()
            .is_some_and(|next| next.intent == prompt.intent)
        {
            self.show_next_deferred();
            return Vec::new();
        }
        self.replay_deferred(prompt.intent)
    }

    fn retain_failed_deferred(&mut self) {
        if let Some(prompt) = self.resolving_deferred.take() {
            self.deferred.prompts.push_front(prompt);
            self.show_next_deferred();
        }
    }

    fn replay_deferred(&mut self, intent: DeferredIntent) -> Vec<Command> {
        match intent {
            DeferredIntent::DeleteConsole(id) => {
                self.overlay = Some(Overlay::DeleteConsole { console_id: id });
                Vec::new()
            }
            DeferredIntent::CloseConsole => {
                let Some(id) = self.active_console_opt().map(|tab| tab.id) else {
                    return Vec::new();
                };
                self.close_console(id)
            }
            DeferredIntent::SetMode(TransactionMode::Auto) => {
                self.set_transaction_mode(TransactionMode::Auto)
            }
            DeferredIntent::SwitchConnection { profile_id, .. } => {
                self.request_connection(profile_id)
            }
            DeferredIntent::DeleteProfile {
                profile_id,
                request_id,
            } => vec![Command::DeleteProfile {
                request_id,
                profile_id,
            }],
            DeferredIntent::Disconnect { connection } => vec![Command::Disconnect { connection }],
            DeferredIntent::Quit => {
                self.should_quit = true;
                vec![Command::Quit]
            }
            DeferredIntent::SetMode(TransactionMode::Manual) => {
                self.set_transaction_mode(TransactionMode::Manual)
            }
        }
    }

    fn close_console(&mut self, id: Uuid) -> Vec<Command> {
        if let Some(index) = self.tabs.iter().position(|tab| tab.id() == id) {
            self.tabs.remove(index);
            if let Some(record) = self.sql_editors.iter_mut().find(|record| record.id == id) {
                record.open = false;
            }
            self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
            self.normalize_focus();
        }
        self.ensure_open_sql_editor();
        vec![self.persist_workspace_command()]
    }

    fn ensure_open_sql_editor(&mut self) {
        if !self.tabs.is_empty() {
            return;
        }
        if let Some(id) = self
            .sql_editors
            .iter()
            .find(|record| !record.open)
            .map(|record| record.id)
        {
            self.open_sql_editor(id);
        } else {
            self.create_sql_editor();
        }
        self.active_tab = 0;
        self.focus = Focus::Editor;
    }

    fn create_sql_editor(&mut self) {
        let name = format!("console_{}", self.next_console_number);
        self.next_console_number += 1;
        self.create_sql_editor_named(name);
    }

    fn create_sql_editor_named(&mut self, name: String) {
        let mut tab = ConsoleTab::new(name);
        tab.execution_target = self.active_profile().map(ExecutionTarget::from_profile);
        let id = tab.id;
        self.editor.open_console(id, "");
        self.sql_editors.push(ConsoleRecord {
            id,
            name: tab.name.clone(),
            execution_target: tab.execution_target.clone(),
            transaction_mode: tab.transaction_mode,
            open: true,
        });
        self.tabs.push(WorkspaceTab::Sql(tab));
    }

    fn open_sql_editor(&mut self, id: Uuid) {
        let Some(record) = self.sql_editors.iter().find(|record| record.id == id) else {
            return;
        };
        let mut tab = ConsoleTab::new(record.name.clone());
        tab.id = id;
        tab.execution_target = record.execution_target.clone();
        tab.transaction_mode = record.transaction_mode;
        if self.editor_text(id).is_err() {
            self.editor.open_console(id, "");
        }
        if let Some(record) = self.sql_editors.iter_mut().find(|record| record.id == id) {
            record.open = true;
        }
        self.tabs.push(WorkspaceTab::Sql(tab));
    }

    fn delete_console(&mut self, id: Uuid) -> Vec<Command> {
        self.tabs.retain(|tab| tab.id() != id);
        self.editor.close_console(id);
        self.sql_editors.retain(|record| record.id != id);
        if self.sql_editors.is_empty() {
            self.create_sql_editor();
        }
        self.ensure_open_sql_editor();
        self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
        vec![self.persist_workspace_command(), Command::DeleteSqlFile(id)]
    }

    fn activate_sql_editor(&mut self, id: Uuid) -> Vec<Command> {
        if let Some(index) = self.tabs.iter().position(|tab| tab.id() == id) {
            self.active_tab = index;
        } else if self.sql_editors.iter().any(|record| record.id == id) {
            self.open_sql_editor(id);
            self.active_tab = self.tabs.len() - 1;
        }
        self.focus = Focus::Editor;
        self.overlay = None;
        vec![self.persist_workspace_command()]
    }

    fn request_clear_outcome(&mut self) -> Vec<Command> {
        let tab = self.active_console();
        if tab.transaction_state != TransactionState::OutcomeUnknown {
            return Vec::new();
        }
        let Some(connection) = self.connection.active_identity() else {
            return Vec::new();
        };
        self.overlay = Some(Overlay::ClearTransactionOutcome {
            console_id: tab.id,
            connection,
            transaction_generation: tab.transaction_generation,
        });
        Vec::new()
    }

    fn confirm_clear_outcome(&mut self) -> Vec<Command> {
        let Some(Overlay::ClearTransactionOutcome {
            console_id,
            connection,
            transaction_generation,
        }) = self.overlay.take()
        else {
            return Vec::new();
        };
        let valid = self.connection.active_identity() == Some(connection)
            && self
                .tabs
                .iter()
                .find(|tab| tab.id() == console_id)
                .and_then(WorkspaceTab::as_console)
                .is_some_and(|tab| {
                    tab.transaction_generation == transaction_generation
                        && tab.transaction_state == TransactionState::OutcomeUnknown
                });
        if !valid {
            self.status_message("Stale transaction outcome verification discarded");
            return Vec::new();
        }
        let tab = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id() == console_id)
            .and_then(WorkspaceTab::as_console_mut)
            .unwrap();
        if let Ok(next) = transaction::transition(tab_snapshot(tab), TransactionEvent::ClearOutcome)
        {
            apply_transaction_snapshot(tab, next);
            tab.output.push(OutputEntry {
                kind: OutputKind::Info,
                message: "Transaction outcome cleared after external verification; no operation was retried".to_owned(),
            });
        }
        Vec::new()
    }

    fn idle_profile_manager_mut(
        &mut self,
        page: ProfileManagerPage,
    ) -> Option<&mut ProfileManagerState> {
        self.profile_manager
            .as_mut()
            .filter(|manager| manager.page == page && manager.operation.is_none())
    }

    fn editable_profile_manager_mut(&mut self) -> Option<&mut ProfileManagerState> {
        self.idle_profile_manager_mut(ProfileManagerPage::Form)
    }

    fn request_profile_delete(&mut self, profile_id: Uuid) {
        if !self.profiles.iter().any(|profile| profile.id == profile_id) {
            return;
        }
        let blocked = self.connection.profile_id == Some(profile_id) && self.has_running_query();
        let mut manager = ProfileManagerState {
            page: ProfileManagerPage::ConfirmDelete,
            delete_profile_id: Some(profile_id),
            ..ProfileManagerState::default()
        };
        if blocked {
            manager.message = Some("Cancel the running query before deleting this profile".into());
        }
        self.profile_manager = Some(manager);
        self.overlay = Some(Overlay::ProfileManager);
    }

    fn confirm_profile_delete(&mut self) -> Vec<Command> {
        let Some(profile_id) = self
            .profile_manager
            .as_ref()
            .and_then(|manager| manager.delete_profile_id)
        else {
            return Vec::new();
        };
        let blocked = self.connection.profile_id == Some(profile_id) && self.has_running_query();
        let active_console_id = self.active_console().id;
        let should_defer = self.connection.profile_id == Some(profile_id)
            && self.transaction_needs_exit(active_console_id);
        let deferred_console_ids = self
            .tabs
            .iter()
            .filter(|tab| self.transaction_needs_exit(tab.id()))
            .map(|tab| tab.id())
            .collect::<Vec<_>>();
        let Some(manager) = self.idle_profile_manager_mut(ProfileManagerPage::ConfirmDelete) else {
            return Vec::new();
        };
        if blocked {
            manager.message = Some("Cancel the running query before deleting this profile".into());
            return Vec::new();
        }
        let request_id = next_profile_request(manager);
        if should_defer {
            return self.defer_intent(
                DeferredIntent::DeleteProfile {
                    profile_id,
                    request_id,
                },
                deferred_console_ids,
            );
        }
        manager.operation = Some(ProfileOperation::Deleting);
        manager.message = None;
        vec![Command::DeleteProfile {
            request_id,
            profile_id,
        }]
    }

    fn test_profile_draft(&mut self) -> Vec<Command> {
        let profiles = &self.profiles;
        let Some(manager) = self.profile_manager.as_mut().filter(|manager| {
            manager.page == ProfileManagerPage::Form && manager.operation.is_none()
        }) else {
            return Vec::new();
        };
        if manager.commit_url().is_err() {
            return Vec::new();
        }
        let Some(draft) = manager.draft.as_ref() else {
            return Vec::new();
        };
        let submission = match draft.validate(profiles) {
            Ok(submission) => submission,
            Err(error) => {
                manager.selected_field = error.field;
                manager.message = Some(error.message);
                return Vec::new();
            }
        };
        let request_id = next_profile_request(manager);
        if let Some(draft) = manager.draft.as_mut() {
            draft.begin_catalog_discovery(submission.discovery_fingerprint);
        }
        manager.operation = Some(ProfileOperation::Testing);
        manager.message = Some("Testing connection...".into());
        vec![Command::TestProfile {
            request_id,
            submission,
        }]
    }

    fn open_profile_scope(&mut self, force: bool) -> Vec<Command> {
        let profiles = &self.profiles;
        let Some(manager) = self.profile_manager.as_mut() else {
            return Vec::new();
        };
        if manager.page == ProfileManagerPage::Form {
            if manager.commit_url().is_err() {
                return Vec::new();
            }
            manager.open_scope_picker();
        } else if manager.page != ProfileManagerPage::Scope {
            return Vec::new();
        }
        if manager.scope_discovery_loading() {
            return Vec::new();
        }
        let Some(draft) = manager.draft.as_ref() else {
            return Vec::new();
        };
        let submission = match draft.validate(profiles) {
            Ok(submission) => submission,
            Err(error) => {
                manager.close_scope_picker();
                manager.selected_field = error.field;
                manager.message = Some(error.message);
                return Vec::new();
            }
        };
        if !force
            && matches!(
                &draft.catalog_discovery,
                crate::model::profile_manager::CatalogDiscoveryState::Fresh(snapshot)
                    if snapshot.fingerprint == submission.discovery_fingerprint
            )
        {
            return Vec::new();
        }
        let request_id = next_profile_request(manager);
        manager.begin_scope_discovery(request_id, submission.discovery_fingerprint);
        vec![Command::DiscoverProfileCatalog {
            request_id,
            submission,
        }]
    }

    fn save_profile_draft(&mut self, connect: bool) -> Vec<Command> {
        let target_profile_id = self
            .profile_manager
            .as_ref()
            .and_then(|manager| manager.draft.as_ref())
            .map(|draft| draft.profile_id());
        let requires_idle_connection = target_profile_id
            .is_some_and(|profile_id| connect || self.connection.profile_id == Some(profile_id));
        if requires_idle_connection && self.has_running_query() {
            if let Some(manager) = self.editable_profile_manager_mut() {
                manager.message = Some(
                    "Cancel the running query before saving or connecting this profile".into(),
                );
            }
            return Vec::new();
        }

        let profiles = &self.profiles;
        let Some(manager) = self.profile_manager.as_mut().filter(|manager| {
            manager.page == ProfileManagerPage::Form && manager.operation.is_none()
        }) else {
            return Vec::new();
        };
        if manager.commit_url().is_err() {
            return Vec::new();
        }
        let Some(draft) = manager.draft.as_ref() else {
            return Vec::new();
        };
        let submission = match draft.validate(profiles) {
            Ok(submission) => submission,
            Err(error) => {
                manager.selected_field = error.field;
                manager.message = Some(error.message);
                return Vec::new();
            }
        };
        let request_id = next_profile_request(manager);
        manager.operation = Some(if connect {
            ProfileOperation::SavingAndConnecting
        } else {
            ProfileOperation::Saving
        });
        manager.message = Some("Saving profile...".into());
        vec![Command::SaveProfile {
            request_id,
            submission,
            connect,
        }]
    }

    fn profile_saved(
        &mut self,
        request_id: u64,
        profile: ConnectionProfile,
        warning: Option<String>,
        change: crate::model::profile_manager::ProfileChange,
        connect: bool,
    ) -> Vec<Command> {
        let expected = if connect {
            ProfileOperation::SavingAndConnecting
        } else {
            ProfileOperation::Saving
        };
        if !self.profile_operation_matches(request_id, &[expected]) {
            return Vec::new();
        }

        let profile_id = profile.id;
        let scope_changed = change.catalog_scope_changed;
        let mut commands = if scope_changed {
            self.cancel_relation_requests_for_profile(profile_id)
        } else {
            Vec::new()
        };
        add_explorer_profile(&mut self.explorer, &profile, ProfileProvenance::Saved);
        if scope_changed && let Some(state) = self.explorer.normalized.profiles.get_mut(&profile_id)
        {
            if state.advance_catalog_epoch().is_none() {
                state.last_error = Some("catalog epoch exhausted".to_owned());
                return Vec::new();
            }
            state.load_states.clear();
            state.pending_requests.clear();
            state.load_errors.clear();
        }
        if let Some(existing) = self
            .profiles
            .iter_mut()
            .find(|existing| existing.id == profile_id)
        {
            *existing = profile;
        } else {
            self.profiles.push(profile);
        }
        if let Some(manager) = self.profile_manager.as_mut() {
            manager.draft = None;
            manager.selected_field = ProfileField::Kind;
            manager.operation = None;
            manager.message = warning.or_else(|| Some("Profile saved".into()));
        }
        if !connect
            && !change.connection_settings_changed
            && !change.credentials_changed
            && scope_changed
            && self.connection.profile_id == Some(profile_id)
        {
            self.explorer.completion_index = Default::default();
            self.active_console_mut().completion = None;
            self.profile_manager = None;
            self.overlay = None;
            commands.extend(self.start_catalog_request(
                CatalogTarget::Databases,
                None,
                CatalogRequestIntent::Refresh,
            ));
            return commands;
        }
        if !connect
            && !change.connection_settings_changed
            && !change.credentials_changed
            && !scope_changed
            && change.display_only_changed
            && self.connection.profile_id == Some(profile_id)
        {
            self.profile_manager = None;
            self.overlay = None;
            return Vec::new();
        }
        if !connect {
            if self.connection.profile_id == Some(profile_id) && self.has_running_query() {
                if let Some(manager) = self.profile_manager.as_mut() {
                    manager.message =
                        Some("Profile saved; cancel the running query before reconnecting".into());
                }
                return Vec::new();
            }
            commands.extend(self.retire_profile_connections(profile_id, None));
            self.profile_manager = None;
            self.overlay = None;
            return commands;
        }
        if self.has_running_query() {
            if let Some(manager) = self.profile_manager.as_mut() {
                manager.message =
                    Some("Profile saved; cancel the running query before connecting".into());
            }
            return Vec::new();
        }
        commands.extend(self.request_connection(profile_id));
        if !commands.is_empty()
            && let Some(manager) = self.profile_manager.as_mut()
        {
            manager.operation = Some(ProfileOperation::Connecting);
        }
        commands
    }

    fn profile_deleted(
        &mut self,
        request_id: u64,
        profile_id: Uuid,
        active_connection: Option<ConnectionIdentity>,
    ) -> Vec<Command> {
        if !self.profile_operation_matches(request_id, &[ProfileOperation::Deleting]) {
            return Vec::new();
        }
        self.explorer.normalized.remove_profile(profile_id);
        self.profiles.retain(|profile| profile.id != profile_id);
        self.profile_manager = None;
        self.overlay = None;
        self.retire_profile_connections(profile_id, active_connection)
    }

    fn profile_operation_matches(&self, request_id: u64, operations: &[ProfileOperation]) -> bool {
        self.profile_manager.as_ref().is_some_and(|manager| {
            manager.request_generation == request_id
                && manager
                    .operation
                    .is_some_and(|operation| operations.contains(&operation))
        })
    }

    fn matching_profile_operation(
        &mut self,
        request_id: u64,
        operations: &[ProfileOperation],
    ) -> Option<&mut ProfileManagerState> {
        self.profile_manager.as_mut().filter(|manager| {
            manager.request_generation == request_id
                && manager
                    .operation
                    .is_some_and(|operation| operations.contains(&operation))
        })
    }

    fn request_connection(&mut self, profile_id: Uuid) -> Vec<Command> {
        if !self.profiles.iter().any(|profile| profile.id == profile_id) || self.has_running_query()
        {
            return Vec::new();
        }
        let Some(profile) = self
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
        else {
            return Vec::new();
        };
        let target = self
            .active_console_opt()
            .and_then(|tab| tab.execution_target.clone())
            .filter(|target| target.profile_id == profile_id && target.is_valid(profile))
            .unwrap_or_else(|| ExecutionTarget::from_profile(profile));
        if self.active_console_opt().is_some_and(|tab| {
            tab.execution_target.as_ref().is_none_or(|current| {
                current.profile_id != profile_id || !current.is_valid(profile)
            })
        }) {
            self.pending_target_console = self.active_console_opt().map(|tab| tab.id);
        }
        if self.connection.profile_id.is_none()
            && self.tabs.len() == 1
            && self.active_console_opt().is_some_and(|tab| {
                tab.generation == 0
                    && tab
                        .execution_target
                        .as_ref()
                        .is_none_or(|old| old.profile_id != profile_id)
            })
        {
            self.active_console_mut().execution_target = Some(target.clone());
        }
        self.request_connection_target(target)
    }

    fn request_connection_target(&mut self, target: ExecutionTarget) -> Vec<Command> {
        let profile_id = target.profile_id;
        if !self.profiles.iter().any(|profile| target.is_valid(profile)) || self.has_running_query()
        {
            self.pending_target_console = None;
            return Vec::new();
        }
        let latest_generation = self
            .connection_request_generation
            .max(self.connection_terminal_generation)
            .max(self.connection.generation)
            .max(self.connection.pending_generation.unwrap_or(0));
        let Some(generation) = latest_generation.checked_add(1) else {
            self.connection.error =
                Some("Connection generation exhausted; restart LazyDB to reconnect".into());
            return Vec::new();
        };
        self.connection_request_generation = generation;
        self.connection.pending_profile_id = Some(profile_id);
        self.connection.pending_generation = Some(generation);
        self.connection.pending_target = Some(target.clone());
        self.connection.status = ConnectionStatus::Connecting;
        self.connection.error = None;
        let mut commands = self.cancel_relation_requests_for_connection(Some(ConnectionIdentity {
            profile_id,
            generation,
        }));
        if let Some(active_profile_id) = self.connection.profile_id.filter(|id| *id != profile_id)
            && let Some(state) = self
                .explorer
                .normalized
                .profiles
                .get_mut(&active_profile_id)
        {
            state.status = ExplorerConnectionStatus::Online;
        }
        if let Some(state) = self.explorer.normalized.profiles.get_mut(&profile_id) {
            state.status = ExplorerConnectionStatus::Linking;
            state.last_error = None;
        }
        commands.push(Command::Connect {
            profile_id,
            generation,
            target,
        });
        commands
    }

    fn execution_target_candidates(&self, profile: &ConnectionProfile) -> Vec<ExecutionTarget> {
        let default = ExecutionTarget::from_profile(profile);
        let mut values = BTreeSet::new();
        if default.is_valid(profile) {
            values.insert((default.database.clone(), default.schema.clone()));
        }
        if let Some(state) = self.explorer.normalized.profiles.get(&profile.id) {
            for entry in state.catalog.entries().values() {
                let selectable = entry.kind == crate::db::catalog::CatalogKind::Schema
                    || (profile.kind == DatabaseKind::MySql
                        && entry.kind == crate::db::catalog::CatalogKind::Database);
                if !selectable {
                    continue;
                }
                let Some(database) = entry.qualified_name.database.clone() else {
                    continue;
                };
                let schema = match profile.kind {
                    DatabaseKind::MySql => Some(database.clone()),
                    DatabaseKind::Postgres | DatabaseKind::Sqlite => {
                        entry.qualified_name.schema.clone()
                    }
                };
                let candidate = ExecutionTarget {
                    profile_id: profile.id,
                    database,
                    schema,
                };
                if candidate.is_valid(profile) {
                    values.insert((candidate.database, candidate.schema));
                }
            }
        }
        values
            .into_iter()
            .map(|(database, schema)| ExecutionTarget {
                profile_id: profile.id,
                database,
                schema,
            })
            .collect()
    }

    fn cancel_relation_requests_for_connection(
        &mut self,
        next_connection: Option<ConnectionIdentity>,
    ) -> Vec<Command> {
        let mut commands = Vec::new();
        for tab in &mut self.tabs {
            let WorkspaceTab::Relation(tab) = tab else {
                continue;
            };
            for pending in [
                cancel_pending_relation(&mut tab.data),
                cancel_pending_relation(&mut tab.ddl),
            ] {
                if let Some(request) = pending
                    && next_connection.is_none_or(|connection| request.connection != connection)
                {
                    commands.push(Command::CancelRelationRequest(request));
                }
            }
        }
        commands
    }

    fn cancel_relation_requests_for_profile(&mut self, profile_id: Uuid) -> Vec<Command> {
        let mut commands = Vec::new();
        for tab in &mut self.tabs {
            let WorkspaceTab::Relation(tab) = tab else {
                continue;
            };
            for pending in [
                cancel_pending_relation(&mut tab.data),
                cancel_pending_relation(&mut tab.ddl),
            ] {
                if let Some(request) = pending
                    && request.connection.profile_id == profile_id
                {
                    commands.push(Command::CancelRelationRequest(request));
                }
            }
        }
        commands
    }

    fn retire_profile_connections(
        &mut self,
        profile_id: Uuid,
        runtime_active: Option<ConnectionIdentity>,
    ) -> Vec<Command> {
        let mut commands = self.cancel_relation_requests_for_profile(profile_id);
        let active = self
            .connection
            .active_identity()
            .filter(|connection| connection.profile_id == profile_id);
        let pending = self
            .connection
            .pending_identity()
            .filter(|connection| connection.profile_id == profile_id);
        let mut identities = Vec::new();
        if let Some(connection) = active {
            identities.push(connection);
            self.connection.profile_id = None;
            self.connection.generation = 0;
            self.connection.server = None;
            self.connection.target = None;
            self.clear_active_catalog(profile_id);
            self.select_nearest_profile(profile_id);
        }
        if let Some(connection) = pending {
            identities.push(connection);
            self.connection.pending_profile_id = None;
            self.connection.pending_generation = None;
            self.connection.pending_target = None;
            self.pending_target_console = None;
        }
        if let Some(connection) =
            runtime_active.filter(|connection| connection.profile_id == profile_id)
            && !identities.contains(&connection)
        {
            identities.push(connection);
        }
        if let Some(generation) = identities
            .iter()
            .map(|connection| connection.generation)
            .max()
        {
            self.connection_terminal_generation =
                self.connection_terminal_generation.max(generation);
            self.connection.error = None;
            self.connection.status = if self.connection.pending_profile_id.is_some() {
                ConnectionStatus::Connecting
            } else if self.connection.profile_id.is_some() {
                ConnectionStatus::Connected
            } else {
                ConnectionStatus::Disconnected
            };
        }
        commands.extend(
            identities
                .into_iter()
                .map(|connection| Command::Disconnect { connection })
                .collect::<Vec<_>>(),
        );
        commands
    }

    fn database_command_identity(&self) -> Option<ConnectionIdentity> {
        if self.connection.status != ConnectionStatus::Connected
            || self.connection.pending_profile_id.is_some()
            || self.profile_operation_blocks_database_commands()
        {
            return None;
        }
        self.connection.active_identity()
    }

    fn profile_operation_blocks_database_commands(&self) -> bool {
        let Some(manager) = self.profile_manager.as_ref() else {
            return false;
        };
        match manager.operation {
            Some(ProfileOperation::SavingAndConnecting) => true,
            Some(ProfileOperation::Saving) => manager
                .draft
                .as_ref()
                .is_some_and(|draft| Some(draft.profile_id()) == self.connection.profile_id),
            Some(ProfileOperation::Deleting) => {
                manager.delete_profile_id == self.connection.profile_id
            }
            _ => false,
        }
    }

    fn has_running_query(&self) -> bool {
        self.tabs.iter().any(|tab| {
            tab.as_console()
                .is_some_and(|tab| tab.query_status == QueryStatus::Running)
                || matches!(tab, WorkspaceTab::Relation(relation) if match relation.view {
                    RelationView::Data => matches!(relation.data, RelationLoad::Loading { .. }),
                    RelationView::Ddl => matches!(relation.ddl, RelationLoad::Loading { .. }),
                })
        })
    }

    fn apply_editor_effects(&mut self, completion: CompletionAfterEdit) -> Vec<Command> {
        let effects = self.editor.drain_effects();
        let mut commands = Vec::new();
        for effect in effects {
            let action = match effect {
                EditorEffect::Changed { .. } => {
                    if matches!(completion, CompletionAfterEdit::Suppress) {
                        self.active_console_mut().completion = None;
                        continue;
                    }
                    if self.active_editor_mode() != EditorMode::Insert {
                        self.active_console_mut().completion = None;
                        continue;
                    }
                    if self
                        .active_editor_text()
                        .is_ok_and(|text| text.ends_with('.'))
                    {
                        commands.extend(self.complete_now());
                    } else if let Some(key) = self.completion_key() {
                        commands.push(Command::ScheduleCompletion(key));
                    }
                    continue;
                }
                EditorEffect::Message(_)
                | EditorEffect::BackwardSearch
                | EditorEffect::ClearTransactionOutcome
                | EditorEffect::SetConnectionTarget(_)
                | EditorEffect::SetDatabaseTarget(_)
                | EditorEffect::SetSchemaTarget(_) => continue,
                EditorEffect::OpenTargetSelector => Action::OpenTargetSelector,
                EditorEffect::ToggleTransaction => {
                    Action::SetTransactionMode(match self.active_console().transaction_mode {
                        TransactionMode::Auto => TransactionMode::Manual,
                        TransactionMode::Manual => TransactionMode::Auto,
                    })
                }
                EditorEffect::SetTransactionModeRequested { manual } => {
                    Action::SetTransactionMode(if manual {
                        TransactionMode::Manual
                    } else {
                        TransactionMode::Auto
                    })
                }
                EditorEffect::Commit => Action::CommitTransaction,
                EditorEffect::Rollback => Action::RollbackTransaction,
                EditorEffect::SubstituteConfirmRequested { count } => {
                    self.overlay = Some(Overlay::SubstituteConfirm { remaining: count });
                    continue;
                }
                EditorEffect::FormatCurrent => {
                    self.format_current();
                    continue;
                }
                EditorEffect::RunCurrent => Action::RunActiveSql,
                EditorEffect::RunAll => Action::RunAllSql,
                EditorEffect::NewConsole => Action::NewConsole,
                EditorEffect::GotoSqlConsole => Action::GotoSqlConsole,
                EditorEffect::CloseConsole => Action::CloseActiveTab,
                EditorEffect::DeleteConsole => Action::RequestDeleteActiveConsole,
                EditorEffect::OpenSqlEditorList => Action::OpenSqlEditorList,
                EditorEffect::FocusPane(focus) => Action::Focus(focus),
                EditorEffect::NextTab => Action::NextTab,
                EditorEffect::PreviousTab => Action::PreviousTab,
                EditorEffect::ShowHelp => Action::ShowHelp,
                EditorEffect::Quit => Action::Quit,
            };
            commands.extend(self.update(action));
        }
        commands
    }

    fn completion_key(&self) -> Option<CompletionScheduleKey> {
        if self.active_editor_mode() != EditorMode::Insert {
            return None;
        }
        let tab = self.active_console_opt()?;
        Some(CompletionScheduleKey {
            console_id: tab.id,
            document_revision: self.active_editor_revision(),
            connection: self.connection.active_identity()?,
            catalog_generation: self.explorer.catalog_generation,
        })
    }

    fn complete_now(&mut self) -> Vec<Command> {
        if self.active_editor_mode() != EditorMode::Insert {
            self.active_console_mut().completion = None;
            return Vec::new();
        }
        let text = self.active_editor_text().unwrap_or_default();
        let snapshot = self
            .active_editor_render_snapshot(EditorViewport {
                width: 0,
                height: 0,
            })
            .ok();
        let cursor = snapshot
            .as_ref()
            .map(|snapshot| cursor_byte(&text, snapshot.cursor.line, snapshot.cursor.column))
            .unwrap_or(text.len());
        let relation_ids = sql::relation_ids_for_completion(
            &text,
            cursor,
            self.sql_dialect(),
            &self.explorer.completion_index,
        );
        let mut commands = Vec::new();
        for relation in relation_ids {
            let owner = crate::model::explorer::ExplorerOwnerId::Catalog(relation.clone());
            let loaded = self
                .explorer
                .normalized
                .profiles
                .get(&relation.profile_id())
                .is_some_and(|profile| {
                    matches!(
                        profile.load_states.get(&owner),
                        Some(crate::model::explorer::ExplorerLoadState::Loaded {
                            next_cursor: None
                        })
                    )
                });
            if !loaded {
                commands.extend(self.start_catalog_request(
                    CatalogTarget::relation_children(relation).unwrap(),
                    None,
                    CatalogRequestIntent::Completion,
                ));
            }
        }
        let completion_context = self
            .active_console_opt()
            .and_then(|tab| tab.execution_target.as_ref())
            .map(|target| sql::CompletionContext {
                database: Some(target.database.as_str()),
                schema: target.schema.as_deref(),
            })
            .unwrap_or_default();
        let candidates = sql::complete(
            &text,
            cursor,
            self.sql_dialect(),
            &self.explorer.completion_index,
            completion_context,
        );
        let Some(tab) = self.active_console_opt_mut() else {
            return Vec::new();
        };
        tab.completion = (!candidates.is_empty()).then_some(CompletionPopup {
            candidates,
            selected: 0,
        });
        commands
    }

    fn accept_completion(&mut self) -> Vec<Command> {
        if self.active_editor_mode() != EditorMode::Insert {
            self.active_console_mut().completion = None;
            return Vec::new();
        }
        let Some(id) = self.active_console_opt().map(|tab| tab.id) else {
            return Vec::new();
        };
        let Some(popup) = self.active_console_mut().completion.take() else {
            return Vec::new();
        };
        let Some(candidate) = popup.candidates.get(popup.selected).cloned() else {
            return Vec::new();
        };
        let _ = self.editor.replace_range(
            id,
            candidate.replace,
            &candidate.insert_text,
            crate::editor::ReplacementCursor::EndOfInsertion,
        );
        self.apply_editor_effects(CompletionAfterEdit::Suppress)
    }

    pub(crate) fn sql_dialect(&self) -> SqlDialect {
        match self.active_profile().map(|profile| profile.kind) {
            Some(DatabaseKind::Postgres) => SqlDialect::Postgres,
            Some(DatabaseKind::MySql) => SqlDialect::MySql,
            Some(DatabaseKind::Sqlite) => SqlDialect::Sqlite,
            None => SqlDialect::Generic,
        }
    }

    fn format_current(&mut self) {
        let Some(id) = self.active_console_opt().map(|tab| tab.id) else {
            return;
        };
        let dialect = self.sql_dialect();
        let scope = match self.editor.current_scope(id, dialect) {
            Ok(Some(scope)) => scope,
            _ => {
                self.overlay = Some(Overlay::Message {
                    title: "FORMAT".into(),
                    body: "No SQL scope at cursor".into(),
                });
                return;
            }
        };
        if scope.kind == sql::ScopeKind::VisualBlock
            || matches!(scope.source, ScopeSource::Block(_))
        {
            self.overlay = Some(Overlay::Message {
                title: "FORMAT".into(),
                body: "Visual Block formatting is unsupported; select a contiguous range".into(),
            });
            return;
        }
        let formatted = match sql::format_sql(&scope.sql, dialect) {
            Ok(formatted) => formatted,
            Err(error) => {
                self.overlay = Some(Overlay::Message {
                    title: "FORMAT".into(),
                    body: error.to_string(),
                });
                return;
            }
        };
        let ScopeSource::Contiguous(range) = scope.source else {
            return;
        };
        if let Err(error) = self.editor.replace_range(
            id,
            range,
            &formatted,
            crate::editor::ReplacementCursor::Start,
        ) {
            self.overlay = Some(Overlay::Message {
                title: "FORMAT".into(),
                body: error.to_string(),
            });
        }
    }

    fn set_transaction_mode(&mut self, mode: TransactionMode) -> Vec<Command> {
        let Some(tab) = self.active_console_opt_mut() else {
            return Vec::new();
        };
        let event = match mode {
            TransactionMode::Manual => TransactionEvent::EnterManual,
            TransactionMode::Auto => TransactionEvent::SetAuto,
        };
        if let Ok(next) = transaction::transition(tab_snapshot(tab), event) {
            apply_transaction_snapshot(tab, next);
        }
        Vec::new()
    }

    fn transaction_control(&mut self, commit: bool) -> Vec<Command> {
        let tab = self.active_console();
        if tab.transaction_mode != TransactionMode::Manual {
            return Vec::new();
        }
        let event = if commit {
            TransactionEvent::Commit
        } else {
            TransactionEvent::Rollback
        };
        let Ok(next) = transaction::transition(tab_snapshot(tab), event) else {
            return Vec::new();
        };
        let id = tab.id;
        let query_generation = tab.generation.saturating_add(1);
        let transaction_generation = tab.transaction_generation;
        let connection = self.database_command_identity();
        let tab = self.active_console_mut();
        tab.generation = query_generation;
        apply_transaction_snapshot(tab, next);
        let Some(connection) = connection else {
            return Vec::new();
        };
        if commit {
            vec![Command::ManualCommit {
                connection,
                tab_id: id,
                query_generation,
                transaction_generation,
            }]
        } else {
            vec![Command::ManualRollback {
                connection,
                tab_id: id,
                query_generation,
                transaction_generation,
            }]
        }
    }

    fn run_active_sql(&mut self, full_buffer: bool) -> Vec<Command> {
        let Some(connection) = self.database_command_identity() else {
            self.status_message("No active database connection");
            return Vec::new();
        };
        let tab_id = self.active_console().id;
        let sql = self.editor_text(tab_id).unwrap_or_default();
        let dialect = self.sql_dialect();
        let scope = if full_buffer {
            (!sql.trim().is_empty()).then(|| sql::ResolvedScope {
                kind: sql::ScopeKind::FullBuffer,
                source: ScopeSource::Contiguous(sql::TextRange::new(0, sql.len())),
                sql: sql.clone(),
            })
        } else {
            self.editor.current_scope(tab_id, dialect).ok().flatten()
        };
        let Some(scope) = scope else {
            self.status_message("No SQL scope at cursor");
            return Vec::new();
        };
        match sql::classify_transaction_sql(&scope.sql, dialect) {
            sql::TransactionSqlClassification::Control(control) => {
                return self.dispatch_transaction_sql(tab_id, connection, control);
            }
            sql::TransactionSqlClassification::Unsupported(_) => {}
            sql::TransactionSqlClassification::Data { .. } => {}
        }
        let tab = self.active_console();
        if tab.query_status == QueryStatus::Running {
            return Vec::new();
        }
        let Some(target) = tab.execution_target.clone() else {
            self.status_message("Select an execution target before running SQL");
            return Vec::new();
        };
        let draft = sql::ExecutionDraft::new(
            tab_id,
            tab.generation,
            connection,
            target,
            tab.transaction_generation,
            self.active_editor_revision(),
            scope.kind,
            scope.source,
            scope.sql,
            dialect,
            tab.transaction_mode,
            tab.transaction_state,
        );
        if draft.has_mixed_transaction_control() {
            self.status_message("Mixed transaction-control and data SQL is rejected");
            return Vec::new();
        }
        if draft.requires_confirmation(self.confirmation_policy == ConfirmationPolicy::Always) {
            self.overlay = Some(Overlay::ExecutionConfirm {
                draft,
                focus: ExecutionConfirmFocus::Cancel,
            });
            return Vec::new();
        }
        self.dispatch_draft(draft)
    }

    fn dispatch_transaction_sql(
        &mut self,
        tab_id: Uuid,
        connection: ConnectionIdentity,
        control: sql::TransactionControl,
    ) -> Vec<Command> {
        use sql::TransactionControl;
        let tab = self.active_console();
        let Some(target) = tab.execution_target.clone() else {
            self.status_message("Select an execution target before running SQL");
            return Vec::new();
        };
        match control {
            TransactionControl::Begin(_)
                if tab.transaction_mode == TransactionMode::Auto
                    && tab.transaction_state == TransactionState::Idle =>
            {
                let next =
                    transaction::transition(tab_snapshot(tab), TransactionEvent::EnterManual)
                        .and_then(|s| transaction::transition(s, TransactionEvent::Start));
                let Ok(next) = next else { return Vec::new() };
                let query_generation = tab.generation.saturating_add(1);
                let transaction_generation = next.generation;
                let tab = self.active_console_mut();
                tab.generation = query_generation;
                apply_transaction_snapshot(tab, next);
                vec![Command::ManualBegin {
                    connection,
                    target,
                    tab_id,
                    query_generation,
                    transaction_generation,
                }]
            }
            TransactionControl::Commit => self.transaction_control(true),
            TransactionControl::Rollback => self.transaction_control(false),
            TransactionControl::Savepoint(_)
            | TransactionControl::ReleaseSavepoint(_)
            | TransactionControl::RollbackToSavepoint(_)
                if tab.transaction_mode == TransactionMode::Manual
                    && tab.transaction_state == TransactionState::Active =>
            {
                self.dispatch_manual_sql(
                    tab_id,
                    connection,
                    self.editor_text(tab_id).unwrap_or_default(),
                )
            }
            _ => {
                self.status_message(
                    "Transaction control is unavailable in the current transaction state",
                );
                Vec::new()
            }
        }
    }

    fn dispatch_manual_sql(
        &mut self,
        tab_id: Uuid,
        connection: ConnectionIdentity,
        sql: String,
    ) -> Vec<Command> {
        let tab = self.active_console();
        let Some(target) = tab.execution_target.clone() else {
            self.status_message("Select an execution target before running SQL");
            return Vec::new();
        };
        if tab.transaction_state != TransactionState::Idle
            && tab.transaction_state != TransactionState::Active
        {
            return Vec::new();
        }
        let mut next = tab_snapshot(tab);
        let starting = next.state == TransactionState::Idle;
        if starting {
            next = match transaction::transition(next, TransactionEvent::Start) {
                Ok(next) => next,
                Err(_) => return Vec::new(),
            };
        }
        let query_generation = tab.generation.saturating_add(1);
        let transaction_generation = next.generation;
        let tab = self.active_console_mut();
        tab.generation = query_generation;
        apply_transaction_snapshot(tab, next);
        vec![Command::ManualExecute {
            connection,
            target,
            tab_id,
            query_generation,
            transaction_generation,
            sql,
        }]
    }

    fn confirm_execution(&mut self) -> Vec<Command> {
        let Some(Overlay::ExecutionConfirm { draft, focus }) = self.overlay.take() else {
            return Vec::new();
        };
        if focus == ExecutionConfirmFocus::Cancel {
            self.retain_execution(draft, ExecutionResult::Cancelled);
            return Vec::new();
        }
        if let Err(message) = self.validate_draft(&draft) {
            self.status_message(&message);
            self.retain_execution(draft, ExecutionResult::Cancelled);
            return Vec::new();
        }
        if draft.has_transaction_control() {
            self.status_message("Transaction-control execution is unavailable until Task 16");
            self.retain_execution(draft, ExecutionResult::Cancelled);
            return Vec::new();
        }
        self.dispatch_draft(draft)
    }

    fn cancel_execution(&mut self) -> Vec<Command> {
        let Some(Overlay::ExecutionConfirm { draft, .. }) = self.overlay.take() else {
            return Vec::new();
        };
        self.retain_execution(draft, ExecutionResult::Cancelled);
        Vec::new()
    }

    fn validate_draft(&self, draft: &sql::ExecutionDraft) -> Result<(), String> {
        let Some(tab) = self
            .tabs
            .iter()
            .find(|tab| tab.id() == draft.console_id)
            .and_then(WorkspaceTab::as_console)
        else {
            return Err("Console no longer exists".to_owned());
        };
        if tab.generation != draft.query_generation {
            return Err("Execution draft is stale: query generation changed".to_owned());
        }
        if self.active_editor_revision_for(draft.console_id) != draft.document_revision {
            return Err("Execution draft is stale: document changed".to_owned());
        }
        if self.connection.active_identity() != Some(draft.connection) {
            return Err("Execution draft is stale: connection changed".to_owned());
        }
        if self.connection.target.as_ref() != Some(&draft.target) {
            return Err("Execution draft is stale: active target changed".to_owned());
        }
        if tab.execution_target.as_ref() != Some(&draft.target) {
            return Err("Execution draft is stale: console target changed".to_owned());
        }
        if tab.transaction_generation != draft.transaction_generation
            || tab.transaction_mode != draft.transaction_mode
            || tab.transaction_state != draft.transaction_state
        {
            return Err("Execution draft is stale: transaction state changed".to_owned());
        }
        if tab.query_status == QueryStatus::Running {
            return Err("A query is already running in this console".to_owned());
        }
        Ok(())
    }

    fn dispatch_draft(&mut self, draft: sql::ExecutionDraft) -> Vec<Command> {
        if let Err(message) = self.validate_draft(&draft) {
            self.status_message(&message);
            return Vec::new();
        }
        let tab = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id() == draft.console_id);
        let Some(tab) = tab.and_then(WorkspaceTab::as_console_mut) else {
            return Vec::new();
        };
        if draft.transaction_mode == TransactionMode::Manual {
            let connection = draft.connection;
            let target = draft.target.clone();
            let tab_id = draft.console_id;
            let sql =
                sql::bounded_query(&draft.sql, draft.dialect).unwrap_or_else(|| draft.sql.clone());
            let mode_state = tab.transaction_state;
            if mode_state == TransactionState::Idle || mode_state == TransactionState::Active {
                let mut snapshot = tab_snapshot(tab);
                if mode_state == TransactionState::Idle {
                    snapshot = match transaction::transition(snapshot, TransactionEvent::Start) {
                        Ok(snapshot) => snapshot,
                        Err(_) => return Vec::new(),
                    };
                }
                tab.generation += 1;
                let query_generation = tab.generation;
                apply_transaction_snapshot(tab, snapshot);
                tab.query_status = QueryStatus::Running;
                tab.last_execution = Some(LastExecution {
                    draft,
                    result: ExecutionResult::Dispatched,
                });
                return vec![Command::ManualExecute {
                    connection,
                    target,
                    tab_id,
                    query_generation,
                    transaction_generation: tab.transaction_generation,
                    sql,
                }];
            }
            return Vec::new();
        }
        tab.generation += 1;
        let generation = tab.generation;
        tab.query_status = QueryStatus::Running;
        tab.last_execution = Some(LastExecution {
            draft: draft.clone(),
            result: ExecutionResult::Dispatched,
        });
        vec![Command::RunQuery {
            connection: draft.connection,
            target: draft.target,
            tab_id: draft.console_id,
            generation,
            sql: sql::bounded_query(&draft.sql, draft.dialect).unwrap_or(draft.sql),
        }]
    }

    fn retain_execution(&mut self, draft: sql::ExecutionDraft, result: ExecutionResult) {
        if let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id() == draft.console_id)
            .and_then(WorkspaceTab::as_console_mut)
        {
            tab.last_execution = Some(LastExecution { draft, result });
        }
    }

    fn active_editor_revision_for(&self, id: Uuid) -> u64 {
        self.editor.revision(id).unwrap_or_default()
    }

    fn status_message(&mut self, message: &str) {
        self.connection.error = Some(message.to_owned());
    }

    fn start_catalog_request(
        &mut self,
        target: CatalogTarget,
        cursor: Option<crate::db::catalog::CatalogCursor>,
        intent: CatalogRequestIntent,
    ) -> Vec<Command> {
        let Some(connection) = self.database_command_identity() else {
            return Vec::new();
        };
        if target
            .profile_id()
            .is_some_and(|profile_id| profile_id != connection.profile_id)
        {
            return Vec::new();
        }
        let Some(scope) = self
            .profiles
            .iter()
            .find(|profile| profile.id == connection.profile_id)
            .map(|profile| profile.catalog_scope.clone())
        else {
            return Vec::new();
        };
        let owner = owner_for_target(connection.profile_id, &target);
        let Some(state) = self
            .explorer
            .normalized
            .profiles
            .get_mut(&connection.profile_id)
        else {
            return Vec::new();
        };
        if state.pending_requests.contains_key(&owner) && intent != CatalogRequestIntent::Refresh {
            return Vec::new();
        }
        let current = state
            .load_states
            .get(&owner)
            .cloned()
            .unwrap_or(ExplorerLoadState::NotLoaded);
        if intent == CatalogRequestIntent::Automatic
            && !matches!(current, ExplorerLoadState::NotLoaded)
        {
            return Vec::new();
        }
        if intent == CatalogRequestIntent::Continuation {
            let expected_cursor = match &current {
                ExplorerLoadState::Loaded { next_cursor } => next_cursor.as_ref(),
                _ => None,
            };
            if expected_cursor != cursor.as_ref() {
                return Vec::new();
            }
        }
        if intent == CatalogRequestIntent::Explicit
            && matches!(current, ExplorerLoadState::Loaded { next_cursor: None })
            && cursor.is_none()
        {
            return Vec::new();
        }
        let Some(request_id) = state.allocate_request_id() else {
            state.last_error = Some("catalog request ID exhausted".to_owned());
            return Vec::new();
        };
        let request = CatalogRequest {
            key: CatalogRequestKey {
                connection,
                catalog_epoch: state.catalog_epoch,
                request_id,
                target,
                cursor,
            },
            scope,
            page_size: 100.min(MAX_CATALOG_PAGE_SIZE),
        };
        if let Err(error) = request.validate() {
            state.last_error = Some(error.to_string());
            return Vec::new();
        }
        state
            .previous_load_states
            .entry(owner.clone())
            .or_insert(current);
        state
            .load_states
            .insert(owner.clone(), ExplorerLoadState::Loading { request_id });
        state.pending_requests.insert(owner, request.clone());
        vec![Command::LoadCatalogPage(request)]
    }

    fn edit_explorer_search(&mut self, edit: impl FnOnce(&mut String)) -> Vec<Command> {
        if self.explorer.search.is_none() {
            return Vec::new();
        }
        self.explorer.edit_search(edit);
        self.explorer.refresh_frontend_search();
        Vec::new()
    }

    fn accept_catalog_page(&mut self, page: CatalogPage) -> Vec<Command> {
        let profile_id = page.key.connection.profile_id;
        if self.connection.active_identity() != Some(page.key.connection) {
            return Vec::new();
        }
        let owner = owner_for_target(profile_id, &page.key.target);
        let pending = self
            .explorer
            .normalized
            .profiles
            .get(&profile_id)
            .and_then(|state| state.pending_requests.get(&owner))
            .cloned();
        let Some(request) = pending.filter(|request| request.key == page.key) else {
            return Vec::new();
        };
        if let Err(error) = page.validate_for(&request) {
            self.fail_catalog_page(
                &request.key,
                ErrorCategory::Internal,
                format!("invalid catalog page: {error}"),
            );
            return Vec::new();
        }

        let mut next_explorer = self.explorer.normalized.clone();
        let apply_result: Result<Vec<crate::db::catalog::CatalogId>, String> =
            match &request.key.target {
                CatalogTarget::Groups { schema } => {
                    let states = page
                        .group_summaries
                        .iter()
                        .map(|summary| {
                            (
                                summary.group,
                                CatalogGroupState {
                                    count: summary.object_count,
                                    completeness: page.completeness,
                                },
                            )
                        })
                        .collect();
                    let catalog = &mut next_explorer.profiles.get_mut(&profile_id).unwrap().catalog;
                    if request.key.cursor.is_some() {
                        catalog.append_group_states(schema, states)
                    } else {
                        catalog.replace_group_states(schema, states)
                    }
                    .map(|()| Vec::new())
                    .map_err(|error| error.to_string())
                }
                _ if request.key.cursor.is_some() => next_explorer
                    .profiles
                    .get_mut(&profile_id)
                    .unwrap()
                    .catalog
                    .append_page(&owner, page.entries.clone())
                    .map(|()| Vec::new())
                    .map_err(|error| error.to_string()),
                _ => next_explorer
                    .replace_page(owner.clone(), page.entries.clone())
                    .map_err(|error| error.to_string()),
            };
        let removed = match apply_result {
            Ok(removed) => removed,
            Err(error) => {
                self.fail_catalog_page(
                    &request.key,
                    ErrorCategory::Internal,
                    format!("invalid catalog tree mutation: {error}"),
                );
                return Vec::new();
            }
        };

        let state = next_explorer.profiles.get_mut(&profile_id).unwrap();
        if !removed.is_empty() {
            let references_removed = |candidate: &ExplorerOwnerId| {
                candidate != &owner
                    && match candidate {
                        ExplorerOwnerId::Profile(_) => false,
                        ExplorerOwnerId::Catalog(id) => removed.contains(id),
                        ExplorerOwnerId::Group { parent, .. } => removed.contains(parent),
                    }
            };
            state
                .load_states
                .retain(|candidate, _| !references_removed(candidate));
            state
                .pending_requests
                .retain(|candidate, _| !references_removed(candidate));
            state
                .previous_load_states
                .retain(|candidate, _| !references_removed(candidate));
            state
                .load_errors
                .retain(|candidate, _| !references_removed(candidate));
        }
        state.pending_requests.remove(&owner);
        state.previous_load_states.remove(&owner);
        state.load_errors.remove(&owner);
        state.load_states.insert(
            owner,
            ExplorerLoadState::Loaded {
                next_cursor: page.next_cursor.clone(),
            },
        );
        if state.pending_requests.is_empty() {
            state.status = ExplorerConnectionStatus::Online;
        }
        self.explorer.normalized = next_explorer;
        let scope = self
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .map(|profile| &profile.catalog_scope);
        if let Some(scope) = scope {
            self.explorer.completion_index.replace_scoped(
                &self.explorer.normalized.profiles[&profile_id]
                    .catalog
                    .entries()
                    .values()
                    .cloned()
                    .collect::<Vec<_>>(),
                scope,
            );
        } else {
            self.explorer.completion_index = Default::default();
        }
        self.explorer.catalog_generation = self.explorer.catalog_generation.saturating_add(1);
        self.explorer.rebuild_projection(profile_id);
        self.explorer.refresh_frontend_search();

        let mut commands = Vec::new();
        if let Some(cursor) = page.next_cursor {
            commands.extend(self.start_catalog_request(
                request.key.target.clone(),
                Some(cursor),
                CatalogRequestIntent::Continuation,
            ));
        }
        match &request.key.target {
            CatalogTarget::Databases => {
                for entry in page.entries {
                    commands.extend(self.start_catalog_request(
                        CatalogTarget::schemas(entry.id).unwrap(),
                        None,
                        CatalogRequestIntent::Automatic,
                    ));
                }
            }
            CatalogTarget::Schemas { .. } => {
                for entry in page.entries {
                    commands.extend(self.start_catalog_request(
                        CatalogTarget::groups(entry.id).unwrap(),
                        None,
                        CatalogRequestIntent::Automatic,
                    ));
                }
            }
            CatalogTarget::Groups { schema } => {
                for summary in page
                    .group_summaries
                    .into_iter()
                    .filter(|summary| search_preload_group(summary.group))
                {
                    commands.extend(self.start_catalog_request(
                        CatalogTarget::objects(schema.clone(), summary.group).unwrap(),
                        None,
                        CatalogRequestIntent::Automatic,
                    ));
                }
            }
            CatalogTarget::Objects { .. } | CatalogTarget::RelationChildren { .. } => {}
        }
        if matches!(&request.key.target, CatalogTarget::RelationChildren { .. })
            && self.active_console_opt().is_some()
        {
            commands.extend(self.complete_now());
        }
        commands
    }

    fn fail_catalog_page(
        &mut self,
        key: &CatalogRequestKey,
        category: ErrorCategory,
        message: String,
    ) {
        if self.connection.active_identity() != Some(key.connection) {
            return;
        }
        let owner = owner_for_target(key.connection.profile_id, &key.target);
        let Some(state) = self
            .explorer
            .normalized
            .profiles
            .get_mut(&key.connection.profile_id)
        else {
            return;
        };
        if state
            .pending_requests
            .get(&owner)
            .is_none_or(|request| request.key != *key)
        {
            return;
        }
        let previous = state
            .previous_load_states
            .remove(&owner)
            .unwrap_or(ExplorerLoadState::NotLoaded);
        let has_snapshot = !matches!(
            previous,
            ExplorerLoadState::NotLoaded
                | ExplorerLoadState::Loading { .. }
                | ExplorerLoadState::Failed { .. }
                | ExplorerLoadState::PermissionDenied { .. }
        );
        let next_cursor = match previous {
            ExplorerLoadState::Loaded { next_cursor }
            | ExplorerLoadState::Stale { next_cursor } => next_cursor,
            _ => None,
        };
        let has_data = match &owner {
            ExplorerOwnerId::Profile(_) => !state.catalog.roots().is_empty(),
            ExplorerOwnerId::Catalog(id) => {
                !state.catalog.children(id).is_empty() || !state.catalog.groups(id).is_empty()
            }
            ExplorerOwnerId::Group { parent, group } => {
                !state.catalog.group_children(parent, *group).is_empty()
            }
        };
        let load_state = if has_snapshot || has_data {
            ExplorerLoadState::Stale { next_cursor }
        } else if category == ErrorCategory::Permission {
            ExplorerLoadState::PermissionDenied {
                request_id: key.request_id,
            }
        } else {
            ExplorerLoadState::Failed {
                request_id: key.request_id,
            }
        };
        state.pending_requests.remove(&owner);
        state.load_errors.insert(owner.clone(), message);
        state.load_states.insert(owner, load_state);
        if state.pending_requests.is_empty() {
            state.status = ExplorerConnectionStatus::Online;
        }
        self.explorer.rebuild_projection(key.connection.profile_id);
    }

    fn selected_catalog_target(&self) -> Option<CatalogTarget> {
        self.target_for_node(self.explorer.selected_id()?)
    }

    fn target_for_node(
        &self,
        node: &crate::model::explorer::ExplorerNodeId,
    ) -> Option<CatalogTarget> {
        use crate::model::explorer::ExplorerNodeId;
        match node {
            ExplorerNodeId::Profile(_) => Some(CatalogTarget::Databases),
            ExplorerNodeId::Catalog(id) => match id.kind {
                crate::db::catalog::CatalogKind::Database => {
                    CatalogTarget::schemas(id.clone()).ok()
                }
                crate::db::catalog::CatalogKind::Schema => CatalogTarget::groups(id.clone()).ok(),
                kind if kind.is_relation() => CatalogTarget::relation_children(id.clone()).ok(),
                _ => None,
            },
            ExplorerNodeId::Group { parent, group } => {
                CatalogTarget::objects(parent.clone(), *group).ok()
            }
            ExplorerNodeId::Status { owner, .. }
            | ExplorerNodeId::Empty { owner }
            | ExplorerNodeId::LoadMore { parent: owner, .. } => self.target_for_owner(owner),
            ExplorerNodeId::EmptyProfiles => None,
        }
    }

    fn target_for_owner(&self, owner: &ExplorerOwnerId) -> Option<CatalogTarget> {
        match owner {
            ExplorerOwnerId::Profile(_) => Some(CatalogTarget::Databases),
            ExplorerOwnerId::Catalog(id) => match id.kind {
                crate::db::catalog::CatalogKind::Database => {
                    CatalogTarget::schemas(id.clone()).ok()
                }
                crate::db::catalog::CatalogKind::Schema => CatalogTarget::groups(id.clone()).ok(),
                kind if kind.is_relation() => CatalogTarget::relation_children(id.clone()).ok(),
                _ => None,
            },
            ExplorerOwnerId::Group { parent, group } => {
                CatalogTarget::objects(parent.clone(), *group).ok()
            }
        }
    }

    fn toggle_explorer_selected(&mut self) -> Vec<Command> {
        use crate::model::explorer::{ExplorerNodeId, StatusRowKind};
        let Some(selected) = self.explorer.selected_id().cloned() else {
            return Vec::new();
        };
        match &selected {
            ExplorerNodeId::LoadMore { parent, cursor } => self
                .target_for_owner(parent)
                .map_or_else(Vec::new, |target| {
                    self.start_catalog_request(
                        target,
                        Some(cursor.clone()),
                        CatalogRequestIntent::Continuation,
                    )
                }),
            ExplorerNodeId::Status {
                owner,
                kind: StatusRowKind::Retry | StatusRowKind::PermissionDenied | StatusRowKind::Stale,
            } => self
                .target_for_owner(owner)
                .map_or_else(Vec::new, |target| {
                    self.start_catalog_request(target, None, CatalogRequestIntent::Explicit)
                }),
            ExplorerNodeId::Catalog(_) | ExplorerNodeId::Group { .. } => {
                let expanded = self.explorer.normalized.expanded.contains(&selected);
                self.explorer.toggle_selected();
                if expanded {
                    return Vec::new();
                }
                self.target_for_node(&selected)
                    .map_or_else(Vec::new, |target| {
                        if self.target_needs_load(&target) {
                            self.start_catalog_request(target, None, CatalogRequestIntent::Explicit)
                        } else {
                            Vec::new()
                        }
                    })
            }
            ExplorerNodeId::Profile(profile_id) => {
                let Some(status) = self
                    .explorer
                    .normalized
                    .profiles
                    .get(profile_id)
                    .map(|profile| profile.status)
                else {
                    return Vec::new();
                };
                match status {
                    ExplorerConnectionStatus::Offline | ExplorerConnectionStatus::Failed => {
                        self.explorer
                            .normalized
                            .expanded
                            .remove(&ExplorerNodeId::Profile(*profile_id));
                        if let Some(profile) = self.explorer.normalized.profiles.get_mut(profile_id)
                        {
                            profile.expand_after_connect = true;
                        }
                        self.request_connection(*profile_id)
                    }
                    ExplorerConnectionStatus::Linking => Vec::new(),
                    ExplorerConnectionStatus::Online | ExplorerConnectionStatus::Syncing => {
                        self.explorer.toggle_selected();
                        Vec::new()
                    }
                }
            }
            ExplorerNodeId::EmptyProfiles => self.update(Action::ProfileStartNew),
            ExplorerNodeId::Status { .. } | ExplorerNodeId::Empty { .. } => Vec::new(),
        }
    }

    fn expand_explorer_selected(&mut self) -> Vec<Command> {
        let Some(selected) = self.explorer.selected_id().cloned() else {
            return Vec::new();
        };
        if let ExplorerNodeId::Profile(profile_id) = selected {
            let status = self
                .explorer
                .normalized
                .profiles
                .get(&profile_id)
                .map(|profile| profile.status);
            if matches!(
                status,
                Some(ExplorerConnectionStatus::Offline | ExplorerConnectionStatus::Failed)
            ) {
                if let Some(profile) = self.explorer.normalized.profiles.get_mut(&profile_id) {
                    profile.expand_after_connect = true;
                }
                return self.request_connection(profile_id);
            }
        }
        let expanded = self.explorer.normalized.expanded.contains(&selected);
        if expanded || !self.explorer.normalized.expand() {
            return Vec::new();
        }
        self.target_for_node(&selected)
            .map_or_else(Vec::new, |target| {
                if self.target_needs_load(&target) {
                    self.start_catalog_request(target, None, CatalogRequestIntent::Explicit)
                } else {
                    Vec::new()
                }
            })
    }

    fn collapse_explorer_selected(&mut self) -> Vec<Command> {
        if self.explorer.normalized.collapse() {
            Vec::new()
        } else {
            self.explorer.normalized.move_to_parent();
            Vec::new()
        }
    }

    fn refresh_explorer_selected(&mut self) -> Vec<Command> {
        self.selected_catalog_target()
            .map_or_else(Vec::new, |target| {
                self.start_catalog_request(target, None, CatalogRequestIntent::Refresh)
            })
    }

    fn primary_explorer_selected(&mut self) -> Vec<Command> {
        let Some(selected) = self.explorer.selected_id().cloned() else {
            return Vec::new();
        };
        if let ExplorerNodeId::Catalog(id) = &selected
            && let Some(entry) = self
                .explorer
                .normalized
                .profiles
                .get(&id.profile_id())
                .and_then(|profile| profile.catalog.get(id))
            && (entry.kind.is_relation() || entry.owning_relation_id().is_some())
        {
            return self.open_selected_relation(RelationView::Data);
        }
        self.toggle_explorer_selected()
    }

    fn target_needs_load(&self, target: &CatalogTarget) -> bool {
        let Some(profile_id) = self.connection.profile_id else {
            return false;
        };
        let owner = owner_for_target(profile_id, target);
        self.explorer
            .normalized
            .profiles
            .get(&profile_id)
            .and_then(|state| state.load_states.get(&owner))
            .is_none_or(|state| {
                matches!(
                    state,
                    ExplorerLoadState::NotLoaded
                        | ExplorerLoadState::Stale { .. }
                        | ExplorerLoadState::Failed { .. }
                        | ExplorerLoadState::PermissionDenied { .. }
                )
            })
    }

    fn clear_active_catalog(&mut self, profile_id: Uuid) {
        self.explorer.connection_changed();
        self.clear_profile_catalog(profile_id, ExplorerConnectionStatus::Offline);
    }

    fn clear_profile_catalog(&mut self, profile_id: Uuid, status: ExplorerConnectionStatus) {
        self.explorer
            .normalized
            .expanded
            .retain(|node| node.profile_id() != Some(profile_id));
        if let Some(state) = self.explorer.normalized.profiles.get_mut(&profile_id) {
            state.status = status;
            state.catalog = crate::model::explorer::CatalogTree::new(profile_id);
            state.load_states.clear();
            state.pending_requests.clear();
            state.previous_load_states.clear();
            state.load_errors.clear();
        }
    }

    fn select_nearest_profile(&mut self, removed_profile_id: Uuid) {
        if self
            .explorer
            .normalized
            .selected
            .as_ref()
            .is_some_and(|node| node.profile_id() != Some(removed_profile_id))
        {
            return;
        }
        let order = &self.explorer.normalized.profile_order;
        let next = order
            .iter()
            .position(|profile_id| *profile_id == removed_profile_id)
            .and_then(|index| {
                order
                    .get(index + 1)
                    .or_else(|| index.checked_sub(1).and_then(|i| order.get(i)))
            })
            .copied();
        self.explorer.normalized.selected = next
            .map(crate::model::explorer::ExplorerNodeId::Profile)
            .or(Some(crate::model::explorer::ExplorerNodeId::EmptyProfiles));
    }

    fn request_profile_disconnect(&mut self, profile_id: Uuid) -> Vec<Command> {
        let Some(connection) = self
            .connection
            .active_identity()
            .filter(|connection| connection.profile_id == profile_id)
        else {
            return Vec::new();
        };
        if self.tabs.iter().any(|tab| {
            tab.as_console()
                .is_some_and(|tab| tab.query_status == QueryStatus::Running)
        }) {
            return Vec::new();
        }
        let mut commands = self.cancel_relation_requests_for_connection(None);
        if self
            .active_console_opt()
            .is_some_and(|tab| self.transaction_needs_exit(tab.id))
        {
            commands.extend(
                self.defer_intent(
                    DeferredIntent::Disconnect { connection },
                    self.tabs
                        .iter()
                        .filter(|tab| self.transaction_needs_exit(tab.id()))
                        .map(|tab| tab.id())
                        .collect::<Vec<_>>(),
                ),
            );
            return commands;
        }
        commands.push(Command::Disconnect { connection });
        commands
    }

    fn open_selected_relation(&mut self, view: RelationView) -> Vec<Command> {
        let Some(ExplorerNodeId::Catalog(selected_id)) = self.explorer.selected_id().cloned()
        else {
            return Vec::new();
        };
        let Some(profile) = self
            .explorer
            .normalized
            .profiles
            .get(&selected_id.profile_id())
        else {
            return Vec::new();
        };
        let Some(entry) = profile.catalog.get(&selected_id) else {
            return Vec::new();
        };
        let Some(relation_id) = profile.catalog.owning_relation_id(&selected_id).cloned() else {
            return Vec::new();
        };
        let Some(relation) = profile.catalog.get(&relation_id) else {
            return Vec::new();
        };
        if !relation.kind.is_relation()
            || relation.id.profile_id() != selected_id.profile_id()
            || entry.id.profile_id() != selected_id.profile_id()
        {
            return Vec::new();
        }
        let key = RelationKey {
            profile_id: relation.id.profile_id(),
            object_id: relation.id.clone(),
        };
        if let Some(index) = self.tabs.iter().position(|tab| {
            matches!(
                tab,
                WorkspaceTab::Relation(existing)
                    if existing.descriptor.key == key
            )
        }) {
            self.active_tab = index;
            if let WorkspaceTab::Relation(tab) = &mut self.tabs[index] {
                tab.view = view;
            }
            self.focus = Focus::Results;
            self.active_tab = index;
            let mut commands = self.load_active_relation(false);
            commands.extend(self.load_relation_columns_if_missing(&relation_id));
            return commands;
        }
        let title = relation.qualified_name.object.clone();
        let descriptor = RelationDescriptor {
            key,
            qualified_name: relation.qualified_name.clone(),
            kind: relation.kind,
            title,
        };
        self.tabs
            .push(WorkspaceTab::Relation(RelationTab::with_descriptor(
                descriptor, view,
            )));
        self.active_tab = self.tabs.len() - 1;
        self.focus = Focus::Results;
        let mut commands = self.load_active_relation(true);
        commands.extend(self.load_relation_columns_if_missing(&relation_id));
        commands
    }

    fn load_relation_columns_if_missing(
        &mut self,
        relation: &crate::db::catalog::CatalogId,
    ) -> Vec<Command> {
        if self
            .explorer
            .completion_index
            .relation_columns(relation)
            .next()
            .is_some()
        {
            return Vec::new();
        }
        let Ok(target) = CatalogTarget::relation_children(relation.clone()) else {
            return Vec::new();
        };
        self.start_catalog_request(target, None, CatalogRequestIntent::Automatic)
    }

    fn ddl_selected(&mut self) -> Vec<Command> {
        self.open_selected_relation(RelationView::Ddl)
    }

    fn active_data_query_mut(&mut self) -> Option<&mut crate::model::data_query::DataQueryState> {
        match self.tabs.get_mut(self.active_tab) {
            Some(WorkspaceTab::Relation(tab)) if tab.view == RelationView::Data => {
                Some(&mut tab.query)
            }
            Some(WorkspaceTab::Sql(tab)) if tab.result_view == ResultView::Data => {
                Some(&mut tab.query)
            }
            _ => None,
        }
    }

    fn refresh_active_data_query_completion(&mut self) {
        let Some(input_kind) = (match self.tabs.get(self.active_tab) {
            Some(WorkspaceTab::Relation(tab)) if tab.view == RelationView::Data => tab.query.focus,
            Some(WorkspaceTab::Sql(tab)) if tab.result_view == ResultView::Data => tab.query.focus,
            _ => None,
        }) else {
            return;
        };
        let (value, cursor, columns) = match self.tabs.get(self.active_tab) {
            Some(WorkspaceTab::Relation(tab)) if tab.view == RelationView::Data => {
                let mut columns = self
                    .explorer
                    .completion_index
                    .relation_columns(&tab.descriptor.key.object_id)
                    .map(|entry| {
                        let type_name = match &entry.metadata {
                            CatalogMetadata::Column(column) => Some(column.native_type.clone()),
                            _ => None,
                        };
                        (entry.qualified_name.object.clone(), type_name)
                    })
                    .collect::<Vec<_>>();
                if let Some(result) = self.relation_result() {
                    columns.extend(
                        result
                            .columns
                            .iter()
                            .map(|column| (column.name.clone(), Some(column.type_name.clone()))),
                    );
                }
                let input = match input_kind {
                    DataQueryInput::Where => &tab.query.where_input,
                    DataQueryInput::OrderBy => &tab.query.order_by_input,
                };
                (input.value().to_owned(), input.cursor(), columns)
            }
            Some(WorkspaceTab::Sql(tab)) if tab.result_view == ResultView::Data => {
                let columns = tab
                    .outcome
                    .as_ref()
                    .and_then(|outcome| outcome.result_sets.last())
                    .map(|result| {
                        result
                            .columns
                            .iter()
                            .map(|column| (column.name.clone(), Some(column.type_name.clone())))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let input = match input_kind {
                    DataQueryInput::Where => &tab.query.where_input,
                    DataQueryInput::OrderBy => &tab.query.order_by_input,
                };
                (input.value().to_owned(), input.cursor(), columns)
            }
            _ => return,
        };
        let Some((replace, prefix)) = data_query_identifier(&value, cursor) else {
            if let Some(query) = self.active_data_query_mut() {
                query.completion = None;
            }
            return;
        };
        let mut candidates = columns
            .into_iter()
            .filter_map(|(name, type_name)| {
                let quality = sql::identifier_match(&name, &prefix)?;
                Some((quality, DataQueryCandidate { name, type_name }))
            })
            .collect::<Vec<_>>();
        let mut seen = std::collections::HashSet::new();
        candidates.retain(|(_, candidate)| seen.insert(candidate.name.to_lowercase()));
        candidates.sort_by(|(left_quality, left), (right_quality, right)| {
            right_quality.cmp(left_quality).then_with(|| {
                left.name
                    .to_lowercase()
                    .cmp(&right.name.to_lowercase())
                    .then_with(|| left.name.cmp(&right.name))
            })
        });
        candidates.truncate(10);
        let completion = (!candidates.is_empty()).then(|| DataQueryCompletion {
            candidates: candidates
                .into_iter()
                .map(|(_, candidate)| candidate)
                .collect(),
            selected: 0,
            replace,
        });
        if let Some(query) = self.active_data_query_mut() {
            query.completion = completion;
        }
    }

    fn move_active_data_query_completion(&mut self, delta: isize) {
        let Some(completion) = self
            .active_data_query_mut()
            .and_then(|query| query.completion.as_mut())
        else {
            return;
        };
        let count = completion.candidates.len();
        if count > 0 {
            completion.selected = completion
                .selected
                .saturating_add_signed(delta)
                .min(count - 1);
        }
    }

    fn accept_active_data_query_completion(&mut self) {
        let dialect = self.sql_dialect();
        let accepted = self.active_data_query_mut().and_then(|query| {
            let completion = query.completion.take()?;
            let candidate = completion.candidates.get(completion.selected)?;
            Some((
                query.focus?,
                completion.replace,
                sql::quote_identifier(&candidate.name, dialect),
            ))
        });
        let Some((focus, replace, insert_text)) = accepted else {
            return;
        };
        if let Some(query) = self.active_data_query_mut() {
            match focus {
                DataQueryInput::Where => query.where_input.replace(replace, &insert_text),
                DataQueryInput::OrderBy => query.order_by_input.replace(replace, &insert_text),
            }
        }
    }

    fn with_active_data_query<F>(&mut self, edit: F)
    where
        F: FnOnce(&mut crate::model::text_input::TextInput),
    {
        let mut clear_derived = false;
        if let Some(query) = self.active_data_query_mut()
            && matches!(
                query.capability,
                DataQueryCapability::Relation | DataQueryCapability::Sql
            )
            && let Some(input) = query.focus
        {
            match input {
                DataQueryInput::Where => edit(&mut query.where_input),
                DataQueryInput::OrderBy => edit(&mut query.order_by_input),
            }
            query.error = None;
            if matches!(query.capability, DataQueryCapability::Sql) {
                clear_derived = query.where_input.value().trim().is_empty()
                    && query.order_by_input.value().trim().is_empty();
                query.submitted = DataQueryOptions::default();
            }
        }
        if clear_derived {
            self.active_console_mut().derived = None;
        }
    }

    fn cancel_active_data_query(&mut self) {
        let mut clear_derived = false;
        if let Some(query) = self.active_data_query_mut()
            && matches!(
                query.capability,
                DataQueryCapability::Relation | DataQueryCapability::Sql
            )
        {
            query.focus = None;
            query.completion = None;
            query.error = None;
            query
                .where_input
                .set(query.submitted.where_clause.clone().unwrap_or_default());
            query
                .order_by_input
                .set(query.submitted.order_by_clause.clone().unwrap_or_default());
            clear_derived = matches!(query.capability, DataQueryCapability::Sql);
        }
        if clear_derived {
            self.active_console_mut().derived = None;
        }
    }

    fn submit_data_query(&mut self) -> Vec<Command> {
        if matches!(
            self.tabs.get(self.active_tab),
            Some(WorkspaceTab::Relation(_))
        ) {
            self.submit_relation_query()
        } else {
            self.submit_sql_query()
        }
    }

    fn submit_sql_query(&mut self) -> Vec<Command> {
        let tab = self.active_console();
        let Some(last) = tab.last_execution.clone() else {
            return Vec::new();
        };
        let where_clause = tab.query.where_input.value().to_owned();
        let order_by_clause = tab.query.order_by_input.value().to_owned();
        if where_clause.trim().is_empty() && order_by_clause.trim().is_empty() {
            let console = self.active_console_mut();
            console.query.submitted = DataQueryOptions::default();
            console.query.error = None;
            console.query.focus = None;
            console.query.completion = None;
            console.derived = None;
            return Vec::new();
        }
        let query = match sql::build_derived_query(
            &last.draft.sql,
            &where_clause,
            &order_by_clause,
            last.draft.dialect,
        ) {
            Ok(query) => query,
            Err(error) => {
                self.active_console_mut().query.error =
                    Some(crate::security::sanitize_terminal_text(&error.to_string()));
                return Vec::new();
            }
        };
        let tab_id = tab.id;
        let source_generation = tab.generation;
        let derived_generation = tab
            .derived
            .as_ref()
            .map_or(0, |derived| derived.generation)
            .saturating_add(1);
        let options = DataQueryOptions {
            where_clause: (!where_clause.trim().is_empty()).then_some(where_clause),
            order_by_clause: (!order_by_clause.trim().is_empty()).then_some(order_by_clause),
        };
        let target = tab.execution_target.clone().unwrap();
        let Some(connection) = self.database_command_identity() else {
            return Vec::new();
        };
        let console = self.active_console_mut();
        console.query.submitted = options.clone();
        console.query.focus = None;
        console.query.completion = None;
        console.query.error = None;
        console.derived = Some(DerivedResultState {
            source: last,
            query: options,
            generation: derived_generation,
            outcome: None,
            error: None,
            running: true,
        });
        vec![Command::RunDerivedQuery {
            connection,
            target,
            tab_id,
            source_generation,
            derived_generation,
            sql: query,
        }]
    }

    fn submit_relation_query(&mut self) -> Vec<Command> {
        let dialect = self.sql_dialect();
        let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab) else {
            return Vec::new();
        };
        if tab.view != RelationView::Data {
            return Vec::new();
        }
        let where_clause = tab.query.where_input.value().to_owned();
        let order_by_clause = tab.query.order_by_input.value().to_owned();
        match sql::validate_relation_preview_options(&where_clause, &order_by_clause, dialect) {
            Ok(options) => {
                tab.query.submitted = options;
                tab.query.error = None;
                tab.query.focus = None;
                tab.query.completion = None;
                self.load_active_relation(true)
            }
            Err(error) => {
                tab.query.error = Some(error.to_string());
                Vec::new()
            }
        }
    }

    fn relation_result(&self) -> Option<crate::db::query::ResultSet> {
        let Some(WorkspaceTab::Relation(tab)) = self.tabs.get(self.active_tab) else {
            return None;
        };
        let load = match tab.view {
            RelationView::Data => &tab.data,
            RelationView::Ddl => return None,
        };
        match load {
            RelationLoad::Ready(snapshot) => snapshot.value.result.result_sets.last().cloned(),
            RelationLoad::Loading { previous, .. }
            | RelationLoad::Failed { previous, .. }
            | RelationLoad::Cancelled { previous } => previous
                .as_ref()
                .and_then(|snapshot| snapshot.value.result.result_sets.last().cloned()),
            RelationLoad::Empty => None,
        }
    }

    fn active_grid_dimensions(&self) -> (usize, usize) {
        match self.tabs.get(self.active_tab) {
            Some(WorkspaceTab::Relation(tab)) if tab.view == RelationView::Data => {
                tab.edit.as_ref().map_or_else(
                    || relation_grid_dimensions(&tab.data),
                    |edit| {
                        (
                            edit.rows.len(),
                            edit.rows.first().map_or(0, |row| row.current.len()),
                        )
                    },
                )
            }
            Some(WorkspaceTab::Sql(tab)) if tab.result_view == ResultView::Data => tab
                .derived
                .as_ref()
                .and_then(|derived| derived.outcome.as_ref())
                .or(tab.outcome.as_ref())
                .and_then(|outcome| outcome.result_sets.last())
                .map(|result| (result.rows.len(), result.columns.len()))
                .unwrap_or((0, 0)),
            _ => (0, 0),
        }
    }

    pub(crate) fn active_record_snapshot(
        &self,
    ) -> Option<(Vec<ColumnMeta>, Vec<CellValue>, usize, usize)> {
        match self.tabs.get(self.active_tab) {
            Some(WorkspaceTab::Sql(tab)) if tab.result_view == ResultView::Data => {
                let result = tab
                    .derived
                    .as_ref()
                    .and_then(|derived| derived.outcome.as_ref())
                    .or(tab.outcome.as_ref())
                    .and_then(|outcome| outcome.result_sets.last())?;
                let row = result.rows.get(tab.grid.selected_row)?.clone();
                Some((
                    result.columns.clone(),
                    row,
                    tab.grid.selected_row,
                    result.rows.len(),
                ))
            }
            Some(WorkspaceTab::Relation(tab)) if tab.view == RelationView::Data => {
                let result = match &tab.data {
                    RelationLoad::Ready(snapshot) => snapshot.value.result.result_sets.last(),
                    RelationLoad::Loading { previous, .. }
                    | RelationLoad::Failed { previous, .. }
                    | RelationLoad::Cancelled { previous } => previous
                        .as_ref()
                        .and_then(|snapshot| snapshot.value.result.result_sets.last()),
                    RelationLoad::Empty => None,
                }?;
                let row = tab
                    .edit
                    .as_ref()
                    .and_then(|edit| edit.rows.get(tab.grid.selected_row))
                    .map(|row| row.current.clone())
                    .or_else(|| result.rows.get(tab.grid.selected_row).cloned())?;
                Some((
                    result.columns.clone(),
                    row,
                    tab.grid.selected_row,
                    tab.edit
                        .as_ref()
                        .map_or(result.rows.len(), |edit| edit.rows.len()),
                ))
            }
            _ => None,
        }
    }

    pub(crate) fn active_grid_dimensions_for_input(&self) -> (usize, usize) {
        self.active_grid_dimensions()
    }

    fn with_active_grid(&mut self, f: impl FnOnce(&mut DataGridState, (usize, usize))) {
        let dimensions = self.active_grid_dimensions();
        match self.tabs.get_mut(self.active_tab) {
            Some(WorkspaceTab::Relation(tab)) if tab.view == RelationView::Data => {
                f(&mut tab.grid, dimensions)
            }
            Some(WorkspaceTab::Sql(tab)) if tab.result_view == ResultView::Data => {
                f(&mut tab.grid, dimensions)
            }
            _ => {}
        }
    }

    fn move_grid(&mut self, rows: isize, columns: isize) {
        self.with_active_grid(|grid, (row_count, column_count)| {
            grid.selected_row = move_bounded(grid.selected_row, rows, row_count);
            grid.selected_column = move_bounded(grid.selected_column, columns, column_count);
            grid.clamp(row_count, column_count);
            grid.ensure_row_visible(row_count);
        });
    }

    fn sync_grid_viewport(&mut self, viewport: crate::model::tab::DataGridViewport) {
        let (row_count, column_count) = self.active_grid_dimensions();
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        if tab.id() != viewport.tab_id {
            return;
        }
        let grid = match tab {
            WorkspaceTab::Sql(tab) if tab.result_view == ResultView::Data => &mut tab.grid,
            WorkspaceTab::Relation(tab) if tab.view == RelationView::Data => &mut tab.grid,
            _ => return,
        };
        grid.column_offset = viewport.column_offset;
        grid.row_offset = viewport.row_offset;
        grid.viewport_rows = viewport.visible_rows;
        grid.clamp(row_count, column_count);
        grid.ensure_row_visible(row_count);
    }

    fn relation_session_mut(&mut self) -> Option<&mut RelationEditSession> {
        self.tabs
            .get_mut(self.active_tab)
            .and_then(|tab| match tab {
                WorkspaceTab::Relation(tab) if tab.view == RelationView::Data => tab.edit.as_mut(),
                _ => None,
            })
    }

    fn relation_edit_cell(&mut self) {
        let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        if tab.view != RelationView::Data {
            return;
        }
        let row = tab.grid.selected_row;
        let column = tab.grid.selected_column;
        let Some(value) = tab
            .edit
            .as_ref()
            .and_then(|edit| edit.rows.get(row))
            .and_then(|r| r.current.get(column))
        else {
            return;
        };
        let mut input = crate::model::text_input::TextInput::default();
        input.set(value.preview(usize::MAX).text);
        if let Some(edit) = tab.edit.as_mut() {
            edit.mode = RelationGridMode::EditCell(CellEditorState { row, column, input });
        }
    }

    fn relation_edit_insert(&mut self, character: char) {
        if let Some(edit) = self.relation_session_mut()
            && let RelationGridMode::EditCell(state) = &mut edit.mode
        {
            state.input.insert(character);
        }
    }

    fn relation_edit_input(
        &mut self,
        operation: impl FnOnce(&mut crate::model::text_input::TextInput),
    ) {
        if let Some(edit) = self.relation_session_mut()
            && let RelationGridMode::EditCell(state) = &mut edit.mode
        {
            operation(&mut state.input);
        }
    }

    fn relation_edit_confirm(&mut self) -> Vec<Command> {
        let Some(connection) = self.database_command_identity() else {
            return Vec::new();
        };
        let Some((
            state,
            old,
            row_id,
            original,
            relation_key,
            target,
            scope,
            metadata,
            columns,
            tab_id,
            tab_generation,
            transaction_snapshot,
        )) = self.tabs.get(self.active_tab).and_then(|tab| match tab {
            WorkspaceTab::Relation(tab) => {
                let edit = tab.edit.as_ref()?;
                let RelationGridMode::EditCell(state) = edit.mode.clone() else {
                    return None;
                };
                let row = edit.rows.get(state.row)?;
                let old = row.current.get(state.column)?.clone();
                let target = self.connection.target.clone()?;
                let scope = self
                    .profiles
                    .iter()
                    .find(|p| p.id == connection.profile_id)?
                    .catalog_scope
                    .clone();
                let metadata = match &tab.ddl {
                    RelationLoad::Ready(s) => crate::db::mutation::metadata_fingerprint(&s.value),
                    _ => return None,
                };
                let columns = self.relation_result()?.columns.clone();
                Some((
                    state,
                    old,
                    row.id,
                    row.original.clone(),
                    tab.descriptor.key.clone(),
                    target,
                    scope,
                    metadata,
                    columns,
                    tab.id,
                    tab.generation,
                    tab.edit.clone(),
                ))
            }
            _ => None,
        })
        else {
            return Vec::new();
        };
        let value = match parse_relation_value(state.input.value(), &old) {
            Ok(value) => value,
            Err(message) => {
                if let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab) {
                    tab.query.error = Some(message);
                }
                return Vec::new();
            }
        };
        let Some(pk_columns) = metadata
            .primary_key
            .iter()
            .map(|name| columns.iter().position(|c| c.name == *name))
            .collect::<Option<Vec<_>>>()
        else {
            return Vec::new();
        };
        if let Some(edit) = self.relation_session_mut() {
            edit.mode = RelationGridMode::Browse;
        }
        let request = crate::db::mutation::RelationMutationRequest {
            tab_id,
            tab_generation,
            edit_generation: tab_generation.saturating_add(1),
            connection,
            target,
            relation: relation_key.object_id.clone(),
            relation_key,
            scope,
            metadata,
            row_id,
            operation: crate::db::mutation::RelationMutation::UpdateCell(
                crate::db::mutation::UpdateCellMutation {
                    row: crate::db::mutation::RowLocator {
                        columns: pk_columns.clone(),
                        values: pk_columns
                            .iter()
                            .filter_map(|i| original.get(*i).cloned())
                            .collect(),
                    },
                    column: state.column,
                    original: old,
                    value: input_value(&value),
                },
            ),
        };
        if let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab) {
            if matches!(
                tab.transaction_state,
                TransactionState::Aborted | TransactionState::OutcomeUnknown
            ) {
                self.status_message("Transaction outcome must be resolved before mutating");
                return Vec::new();
            }
            if !matches!(
                tab.transaction_state,
                TransactionState::Idle | TransactionState::Active
            ) {
                self.status_message("Wait for the relation transaction to finish before mutating");
                return Vec::new();
            }
            if tab.transaction_state == TransactionState::Idle {
                tab.transaction_snapshot = transaction_snapshot;
                tab.transaction_generation = tab.transaction_generation.saturating_add(1);
            }
            tab.transaction_state = TransactionState::Active;
        }
        vec![Command::RelationMutation { request }]
    }

    fn relation_commit(&mut self, commit: bool) -> Vec<Command> {
        let connection = match self.database_command_identity() {
            Some(c) => c,
            None => return Vec::new(),
        };
        let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab) else {
            return Vec::new();
        };
        if tab.transaction_state != TransactionState::Active {
            return Vec::new();
        }
        tab.transaction_state = if commit {
            TransactionState::Committing
        } else {
            TransactionState::RollingBack
        };
        if commit {
            vec![Command::RelationCommit {
                tab_id: tab.id,
                generation: tab.transaction_generation,
                connection,
            }]
        } else {
            vec![Command::RelationRollback {
                tab_id: tab.id,
                generation: tab.transaction_generation,
                connection,
            }]
        }
    }

    fn relation_transaction_started(
        &mut self,
        tab_id: Uuid,
        generation: u64,
        _connection: ConnectionIdentity,
    ) {
        if let Some(WorkspaceTab::Relation(tab)) = self.tabs.iter_mut().find(|t| t.id() == tab_id)
            && tab.transaction_generation == generation
        {
            tab.transaction_state = TransactionState::Active;
        }
    }

    fn relation_transaction_failed(
        &mut self,
        tab_id: Uuid,
        generation: u64,
        _connection: ConnectionIdentity,
        message: String,
    ) {
        if let Some(WorkspaceTab::Relation(tab)) = self.tabs.iter_mut().find(|t| t.id() == tab_id)
            && tab.transaction_generation == generation
        {
            tab.transaction_state = TransactionState::Idle;
            tab.transaction_snapshot = None;
        }
        self.status_message(&message);
    }

    fn relation_mutation_result(
        &mut self,
        request: crate::db::mutation::RelationMutationRequest,
        result: Result<crate::db::mutation::MutationResult, String>,
    ) {
        let Some(WorkspaceTab::Relation(tab)) =
            self.tabs.iter_mut().find(|t| t.id() == request.tab_id)
        else {
            return;
        };
        let Some(edit) = tab.edit.as_mut() else {
            return;
        };
        match result {
            Ok(crate::db::mutation::MutationResult::Updated { row }) => {
                if edit.pending_mutation_history.is_some() {
                    edit.complete_mutation();
                } else {
                    let mut inverse = request.clone();
                    if let crate::db::mutation::RelationMutation::UpdateCell(update) =
                        &request.operation
                    {
                        let new_original = row
                            .get(update.column)
                            .cloned()
                            .unwrap_or_else(|| update.original.clone());
                        let new_locator = update
                            .row
                            .columns
                            .iter()
                            .zip(&update.row.values)
                            .map(|(column, value)| {
                                if *column == update.column {
                                    row.get(*column).cloned().unwrap_or_else(|| value.clone())
                                } else {
                                    value.clone()
                                }
                            })
                            .collect();
                        inverse.operation = crate::db::mutation::RelationMutation::UpdateCell(
                            crate::db::mutation::UpdateCellMutation {
                                row: crate::db::mutation::RowLocator {
                                    columns: update.row.columns.clone(),
                                    values: new_locator,
                                },
                                column: update.column,
                                original: new_original,
                                value: input_value(&update.original),
                            },
                        );
                    }
                    edit.record_mutation(RelationMutationHistory {
                        forward: request.clone(),
                        inverse,
                    });
                }
                if let Some(r) = edit.rows.iter_mut().find(|r| r.id == request.row_id) {
                    r.current = row.clone();
                    r.original = row;
                    r.state = crate::model::relation_edit::EditableRowState::Clean;
                }
            }
            Ok(crate::db::mutation::MutationResult::Deleted { .. }) => {
                if edit.pending_mutation_history.is_none() {
                    edit.record_mutation(RelationMutationHistory {
                        forward: request.clone(),
                        inverse: request.clone(),
                    });
                }
                if let crate::db::mutation::RelationMutation::DeleteRows(rows) = &request.operation
                {
                    for mutation in rows {
                        if let Some(row) = edit
                            .rows
                            .iter_mut()
                            .find(|row| row.original == mutation.original)
                        {
                            row.state = crate::model::relation_edit::EditableRowState::Deleted;
                        }
                    }
                }
            }
            Ok(crate::db::mutation::MutationResult::Inserted { row }) => {
                if let Some(r) = edit.rows.iter_mut().find(|r| r.id == request.row_id) {
                    r.mark_inserted(row);
                }
                if edit.pending_mutation_history.is_none() {
                    edit.record_mutation(RelationMutationHistory {
                        forward: request.clone(),
                        inverse: request.clone(),
                    });
                }
            }
            Err(message) => {
                edit.pending_mutation_history = None;
                let ids = match &request.operation {
                    crate::db::mutation::RelationMutation::DeleteRows(rows) => rows
                        .iter()
                        .filter_map(|mutation| {
                            edit.rows
                                .iter()
                                .find(|row| row.original == mutation.original)
                                .map(|row| row.id)
                        })
                        .collect::<Vec<_>>(),
                    _ => vec![request.row_id],
                };
                for id in ids {
                    if let Some(r) = edit.rows.iter_mut().find(|r| r.id == id) {
                        r.mark_conflict(message.clone());
                    }
                }
                self.status_message(&message);
            }
        }
    }

    fn relation_transaction_finished(
        &mut self,
        tab_id: Uuid,
        generation: u64,
        _connection: ConnectionIdentity,
        success: bool,
        error: Option<(String, bool)>,
    ) {
        if let Some(WorkspaceTab::Relation(tab)) = self.tabs.iter_mut().find(|t| t.id() == tab_id) {
            if tab.transaction_generation != generation {
                return;
            }
            if success {
                if tab.transaction_state == TransactionState::RollingBack {
                    tab.edit = tab.transaction_snapshot.clone();
                }
                tab.transaction_snapshot = None;
                tab.transaction_state = TransactionState::Idle;
            } else {
                tab.transaction_state = if error.as_ref().is_some_and(|(_, unknown)| *unknown) {
                    TransactionState::OutcomeUnknown
                } else {
                    TransactionState::Active
                };
            }
        }
        if let Some((message, _)) = error {
            self.status_message(&message);
        }
    }

    fn relation_edit_cancel(&mut self) {
        if let Some(edit) = self.relation_session_mut() {
            edit.mode = RelationGridMode::Browse;
        }
    }

    fn relation_visual_line(&mut self) {
        let row = self.active_grid_row();
        if let Some(edit) = self.relation_session_mut() {
            edit.mode = RelationGridMode::VisualLine { anchor: row };
        }
    }

    fn active_grid_row(&self) -> usize {
        self.tabs
            .get(self.active_tab)
            .and_then(|tab| match tab {
                WorkspaceTab::Relation(tab) => Some(tab.grid.selected_row),
                _ => None,
            })
            .unwrap_or(0)
    }

    fn relation_delete_current(&mut self) -> Vec<Command> {
        let row = self.active_grid_row();
        self.relation_delete_range(row..=row)
    }
    fn relation_delete_selected(&mut self) -> Vec<Command> {
        let row = self.active_grid_row();
        let Some((start, end)) = self.tabs.get(self.active_tab).and_then(|tab| match tab {
            WorkspaceTab::Relation(tab) => tab.edit.as_ref()?.visual_range(row),
            _ => None,
        }) else {
            return Vec::new();
        };
        let commands = self.relation_delete_range(start..=end);
        if let Some(edit) = self.relation_session_mut() {
            edit.mode = RelationGridMode::Browse;
        }
        commands
    }

    fn relation_delete_range(&mut self, range: std::ops::RangeInclusive<usize>) -> Vec<Command> {
        let Some(request) = self.relation_request_for_rows(range) else {
            return Vec::new();
        };
        vec![Command::RelationMutation { request }]
    }

    fn relation_request_for_rows(
        &mut self,
        range: std::ops::RangeInclusive<usize>,
    ) -> Option<crate::db::mutation::RelationMutationRequest> {
        use crate::db::mutation::{DeleteRowMutation, RelationMutation, RowLocator};
        let connection = self.database_command_identity()?;
        let (mut request, snapshot) =
            self.relation_context(|tab, edit, columns, metadata, target, scope| {
                let pk_columns = metadata
                    .primary_key
                    .iter()
                    .map(|name| columns.iter().position(|column| column.name == *name))
                    .collect::<Option<Vec<_>>>()?;
                let mut mutations = Vec::new();
                let mut ids = Vec::new();
                for index in range {
                    let row = edit.rows.get(index)?;
                    if matches!(
                        row.state,
                        crate::model::relation_edit::EditableRowState::Deleted
                    ) {
                        continue;
                    }
                    ids.push(row.id);
                    mutations.push(DeleteRowMutation {
                        row: RowLocator {
                            columns: pk_columns.clone(),
                            values: pk_columns
                                .iter()
                                .filter_map(|i| row.original.get(*i).cloned())
                                .collect(),
                        },
                        original: row.original.clone(),
                    });
                }
                if mutations.is_empty() {
                    return None;
                }
                let request = crate::db::mutation::RelationMutationRequest {
                    tab_id: tab.id,
                    tab_generation: tab.generation,
                    edit_generation: tab.transaction_generation.saturating_add(1),
                    row_id: ids[0],
                    connection,
                    target,
                    relation: tab.descriptor.key.object_id.clone(),
                    relation_key: tab.descriptor.key.clone(),
                    scope,
                    metadata,
                    operation: RelationMutation::DeleteRows(mutations),
                };
                Some((request, tab.transaction_snapshot.clone()))
            })?;
        self.activate_relation_transaction(&mut request, snapshot)?;
        Some(request)
    }

    fn relation_insert_command(
        &mut self,
        row_id: crate::model::relation_edit::EditableRowId,
        values: Vec<crate::db::value::CellValue>,
    ) -> Vec<Command> {
        use crate::db::mutation::{InsertRowMutation, RelationMutation};
        let Some((mut request, snapshot)) =
            self.relation_context(|tab, _edit, _columns, metadata, target, scope| {
                let columns = (0..values.len()).collect::<Vec<_>>();
                let supplied = columns
                    .iter()
                    .map(|index| input_value(&values[*index]))
                    .collect();
                Some((
                    crate::db::mutation::RelationMutationRequest {
                        tab_id: tab.id,
                        tab_generation: tab.generation,
                        edit_generation: tab.transaction_generation.saturating_add(1),
                        row_id,
                        connection: self.database_command_identity()?,
                        target,
                        relation: tab.descriptor.key.object_id.clone(),
                        relation_key: tab.descriptor.key.clone(),
                        scope,
                        metadata,
                        operation: RelationMutation::InsertRow(InsertRowMutation {
                            columns,
                            values: supplied,
                        }),
                    },
                    tab.transaction_snapshot.clone(),
                ))
            })
        else {
            return Vec::new();
        };
        if self
            .activate_relation_transaction(&mut request, snapshot)
            .is_none()
        {
            return Vec::new();
        }
        vec![Command::RelationMutation { request }]
    }

    fn relation_context<T>(
        &self,
        f: impl FnOnce(
            &RelationTab,
            &RelationEditSession,
            Vec<crate::db::query::ColumnMeta>,
            crate::db::mutation::MetadataFingerprint,
            ExecutionTarget,
            crate::profile::CatalogScope,
        ) -> Option<T>,
    ) -> Option<T> {
        let connection = self.database_command_identity()?;
        let WorkspaceTab::Relation(tab) = self.tabs.get(self.active_tab)? else {
            return None;
        };
        let edit = tab.edit.as_ref()?;
        let target = self.connection.target.clone()?;
        let scope = self
            .profiles
            .iter()
            .find(|profile| profile.id == connection.profile_id)?
            .catalog_scope
            .clone();
        let metadata = match &tab.ddl {
            RelationLoad::Ready(ddl) => crate::db::mutation::metadata_fingerprint(&ddl.value),
            _ => return None,
        };
        let columns = self.relation_result()?.columns;
        f(tab, edit, columns, metadata, target, scope)
    }

    fn activate_relation_transaction(
        &mut self,
        request: &mut crate::db::mutation::RelationMutationRequest,
        snapshot: Option<RelationEditSession>,
    ) -> Option<()> {
        let connection = request.connection;
        let WorkspaceTab::Relation(tab) = self.tabs.get_mut(self.active_tab)? else {
            return None;
        };
        if matches!(
            tab.transaction_state,
            TransactionState::Aborted | TransactionState::OutcomeUnknown
        ) {
            self.status_message("Transaction outcome must be resolved before mutating");
            return None;
        }
        if !matches!(
            tab.transaction_state,
            TransactionState::Idle | TransactionState::Active
        ) {
            self.status_message("Wait for the relation transaction to finish before mutating");
            return None;
        }
        if tab.transaction_state == TransactionState::Idle {
            tab.transaction_snapshot = snapshot;
            tab.transaction_generation = tab.transaction_generation.saturating_add(1);
        }
        request.edit_generation = tab.transaction_generation;
        tab.transaction_state = TransactionState::Active;
        request.connection = connection;
        Some(())
    }
    fn relation_yank(&mut self, selected: bool) {
        let row = self.active_grid_row();
        if let Some(edit) = self.relation_session_mut() {
            let row = if selected {
                edit.visual_range(row).map_or(row, |(start, _)| start)
            } else {
                row
            };
            edit.yank_row(row);
        }
    }
    fn relation_paste(&mut self) -> Vec<Command> {
        let row = self.active_grid_row();
        let Some(edit) = self.relation_session_mut() else {
            return Vec::new();
        };
        let position = row.saturating_add(1);
        if !edit.paste_row(position) {
            return Vec::new();
        }
        let Some(row) = edit.rows.get(position) else {
            return Vec::new();
        };
        let row_id = row.id;
        let values = row.current.clone();
        self.relation_insert_command(row_id, values)
    }
    fn relation_insert_row(&mut self) -> Vec<Command> {
        let row = self.active_grid_row();
        let columns = self.relation_result().map_or(0, |r| r.columns.len());
        let Some(edit) = self.relation_session_mut() else {
            return Vec::new();
        };
        let row_id = edit.insert_row(row, vec![crate::db::value::CellValue::Null; columns]);
        let values = edit
            .rows
            .iter()
            .find(|row| row.id == row_id)
            .map(|row| row.current.clone())
            .unwrap_or_default();
        self.relation_insert_command(row_id, values)
    }
    fn relation_undo(&mut self) -> Vec<Command> {
        self.relation_history_command(PendingMutationHistory::Undo)
    }
    fn relation_redo(&mut self) -> Vec<Command> {
        self.relation_history_command(PendingMutationHistory::Redo)
    }

    fn relation_history_command(&mut self, direction: PendingMutationHistory) -> Vec<Command> {
        let Some(connection) = self.database_command_identity() else {
            return Vec::new();
        };
        let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab) else {
            return Vec::new();
        };
        if matches!(
            tab.transaction_state,
            TransactionState::Aborted | TransactionState::OutcomeUnknown
        ) {
            self.status_message("Transaction outcome must be resolved before mutating");
            return Vec::new();
        }
        if tab.transaction_state != TransactionState::Active {
            self.status_message("Relation transaction is not active");
            return Vec::new();
        }
        let Some(edit) = tab.edit.as_mut() else {
            return Vec::new();
        };
        let Some(mut request) = edit.pending_mutation(direction) else {
            return Vec::new();
        };
        request.connection = connection;
        request.tab_generation = tab.generation;
        request.edit_generation = tab.transaction_generation;
        vec![Command::RelationMutation { request }]
    }

    fn select_grid(&mut self, row: usize, column: usize) {
        self.with_active_grid(|grid, (row_count, column_count)| {
            grid.selected_row = row.min(row_count.saturating_sub(1));
            grid.selected_column = column.min(column_count.saturating_sub(1));
            grid.clamp(row_count, column_count);
            grid.ensure_row_visible(row_count);
        });
    }

    fn set_grid_column_offset(&mut self, offset: usize) {
        self.with_active_grid(|grid, (row_count, column_count)| {
            let offset = offset.min(column_count.saturating_sub(1));
            grid.column_offset = offset;
            grid.selected_column = offset;
            grid.clamp(row_count, column_count);
        });
    }

    fn resize_grid_column(&mut self, delta: i16) {
        let base = self
            .relation_result()
            .map(|result| automatic_relation_column_widths(&result));
        let sql_base = self
            .active_console_opt()
            .and_then(|tab| {
                tab.outcome
                    .as_ref()
                    .and_then(|outcome| outcome.result_sets.last())
            })
            .map(automatic_relation_column_widths);
        let base = base.or(sql_base);
        self.with_active_grid(|grid, (rows, columns)| {
            let Some(base) = base.as_deref() else { return };
            let column = grid.selected_column;
            if column >= columns || column >= base.len() {
                return;
            }
            if grid.column_widths.len() < base.len() {
                grid.column_widths.resize(base.len(), None);
            }
            let current = grid.column_widths[column].unwrap_or(base[column]);
            grid.column_widths[column] = Some((current as i16 + delta).clamp(6, 80) as u16);
            grid.clamp(rows, columns);
        });
    }

    fn reset_grid_column_width(&mut self) {
        self.with_active_grid(|grid, (_, columns)| {
            if grid.selected_column < columns && grid.selected_column < grid.column_widths.len() {
                grid.column_widths[grid.selected_column] = None;
            }
        });
    }

    fn set_grid_column_width(&mut self, column: usize, width: u16) {
        self.with_active_grid(|grid, (rows, columns)| {
            if column < columns {
                if grid.column_widths.len() <= column {
                    grid.column_widths.resize(column + 1, None);
                }
                grid.column_widths[column] = Some(width.clamp(6, 80));
                grid.clamp(rows, columns);
            }
        });
    }

    fn load_active_relation(&mut self, refresh: bool) -> Vec<Command> {
        if let Some(WorkspaceTab::Relation(tab)) = self.tabs.get(self.active_tab)
            && tab.transaction_state != TransactionState::Idle
        {
            self.status_message("Resolve the active relation transaction before refreshing");
            return Vec::new();
        }
        let Some(connection) = self.database_command_identity() else {
            return Vec::new();
        };
        let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab) else {
            return Vec::new();
        };
        if tab.descriptor.key.profile_id != connection.profile_id {
            return Vec::new();
        }
        let Some(profile) = self
            .profiles
            .iter()
            .find(|profile| profile.id == connection.profile_id)
        else {
            return Vec::new();
        };
        if !relation_is_in_scope(tab, &profile.catalog_scope) {
            return Vec::new();
        }
        let kind = match tab.view {
            RelationView::Data => RelationRequestKind::Preview,
            RelationView::Ddl => RelationRequestKind::Ddl,
        };
        let should_load = match tab.view {
            RelationView::Data => {
                refresh
                    || matches!(
                        tab.data,
                        RelationLoad::Empty
                            | RelationLoad::Failed { .. }
                            | RelationLoad::Cancelled { .. }
                    )
            }
            RelationView::Ddl => {
                refresh
                    || matches!(
                        tab.ddl,
                        RelationLoad::Empty
                            | RelationLoad::Failed { .. }
                            | RelationLoad::Cancelled { .. }
                    )
            }
        };
        if !should_load {
            return Vec::new();
        }
        let previous_request = match tab.view {
            RelationView::Data => pending_relation_request(&tab.data),
            RelationView::Ddl => pending_relation_request(&tab.ddl),
        };
        let request = RelationRequest {
            tab_id: tab.id,
            tab_generation: tab.generation,
            request_id: tab.next_request_id,
            connection,
            relation: tab.descriptor.key.clone(),
            kind,
            scope: profile.catalog_scope.clone(),
            options: tab.query.submitted.clone(),
        };
        tab.next_request_id = tab.next_request_id.saturating_add(1);
        match kind {
            RelationRequestKind::Preview => {
                let previous = match std::mem::replace(&mut tab.data, RelationLoad::Empty) {
                    RelationLoad::Ready(snapshot) => Some(snapshot),
                    RelationLoad::Loading { previous, .. }
                    | RelationLoad::Failed { previous, .. }
                    | RelationLoad::Cancelled { previous } => previous,
                    RelationLoad::Empty => None,
                };
                tab.data = RelationLoad::Loading {
                    request: request.clone(),
                    previous,
                };
                previous_request
                    .map(Command::CancelRelationRequest)
                    .into_iter()
                    .chain([Command::LoadRelationPreview(request)])
                    .collect()
            }
            RelationRequestKind::Ddl => {
                let previous = match std::mem::replace(&mut tab.ddl, RelationLoad::Empty) {
                    RelationLoad::Ready(snapshot) => Some(snapshot),
                    RelationLoad::Loading { previous, .. }
                    | RelationLoad::Failed { previous, .. }
                    | RelationLoad::Cancelled { previous } => previous,
                    RelationLoad::Empty => None,
                };
                tab.ddl = RelationLoad::Loading {
                    request: request.clone(),
                    previous,
                };
                previous_request
                    .map(Command::CancelRelationRequest)
                    .into_iter()
                    .chain([Command::LoadRelationDdl(request)])
                    .collect()
            }
        }
    }

    fn accept_relation(
        &mut self,
        request: RelationRequest,
        result: Result<RelationSnapshot, String>,
    ) -> Vec<Command> {
        let Some(tab_index) = self.tabs.iter().position(|tab| {
            match tab {
                WorkspaceTab::Relation(tab) if tab.id == request.tab_id => Some(tab),
                _ => None,
            }
            .is_some()
        }) else {
            return Vec::new();
        };
        let current = match &self.tabs[tab_index] {
            WorkspaceTab::Relation(tab) => tab,
            _ => return Vec::new(),
        };
        if !self.relation_result_is_current(&request, current) {
            return Vec::new();
        }
        let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(tab_index) else {
            return Vec::new();
        };
        match (request.kind, result) {
            (RelationRequestKind::Preview, Ok(RelationSnapshot::Preview(snapshot))) => {
                if matches!(&tab.data, RelationLoad::Loading { request: pending, .. } if pending == &request)
                {
                    let rows = snapshot
                        .result
                        .result_sets
                        .last()
                        .map(|result| result.rows.clone());
                    tab.data = RelationLoad::Ready(crate::model::relation::OwnedSnapshot {
                        value: snapshot,
                        attribution: crate::model::relation::SnapshotAttribution {
                            connection: request.connection,
                            profile_id: request.connection.profile_id,
                            scope: request.scope.clone(),
                        },
                    });
                    tab.edit = rows.map(RelationEditSession::from_rows);
                }
            }
            (RelationRequestKind::Ddl, Ok(RelationSnapshot::Ddl(snapshot))) => {
                if matches!(&tab.ddl, RelationLoad::Loading { request: pending, .. } if pending == &request)
                {
                    tab.ddl = RelationLoad::Ready(crate::model::relation::OwnedSnapshot {
                        value: *snapshot,
                        attribution: crate::model::relation::SnapshotAttribution {
                            connection: request.connection,
                            profile_id: request.connection.profile_id,
                            scope: request.scope.clone(),
                        },
                    });
                }
            }
            (RelationRequestKind::Preview, Err(message)) => {
                if let RelationLoad::Loading {
                    previous,
                    request: pending,
                } = &tab.data
                    && pending == &request
                {
                    tab.data = RelationLoad::Failed {
                        message,
                        previous: previous.clone(),
                    };
                }
            }
            (RelationRequestKind::Ddl, Err(message)) => {
                if let RelationLoad::Loading {
                    previous,
                    request: pending,
                } = &tab.ddl
                    && pending == &request
                {
                    tab.ddl = RelationLoad::Failed {
                        message,
                        previous: previous.clone(),
                    };
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn relation_result_is_current(&self, request: &RelationRequest, tab: &RelationTab) -> bool {
        self.connection.active_identity() == Some(request.connection)
            && tab.id == request.tab_id
            && tab.generation == request.tab_generation
            && tab.descriptor.key == request.relation
            && match request.kind {
                RelationRequestKind::Preview => {
                    matches!(&tab.data, RelationLoad::Loading { request: pending, .. } if pending == request)
                }
                RelationRequestKind::Ddl => matches!(
                    &tab.ddl,
                    RelationLoad::Loading { request: pending, .. } if pending == request
                ),
            }
            && self
                .profiles
                .iter()
                .find(|profile| profile.id == request.connection.profile_id)
                .is_some_and(|profile| relation_is_in_scope(tab, &profile.catalog_scope))
    }

    fn manual_matches(
        &self,
        tab_id: Uuid,
        query_generation: u64,
        transaction_generation: u64,
        connection: ConnectionIdentity,
        state: TransactionState,
    ) -> bool {
        self.connection.active_identity() == Some(connection)
            && self
                .tabs
                .iter()
                .find(|tab| tab.id() == tab_id)
                .and_then(WorkspaceTab::as_console)
                .is_some_and(|tab| {
                    tab.generation == query_generation
                        && tab.transaction_generation == transaction_generation
                        && tab.transaction_state == state
                })
    }

    fn finish_query(
        &mut self,
        tab_id: Uuid,
        generation: u64,
        outcome: crate::db::query::QueryOutcome,
        _manual: bool,
    ) {
        let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id() == tab_id)
            .and_then(WorkspaceTab::as_console_mut)
        else {
            return;
        };
        let rows = outcome.stats.row_count;
        let total_ms = outcome.stats.total().as_millis();
        let (is_query, execution_log) = tab
            .last_execution
            .as_ref()
            .filter(|last| last.draft.query_generation + 1 == generation)
            .map(|last| {
                (
                    last.draft.statement_count == 1
                        && last.draft.risks.len() == 1
                        && last.draft.risks[0] == sql::SqlRisk::ReadOnly,
                    format_execution_log(last, &outcome),
                )
            })
            .unwrap_or((true, None));
        tab.query_status = QueryStatus::Idle;
        if let Some((context, summary)) = execution_log {
            tab.output.push(OutputEntry {
                kind: OutputKind::Success,
                message: context,
            });
            tab.output.push(OutputEntry {
                kind: OutputKind::Success,
                message: summary,
            });
        } else {
            tab.output.push(OutputEntry {
                kind: OutputKind::Success,
                message: format!("{rows} row(s) retrieved in {total_ms} ms"),
            });
        }
        tab.outcome = Some(outcome);
        tab.result_view = if is_query {
            ResultView::Data
        } else {
            ResultView::Output
        };
        tab.query.capability = tab.last_execution.as_ref().map_or(
            DataQueryCapability::Unavailable("Run a read-only query first".into()),
            |last| {
                if sql::derived_query_capable(&last.draft.sql, last.draft.dialect) {
                    DataQueryCapability::Sql
                } else {
                    DataQueryCapability::Unavailable(
                        "SQL result filtering requires one read-only SELECT query".into(),
                    )
                }
            },
        );
        if let Some(last) = tab.last_execution.as_mut()
            && last.draft.query_generation + 1 == generation
        {
            last.result = ExecutionResult::Succeeded;
        }
    }

    fn pending_connection_matches(&self, profile_id: Uuid, generation: u64) -> bool {
        self.connection.pending_profile_id == Some(profile_id)
            && self.connection.pending_generation == Some(generation)
    }
}

fn relation_is_in_scope(tab: &RelationTab, scope: &crate::profile::CatalogScope) -> bool {
    let name = &tab.descriptor.qualified_name;
    name.database.as_deref().is_none_or(|database| {
        scope.allows_database(database)
            && name
                .schema
                .as_deref()
                .is_none_or(|schema| scope.allows_schema(database, schema))
    })
}

fn search_preload_group(group: crate::db::catalog::ObjectGroup) -> bool {
    matches!(
        group,
        crate::db::catalog::ObjectGroup::Tables
            | crate::db::catalog::ObjectGroup::Views
            | crate::db::catalog::ObjectGroup::MaterializedViews
            | crate::db::catalog::ObjectGroup::Functions
            | crate::db::catalog::ObjectGroup::Procedures
            | crate::db::catalog::ObjectGroup::Sequences
            | crate::db::catalog::ObjectGroup::Types
            | crate::db::catalog::ObjectGroup::Triggers
    )
}

fn add_explorer_profile(
    explorer: &mut ExplorerState,
    profile: &ConnectionProfile,
    provenance: ProfileProvenance,
) {
    let endpoint = match profile.kind {
        DatabaseKind::Sqlite => profile
            .sqlite_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .or_else(|| profile.database.clone())
            .unwrap_or_default(),
        DatabaseKind::Postgres | DatabaseKind::MySql => {
            let host = profile.host.as_deref().unwrap_or_default();
            profile
                .port
                .map_or_else(|| host.to_owned(), |port| format!("{host}:{port}"))
        }
    };
    explorer.normalized.add_profile_with_metadata(
        profile.id,
        profile.name.clone(),
        profile.kind,
        endpoint,
        provenance,
    );
}

fn tab_snapshot(tab: &ConsoleTab) -> transaction::TransactionSnapshot {
    transaction::TransactionSnapshot {
        mode: tab.transaction_mode,
        state: tab.transaction_state,
        generation: tab.transaction_generation,
    }
}

fn apply_transaction_snapshot(tab: &mut ConsoleTab, snapshot: transaction::TransactionSnapshot) {
    tab.transaction_mode = snapshot.mode;
    tab.transaction_state = snapshot.state;
    tab.transaction_generation = snapshot.generation;
}

fn append_failed_execution_output(tab: &mut ConsoleTab, generation: u64, message: String) {
    if let Some(last) = tab
        .last_execution
        .as_ref()
        .filter(|last| last.draft.query_generation + 1 == generation)
    {
        let target = crate::security::sanitize_terminal_text(&format!(
            "{}{}",
            last.draft.target.database,
            last.draft
                .target
                .schema
                .as_deref()
                .map_or(String::new(), |schema| format!(".{schema}"))
        ));
        let sql = crate::security::sanitize_terminal_text(&last.draft.sql)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|elapsed| format_timestamp(elapsed.as_secs(), elapsed.subsec_millis()));
        let timestamp = elapsed.unwrap_or_else(|| "unknown time".to_owned());
        tab.output.push(OutputEntry {
            kind: OutputKind::Info,
            message: format!("[{timestamp}] {target}> {sql}"),
        });
    }
    tab.output.push(OutputEntry {
        kind: OutputKind::Error,
        message,
    });
}

fn format_execution_log(
    last: &LastExecution,
    outcome: &crate::db::query::QueryOutcome,
) -> Option<(String, String)> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    let timestamp = format_timestamp(elapsed.as_secs(), elapsed.subsec_millis());
    let target = crate::security::sanitize_terminal_text(&format!(
        "{}{}",
        last.draft.target.database,
        last.draft
            .target
            .schema
            .as_deref()
            .map_or(String::new(), |schema| format!(".{schema}"))
    ));
    let sql = crate::security::sanitize_terminal_text(&last.draft.sql)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let context = format!("[{timestamp}] {target}> {sql}");
    let stats = &outcome.stats;
    let total_ms = stats.total().as_millis();
    let summary = if last.draft.statement_count == 1
        && last.draft.risks.len() == 1
        && last.draft.risks[0] == sql::SqlRisk::ReadOnly
    {
        format!(
            "[{timestamp}] {} rows retrieved starting from 1 in {total_ms} ms (execution: {} ms, fetching: {} ms)",
            stats.row_count,
            stats.execution.as_millis(),
            stats.fetch.as_millis(),
        )
    } else {
        let affected_rows = outcome
            .result_sets
            .iter()
            .map(|result| result.affected_rows)
            .sum::<u64>();
        format!(
            "[{timestamp}] {affected_rows} row(s) affected in {total_ms} ms (execution: {} ms, fetching: {} ms)",
            stats.execution.as_millis(),
            stats.fetch.as_millis(),
        )
    };
    Some((context, summary))
}

fn format_timestamp(seconds: u64, millis: u32) -> String {
    let days = (seconds / 86_400) as i64;
    let day_seconds = seconds % 86_400;
    let (year, month, day) = civil_date(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}:{millis:03}",
        day_seconds / 3_600,
        day_seconds / 60 % 60,
        day_seconds % 60,
    )
}

fn civil_date(days_since_epoch: i64) -> (i64, i64, i64) {
    let days = days_since_epoch + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    (year + i64::from(month <= 2), month, day)
}

fn cursor_byte(text: &str, line: usize, column: usize) -> usize {
    let mut offset = 0;
    for (index, value) in text.split('\n').enumerate() {
        if index == line {
            return offset
                + value
                    .char_indices()
                    .nth(column)
                    .map(|(index, _)| index)
                    .unwrap_or(value.len());
        }
        offset += value.len() + 1;
    }
    text.len()
}

fn next_profile_request(manager: &mut ProfileManagerState) -> u64 {
    manager.request_generation = manager.request_generation.wrapping_add(1);
    manager.request_generation
}

fn move_bounded(current: usize, delta: isize, count: usize) -> usize {
    if count == 0 {
        0
    } else {
        current
            .saturating_add_signed(delta)
            .min(count.saturating_sub(1))
    }
}

fn parse_relation_value(
    value: &str,
    old: &crate::db::value::CellValue,
) -> Result<crate::db::value::CellValue, String> {
    use crate::db::value::CellValue;
    if value.eq_ignore_ascii_case("null") {
        return Ok(CellValue::Null);
    }
    match old {
        CellValue::Boolean(_) => value
            .parse()
            .map(CellValue::Boolean)
            .map_err(|_| "invalid boolean".into()),
        CellValue::Integer(_) => value
            .parse()
            .map(CellValue::Integer)
            .map_err(|_| "invalid integer".into()),
        CellValue::Unsigned(_) => value
            .parse()
            .map(CellValue::Unsigned)
            .map_err(|_| "invalid unsigned integer".into()),
        CellValue::Float(_) => value
            .parse()
            .map(CellValue::Float)
            .map_err(|_| "invalid floating-point number".into()),
        CellValue::Date(_) => value
            .parse()
            .map(CellValue::Date)
            .map_err(|_| "invalid date; expected YYYY-MM-DD".into()),
        CellValue::Time(_) => value
            .parse()
            .map(CellValue::Time)
            .map_err(|_| "invalid time; expected HH:MM:SS[.fraction]".into()),
        CellValue::DateTime(_) => value
            .parse()
            .map(CellValue::DateTime)
            .map_err(|_| "invalid datetime; expected YYYY-MM-DD HH:MM:SS[.fraction]".into()),
        CellValue::Timestamp(_) => value
            .parse()
            .map(CellValue::Timestamp)
            .map_err(|_| "invalid timestamp; expected an RFC 3339 timestamp".into()),
        _ => Ok(CellValue::Text(value.into())),
    }
}

fn input_value(value: &crate::db::value::CellValue) -> crate::db::mutation::InputValue {
    use crate::db::mutation::InputValue;
    match value {
        crate::db::value::CellValue::Null => InputValue::Null,
        _ => InputValue::Value(value.clone()),
    }
}

fn relation_grid_dimensions(load: &RelationLoad<crate::db::RelationPreview>) -> (usize, usize) {
    let snapshot = match load {
        RelationLoad::Ready(snapshot) => Some(snapshot),
        RelationLoad::Loading { previous, .. }
        | RelationLoad::Failed { previous, .. }
        | RelationLoad::Cancelled { previous } => previous.as_ref(),
        RelationLoad::Empty => None,
    };
    snapshot
        .and_then(|snapshot| snapshot.value.result.result_sets.last())
        .map_or((0, 0), |result| (result.rows.len(), result.columns.len()))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use uuid::Uuid;

    use super::App;
    use crate::{
        action::{Action, Command},
        db::{
            catalog::{CatalogId, CatalogKind},
            mutation::{
                InputValue, MetadataFingerprint, MutationResult, RelationMutation,
                RelationMutationRequest,
            },
            query::{QueryOutcome, QueryStats, ResultSet},
            value::CellValue,
        },
        identity::ConnectionIdentity,
        model::explorer::ExplorerConnectionStatus,
        model::relation::{RelationRequest, RelationRequestKind, RelationSnapshot, RelationTab},
        model::tab::{GridRowAlignment, GridRowTarget, GridScrollAmount, ResultView, WorkspaceTab},
        model::transaction::{TransactionExitChoice, TransactionMode, TransactionState},
        model::workspace::{ConnectionStatus, Focus, Overlay, QueryStatus},
        model::{
            execution_target::ExecutionTarget,
            relation::RelationKey,
            relation_edit::{EditableRowState, RelationEditSession},
        },
        profile::import_connection_url,
        profile::{CatalogScope, DatabaseKind},
    };

    fn empty_outcome() -> QueryOutcome {
        QueryOutcome {
            result_sets: vec![ResultSet::default()],
            stats: QueryStats::new(Duration::from_millis(2), Duration::from_millis(3), 0),
        }
    }

    fn connected_query_app(sql: &str) -> (App, Uuid, u64) {
        let profile = import_connection_url("postgres://localhost/kms", Some("kms"))
            .unwrap()
            .profile;
        let profile_id = profile.id;
        let mut app = App::new(vec![profile]);
        app.connection.profile_id = Some(profile_id);
        app.connection.generation = 1;
        app.connection.status = ConnectionStatus::Connected;
        app.connection.target = app.active_console().execution_target.clone();
        app.update(Action::ReplaceEditor(sql.into()));
        let mut commands = app.update(Action::RunActiveSql);
        if commands.is_empty() {
            app.update(Action::ToggleExecutionConfirmationFocus);
            commands = app.update(Action::ConfirmExecution);
        }
        let (tab_id, generation) = match &commands[0] {
            Command::RunQuery {
                tab_id, generation, ..
            } => (*tab_id, *generation),
            command => panic!("unexpected command: {command:?}"),
        };
        (app, tab_id, generation)
    }

    #[test]
    fn query_completion_focuses_data_and_records_execution_details() {
        let (mut app, tab_id, generation) = connected_query_app("SELECT * FROM tools.sys_user");
        app.update(Action::QueryFinished {
            tab_id,
            generation,
            connection: app.connection.active_identity().unwrap(),
            outcome: QueryOutcome {
                result_sets: vec![ResultSet {
                    rows: vec![vec![]; 137],
                    ..ResultSet::default()
                }],
                stats: QueryStats::new(Duration::from_millis(21), Duration::from_millis(397), 137),
            },
        });

        let tab = app.active_console();
        assert_eq!(tab.result_view, ResultView::Data);
        assert!(
            tab.output
                .iter()
                .any(|entry| { entry.message.contains("> SELECT * FROM tools.sys_user") })
        );
        assert!(tab.output.iter().any(|entry| {
            entry
                .message
                .contains("137 rows retrieved starting from 1 in 418 ms")
                && entry.message.contains("execution: 21 ms")
                && entry.message.contains("fetching: 397 ms")
        }));
    }

    #[test]
    fn query_failure_records_sql_before_database_error() {
        let (mut app, tab_id, generation) = connected_query_app("SELECT * FROM sys_user1;");
        app.update(Action::QueryFailed {
            tab_id,
            generation,
            connection: app.connection.active_identity().unwrap(),
            message: "relation \"sys_user1\" does not exist".into(),
        });

        let entries = &app.active_console().output;
        assert_eq!(entries.len(), 2);
        assert!(entries[0].message.contains("> SELECT * FROM sys_user1;"));
        assert_eq!(entries[1].message, "relation \"sys_user1\" does not exist");
    }

    #[test]
    fn starting_sql_execution_does_not_add_placeholder_output() {
        let (app, _, _) = connected_query_app("SELECT 1");
        assert!(
            app.active_console()
                .output
                .iter()
                .all(|entry| entry.message != "Executing SQL")
        );
    }

    #[test]
    fn non_query_completion_focuses_output_and_appends_execution_details() {
        let (mut app, tab_id, generation) =
            connected_query_app("UPDATE tools.sys_user SET enabled = true");
        let connection = app.connection.active_identity().unwrap();
        let outcome = QueryOutcome {
            result_sets: vec![ResultSet {
                affected_rows: 3,
                ..ResultSet::default()
            }],
            stats: QueryStats::new(Duration::from_millis(9), Duration::ZERO, 0),
        };
        app.update(Action::QueryFinished {
            tab_id,
            generation,
            connection,
            outcome: outcome.clone(),
        });
        let first_count = app.active_console().output.len();

        let (tab_id, generation) = {
            app.update(Action::ReplaceEditor("DELETE FROM tools.sys_user".into()));
            let mut commands = app.update(Action::RunActiveSql);
            if commands.is_empty() {
                app.update(Action::ToggleExecutionConfirmationFocus);
                commands = app.update(Action::ConfirmExecution);
            }
            match &commands[0] {
                Command::RunQuery {
                    tab_id, generation, ..
                } => (*tab_id, *generation),
                command => panic!("unexpected command: {command:?}"),
            }
        };
        app.update(Action::QueryFinished {
            tab_id,
            generation,
            connection,
            outcome,
        });

        let tab = app.active_console();
        assert_eq!(tab.result_view, ResultView::Output);
        assert!(tab.output.len() > first_count);
        assert!(tab.output.iter().any(|entry| {
            entry
                .message
                .contains("> UPDATE tools.sys_user SET enabled = true")
        }));
        assert!(tab.output.iter().any(|entry| {
            entry.message.contains("3 row(s) affected in 9 ms")
                && entry.message.contains("execution: 9 ms")
                && entry.message.contains("fetching: 0 ms")
        }));
    }

    fn sql_result_app() -> App {
        let mut app = App::new(Vec::new());
        app.active_console_mut().outcome = Some(QueryOutcome {
            result_sets: vec![ResultSet {
                columns: vec![
                    crate::db::query::ColumnMeta {
                        name: "id".into(),
                        type_name: "int".into(),
                    },
                    crate::db::query::ColumnMeta {
                        name: "name".into(),
                        type_name: "text".into(),
                    },
                ],
                rows: vec![vec![
                    crate::db::value::CellValue::Integer(1),
                    crate::db::value::CellValue::Text("a".into()),
                ]],
                affected_rows: 0,
            }],
            stats: QueryStats::new(Duration::ZERO, Duration::ZERO, 1),
        });
        app
    }

    fn relation_mutation_app() -> (App, RelationMutationRequest) {
        let mut app = App::new(Vec::new());
        let mut tab = RelationTab::new("items");
        tab.edit = Some(RelationEditSession::from_rows(vec![vec![
            CellValue::Integer(1),
            CellValue::Text("draft".into()),
        ]]));
        let tab_id = tab.id;
        app.tabs.push(WorkspaceTab::Relation(tab));
        app.active_tab = 1;
        let profile_id = uuid::Uuid::nil();
        let object_id = CatalogId::new(profile_id, CatalogKind::Table, ["items"]);
        let connection = ConnectionIdentity {
            profile_id,
            generation: 1,
        };
        let request = RelationMutationRequest {
            tab_id,
            tab_generation: 0,
            edit_generation: 1,
            row_id: crate::model::relation_edit::EditableRowId(1),
            connection,
            target: ExecutionTarget {
                profile_id,
                database: "items".into(),
                schema: None,
            },
            relation: object_id.clone(),
            relation_key: RelationKey {
                profile_id,
                object_id,
            },
            scope: CatalogScope::for_profile(DatabaseKind::Sqlite, "items", None),
            metadata: MetadataFingerprint {
                relation: "items".into(),
                columns: vec![("id".into(), "INTEGER".into(), false)],
                primary_key: vec!["id".into()],
            },
            operation: RelationMutation::InsertRow(crate::db::mutation::InsertRowMutation {
                columns: vec![0],
                values: vec![InputValue::Value(CellValue::Integer(1))],
            }),
        };
        (app, request)
    }

    #[test]
    fn sql_grid_resize_and_reset_use_shared_state() {
        let mut app = sql_result_app();
        app.update(Action::GridSelect { row: 0, column: 1 });
        app.update(Action::GridResizeColumn(10));
        assert_eq!(
            app.active_console().grid.column_widths,
            vec![None, Some(16)]
        );
        app.update(Action::GridResetColumnWidth);
        assert_eq!(app.active_console().grid.column_widths, vec![None, None]);
    }

    #[test]
    fn grid_vim_navigation_updates_only_vertical_state() {
        let mut app = sql_result_app();
        let result = app
            .active_console_mut()
            .outcome
            .as_mut()
            .unwrap()
            .result_sets
            .last_mut()
            .unwrap();
        result.rows = (0..20)
            .map(|row| vec![CellValue::Integer(row), CellValue::Text(row.to_string())])
            .collect();
        let tab_id = app.active_console().id;
        app.update(Action::GridSelect { row: 6, column: 1 });
        app.update(Action::GridViewportChanged(
            crate::model::tab::DataGridViewport {
                tab_id,
                column_offset: 1,
                row_offset: 4,
                visible_rows: 5,
            },
        ));
        app.active_console_mut().grid.column_widths = vec![Some(10), Some(12)];
        let horizontal = (
            app.active_console().grid.selected_column,
            app.active_console().grid.column_offset,
            app.active_console().grid.column_widths.clone(),
        );

        app.update(Action::GridSelectRow(GridRowTarget::ViewMiddle));
        assert_eq!(
            (
                app.active_console().grid.selected_row,
                app.active_console().grid.row_offset
            ),
            (6, 4)
        );
        app.update(Action::GridScrollRows {
            direction: 1,
            amount: GridScrollAmount::HalfPage,
        });
        assert_eq!(
            (
                app.active_console().grid.selected_row,
                app.active_console().grid.row_offset
            ),
            (8, 6)
        );
        app.update(Action::GridAlignSelectedRow(GridRowAlignment::Bottom));
        assert_eq!(
            (
                app.active_console().grid.selected_row,
                app.active_console().grid.row_offset
            ),
            (8, 4)
        );
        app.update(Action::GridSelectRow(GridRowTarget::Last));
        assert_eq!(
            (
                app.active_console().grid.selected_row,
                app.active_console().grid.row_offset
            ),
            (19, 15)
        );
        app.update(Action::GridSelectRow(GridRowTarget::First));
        assert_eq!(
            (
                app.active_console().grid.selected_row,
                app.active_console().grid.row_offset
            ),
            (0, 0)
        );
        assert_eq!(
            (
                app.active_console().grid.selected_column,
                app.active_console().grid.column_offset,
                app.active_console().grid.column_widths.clone(),
            ),
            horizontal
        );
    }

    #[test]
    fn filtered_relation_preview_replaces_the_editable_rows() {
        let mut profile = import_connection_url("sqlite::memory:", Some("test"))
            .unwrap()
            .profile;
        profile.catalog_scope = CatalogScope::for_profile(DatabaseKind::Sqlite, "", None);
        let profile_id = profile.id;
        let mut app = App::new(vec![profile]);
        app.connection.profile_id = Some(profile_id);
        app.connection.generation = 1;
        app.connection.status = ConnectionStatus::Connected;

        let tab = RelationTab::new("users");
        let tab_id = tab.id;
        app.tabs.push(WorkspaceTab::Relation(tab));
        app.active_tab = app.tabs.len() - 1;

        let relation = match &app.tabs[app.active_tab] {
            WorkspaceTab::Relation(tab) => tab.descriptor.key.clone(),
            WorkspaceTab::Sql(_) => unreachable!(),
        };
        let scope = app.profiles[0].catalog_scope.clone();
        let request = RelationRequest {
            tab_id,
            tab_generation: 0,
            request_id: 1,
            connection: app.connection.active_identity().unwrap(),
            relation,
            kind: RelationRequestKind::Preview,
            scope: scope.clone(),
            options: Default::default(),
        };
        if let WorkspaceTab::Relation(tab) = &mut app.tabs[app.active_tab] {
            tab.data = crate::model::relation::RelationLoad::Loading {
                request: request.clone(),
                previous: None,
            };
            tab.edit = Some(RelationEditSession::from_rows(vec![vec![CellValue::Text(
                "test100".into(),
            )]]));
        }

        let result = ResultSet {
            columns: vec![crate::db::query::ColumnMeta {
                name: "username".into(),
                type_name: "text".into(),
            }],
            rows: vec![vec![CellValue::Text("test".into())]],
            affected_rows: 0,
        };
        app.update(Action::RelationSucceeded {
            request,
            snapshot: Box::new(RelationSnapshot::Preview(crate::db::RelationPreview {
                sql: "SELECT * FROM users WHERE username = 'test'".into(),
                result: QueryOutcome {
                    result_sets: vec![result],
                    stats: QueryStats::new(Duration::ZERO, Duration::ZERO, 1),
                },
            })),
        });

        let WorkspaceTab::Relation(tab) = &app.tabs[app.active_tab] else {
            unreachable!()
        };
        assert_eq!(tab.edit.as_ref().unwrap().rows.len(), 1);
        assert_eq!(
            tab.edit.as_ref().unwrap().rows[0].current[0],
            CellValue::Text("test".into())
        );
    }

    #[test]
    fn grid_viewport_sync_is_identity_safe_and_clamped() {
        let mut app = sql_result_app();
        let tab_id = app.active_console().id;
        app.update(Action::GridViewportChanged(
            crate::model::tab::DataGridViewport {
                tab_id,
                column_offset: 1,
                row_offset: 9,
                visible_rows: 3,
            },
        ));
        assert_eq!(app.active_console().grid.column_offset, 1);
        assert_eq!(app.active_console().grid.row_offset, 0);
        assert_eq!(app.active_console().grid.viewport_rows, 3);

        app.update(Action::GridViewportChanged(
            crate::model::tab::DataGridViewport {
                tab_id: Uuid::new_v4(),
                column_offset: 0,
                row_offset: 0,
                visible_rows: 7,
            },
        ));
        assert_eq!(app.active_console().grid.column_offset, 1);
        assert_eq!(app.active_console().grid.viewport_rows, 3);
    }

    #[test]
    fn grid_navigation_scrolls_rows_only_after_crossing_viewport_edges() {
        let mut app = sql_result_app();
        let result = app
            .active_console_mut()
            .outcome
            .as_mut()
            .unwrap()
            .result_sets
            .last_mut()
            .unwrap();
        result.rows = (0..10)
            .map(|row| vec![CellValue::Integer(row), CellValue::Text(row.to_string())])
            .collect();
        let tab_id = app.active_console().id;
        app.update(Action::GridViewportChanged(
            crate::model::tab::DataGridViewport {
                tab_id,
                column_offset: 0,
                row_offset: 0,
                visible_rows: 3,
            },
        ));
        app.update(Action::GridSelect { row: 1, column: 0 });

        app.update(Action::GridMove {
            rows: 1,
            columns: 0,
        });
        assert_eq!(
            (
                app.active_console().grid.selected_row,
                app.active_console().grid.row_offset
            ),
            (2, 0)
        );
        app.update(Action::GridMove {
            rows: 1,
            columns: 0,
        });
        assert_eq!(
            (
                app.active_console().grid.selected_row,
                app.active_console().grid.row_offset
            ),
            (3, 1)
        );
        app.update(Action::GridMove {
            rows: -1,
            columns: 0,
        });
        assert_eq!(
            (
                app.active_console().grid.selected_row,
                app.active_console().grid.row_offset
            ),
            (2, 1)
        );
        app.update(Action::GridMove {
            rows: -1,
            columns: 0,
        });
        assert_eq!(
            (
                app.active_console().grid.selected_row,
                app.active_console().grid.row_offset
            ),
            (1, 1)
        );
        app.update(Action::GridMove {
            rows: -1,
            columns: 0,
        });
        assert_eq!(
            (
                app.active_console().grid.selected_row,
                app.active_console().grid.row_offset
            ),
            (0, 0)
        );
    }

    #[test]
    fn relation_insert_success_marks_row_inserted_and_keeps_returned_values() {
        let (mut app, request) = relation_mutation_app();
        app.update(Action::RelationMutationSucceeded {
            request,
            result: MutationResult::Inserted {
                row: vec![CellValue::Integer(7), CellValue::Text("server".into())],
            },
        });
        let WorkspaceTab::Relation(tab) = &app.tabs[app.active_tab] else {
            panic!("expected relation tab")
        };
        let row = &tab.edit.as_ref().unwrap().rows[0];
        assert_eq!(row.state, EditableRowState::Inserted);
        assert_eq!(row.current[0], CellValue::Integer(7));
        assert_eq!(row.current[1], CellValue::Text("server".into()));
    }

    #[test]
    fn relation_mutation_failure_marks_conflict_without_aborting_transaction() {
        let (mut app, request) = relation_mutation_app();
        app.update(Action::RelationMutationFailed {
            request,
            message: "conflict".into(),
        });
        let WorkspaceTab::Relation(tab) = &app.tabs[app.active_tab] else {
            panic!("expected relation tab")
        };
        assert!(matches!(
            tab.edit.as_ref().unwrap().rows[0].state,
            EditableRowState::Conflict { .. }
        ));
        assert_ne!(tab.transaction_state, TransactionState::Aborted);
        assert_ne!(tab.transaction_state, TransactionState::OutcomeUnknown);
    }

    #[test]
    fn names_new_consoles_without_reusing_sequence_numbers() {
        let mut app = App::new(Vec::new());
        assert_eq!(app.active_console().name, "console");

        app.update(Action::NewConsole);
        app.update(Action::NewConsole);
        assert_eq!(
            app.tabs
                .iter()
                .map(crate::model::tab::WorkspaceTab::title)
                .collect::<Vec<_>>(),
            ["console", "console_2", "console_3"]
        );

        app.update(Action::CloseActiveTab);
        app.update(Action::NewConsole);
        assert_eq!(app.active_console().name, "console_4");
    }

    #[test]
    fn closing_active_tab_chooses_the_left_neighbor() {
        let mut app = App::new(Vec::new());
        app.update(Action::NewConsole);
        app.update(Action::NewConsole);
        assert_eq!(app.active_tab, 2);

        app.update(Action::CloseActiveTab);

        assert_eq!(app.active_tab, 1);
        assert_eq!(app.active_console().name, "console_2");
    }

    #[test]
    fn keeps_at_least_one_console_open() {
        let mut app = App::new(Vec::new());

        app.update(Action::CloseActiveTab);

        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.active_console().name, "console");
    }

    #[test]
    fn goto_sql_console_activates_the_first_available_sql_tab() {
        let mut app = App::new(Vec::new());
        app.update(Action::NewConsole);
        app.tabs
            .push(WorkspaceTab::Relation(RelationTab::new("users")));
        app.active_tab = 2;
        app.focus = Focus::Results;

        app.update(Action::GotoSqlConsole);

        assert_eq!(app.active_tab, 0);
        assert_eq!(app.focus, Focus::Editor);
    }

    #[test]
    fn cycles_focus_in_both_directions() {
        let mut app = App::new(Vec::new());
        assert_eq!(app.focus, Focus::Editor);

        app.update(Action::FocusNext);
        assert_eq!(app.focus, Focus::Results);
        app.update(Action::FocusNext);
        assert_eq!(app.focus, Focus::Explorer);
        app.update(Action::FocusPrevious);
        assert_eq!(app.focus, Focus::Results);
    }

    #[test]
    fn help_is_scoped_and_escape_dismisses_it() {
        let mut app = App::new(Vec::new());
        app.focus = Focus::Explorer;

        app.update(Action::ShowHelp);
        assert_eq!(
            app.overlay,
            Some(Overlay::Help(crate::help::HelpState::new(Focus::Explorer)))
        );
        app.update(Action::DismissOverlay);
        assert_eq!(app.overlay, None);
    }

    #[test]
    fn stale_query_results_cannot_replace_newer_runs() {
        let profile = import_connection_url(":memory:", Some("query"))
            .unwrap()
            .profile;
        let profile_id = profile.id;
        let mut app = App::new(vec![profile]);
        app.connection.profile_id = Some(profile_id);
        app.connection.generation = 1;
        app.connection.status = ConnectionStatus::Connected;
        app.connection.target = app.active_console().execution_target.clone();
        app.update(Action::ReplaceEditor("SELECT 1".into()));
        let commands = app.update(Action::RunActiveSql);
        let (tab_id, generation) = match &commands[0] {
            Command::RunQuery {
                tab_id, generation, ..
            } => (*tab_id, *generation),
            command => panic!("unexpected command: {command:?}"),
        };
        assert_eq!(app.active_console().query_status, QueryStatus::Running);

        app.update(Action::QueryFinished {
            tab_id,
            generation: generation.saturating_sub(1),
            connection: app.connection.active_identity().unwrap(),
            outcome: empty_outcome(),
        });
        assert!(app.active_console().outcome.is_none());

        app.update(Action::QueryFinished {
            tab_id,
            generation,
            connection: app.connection.active_identity().unwrap(),
            outcome: empty_outcome(),
        });
        assert!(app.active_console().outcome.is_some());
        assert_eq!(app.active_console().query_status, QueryStatus::Idle);
    }

    #[test]
    fn empty_sql_does_not_start_a_query() {
        let mut app = App::new(Vec::new());

        assert!(app.update(Action::RunActiveSql).is_empty());
        assert_eq!(app.active_console().query_status, QueryStatus::Idle);
    }

    #[test]
    fn stale_connection_events_are_ignored() {
        let profile = import_connection_url("sqlite::memory:", Some("demo"))
            .unwrap()
            .profile;
        let profile_id = profile.id;
        let mut app = App::new(vec![profile]);
        let commands = app.update(Action::RequestProfileConnect { profile_id });
        let generation = match commands[0] {
            Command::Connect { generation, .. } => generation,
            ref command => panic!("unexpected command: {command:?}"),
        };

        app.update(Action::ConnectionFailed {
            profile_id,
            generation: generation + 1,
            message: "late".into(),
        });
        assert!(app.connection.error.is_none());
    }

    #[test]
    fn invalidated_active_connection_fails_and_blocks_manual_transactions() {
        let first = import_connection_url("sqlite::memory:", Some("first"))
            .unwrap()
            .profile;
        let mut app = App::new(vec![first.clone()]);
        app.connection.profile_id = Some(first.id);
        app.connection.generation = 1;
        app.connection.status = ConnectionStatus::Connected;
        app.connection.server = Some(crate::db::ServerInfo {
            kind: crate::profile::DatabaseKind::Sqlite,
            version: "test".into(),
            database: "memory".into(),
        });
        app.explorer
            .nodes
            .push(crate::db::catalog::CatalogNode::new(
                crate::db::catalog::CatalogId::new(
                    first.id,
                    crate::db::catalog::CatalogKind::Table,
                    ["example"],
                ),
                None,
                "example",
                "table",
                None,
                false,
            ));
        app.active_console_mut().transaction_generation = 3;
        app.active_console_mut().transaction_state =
            crate::model::transaction::TransactionState::Active;

        app.update(Action::ConnectionInvalidated {
            connection: ConnectionIdentity {
                profile_id: first.id,
                generation: 1,
            },
            message: "Reconnect before running more queries".into(),
        });

        assert_eq!(app.connection.active_identity(), None);
        assert_eq!(app.connection.status, ConnectionStatus::Failed);
        assert!(app.connection.server.is_none());
        assert_eq!(
            app.connection.error.as_deref(),
            Some("Reconnect before running more queries")
        );
        assert_eq!(
            app.active_console().transaction_state,
            crate::model::transaction::TransactionState::OutcomeUnknown
        );
        assert!(app.explorer.nodes.is_empty());
    }

    #[test]
    fn invalidated_active_connection_cancels_pending_switch() {
        let first = import_connection_url("sqlite::memory:", Some("first"))
            .unwrap()
            .profile;
        let second = import_connection_url("sqlite::memory:", Some("second"))
            .unwrap()
            .profile;
        let mut app = App::new(vec![first.clone(), second.clone()]);
        app.connection.profile_id = Some(first.id);
        app.connection.generation = 4;
        app.connection.pending_profile_id = Some(second.id);
        app.connection.pending_generation = Some(5);
        app.connection.status = ConnectionStatus::Connecting;

        app.update(Action::ConnectionInvalidated {
            connection: ConnectionIdentity {
                profile_id: first.id,
                generation: 4,
            },
            message: "Reconnect before running more queries".into(),
        });

        assert_eq!(app.connection.active_identity(), None);
        assert_eq!(app.connection.pending_profile_id, None);
        assert_eq!(app.connection.pending_generation, None);
        assert_eq!(app.connection.status, ConnectionStatus::Failed);
        assert_eq!(
            app.connection.error.as_deref(),
            Some("Reconnect before running more queries")
        );
        assert_eq!(
            app.explorer.normalized.profiles[&first.id].status,
            ExplorerConnectionStatus::Failed
        );
        assert_eq!(
            app.explorer.normalized.profiles[&second.id].status,
            ExplorerConnectionStatus::Failed
        );
    }

    #[test]
    fn unknown_transaction_outcome_can_exit_without_retrying_database_command() {
        let profile = import_connection_url("sqlite::memory:", Some("offline"))
            .unwrap()
            .profile;
        let mut app = App::new(vec![profile.clone()]);
        let connection = ConnectionIdentity {
            profile_id: profile.id,
            generation: 1,
        };
        app.connection.profile_id = Some(connection.profile_id);
        app.connection.generation = connection.generation;
        app.connection.status = ConnectionStatus::Connected;
        app.connection.target = app.active_console().execution_target.clone();
        app.active_console_mut().transaction_mode = TransactionMode::Manual;
        app.active_console_mut().transaction_state = TransactionState::Active;

        app.update(Action::ConnectionInvalidated {
            connection,
            message: "connection lost".into(),
        });
        assert_eq!(
            app.active_console().transaction_state,
            TransactionState::OutcomeUnknown
        );

        assert!(app.update(Action::Quit).is_empty());
        assert!(matches!(
            app.overlay,
            Some(Overlay::TransactionExitConfirm {
                choice: TransactionExitChoice::Abandon,
                ..
            })
        ));
        assert!(matches!(
            app.update(Action::ConfirmTransactionExit).as_slice(),
            [Command::Quit]
        ));
        assert!(app.should_quit);
        assert_eq!(
            app.active_console().transaction_state,
            TransactionState::Idle
        );
    }

    #[test]
    fn stale_connection_invalidation_does_not_affect_newer_generation() {
        let profile = import_connection_url("sqlite::memory:", Some("demo"))
            .unwrap()
            .profile;
        let mut app = App::new(vec![profile.clone()]);
        app.connection.profile_id = Some(profile.id);
        app.connection.generation = 8;
        app.connection.status = ConnectionStatus::Connected;
        app.active_console_mut().transaction_generation = 2;
        app.active_console_mut().transaction_state =
            crate::model::transaction::TransactionState::Active;

        app.update(Action::ConnectionInvalidated {
            connection: ConnectionIdentity {
                profile_id: profile.id,
                generation: 7,
            },
            message: "stale quarantine".into(),
        });

        assert_eq!(
            app.connection.active_identity(),
            Some(ConnectionIdentity {
                profile_id: profile.id,
                generation: 8,
            })
        );
        assert_eq!(app.connection.status, ConnectionStatus::Connected);
        assert!(app.connection.error.is_none());
        assert_eq!(
            app.active_console().transaction_state,
            crate::model::transaction::TransactionState::Active
        );
    }
}
