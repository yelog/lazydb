use std::collections::{BTreeSet, HashSet};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use uuid::Uuid;

use crate::{
    action::{Action, Command},
    cli::ConfirmationPolicy,
    db::{
        ErrorCategory,
        catalog::{
            CatalogPage, CatalogRequest, CatalogRequestKey, CatalogTarget, MAX_CATALOG_PAGE_SIZE,
        },
    },
    editor::{EditorEffect, EditorError, EditorWorkspace},
    model::{
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
            RelationDescriptor, RelationKey, RelationLoad, RelationQueryInput, RelationRequest,
            RelationRequestKind, RelationSnapshot, RelationTab, RelationView,
            automatic_relation_column_widths,
        },
        tab::{
            CompletionPopup, ConsoleTab, ExecutionResult, LastExecution, OutputEntry, OutputKind,
            ResultView, WorkspaceTab,
        },
        transaction::{
            self, DeferredIntent, DeferredIntentQueue, DeferredTransactionPrompt, TransactionEvent,
            TransactionExitChoice, TransactionMode, TransactionState,
        },
        workspace::{
            ConnectionIdentity, ConnectionState, ConnectionStatus, ExecutionConfirmFocus,
            ExplorerState, Focus, ManualCancelFocus, Overlay, QueryStatus,
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

pub struct App {
    pub profiles: Vec<ConnectionProfile>,
    pub connection: ConnectionState,
    pub explorer: ExplorerState,
    pub tabs: Vec<WorkspaceTab>,
    pub active_tab: usize,
    pub focus: Focus,
    pub overlay: Option<Overlay>,
    pub profile_manager: Option<ProfileManagerState>,
    pub system_credential_availability: crate::persistence::secrets::SecretStoreAvailability,
    pub should_quit: bool,
    next_console_number: usize,
    connection_request_generation: u64,
    connection_terminal_generation: u64,
    editor: EditorWorkspace,
    confirmation_policy: ConfirmationPolicy,
    deferred: DeferredIntentQueue,
    resolving_deferred: Option<DeferredTransactionPrompt>,
    pending_target_console: Option<Uuid>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CatalogRequestIntent {
    Automatic,
    Continuation,
    Explicit,
    Refresh,
    Completion,
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
        let mut tab = ConsoleTab::new("console");
        tab.execution_target = profiles.first().map(ExecutionTarget::from_profile);
        let tab_id = tab.id;
        let mut editor = EditorWorkspace::new();
        editor.open_console(tab_id, "");
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
        Self {
            profiles,
            connection: ConnectionState::default(),
            explorer,
            tabs: vec![WorkspaceTab::Sql(tab)],
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
            editor,
            confirmation_policy,
            deferred: DeferredIntentQueue::default(),
            resolving_deferred: None,
            pending_target_console: None,
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
            .tabs
            .iter()
            .filter_map(WorkspaceTab::as_console)
            .map(|tab| PersistedConsole {
                id: tab.id,
                name: tab.name.clone(),
                sql_file: format!("{}.sql", tab.id).into(),
                target: tab.execution_target.clone(),
                transaction_mode: tab.transaction_mode,
            })
            .collect::<Vec<_>>();
        let sql = consoles
            .iter()
            .map(|console| (console.id, self.editor_text(console.id).unwrap_or_default()))
            .collect();
        let active_console = self
            .active_console_opt()
            .map(|tab| tab.id)
            .or_else(|| consoles.first().map(|console| console.id))
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
        self.editor = EditorWorkspace::new();
        for persisted in snapshot.consoles {
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
            self.tabs.push(WorkspaceTab::Sql(tab));
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
            Id::ExplorerMoveDown => vec![Action::ExplorerMove(1)],
            Id::ExplorerMoveUp => vec![Action::ExplorerMove(-1)],
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
                view: RelationView::Structure,
            }],
            Id::EditorInsert => vec![editor_key(KeyCode::Char('i'))],
            Id::EditorNormal => vec![editor_key(KeyCode::Esc)],
            Id::EditorUndo => vec![editor_key(KeyCode::Char('u'))],
            Id::EditorRedo => vec![editor_control_key(KeyCode::Char('r'))],
            Id::EditorRun => vec![Action::RunActiveSql],
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
                && matches!(action, Action::GridMove { .. } | Action::GridSelect { .. }))
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
                    | Action::GridSelect { .. }
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
            )
        {
            return Vec::new();
        }
        match action {
            Action::NewConsole => {
                let name = format!("console_{}", self.next_console_number);
                self.next_console_number += 1;
                let mut tab = ConsoleTab::new(name);
                tab.execution_target = self.active_profile().map(ExecutionTarget::from_profile);
                let id = tab.id;
                self.tabs.push(WorkspaceTab::Sql(tab));
                self.editor.open_console(id, "");
                self.active_tab = self.tabs.len() - 1;
                self.focus = Focus::Editor;
                vec![self.persist_workspace_command()]
            }
            Action::CloseActiveTab => {
                if self.tabs.len() > 1 {
                    let was_console = self.tabs[self.active_tab].as_console().is_some();
                    let id = self.tabs[self.active_tab].id();
                    if self.active_console_opt().is_some() && self.transaction_needs_exit(id) {
                        return self.defer_intent(DeferredIntent::CloseConsole, [id]);
                    }
                    let is_final_console = was_console
                        && self
                            .tabs
                            .iter()
                            .filter(|tab| tab.as_console().is_some())
                            .count()
                            == 1;
                    let cancel = match self.tabs.get_mut(self.active_tab) {
                        Some(WorkspaceTab::Relation(tab)) => {
                            let requests = [
                                pending_relation_request(&tab.data),
                                pending_relation_request(&tab.structure),
                            ];
                            tab.data = cancel_relation_load(&tab.data);
                            tab.structure = cancel_relation_load(&tab.structure);
                            requests
                                .into_iter()
                                .flatten()
                                .map(Command::CancelRelationRequest)
                                .collect()
                        }
                        _ => Vec::new(),
                    };
                    self.tabs.remove(self.active_tab);
                    if was_console {
                        self.editor.close_console(id);
                    }
                    if is_final_console {
                        let mut tab =
                            ConsoleTab::new(format!("console_{}", self.next_console_number));
                        tab.execution_target =
                            self.active_profile().map(ExecutionTarget::from_profile);
                        self.next_console_number += 1;
                        let id = tab.id;
                        self.tabs.push(WorkspaceTab::Sql(tab));
                        self.editor.open_console(id, "");
                    }
                    self.active_tab = self.active_tab.saturating_sub(1);
                    self.normalize_focus();
                    let mut commands = cancel;
                    commands.push(self.persist_workspace_command());
                    commands
                } else {
                    Vec::new()
                }
            }
            Action::NextTab => {
                self.active_tab = (self.active_tab + 1) % self.tabs.len();
                self.normalize_focus();
                Vec::new()
            }
            Action::PreviousTab => {
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
                Vec::new()
            }
            Action::ShowHelp => {
                self.overlay = Some(Overlay::Help(crate::help::HelpState::new(self.focus)));
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
                self.apply_editor_effects()
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
                self.apply_editor_effects()
            }
            Action::EditorPaste(text) => {
                let Some(id) = self.active_console_opt().map(|tab| tab.id) else {
                    return Vec::new();
                };
                self.active_console_opt_mut().unwrap().completion = None;
                if self.editor.paste(id, &text).is_err() {
                    return Vec::new();
                }
                self.apply_editor_effects()
            }
            Action::EditorViewportChanged(viewport) => {
                let Some(id) = self.active_console_opt().map(|tab| tab.id) else {
                    return Vec::new();
                };
                let _ = self.editor.set_viewport(id, viewport);
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
                self.resolve_transaction_exit(TransactionExitChoice::Commit)
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
                    RelationView::Structure => cancel_pending_relation(&mut tab.structure),
                };
                request
                    .map(Command::CancelRelationRequest)
                    .into_iter()
                    .collect()
            }
            Action::FocusRelationQueryInput(input) => {
                if let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab)
                    && tab.view == RelationView::Data
                {
                    tab.query.focus = Some(input);
                    tab.query.error = None;
                }
                Vec::new()
            }
            Action::RelationQueryInsert(character) => {
                self.with_active_relation_query(|input| input.insert(character));
                Vec::new()
            }
            Action::RelationQueryBackspace => {
                self.with_active_relation_query(|input| input.backspace());
                Vec::new()
            }
            Action::RelationQueryDelete => {
                self.with_active_relation_query(|input| input.delete());
                Vec::new()
            }
            Action::RelationQueryMoveLeft => {
                self.with_active_relation_query(|input| input.move_left());
                Vec::new()
            }
            Action::RelationQueryMoveRight => {
                self.with_active_relation_query(|input| input.move_right());
                Vec::new()
            }
            Action::RelationQueryMoveHome => {
                self.with_active_relation_query(|input| input.move_home());
                Vec::new()
            }
            Action::RelationQueryMoveEnd => {
                self.with_active_relation_query(|input| input.move_end());
                Vec::new()
            }
            Action::RelationQueryClear => {
                self.with_active_relation_query(|input| input.set(""));
                Vec::new()
            }
            Action::CancelRelationQueryInput => {
                if let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab) {
                    tab.query.focus = None;
                    tab.query.error = None;
                    tab.query
                        .where_input
                        .set(tab.query.submitted.where_clause.clone().unwrap_or_default());
                    tab.query.order_by_input.set(
                        tab.query
                            .submitted
                            .order_by_clause
                            .clone()
                            .unwrap_or_default(),
                    );
                }
                Vec::new()
            }
            Action::SubmitRelationQuery => self.submit_relation_query(),
            Action::ResizeRelationColumn(delta) => {
                self.resize_relation_column(delta);
                Vec::new()
            }
            Action::ResetRelationColumnWidth => {
                self.reset_relation_column_width();
                Vec::new()
            }
            Action::StartRelationColumnResize { column, width } => {
                self.set_relation_column_width(column, width);
                Vec::new()
            }
            Action::SetRelationColumnWidth { column, width } => {
                self.set_relation_column_width(column, width);
                Vec::new()
            }
            Action::EndRelationColumnResize => Vec::new(),
            Action::PreviewSelected => self.open_selected_relation(RelationView::Data),
            Action::DdlSelected => self.ddl_selected(),
            Action::RelationSucceeded { request, snapshot } => {
                self.accept_relation(request, Ok(*snapshot))
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
                self.clear_active_catalog(connection.profile_id);
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
            Action::CatalogPageLoaded(page) => self.accept_catalog_page(page),
            Action::CatalogPageFailed {
                key,
                category,
                message,
            } => {
                self.fail_catalog_page(&key, category, message);
                Vec::new()
            }
            Action::QueryFinished {
                tab_id,
                generation,
                connection,
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
                if tab.generation != generation
                    || self.connection.active_identity() != Some(connection)
                {
                    return Vec::new();
                }
                let total_ms = outcome.stats.total().as_millis();
                let rows = outcome.stats.row_count;
                tab.query_status = QueryStatus::Idle;
                tab.output.push(OutputEntry {
                    kind: OutputKind::Success,
                    message: format!("{rows} row(s) retrieved in {total_ms} ms"),
                });
                tab.outcome = Some(outcome);
                tab.result_view = ResultView::Data;
                if let Some(last) = tab.last_execution.as_mut()
                    && last.draft.query_generation + 1 == generation
                {
                    last.result = ExecutionResult::Succeeded;
                }
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
                tab.output.push(OutputEntry {
                    kind: OutputKind::Error,
                    message,
                });
                tab.result_view = ResultView::Output;
                if let Some(last) = tab.last_execution.as_mut()
                    && last.draft.query_generation + 1 == generation
                {
                    last.result = ExecutionResult::Failed;
                }
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
                    tab.output.push(OutputEntry {
                        kind: OutputKind::Error,
                        message,
                    });
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
            Action::ExplorerSelect(id) => {
                self.explorer.select_id(id);
                self.focus = Focus::Explorer;
                Vec::new()
            }
            Action::GridMove { rows, columns } => {
                if self.is_active_relation_tab() {
                    let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab)
                    else {
                        return Vec::new();
                    };
                    let (row_count, column_count) = relation_grid_dimensions(&tab.data);
                    tab.grid.selected_row = move_bounded(tab.grid.selected_row, rows, row_count);
                    tab.grid.selected_column =
                        move_bounded(tab.grid.selected_column, columns, column_count);
                } else {
                    let tab = self.active_console_mut();
                    let (row_count, column_count) = tab
                        .outcome
                        .as_ref()
                        .and_then(|outcome| outcome.result_sets.last())
                        .map(|result| (result.rows.len(), result.columns.len()))
                        .unwrap_or((0, 0));
                    tab.selected_row = move_bounded(tab.selected_row, rows, row_count);
                    tab.selected_column = move_bounded(tab.selected_column, columns, column_count);
                }
                Vec::new()
            }
            Action::GridSelect { row, column } => {
                if self.is_active_relation_tab() {
                    let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab)
                    else {
                        return Vec::new();
                    };
                    let (row_count, column_count) = relation_grid_dimensions(&tab.data);
                    tab.grid.selected_row = row.min(row_count.saturating_sub(1));
                    tab.grid.selected_column = column.min(column_count.saturating_sub(1));
                } else {
                    let tab = self.active_console_mut();
                    let (row_count, column_count) = tab
                        .outcome
                        .as_ref()
                        .and_then(|outcome| outcome.result_sets.last())
                        .map(|result| (result.rows.len(), result.columns.len()))
                        .unwrap_or((0, 0));
                    tab.selected_row = row.min(row_count.saturating_sub(1));
                    tab.selected_column = column.min(column_count.saturating_sub(1));
                }
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
        self.overlay = Some(Overlay::TransactionExitConfirm {
            prompt,
            choice: TransactionExitChoice::Rollback,
        });
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
            DeferredIntent::CloseConsole => {
                if self.tabs.len() > 1 {
                    let id = self.active_console().id;
                    if let Some(index) = self.tabs.iter().position(|tab| tab.id() == id) {
                        self.tabs.remove(index);
                        self.editor.close_console(id);
                        self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
                    }
                }
                vec![self.persist_workspace_command()]
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
                cancel_pending_relation(&mut tab.structure),
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
                cancel_pending_relation(&mut tab.structure),
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
            tab.as_console().is_some_and(|tab| tab.query_status == QueryStatus::Running)
                || matches!(tab, WorkspaceTab::Relation(relation) if match relation.view {
                    RelationView::Data => matches!(relation.data, RelationLoad::Loading { .. }),
                    RelationView::Structure => matches!(relation.structure, RelationLoad::Loading { .. }),
                })
        })
    }

    fn apply_editor_effects(&mut self) -> Vec<Command> {
        let effects = self.editor.drain_effects();
        let mut commands = Vec::new();
        for effect in effects {
            let action = match effect {
                EditorEffect::Changed { .. } => {
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
                | EditorEffect::ToggleTransaction
                | EditorEffect::ClearTransactionOutcome
                | EditorEffect::SetConnectionTarget(_)
                | EditorEffect::SetDatabaseTarget(_)
                | EditorEffect::SetSchemaTarget(_) => continue,
                EditorEffect::OpenTargetSelector => Action::OpenTargetSelector,
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
        let tab = self.active_console_opt()?;
        Some(CompletionScheduleKey {
            console_id: tab.id,
            document_revision: self.active_editor_revision(),
            connection: self.connection.active_identity()?,
            catalog_generation: self.explorer.catalog_generation,
        })
    }

    fn complete_now(&mut self) -> Vec<Command> {
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
        let default_schema = self
            .active_console_opt()
            .and_then(|tab| tab.execution_target.as_ref())
            .and_then(|target| target.schema.clone())
            .or_else(|| {
                self.active_profile()
                    .and_then(|profile| profile.default_schema.clone())
            });
        let candidates = sql::complete(
            &text,
            cursor,
            self.sql_dialect(),
            &self.explorer.completion_index,
            default_schema.as_deref(),
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
        self.apply_editor_effects()
    }

    fn sql_dialect(&self) -> SqlDialect {
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
            let sql = draft.sql.clone();
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
        tab.output.push(OutputEntry {
            kind: OutputKind::Info,
            message: "Executing SQL".to_owned(),
        });
        tab.last_execution = Some(LastExecution {
            draft: draft.clone(),
            result: ExecutionResult::Dispatched,
        });
        vec![Command::RunQuery {
            connection: draft.connection,
            target: draft.target,
            tab_id: draft.console_id,
            generation,
            sql: draft.sql,
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
                    .filter(|summary| completion_group(summary.group))
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
            return self.load_active_relation(false);
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
        self.load_active_relation(true)
    }

    fn ddl_selected(&mut self) -> Vec<Command> {
        self.open_selected_relation(RelationView::Structure)
    }

    fn with_active_relation_query<F>(&mut self, edit: F)
    where
        F: FnOnce(&mut crate::model::text_input::TextInput),
    {
        if let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab)
            && tab.view == RelationView::Data
            && let Some(input) = tab.query.focus
        {
            match input {
                RelationQueryInput::Where => edit(&mut tab.query.where_input),
                RelationQueryInput::OrderBy => edit(&mut tab.query.order_by_input),
            }
            tab.query.error = None;
        }
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
            RelationView::Structure => return None,
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

    fn resize_relation_column(&mut self, delta: i16) {
        let widths = self
            .relation_result()
            .map(|result| automatic_relation_column_widths(&result));
        let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        if tab.view != RelationView::Data {
            return;
        }
        let Some(base) = widths else { return };
        let column = tab.grid.selected_column;
        if column >= base.len() {
            return;
        }
        if tab.column_widths.len() < base.len() {
            tab.column_widths.resize(base.len(), None);
        }
        let current = tab.column_widths[column].unwrap_or(base[column]);
        tab.column_widths[column] = Some((current as i16 + delta).clamp(6, 80) as u16);
    }

    fn reset_relation_column_width(&mut self) {
        if let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab)
            && tab.view == RelationView::Data
            && tab.grid.selected_column < tab.column_widths.len()
        {
            tab.column_widths[tab.grid.selected_column] = None;
        }
    }

    fn set_relation_column_width(&mut self, column: usize, width: u16) {
        let Some(WorkspaceTab::Relation(tab)) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        if tab.view != RelationView::Data {
            return;
        }
        if tab.column_widths.len() <= column {
            tab.column_widths.resize(column + 1, None);
        }
        tab.column_widths[column] = Some(width.clamp(6, 80));
    }

    fn load_active_relation(&mut self, refresh: bool) -> Vec<Command> {
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
            RelationView::Structure => RelationRequestKind::Structure,
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
            RelationView::Structure => {
                refresh
                    || matches!(
                        tab.structure,
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
            RelationView::Structure => pending_relation_request(&tab.structure),
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
            RelationRequestKind::Structure => {
                let previous = match std::mem::replace(&mut tab.structure, RelationLoad::Empty) {
                    RelationLoad::Ready(snapshot) => Some(snapshot),
                    RelationLoad::Loading { previous, .. }
                    | RelationLoad::Failed { previous, .. }
                    | RelationLoad::Cancelled { previous } => previous,
                    RelationLoad::Empty => None,
                };
                tab.structure = RelationLoad::Loading {
                    request: request.clone(),
                    previous,
                };
                previous_request
                    .map(Command::CancelRelationRequest)
                    .into_iter()
                    .chain([Command::LoadRelationStructure(request)])
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
                    tab.data = RelationLoad::Ready(crate::model::relation::OwnedSnapshot {
                        value: snapshot,
                        attribution: crate::model::relation::SnapshotAttribution {
                            connection: request.connection,
                            profile_id: request.connection.profile_id,
                            scope: request.scope.clone(),
                        },
                    });
                }
            }
            (RelationRequestKind::Structure, Ok(RelationSnapshot::Structure(snapshot))) => {
                if matches!(&tab.structure, RelationLoad::Loading { request: pending, .. } if pending == &request)
                {
                    tab.structure = RelationLoad::Ready(crate::model::relation::OwnedSnapshot {
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
            (RelationRequestKind::Structure, Err(message)) => {
                if let RelationLoad::Loading {
                    previous,
                    request: pending,
                } = &tab.structure
                    && pending == &request
                {
                    tab.structure = RelationLoad::Failed {
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
                RelationRequestKind::Structure => matches!(
                    &tab.structure,
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
        tab.query_status = QueryStatus::Idle;
        tab.output.push(OutputEntry {
            kind: OutputKind::Success,
            message: format!("{rows} row(s) retrieved in {total_ms} ms"),
        });
        tab.outcome = Some(outcome);
        tab.result_view = ResultView::Data;
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

fn completion_group(group: crate::db::catalog::ObjectGroup) -> bool {
    matches!(
        group,
        crate::db::catalog::ObjectGroup::Tables
            | crate::db::catalog::ObjectGroup::Views
            | crate::db::catalog::ObjectGroup::MaterializedViews
            | crate::db::catalog::ObjectGroup::Functions
            | crate::db::catalog::ObjectGroup::Procedures
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

    use super::App;
    use crate::{
        action::{Action, Command},
        db::query::{QueryOutcome, QueryStats, ResultSet},
        model::explorer::ExplorerConnectionStatus,
        model::relation::RelationTab,
        model::tab::WorkspaceTab,
        model::workspace::{ConnectionIdentity, ConnectionStatus, Focus, Overlay, QueryStatus},
        profile::import_connection_url,
    };

    fn empty_outcome() -> QueryOutcome {
        QueryOutcome {
            result_sets: vec![ResultSet::default()],
            stats: QueryStats::new(Duration::from_millis(2), Duration::from_millis(3), 0),
        }
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
